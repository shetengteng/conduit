"""End-to-end tests for the local SOCKS5 proxy.

We simulate three scenarios on pure loopback fixtures so the test suite
stays hermetic:

1. **Direct path** — target resolves to a local echo server, resolver
   says ``direct``, bytes round-trip through the SOCKS5 frontend.
2. **Proxy path** — resolver says ``proxy``; a fake HTTP-CONNECT server
   plays the role of Conduit Server and tunnels into the same echo.
3. **Self-heal** — resolver hands back ``direct`` for a *closed* port,
   the proxy fails the connect, then flips the cache to ``proxy`` and
   retries through the fake HTTP-CONNECT server.

Cross-references:
* design/2026-04-30-3-Conduit-Client-客户端可行性报告.md §3.5 (self-heal)
"""
from __future__ import annotations

import asyncio
import socket
import struct
from datetime import timedelta

import pytest

from local_proxy import LocalProxyServer, ServerEndpoint
from route_cache import RouteCache, RouteEntry, _utcnow
from route_resolver import RouteResolver

PROBE = b"hello-conduit\n"


# ---------------------------------------------------------------------------
# fixtures
# ---------------------------------------------------------------------------


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@pytest.fixture
async def echo_server():
    async def _h(reader, writer):
        try:
            while True:
                chunk = await reader.read(4096)
                if not chunk:
                    break
                writer.write(chunk)
                await writer.drain()
        finally:
            try:
                writer.close()
                await writer.wait_closed()
            except Exception:
                pass

    srv = await asyncio.start_server(_h, "127.0.0.1", 0)
    host, port = srv.sockets[0].getsockname()[:2]
    try:
        yield host, port
    finally:
        srv.close()
        await srv.wait_closed()


