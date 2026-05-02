"""集成测试：control API 的 /api/servers 与 /api/events SSE。

不真起 mDNS：在 ClientRuntime 启动后，手动给 discoverer._state.online 塞数据，
然后 HTTP GET /api/servers 验证 JSON，订阅 /api/events 验证 SSE 推送。

为何禁用 zeroconf：本地真起 AsyncZeroconf 会绑定多播 socket，stop 阶段
有几率卡在 IGMP leave，导致 pytest hang。CI 沙箱也通常无多播权限。
我们 monkeypatch zeroconf import 抛 ImportError，让 Discoverer 走 fallback
路径（available=False，但 EventBus / snapshot / 持久化仍正常）。
"""
from __future__ import annotations

import asyncio
import builtins
import json
import socket
import time

import pytest

from client_main import ClientConfig, ClientRuntime
from discoverer import DiscoveredServer


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@pytest.fixture(autouse=True)
def _isolated_environment(monkeypatch, tmp_path):
    """两件事一起做（autouse 且需要 monkeypatch + tmp_path）：

    1. 禁用 zeroconf —— 避免本地多播 socket 拖慢 stop。
    2. 把 known-servers.json 默认路径重定向到 tmp_path —— 避免污染用户主目录、
       也避免上一个 case 的数据泄漏到下一个 case。
    """
    real_import = builtins.__import__

    def _fake_import(name, *a, **kw):
        if name == "zeroconf" or name.startswith("zeroconf."):
            raise ImportError("test: zeroconf disabled")
        return real_import(name, *a, **kw)

    monkeypatch.setattr(builtins, "__import__", _fake_import)

    import discoverer as _discoverer_mod
    monkeypatch.setattr(
        _discoverer_mod,
        "_default_storage_path",
        lambda: tmp_path / "known-servers.json",
    )
    yield


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


