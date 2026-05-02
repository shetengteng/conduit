"""GET /api/diagnose 集成测试 —— M-δ。"""
from __future__ import annotations

import builtins
import socket

import pytest
import pytest_asyncio
from aiohttp.test_utils import TestServer, TestClient

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
    # proxy 没真起,actual_port 用 bind_port 兜底(这是 LocalProxyServer 的行为)
    yield rt


@pytest.mark.asyncio
async def test_diagnose_idle_returns_5_checks(runtime):
    app = build_app(runtime, loopback_only=False)
    server = TestServer(app)
    client = TestClient(server)
    await client.start_server()
    try:
        resp = await client.get("/api/diagnose")
        body = await resp.json()
        assert resp.status == 200
        assert "ok" in body
        assert "checks" in body
        keys = [c["key"] for c in body["checks"]]
        assert keys == ["sidecar", "mdns", "server_reach", "pac", "system_proxy"]
    finally:
        await client.close()


@pytest.mark.asyncio
async def test_diagnose_idle_mdns_unavailable_yields_remediation(runtime):
    app = build_app(runtime, loopback_only=False)
    server = TestServer(app)
    client = TestClient(server)
    await client.start_server()
    try:
        resp = await client.get("/api/diagnose")
        body = await resp.json()
        mdns_check = next(c for c in body["checks"] if c["key"] == "mdns")
        # zeroconf 被禁,available 应该 False -> mdns ok=False + 有 remediation
        assert mdns_check["ok"] is False
        assert "zeroconf" in (mdns_check["remediation"] or "")
    finally:
        await client.close()


@pytest.mark.asyncio
async def test_diagnose_skips_server_reach_when_disconnected(runtime):
    app = build_app(runtime, loopback_only=False)
    server = TestServer(app)
    client = TestClient(server)
    await client.start_server()
    try:
        resp = await client.get("/api/diagnose")
        body = await resp.json()
        sr = next(c for c in body["checks"] if c["key"] == "server_reach")
        # 未连接 -> 跳过(标 ok=True + 描述)
        assert sr["ok"] is True
        assert "未连接" in sr["detail"] or "跳过" in sr["detail"]
    finally:
        await client.close()


@pytest.mark.asyncio
async def test_diagnose_sidecar_reports_pid_and_uptime(runtime):
    app = build_app(runtime, loopback_only=False)
    server = TestServer(app)
    client = TestClient(server)
    await client.start_server()
    try:
        resp = await client.get("/api/diagnose")
        body = await resp.json()
        sd = next(c for c in body["checks"] if c["key"] == "sidecar")
        assert sd["ok"] is True
        assert "PID" in sd["detail"]
    finally:
        await client.close()


@pytest.mark.asyncio
async def test_diagnose_overall_ok_aggregates_subchecks(runtime):
    app = build_app(runtime, loopback_only=False)
    server = TestServer(app)
    client = TestClient(server)
    await client.start_server()
    try:
        resp = await client.get("/api/diagnose")
        body = await resp.json()
        # 因为 mdns 失败,overall 一定 False
        assert body["ok"] is False
    finally:
        await client.close()