@pytest.fixture
async def fake_http_connect_server(echo_server):
    """Pretend to be the Conduit Server's HTTP-CONNECT proxy.

    Whatever target the SOCKS5 frontend asks us to tunnel, we always
    splice the connection into the same echo target so the test can
    verify end-to-end relay.
    """
    target_host, target_port = echo_server

    async def _h(client_reader, client_writer):
        try:
            request_line = await asyncio.wait_for(client_reader.readline(), timeout=2)
            method = request_line.decode().split(" ", 1)[0].upper()
            assert method == "CONNECT", f"unexpected request: {request_line!r}"
            while True:
                line = await asyncio.wait_for(client_reader.readline(), timeout=2)
                if line in (b"\r\n", b"\n", b""):
                    break

            client_writer.write(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            await client_writer.drain()

            t_reader, t_writer = await asyncio.open_connection(target_host, target_port)

            async def _half(r, w):
                try:
                    while True:
                        chunk = await r.read(4096)
                        if not chunk:
                            break
                        w.write(chunk)
                        await w.drain()
                except Exception:
                    pass
                finally:
                    try:
                        w.close()
                    except Exception:
                        pass

            await asyncio.gather(
                _half(client_reader, t_writer),
                _half(t_reader, client_writer),
                return_exceptions=True,
            )
        except Exception:
            try:
                client_writer.close()
            except Exception:
                pass

    srv = await asyncio.start_server(_h, "127.0.0.1", 0)
    host, port = srv.sockets[0].getsockname()[:2]
    try:
        yield ServerEndpoint(host=host, port=port)
    finally:
        srv.close()
        await srv.wait_closed()


@pytest.fixture
async def local_proxy_factory(fake_http_connect_server):
    """Build a started ``LocalProxyServer`` wired to a fresh cache/resolver."""
    started: list[LocalProxyServer] = []

    async def _factory():
        cache = RouteCache()
        resolver = RouteResolver(cache, probe_timeout=0.4)
        proxy = LocalProxyServer(
            resolver,
            bind_host="127.0.0.1",
            bind_port=0,
            server_endpoint=fake_http_connect_server,
            handshake_timeout=2.0,
            direct_connect_timeout=1.5,
            proxy_connect_timeout=2.0,
        )
        await proxy.start()
        started.append(proxy)
        return proxy, cache, resolver

    yield _factory

    for proxy in started:
        await proxy.stop()


# ---------------------------------------------------------------------------
# minimal SOCKS5 client
# ---------------------------------------------------------------------------


async def _socks5_connect_via_proxy(
    proxy_host: str,
    proxy_port: int,
    target_host: str,
    target_port: int,
) -> tuple[asyncio.StreamReader, asyncio.StreamWriter]:
    reader, writer = await asyncio.open_connection(proxy_host, proxy_port)

    writer.write(b"\x05\x01\x00")
    await writer.drain()
    auth_resp = await reader.readexactly(2)
    assert auth_resp == b"\x05\x00", f"unexpected auth resp {auth_resp!r}"

    host_bytes = target_host.encode("ascii")
    req = (
        b"\x05\x01\x00"
        + b"\x03"
        + bytes([len(host_bytes)]) + host_bytes
        + struct.pack(">H", target_port)
    )
    writer.write(req)
    await writer.drain()

    head = await reader.readexactly(4)
    assert head[1] == 0x00, f"socks reply rep={head[1]:#x}"
    atyp = head[3]
    if atyp == 0x01:
        await reader.readexactly(4)
    elif atyp == 0x04:
        await reader.readexactly(16)
    elif atyp == 0x03:
        ln = (await reader.readexactly(1))[0]
        await reader.readexactly(ln)
    await reader.readexactly(2)
    return reader, writer


async def _send_recv(reader, writer, payload: bytes, *, expected_len: int) -> bytes:
    writer.write(payload)
    await writer.drain()
    out = b""
    while len(out) < expected_len:
        chunk = await asyncio.wait_for(reader.read(expected_len - len(out)), timeout=2)
        if not chunk:
            break
        out += chunk
    return out


# ---------------------------------------------------------------------------
# tests
# ---------------------------------------------------------------------------


async def test_listener_starts_and_stops_cleanly(local_proxy_factory):
    proxy, _cache, _resolver = await local_proxy_factory()
    assert proxy.is_running
    assert proxy.actual_port > 0
    await proxy.stop()
    assert not proxy.is_running


async def test_direct_path_relays_bytes(local_proxy_factory, echo_server):
    proxy, cache, _resolver = await local_proxy_factory()
    target_host, target_port = echo_server

    cache.set(
        "localhost",
        RouteEntry(
            host="localhost",
            direction="direct",
            expires_at=_utcnow() + timedelta(minutes=5),
            source="manual",
        ),
    )

    proxy_host, proxy_port = "127.0.0.1", proxy.actual_port
    target_host_for_socks = "localhost" if target_host == "127.0.0.1" else target_host
    reader, writer = await _socks5_connect_via_proxy(
        proxy_host, proxy_port, target_host_for_socks, target_port,
    )
    try:
        out = await _send_recv(reader, writer, PROBE, expected_len=len(PROBE))
        assert out == PROBE
    finally:
        writer.close()
        try:
            await writer.wait_closed()
        except Exception:
            pass

    assert proxy.stats["direct"] >= 1
    assert proxy.stats["proxy"] == 0


async def test_proxy_path_via_fake_server(local_proxy_factory, echo_server):
    proxy, cache, _resolver = await local_proxy_factory()
    target_host, target_port = echo_server

    cache.set(
        "git.zoom.us",
        RouteEntry(
            host="git.zoom.us",
            direction="proxy",
            expires_at=_utcnow() + timedelta(minutes=5),
            source="manual",
        ),
    )

    reader, writer = await _socks5_connect_via_proxy(
        "127.0.0.1", proxy.actual_port,
        "git.zoom.us", target_port,
    )
    try:
        out = await _send_recv(reader, writer, PROBE, expected_len=len(PROBE))
        assert out == PROBE
    finally:
        writer.close()
        try:
            await writer.wait_closed()
        except Exception:
            pass

    assert proxy.stats["proxy"] >= 1
    assert proxy.stats["direct"] == 0


async def test_self_heal_when_direct_fails(local_proxy_factory):
    proxy, cache, _resolver = await local_proxy_factory()

    closed_port = _free_port()

    cache.set(
        "totally-not-real.invalid",
        RouteEntry(
            host="totally-not-real.invalid",
            direction="direct",
            expires_at=_utcnow() + timedelta(minutes=5),
            source="manual",
        ),
    )

    reader, writer = await _socks5_connect_via_proxy(
        "127.0.0.1", proxy.actual_port,
        "totally-not-real.invalid", closed_port,
    )
    try:
        out = await _send_recv(reader, writer, PROBE, expected_len=len(PROBE))
        assert out == PROBE
    finally:
        writer.close()
        try:
            await writer.wait_closed()
        except Exception:
            pass

    healed = cache.get("totally-not-real.invalid")
    assert healed is not None
    assert healed.direction == "proxy"
    assert proxy.stats["self_healed"] >= 1
    assert proxy.stats["proxy"] >= 1
