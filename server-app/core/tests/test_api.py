"""Tests for the HTTP control API endpoints."""
from __future__ import annotations

import asyncio
import json

import aiohttp
import pytest

pytestmark = pytest.mark.asyncio


def _api_base(cfg) -> str:
    return f"http://127.0.0.1:{cfg.api_port}"


async def test_healthz_returns_breakdown(core):
    async with aiohttp.ClientSession() as s:
        async with s.get(f"{_api_base(core.cfg)}/healthz") as r:
            assert r.status == 200
            body = await r.json()
            assert "ready" in body
            assert isinstance(body["checks"], list)
            assert body["running"] is True


async def test_status_returns_runtime_info(core):
    async with aiohttp.ClientSession() as s:
        async with s.get(f"{_api_base(core.cfg)}/api/status") as r:
            assert r.status == 200
            body = await r.json()
            assert body["running"] is True
            assert body["http_port"] == core.cfg.http_port
            assert body["clients_count"] == 0
            assert body["pac_url"].startswith("http://127.0.0.1:")


async def test_clients_empty(core):
    async with aiohttp.ClientSession() as s:
        async with s.get(f"{_api_base(core.cfg)}/api/clients") as r:
            assert r.status == 200
            body = await r.json()
            assert body["count"] == 0
            assert body["clients"] == []
            # 新增 passive 字段
            assert body["passive_count"] == 0
            assert body["passive_clients"] == []


# ---------------------------------------------------------------------------
# Passive client heartbeat (LAN-facing HTTP proxy port)
# ---------------------------------------------------------------------------


async def _http_proxy_get_text(host: str, port: int, path: str, *, timeout: float = 2.0) -> tuple[int, dict, bytes]:
    """通过 server 的 HTTP proxy 端口直接 GET (代理本身的元路径,不走 CONNECT)。"""
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
    finally:
        try:
            w.close()
            await w.wait_closed()
        except Exception:
            pass

    head_lines = head.decode("ascii", errors="ignore").split("\r\n")
    status_line = head_lines[0]
    status = int(status_line.split(" ")[1])
    headers = {}
    for line in head_lines[1:]:
        if ":" in line:
            k, _, v = line.partition(":")
            headers[k.strip().lower()] = v.strip()
    return status, headers, body


async def test_proxy_heartbeat_creates_passive_client(core):
    status, headers, body = await _http_proxy_get_text(
        "127.0.0.1", core.cfg.http_port,
        "/api/clients/heartbeat?name=TestMac&version=0.1.0",
    )
    assert status == 200
    assert "application/json" in headers.get("content-type", "")
    payload = json.loads(body)
    assert payload["ok"] is True
    assert payload["created"] is True
    assert payload["ttl_sec"] == 60

    async with aiohttp.ClientSession() as s:
        async with s.get(f"{_api_base(core.cfg)}/api/clients") as r:
            data = await r.json()
            assert data["passive_count"] == 1
            pc = data["passive_clients"][0]
            assert pc["client_name"] == "TestMac"
            assert pc["version"] == "0.1.0"
            assert pc["peer_ip"] == "127.0.0.1"


async def test_proxy_heartbeat_idempotent_on_repeat(core):
    for _ in range(3):
        status, _, body = await _http_proxy_get_text(
            "127.0.0.1", core.cfg.http_port,
            "/api/clients/heartbeat?name=TestMac&version=0.1.0",
        )
        assert status == 200

    async with aiohttp.ClientSession() as s:
        async with s.get(f"{_api_base(core.cfg)}/api/clients") as r:
            data = await r.json()
            assert data["passive_count"] == 1


async def test_status_includes_passive_count_after_heartbeat(core):
    await _http_proxy_get_text(
        "127.0.0.1", core.cfg.http_port,
        "/api/clients/heartbeat?name=TestMac&version=0.1.0",
    )
    async with aiohttp.ClientSession() as s:
        async with s.get(f"{_api_base(core.cfg)}/api/status") as r:
            body = await r.json()
            assert body["passive_clients_count"] == 1


async def test_traffic_default_window(core):
    async with aiohttp.ClientSession() as s:
        async with s.get(f"{_api_base(core.cfg)}/api/traffic") as r:
            assert r.status == 200
            body = await r.json()
            assert body["window_sec"] == 60
            assert "series" in body


async def test_traffic_bad_param_400(core):
    async with aiohttp.ClientSession() as s:
        async with s.get(f"{_api_base(core.cfg)}/api/traffic?window=abc") as r:
            assert r.status == 400
            body = await r.json()
            assert body["error"]["code"] == "BAD_PARAM"


async def test_admin_stop_post_only(core):
    async with aiohttp.ClientSession() as s:
        async with s.get(f"{_api_base(core.cfg)}/api/admin/stop") as r:
            assert r.status == 405


async def test_admin_stop_triggers_shutdown(core):
    async with aiohttp.ClientSession() as s:
        async with s.post(f"{_api_base(core.cfg)}/api/admin/stop") as r:
            assert r.status == 200
            body = await r.json()
            assert body["ok"] is True

    for _ in range(20):
        if not core.running:
            break
        await asyncio.sleep(0.1)
    assert core.running is False


async def test_sse_emits_ready_then_session_events(core, echo_target):
    """Open SSE → trigger one proxy session → see client_connected/disconnected."""
    timeout = aiohttp.ClientTimeout(total=8)
    async with aiohttp.ClientSession(timeout=timeout) as s:
        async with s.get(f"{_api_base(core.cfg)}/api/events") as resp:
            assert resp.status == 200
            assert "text/event-stream" in resp.headers["Content-Type"]

            async def read_events(want: int) -> list[tuple[str, dict]]:
                out: list[tuple[str, dict]] = []
                cur_evt = None
                cur_data = None
                async for raw in resp.content:
                    line = raw.decode("utf-8", errors="ignore").rstrip("\r\n")
                    if line.startswith("event:"):
                        cur_evt = line[6:].strip()
                    elif line.startswith("data:"):
                        cur_data = line[5:].strip()
                    elif line == "":
                        if cur_evt and cur_data:
                            out.append((cur_evt, json.loads(cur_data)))
                            cur_evt = cur_data = None
                            if len(out) >= want:
                                return out
                return out

            consumer = asyncio.create_task(read_events(3))
            await asyncio.sleep(0.2)

            r, w = await asyncio.open_connection("127.0.0.1", core.cfg.http_port)
            w.write(b"CONNECT 127.0.0.1:19999 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            await w.drain()
            await r.readuntil(b"\r\n\r\n")
            w.write(b"x")
            await w.drain()
            await r.read(1)
            w.close()
            try:
                await w.wait_closed()
            except Exception:
                pass

            events = await asyncio.wait_for(consumer, timeout=5)
            types = [e[0] for e in events]
            assert "ready" in types
            assert "client_connected" in types
            assert "client_disconnected" in types
