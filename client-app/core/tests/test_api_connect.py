"""集成测试:POST /api/connect/{server_id} + /api/disconnect + /api/connection
+ connect_progress SSE 5 步流。

测试策略:
- 不真起 mDNS / system_proxy (system_proxy 需 sudo / 真 networksetup,直接 mock)
- 用本地 TCP listener 假扮 server 的 HTTP 与 SOCKS 端口,以及一个 PAC 服务
- 直接 inject DiscoveredServer 到 discoverer._state.online
"""
from __future__ import annotations

import asyncio
import builtins
import json
import socket
import time
from unittest.mock import patch

import pytest

from client_main import ClientConfig, ClientRuntime
from discoverer import DiscoveredServer


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _make_cfg(api_port: int) -> ClientConfig:
    return ClientConfig(
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


@pytest.fixture(autouse=True)
def _isolated(monkeypatch, tmp_path):
    """禁 zeroconf + 重定向 known-servers.json。"""
    real_import = builtins.__import__

    def _fake_import(name, *a, **kw):
        if name == "zeroconf" or name.startswith("zeroconf."):
            raise ImportError("test: zeroconf disabled")
        return real_import(name, *a, **kw)

    monkeypatch.setattr(builtins, "__import__", _fake_import)
    import discoverer as _d
    monkeypatch.setattr(_d, "_default_storage_path",
                        lambda: tmp_path / "known-servers.json")
    yield


# ---------------------------------------------------------------------------
# 假 server fixture:同时提供 HTTP(PAC) 与 SOCKS 端口
# ---------------------------------------------------------------------------


PAC_BODY = (
    "function FindProxyForURL(url, host) {\n"
    "    var PROXY = \"PROXY 127.0.0.1:9999\";\n"
    "    if (dnsDomainIs(host, \"zoom.us\")) { return PROXY; }\n"
    "    return \"DIRECT\";\n"
    "}\n"
).encode("utf-8")


async def _http_pac_handler(reader, writer):
    try:
        # 读 request line + headers
        while True:
            line = await asyncio.wait_for(reader.readline(), timeout=2.0)
            if line in (b"\r\n", b"\n", b""):
                break
        writer.write(b"HTTP/1.1 200 OK\r\n")
        writer.write(b"Content-Type: application/x-ns-proxy-autoconfig\r\n")
        writer.write(f"Content-Length: {len(PAC_BODY)}\r\n".encode())
        writer.write(b"Connection: close\r\n\r\n")
        writer.write(PAC_BODY)
        await writer.drain()
    except Exception:
        pass
    finally:
        try:
            writer.close()
            await writer.wait_closed()
        except Exception:
            pass


async def _socks_handler(reader, writer):
    """SOCKS5 端口仅用于 probe TCP 三次握手,不需要协议响应。"""
    try:
        writer.close()
        await writer.wait_closed()
    except Exception:
        pass


@pytest.fixture
async def fake_server():
    """同时起 PAC HTTP 和假 SOCKS 端口。"""
    http_srv = await asyncio.start_server(_http_pac_handler, "127.0.0.1", 0)
    socks_srv = await asyncio.start_server(_socks_handler, "127.0.0.1", 0)
    host, http_port = http_srv.sockets[0].getsockname()[:2]
    _, socks_port = socks_srv.sockets[0].getsockname()[:2]
    try:
        yield (host, http_port, socks_port)
    finally:
        http_srv.close()
        socks_srv.close()
        await http_srv.wait_closed()
        await socks_srv.wait_closed()


def _inject_server(rt: ClientRuntime, host: str, http_port: int, socks_port: int) -> DiscoveredServer:
    srv = DiscoveredServer(
        server_id=f"alpha@{host}:{http_port}",
        name="alpha", host=host, port=http_port, socks=socks_port, api=9090,
        vpn=True, version="0.1.0", pac="/proxy.pac",
        source="mdns", last_seen_at=time.time(), healthy=True,
    )
    rt.discoverer._state.online[srv.server_id] = srv
    return srv


# ---------------------------------------------------------------------------
# helpers:HTTP 客户端
# ---------------------------------------------------------------------------


async def _http_request(method: str, host: str, port: int, path: str, *, timeout: float = 5.0) -> tuple[bytes, bytes]:
    r, w = await asyncio.open_connection(host, port)
    try:
        w.write(
            f"{method} {path} HTTP/1.1\r\n"
            f"Host: {host}\r\n"
            f"Content-Length: 0\r\n"
            f"Connection: close\r\n\r\n".encode()
        )
        await w.drain()
        head = await asyncio.wait_for(r.readuntil(b"\r\n\r\n"), timeout=timeout)
        body = await asyncio.wait_for(r.read(), timeout=timeout)
        return head, body
    finally:
        try:
            w.close()
            await w.wait_closed()
        except Exception:
            pass


# ---------------------------------------------------------------------------
# /api/connection initial state
# ---------------------------------------------------------------------------


async def test_connection_initial_state_is_idle():
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    try:
        _, body = await _http_request("GET", "127.0.0.1", api_port, "/api/connection")
        data = json.loads(body)
        assert data["ok"] is True
        assert data["state"] == "idle"
        assert data["server"] is None
        assert data["heartbeat"] is None
    finally:
        await rt.stop()


# ---------------------------------------------------------------------------
# /api/connect 404 / 200 / 后续 /api/connection
# ---------------------------------------------------------------------------


async def test_connect_returns_404_when_server_id_unknown():
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    try:
        head, body = await _http_request("POST", "127.0.0.1", api_port, "/api/connect/ghost@1.1.1.1:80")
        assert b"HTTP/1.1 404" in head
        data = json.loads(body)
        assert data["error"]["code"] == "NOT_FOUND"
    finally:
        await rt.stop()


async def test_connect_full_5_steps_and_state_becomes_connected(fake_server):
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    try:
        host, http_port, socks_port = fake_server
        srv = _inject_server(rt, host, http_port, socks_port)

        head, body = await _http_request("POST", "127.0.0.1", api_port,
                                         f"/api/connect/{srv.server_id}",
                                         timeout=10.0)
        assert b"HTTP/1.1 200" in head, f"got {head!r} body={body!r}"
        data = json.loads(body)
        assert data["ok"] is True
        assert data["state"] == "connected"
        assert data["server"]["server_id"] == srv.server_id
        assert data["heartbeat"] is not None  # heartbeat 已起

        # 校验状态机
        assert rt.connection_state == "connected"
        assert rt.connected_server.server_id == srv.server_id
        assert rt.proxy.server_endpoint is not None
        assert rt.proxy.server_endpoint.host == host
        assert rt.proxy.server_endpoint.port == http_port

        # PAC 预填验证
        assert rt.cache.get("*.zoom.us") is not None

        # 再 GET /api/connection 校准
        _, body2 = await _http_request("GET", "127.0.0.1", api_port, "/api/connection")
        data2 = json.loads(body2)
        assert data2["state"] == "connected"
        assert data2["server"]["name"] == "alpha"
    finally:
        # 必须 disconnect 否则 heartbeat 协程会拖 stop
        await rt.disconnect()
        await rt.stop()


async def test_connect_step1_fails_when_socks_unreachable():
    """server 的 HTTP 通,SOCKS 端口关 -> step1 probe 失败 -> 状态 failed。"""
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    try:
        # 只起 HTTP,不起 SOCKS
        http_srv = await asyncio.start_server(_http_pac_handler, "127.0.0.1", 0)
        try:
            host, http_port = http_srv.sockets[0].getsockname()[:2]
            socks_port = _free_port()  # 关
            srv = _inject_server(rt, host, http_port, socks_port)

            head, body = await _http_request("POST", "127.0.0.1", api_port,
                                             f"/api/connect/{srv.server_id}",
                                             timeout=10.0)
            assert b"HTTP/1.1 502" in head  # CONNECT_FAILED -> 502
            data = json.loads(body)
            assert data["error"]["code"] == "CONNECT_FAILED"
            assert data["step"] == 1
            assert data["step_key"] == "probe"

            # 状态机 -> failed,endpoint 应回滚到 None
            assert rt.connection_state == "failed"
            assert rt.proxy.server_endpoint is None
        finally:
            http_srv.close()
            await http_srv.wait_closed()
    finally:
        await rt.stop()


async def test_disconnect_returns_to_idle(fake_server):
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    try:
        host, http_port, socks_port = fake_server
        srv = _inject_server(rt, host, http_port, socks_port)
        await rt.connect_to(srv)
        assert rt.connection_state == "connected"

        head, body = await _http_request("POST", "127.0.0.1", api_port, "/api/disconnect")
        assert b"HTTP/1.1 200" in head
        data = json.loads(body)
        assert data["ok"] is True
        assert data["state"] == "idle"

        assert rt.connection_state == "idle"
        assert rt.proxy.server_endpoint is None
        assert rt.connected_server is None
        assert rt.heartbeat is None
    finally:
        await rt.stop()


# ---------------------------------------------------------------------------
# SSE: connect_progress 5 步全到位
# ---------------------------------------------------------------------------


async def _read_until_contains(reader, needle: bytes, *, timeout: float = 5.0) -> bytes:
    buf = bytearray()
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise asyncio.TimeoutError(f"timeout, got {bytes(buf)!r}")
        chunk = await asyncio.wait_for(reader.read(2048), timeout=remaining)
        if not chunk:
            raise ConnectionError(f"closed, got {bytes(buf)!r}")
        buf.extend(chunk)
        if needle in buf:
            return bytes(buf)


async def test_sse_emits_all_5_connect_progress_events(fake_server):
    """订阅 EventBus(不走 HTTP SSE,绕开 chunked 编码与 server cleanup hang),
    检查 5 步 connect_progress 全 publish 到位 + connect_done。"""
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    try:
        host, http_port, socks_port = fake_server
        srv = _inject_server(rt, host, http_port, socks_port)

        q = await rt.bus.subscribe()
        try:
            await rt.connect_to(srv)
            assert rt.connection_state == "connected"

            # 收集所有事件,直到 q drain
            collected: list[tuple[str, dict]] = []
            while True:
                try:
                    evt = await asyncio.wait_for(q.get(), timeout=0.3)
                    collected.append((evt.type, evt.payload))
                except asyncio.TimeoutError:
                    break

            progress_keys = [
                p["key"] for (t, p) in collected
                if t == "connect_progress" and p["status"] == "ok"
            ]
            assert progress_keys == [
                "probe", "fetch_pac", "prefill_cache", "switch_endpoint", "start_heartbeat"
            ], f"got progress_keys={progress_keys}, all={collected}"

            assert any(t == "connect_done" for t, _ in collected), \
                f"no connect_done in {collected}"
            states = [p.get("state") for t, p in collected if t == "connection_state_changed"]
            assert "connecting" in states
            assert "connected" in states
        finally:
            await rt.bus.unsubscribe(q)
    finally:
        await rt.disconnect()
        await rt.stop()
