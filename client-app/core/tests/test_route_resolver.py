"""Tests for client-app route_resolver.

The resolver pipeline is documented in
design/2026-04-30-3-Conduit-Client-客户端可行性报告.md §3.4 / §3.10.

We exercise:

* private-IP fast path,
* pattern-based PAC pre-fill,
* probe-driven decisions (with real loopback echo server),
* self-heal flip,
* global ``a_unreachable`` downgrade.
"""
from __future__ import annotations

import asyncio
import socket

import pytest

from route_cache import RouteCache, RouteEntry, _utcnow
from route_resolver import (
    RouteResolver,
    _is_private_ip,
    _pattern_match,
    tcp_probe,
)
from datetime import timedelta


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def _entry(host: str, direction: str, *, source: str = "probe", ttl_sec: int = 300):
    return RouteEntry(
        host=host,
        direction=direction,  # type: ignore[arg-type]
        expires_at=_utcnow() + timedelta(seconds=ttl_sec),
        source=source,  # type: ignore[arg-type]
    )


@pytest.fixture
async def echo_server():
    """Yield (host, port) of a TCP listener accepting any connection."""
    async def _h(reader, writer):
        try:
            writer.close()
            await writer.wait_closed()
        except Exception:
            pass

    srv = await asyncio.start_server(_h, "127.0.0.1", 0)
    sock = srv.sockets[0]
    host, port = sock.getsockname()[:2]
    try:
        yield host, port
    finally:
        srv.close()
        await srv.wait_closed()


def _closed_port() -> int:
    """Return a port that is *almost certainly* not listening."""
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    return port


# ---------------------------------------------------------------------------
# pure helpers
# ---------------------------------------------------------------------------


def test_is_private_ip_matrix():
    assert _is_private_ip("127.0.0.1") is True
    assert _is_private_ip("10.0.0.5") is True
    assert _is_private_ip("172.16.4.7") is True
    assert _is_private_ip("192.168.1.1") is True
    assert _is_private_ip("169.254.1.1") is True   # link-local
    assert _is_private_ip("8.8.8.8") is False
    assert _is_private_ip("baidu.com") is False     # domain → False
    assert _is_private_ip("not-an-ip") is False


def test_pattern_match_supports_wildcard_dot_and_exact():
    assert _pattern_match("*.zoom.us", "foo.zoom.us") is True
    assert _pattern_match("*.zoom.us", "zoom.us") is True       # bare apex
    assert _pattern_match("*.zoom.us", "baidu.com") is False
    assert _pattern_match(".zoom.us", "foo.zoom.us") is True
    assert _pattern_match(".zoom.us", "zoom.us") is True
    assert _pattern_match("zoom.us", "zoom.us") is True
    assert _pattern_match("zoom.us", "foo.zoom.us") is False
    # Case-insensitive: both wildcard and exact forms.
    assert _pattern_match("*.Zoom.US", "FOO.zoom.us") is True
    assert _pattern_match("Zoom.US", "ZOOM.us") is True


# ---------------------------------------------------------------------------
# tcp_probe
# ---------------------------------------------------------------------------


async def test_tcp_probe_returns_true_for_listening_port(echo_server):
    host, port = echo_server
    assert await tcp_probe(host, port, timeout=1.0) is True


async def test_tcp_probe_returns_false_for_closed_port():
    port = _closed_port()
    assert await tcp_probe("127.0.0.1", port, timeout=0.5) is False


async def test_tcp_probe_returns_false_on_unresolvable_host():
    assert await tcp_probe(
        "this-host-should-not-exist.invalid", 80, timeout=0.5
    ) is False


# ---------------------------------------------------------------------------
# resolve()
# ---------------------------------------------------------------------------


async def test_resolve_global_override_forces_direct():
    cache = RouteCache()
    cache.set("git.zoom.us", _entry("git.zoom.us", "proxy"))
    r = RouteResolver(cache)
    r.set_global_mode("a_unreachable")
    d = await r.resolve("git.zoom.us", 443)
    assert d.direction == "direct"
    assert d.source == "global_override"


async def test_resolve_private_ip_fast_path():
    r = RouteResolver(RouteCache())
    d = await r.resolve("192.168.1.5", 22)
    assert d.direction == "direct"
    assert d.source == "private_ip"


async def test_resolve_cache_hit_short_circuits_probe():
    cache = RouteCache()
    cache.set("git.zoom.us", _entry("git.zoom.us", "proxy"))
    r = RouteResolver(cache)
    d = await r.resolve("git.zoom.us", 443)
    assert d.direction == "proxy"
    assert d.source == "cache"


async def test_resolve_pattern_match_writes_exact_host():
    cache = RouteCache()
    cache.set("*.zoom.us", _entry("*.zoom.us", "proxy", source="pac"))
    r = RouteResolver(cache)
    d = await r.resolve("foo.zoom.us", 443)
    assert d.direction == "proxy"
    assert d.source == "pattern"
    cached = cache.get("foo.zoom.us")
    assert cached is not None
    assert cached.direction == "proxy"


async def test_resolve_probe_hit_marks_direct(echo_server):
    """Use 'localhost' so the resolver doesn't take the private-IP fast
    path before reaching the probe branch we want to exercise."""
    cache = RouteCache()
    r = RouteResolver(cache, probe_timeout=1.0)
    _, port = echo_server
    d = await r.resolve("localhost", port)
    assert d.direction == "direct"
    assert d.source == "probe"
    assert cache.get("localhost") is not None


async def test_resolve_probe_miss_marks_proxy():
    cache = RouteCache()
    r = RouteResolver(cache, probe_timeout=0.4)
    port = _closed_port()
    d = await r.resolve("127.0.0.1-not-an-ip.invalid", port)
    assert d.direction == "proxy"
    assert d.source == "probe"


# ---------------------------------------------------------------------------
# self-heal
# ---------------------------------------------------------------------------


def test_mark_direct_failed_flips_cache_to_proxy():
    cache = RouteCache()
    cache.set("baidu.com", _entry("baidu.com", "direct"))
    r = RouteResolver(cache)
    d = r.mark_direct_failed("baidu.com", 80)
    assert d.direction == "proxy"
    assert cache.get("baidu.com").direction == "proxy"


def test_mark_proxy_failed_invalidates_cache():
    cache = RouteCache()
    cache.set("git.zoom.us", _entry("git.zoom.us", "proxy"))
    r = RouteResolver(cache)
    r.mark_proxy_failed("git.zoom.us", 443)
    assert cache.get("git.zoom.us") is None


# ---------------------------------------------------------------------------
# global mode
# ---------------------------------------------------------------------------


def test_set_global_mode_a_unreachable_flushes_proxy_only():
    cache = RouteCache()
    cache.set("git.zoom.us", _entry("git.zoom.us", "proxy"))
    cache.set("baidu.com", _entry("baidu.com", "direct"))
    r = RouteResolver(cache)
    r.set_global_mode("a_unreachable")
    assert cache.get("git.zoom.us") is None
    assert cache.get("baidu.com") is not None
    assert r.global_mode == "a_unreachable"


def test_set_global_mode_idempotent():
    cache = RouteCache()
    cache.set("git.zoom.us", _entry("git.zoom.us", "proxy"))
    r = RouteResolver(cache)
    r.set_global_mode("a_unreachable")
    # calling again should not throw or re-flush
    r.set_global_mode("a_unreachable")
    assert cache.get("git.zoom.us") is None