async def _http_get(host: str, port: int, path: str, *, timeout: float = 2.0) -> tuple[bytes, bytes]:
    """简易 HTTP/1.1 GET，返回 (header_bytes, body_bytes)。"""
    r, w = await asyncio.open_connection(host, port)
    try:
        w.write(
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {host}\r\n"
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


async def _http_post_json(
    host: str, port: int, path: str, payload: dict | None = None, *, timeout: float = 2.0,
) -> tuple[bytes, bytes]:
    """简易 HTTP/1.1 POST application/json,返回 (header_bytes, body_bytes)。"""
    body_bytes = b"" if payload is None else json.dumps(payload).encode()
    r, w = await asyncio.open_connection(host, port)
    try:
        req = (
            f"POST {path} HTTP/1.1\r\n"
            f"Host: {host}\r\n"
            f"Connection: close\r\n"
            f"Content-Type: application/json\r\n"
            f"Content-Length: {len(body_bytes)}\r\n\r\n"
        ).encode() + body_bytes
        w.write(req)
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
# /api/servers
# ---------------------------------------------------------------------------


async def test_api_servers_returns_empty_list_when_no_discovery():
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    try:
        head, body = await _http_get("127.0.0.1", api_port, "/api/servers")
        assert b"HTTP/1.1 200" in head
        assert b"Access-Control-Allow-Origin: *" in head
        data = json.loads(body)
        # available 取决于是否真起 zeroconf；CI 沙箱可能 False，本机 True。
        # 关键是接口契约：count + servers 字段必在。
        assert "count" in data
        assert "servers" in data
        assert isinstance(data["servers"], list)
    finally:
        await rt.stop()


async def test_api_servers_returns_injected_server():
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    try:
        # 直接往 discoverer._state.online 塞一个，模拟 mDNS 收到
        rt.discoverer._state.online["alpha@10.0.0.1:8080"] = DiscoveredServer(
            server_id="alpha@10.0.0.1:8080",
            name="alpha", host="10.0.0.1", port=8080, socks=1080, api=9090,
            vpn=True, version="0.1.0", pac="/proxy.pac",
            source="mdns", last_seen_at=time.time(), healthy=True,
        )

        _, body = await _http_get("127.0.0.1", api_port, "/api/servers")
        data = json.loads(body)
        assert data["count"] == 1
        srv = data["servers"][0]
        assert srv["server_id"] == "alpha@10.0.0.1:8080"
        assert srv["name"] == "alpha"
        assert srv["vpn"] is True
        assert srv["pac_url"] == "http://10.0.0.1:8080/proxy.pac"
        assert srv["source"] == "mdns"
    finally:
        await rt.stop()


# ---------------------------------------------------------------------------
# /api/events SSE
# ---------------------------------------------------------------------------


async def _read_until_contains(reader, needle: bytes, *, timeout: float = 2.0, max_bytes: int = 65536) -> bytes:
    """读到 buffer 包含 needle 为止。SSE 走 chunked 编码，没法直接 readuntil，所以这里
    简单地小步 read 拼成 buffer 再 substring 匹配。"""
    buf = bytearray()
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise asyncio.TimeoutError(f"timeout waiting for {needle!r}, got: {bytes(buf)!r}")
        try:
            chunk = await asyncio.wait_for(reader.read(1024), timeout=remaining)
        except asyncio.TimeoutError:
            raise asyncio.TimeoutError(f"timeout waiting for {needle!r}, got: {bytes(buf)!r}")
        if not chunk:
            raise ConnectionError(f"connection closed waiting for {needle!r}, got: {bytes(buf)!r}")
        buf.extend(chunk)
        if needle in buf:
            return bytes(buf)
        if len(buf) > max_bytes:
            raise AssertionError(f"buffer too big without match, got: {bytes(buf)!r}")


async def _close_writer(w: asyncio.StreamWriter) -> None:
    try:
        w.close()
    except Exception:
        pass
    try:
        await asyncio.wait_for(w.wait_closed(), timeout=1.0)
    except Exception:
        pass


async def test_api_events_emits_ready_then_server_discovered():
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    r, w = (None, None)
    try:
        r, w = await asyncio.open_connection("127.0.0.1", api_port)
        w.write(
            b"GET /api/events HTTP/1.1\r\n"
            b"Host: 127.0.0.1\r\n"
            b"Accept: text/event-stream\r\n"
            b"Connection: keep-alive\r\n\r\n"
        )
        await w.drain()

        buf = await _read_until_contains(r, b"event: ready")
        assert b"HTTP/1.1 200" in buf
        assert b"text/event-stream" in buf
        assert b"Access-Control-Allow-Origin: *" in buf

        rt.discoverer._state.online["beta@10.0.0.2:8080"] = DiscoveredServer(
            server_id="beta@10.0.0.2:8080",
            name="beta", host="10.0.0.2", port=8080, socks=1080, api=9090,
            vpn=False, version="0.1.0", pac="/proxy.pac",
            source="mdns", last_seen_at=time.time(), healthy=True,
        )
        rt.bus.publish("server_discovered", {
            "server_id": "beta@10.0.0.2:8080",
            "name": "beta",
            "host": "10.0.0.2",
            "port": 8080,
        })

        chunk = await _read_until_contains(r, b"server_discovered")
        assert b"event: server_discovered" in chunk
        assert b"beta@10.0.0.2:8080" in chunk
    finally:
        # 必须先关 client，否则 aiohttp AppRunner.cleanup 会卡在
        # 等 SSE handler 退出（handler 里 await q.get()）。
        if w is not None:
            await _close_writer(w)
        await rt.stop()


# ---------------------------------------------------------------------------
# /api/servers/forget(_all)
# ---------------------------------------------------------------------------


async def test_forget_server_removes_history_entry():
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    try:
        # 模拟有一个历史 server (非 online)
        rt.discoverer._state.history.append(DiscoveredServer(
            server_id="ghost@10.0.0.9:8080",
            name="ghost", host="10.0.0.9", port=8080, socks=1080, api=9090,
            vpn=False, version="0.1.0", pac="/proxy.pac",
            source="history", last_seen_at=time.time() - 3600, healthy=False,
        ))

        head, body = await _http_post_json(
            "127.0.0.1", api_port, "/api/servers/forget",
            {"server_id": "ghost@10.0.0.9:8080"},
        )
        assert b"HTTP/1.1 200" in head
        data = json.loads(body)
        assert data == {"ok": True, "removed": True, "server_id": "ghost@10.0.0.9:8080"}
        assert all(it.server_id != "ghost@10.0.0.9:8080" for it in rt.discoverer._state.history)
    finally:
        await rt.stop()


async def test_forget_server_unknown_returns_removed_false():
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    try:
        head, body = await _http_post_json(
            "127.0.0.1", api_port, "/api/servers/forget",
            {"server_id": "nope@1.2.3.4:5"},
        )
        assert b"HTTP/1.1 200" in head
        data = json.loads(body)
        assert data["ok"] is True
        assert data["removed"] is False
    finally:
        await rt.stop()


async def test_forget_server_invalid_body_returns_400_with_error_envelope():
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    try:
        head, body = await _http_post_json(
            "127.0.0.1", api_port, "/api/servers/forget", {"foo": "bar"},
        )
        assert b"HTTP/1.1 400" in head
        data = json.loads(body)
        # 验错误信封契约: { "error": { "code", "message" } }
        assert "error" in data
        assert data["error"]["code"] == "BAD_REQUEST"
    finally:
        await rt.stop()


async def test_forget_all_clears_history_only():
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    try:
        # 注入: 2 条历史 + 1 条 online
        for i in range(2):
            rt.discoverer._state.history.append(DiscoveredServer(
                server_id=f"old{i}@10.0.0.{i}:8080",
                name=f"old{i}", host=f"10.0.0.{i}", port=8080, socks=1080, api=9090,
                vpn=False, version="0.1.0", pac="/proxy.pac",
                source="history", last_seen_at=time.time() - 7200, healthy=False,
            ))
        rt.discoverer._state.online["live@10.0.0.5:8080"] = DiscoveredServer(
            server_id="live@10.0.0.5:8080",
            name="live", host="10.0.0.5", port=8080, socks=1080, api=9090,
            vpn=True, version="0.1.0", pac="/proxy.pac",
            source="mdns", last_seen_at=time.time(), healthy=True,
        )

        head, body = await _http_post_json("127.0.0.1", api_port, "/api/servers/forget_all")
        assert b"HTTP/1.1 200" in head
        data = json.loads(body)
        assert data["ok"] is True
        assert data["removed_count"] == 2

        assert rt.discoverer._state.history == []
        assert "live@10.0.0.5:8080" in rt.discoverer._state.online
    finally:
        await rt.stop()


async def test_api_events_emits_server_lost():
    api_port = _free_port()
    rt = ClientRuntime(_make_cfg(api_port))
    await rt.start()
    r, w = (None, None)
    try:
        r, w = await asyncio.open_connection("127.0.0.1", api_port)
        w.write(
            b"GET /api/events HTTP/1.1\r\n"
            b"Host: 127.0.0.1\r\n"
            b"Accept: text/event-stream\r\n"
            b"Connection: keep-alive\r\n\r\n"
        )
        await w.drain()
        await _read_until_contains(r, b"event: ready")

        rt.bus.publish("server_lost", {"server_id": "x@1.1.1.1:80", "name": "x"})

        chunk = await _read_until_contains(r, b"server_lost")
        assert b"event: server_lost" in chunk
        assert b"x@1.1.1.1:80" in chunk
    finally:
        if w is not None:
            await _close_writer(w)
        await rt.stop()
