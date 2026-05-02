"""Smoke tests for ``client_main`` ClientRuntime + CLI parsing.

We do *not* exercise ``MacSystemProxy`` here (covered by
``test_system_proxy.py``).  ``ClientRuntime`` is started without a system
proxy so the test is hermetic and runs on Linux CI too.
"""
from __future__ import annotations

import asyncio
import socket

import pytest

from client_main import (
    ClientConfig,
    ClientRuntime,
    _fetch_pac_once,
    _prefill_cache_from_pac,
    parse_args,
)
from local_proxy import ServerEndpoint
from route_cache import RouteCache


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


# ---------------------------------------------------------------------------
# parse_args
# ---------------------------------------------------------------------------


def test_parse_args_defaults():
    cfg = parse_args([])
    assert cfg.bind_host == "127.0.0.1"
    assert cfg.bind_port == 7890
    assert cfg.server_host is None
    assert cfg.server_port == 8080
    assert cfg.pac_path == "/proxy.pac"
    assert cfg.enable_system_proxy is True
    assert cfg.watchdog_ppid is None


def test_parse_args_overrides():
    cfg = parse_args([
        "--bind-port", "17890",
        "--api-port", "18091",
        "--server-host", "192.168.1.5",
        "--server-port", "9999",
        "--no-system-proxy",
        "--watchdog-ppid", "12345",
        "--log-level", "DEBUG",
    ])
    assert cfg.bind_port == 17890
    assert cfg.api_port == 18091
    assert cfg.server_host == "192.168.1.5"
    assert cfg.server_port == 9999
    assert cfg.enable_system_proxy is False
    assert cfg.watchdog_ppid == 12345
    assert cfg.log_level == "DEBUG"


def test_parse_args_default_api_port_is_8091():
    cfg = parse_args([])
    assert cfg.api_port == 8091


# ---------------------------------------------------------------------------
# pac prefill
# ---------------------------------------------------------------------------


def test_prefill_cache_from_pac_inserts_proxy_patterns_only():
    pac_text = """
    function FindProxyForURL(url, host) {
        var PROXY = "PROXY x:1";
        if (dnsDomainIs(host, "zoom.us")) { return PROXY; }
        if (dnsDomainIs(host, "baidu.com")) { return "DIRECT"; }
        return "DIRECT";
    }
    """
    cache = RouteCache()
    n = _prefill_cache_from_pac(cache, pac_text)
    assert n == 1
    e = cache.get("*.zoom.us")
    assert e is not None
    assert e.direction == "proxy"
    assert e.source == "pac"


# ---------------------------------------------------------------------------
# fetch_pac
# ---------------------------------------------------------------------------


@pytest.fixture
async def fake_pac_server():
    """Tiny aiohttp-free HTTP/1.1 server that serves a PAC document."""
    pac_body = (
        "function FindProxyForURL(url, host) {\n"
        "    var PROXY = \"PROXY 192.168.1.5:8080\";\n"
        "    if (dnsDomainIs(host, \"zoom.us\")) { return PROXY; }\n"
        "    return \"DIRECT\";\n"
        "}\n"
    )
    body_bytes = pac_body.encode("utf-8")

    async def _h(reader, writer):
        try:
            while True:
                line = await asyncio.wait_for(reader.readline(), timeout=2)
                if line in (b"\r\n", b"\n", b""):
                    break
            writer.write(b"HTTP/1.1 200 OK\r\n")
            writer.write(b"Content-Type: application/x-ns-proxy-autoconfig\r\n")
            writer.write(f"Content-Length: {len(body_bytes)}\r\n".encode())
            writer.write(b"Connection: close\r\n\r\n")
            writer.write(body_bytes)
            await writer.drain()
        except Exception:
            pass
        finally:
            try:
                writer.close()
                await writer.wait_closed()
            except Exception:
                pass

    srv = await asyncio.start_server(_h, "127.0.0.1", 0)
    host, port = srv.sockets[0].getsockname()[:2]
    try:
        yield ServerEndpoint(host=host, port=port)
    finally:
        srv.close()
        await srv.wait_closed()


async def test_fetch_pac_once_returns_body(fake_pac_server):
    text = await _fetch_pac_once(fake_pac_server, "/proxy.pac")
    assert text is not None
    assert "dnsDomainIs(host, \"zoom.us\")" in text


async def test_fetch_pac_once_returns_none_on_unreachable():
    closed = ServerEndpoint(host="127.0.0.1", port=_free_port())
    text = await _fetch_pac_once(closed, "/proxy.pac")
    assert text is None


# ---------------------------------------------------------------------------
# ClientRuntime start/stop
# ---------------------------------------------------------------------------


async def test_runtime_starts_socks_listener_without_endpoint():
    cfg = ClientConfig(
        bind_host="127.0.0.1",
        bind_port=0,
        api_port=_free_port(),
        server_host=None,
        server_port=8080,
        pac_path="/proxy.pac",
        enable_system_proxy=False,
        log_level="WARNING",
        watchdog_ppid=None,
    )
    rt = ClientRuntime(cfg)
    await rt.start()
    try:
        assert rt.proxy.is_running
        assert rt.proxy.actual_port > 0

        r, w = await asyncio.open_connection("127.0.0.1", rt.proxy.actual_port)
        w.write(b"\x05\x01\x00")
        await w.drain()
        auth = await asyncio.wait_for(r.readexactly(2), timeout=2)
        assert auth == b"\x05\x00"
        w.close()
        try:
            await w.wait_closed()
        except Exception:
            pass
    finally:
        await rt.stop()
        assert not rt.proxy.is_running


async def test_runtime_pre_fills_pac_when_endpoint_set(fake_pac_server):
    cfg = ClientConfig(
        bind_host="127.0.0.1",
        bind_port=0,
        api_port=_free_port(),
        server_host=fake_pac_server.host,
        server_port=fake_pac_server.port,
        pac_path="/proxy.pac",
        enable_system_proxy=False,
        log_level="WARNING",
        watchdog_ppid=None,
    )
    rt = ClientRuntime(cfg)
    await rt.start()
    try:
        assert rt.cache.get("*.zoom.us") is not None
    finally:
        await rt.stop()


async def test_runtime_starts_control_api_with_healthz_200():
    """M-α: control API 能起，healthz 返 200，CORS 头到位。"""
    api_port = _free_port()
    cfg = ClientConfig(
        bind_host="127.0.0.1",
        bind_port=0,
        api_port=api_port,
        server_host=None,
        server_port=8080,
        pac_path="/proxy.pac",
        enable_system_proxy=False,
        log_level="WARNING",
        watchdog_ppid=None,
    )
    rt = ClientRuntime(cfg)
    await rt.start()
    try:
        r, w = await asyncio.open_connection("127.0.0.1", api_port)
        w.write(
            b"GET /healthz HTTP/1.1\r\n"
            b"Host: 127.0.0.1\r\n"
            b"Connection: close\r\n\r\n"
        )
        await w.drain()
        head = await asyncio.wait_for(r.readuntil(b"\r\n\r\n"), timeout=2)
        assert b"HTTP/1.1 200" in head
        assert b"Access-Control-Allow-Origin: *" in head
        body = await asyncio.wait_for(r.read(), timeout=2)
        assert b'"ready": true' in body
        w.close()
        try:
            await w.wait_closed()
        except Exception:
            pass
    finally:
        await rt.stop()
