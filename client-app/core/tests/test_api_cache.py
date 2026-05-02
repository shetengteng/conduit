"""GET / DELETE /api/cache + GET /api/traffic 集成测试 —— M-γ。"""
from __future__ import annotations

import builtins
import socket

import pytest
import pytest_asyncio
from aiohttp.test_utils import TestServer, TestClient

from route_cache import RouteEntry
from api.server import build_app


def _free_port() -> int:
    s = socket.socket()
    try:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]
    finally:
        s.close()


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


@pytest_asyncio.fixture
async def runtime():
    import client_main as cm
    cfg = cm.ClientConfig(
        bind_host="127.0.0.1", bind_port=_free_port(),
        api_port=_free_port(), server_host=None, server_port=8080,
        pac_path="/proxy.pac", enable_system_proxy=False,
        log_level="WARNING", watchdog_ppid=None,
    )
    rt = cm.ClientRuntime(cfg)
    yield rt


@pytest.mark.asyncio
async def test_cache_endpoint_returns_empty_then_populated(runtime):
    app = build_app(runtime, loopback_only=False)
    server = TestServer(app)
    client = TestClient(server)
    await client.start_server()
    try:
        resp = await client.get("/api/cache")
        body = await resp.json()
        assert body["count"] == 0
        assert body["total"] == 0

        from datetime import datetime, timedelta, timezone
        runtime.cache.set("api.example.com", RouteEntry(
            host="api.example.com", direction="proxy",
            expires_at=datetime.now(timezone.utc) + timedelta(minutes=5),
            source="probe", hit_count=3,
        ))
        resp = await client.get("/api/cache")
        body = await resp.json()
        assert body["count"] == 1
        assert body["entries"][0]["host"] == "api.example.com"
        assert body["entries"][0]["direction"] == "proxy"
        assert body["entries"][0]["hit_count"] == 3
        assert body["stats"]["proxy_count"] == 1
    finally:
        await client.close()


@pytest.mark.asyncio
async def test_cache_endpoint_filter_by_direction(runtime):
    app = build_app(runtime, loopback_only=False)
    server = TestServer(app)
    client = TestClient(server)
    await client.start_server()
    try:
        runtime.cache.set_direction("a.example.com", "direct")
        runtime.cache.set_direction("b.example.com", "proxy")

        resp = await client.get("/api/cache", params={"direction": "proxy"})
        body = await resp.json()
        assert body["count"] == 1
        assert body["entries"][0]["host"] == "b.example.com"
    finally:
        await client.close()


@pytest.mark.asyncio
async def test_cache_endpoint_delete_flushes(runtime):
    app = build_app(runtime, loopback_only=False)
    server = TestServer(app)
    client = TestClient(server)
    await client.start_server()
    try:
        runtime.cache.set_direction("a.example.com", "direct")
        runtime.cache.set_direction("b.example.com", "proxy")

        resp = await client.delete("/api/cache")
        body = await resp.json()
        assert body["ok"] is True
        assert body["removed"] == 2

        resp = await client.get("/api/cache")
        body = await resp.json()
        assert body["count"] == 0
    finally:
        await client.close()


@pytest.mark.asyncio
async def test_traffic_endpoint_returns_zero_when_disconnected(runtime):
    app = build_app(runtime, loopback_only=False)
    server = TestServer(app)
    client = TestClient(server)
    await client.start_server()
    try:
        resp = await client.get("/api/traffic")
        body = await resp.json()
        assert body["uplink_bytes"] == 0
        assert body["downlink_bytes"] == 0
        assert body["total_uplink"] == 0
        assert body["total_downlink"] == 0
    finally:
        await client.close()


@pytest.mark.asyncio
async def test_traffic_endpoint_reflects_meter_when_present(runtime):
    from traffic_meter import TrafficMeter
    runtime.traffic_meter = TrafficMeter(runtime.bus, tick_interval=10.0)
    await runtime.traffic_meter.start()
    try:
        await runtime.traffic_meter.on_chunk(uplink=111, downlink=222)
        app = build_app(runtime, loopback_only=False)
        server = TestServer(app)
        client = TestClient(server)
        await client.start_server()
        try:
            resp = await client.get("/api/traffic")
            body = await resp.json()
            assert body["total_uplink"] == 111
            assert body["total_downlink"] == 222
        finally:
            await client.close()
    finally:
        await runtime.traffic_meter.stop()
        runtime.traffic_meter = None
