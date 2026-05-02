"""Tests for ProxyCore start/stop/status lifecycle."""
from __future__ import annotations

import asyncio

import pytest

from proxy_core import ProxyCore

pytestmark = pytest.mark.asyncio


async def test_start_stop_idempotent(core_cfg):
    c = ProxyCore(core_cfg)
    await c.start()
    assert c.running is True
    await c.start()  # second call is a no-op
    assert c.running is True
    await c.stop()
    assert c.running is False
    await c.stop()  # idempotent
    assert c.running is False


async def test_start_stop_three_times_releases_ports(core_cfg):
    for _ in range(3):
        c = ProxyCore(core_cfg)
        await c.start()
        assert c.running is True
        await c.stop()
        assert c.running is False


async def test_status_shape_when_running(core):
    s = await core.status()
    assert s["running"] is True
    assert s["http_port"] == core.cfg.http_port
    assert s["socks5_port"] == core.cfg.socks_port
    assert s["api_port"] == core.cfg.api_port
    assert s["clients_count"] == 0
    assert "vpn" in s and "available" in s["vpn"]
    assert "lan" in s and "available" in s["lan"]


async def test_proxy_session_registers_in_registry(core, echo_target):
    r, w = await asyncio.open_connection("127.0.0.1", core.cfg.http_port)
    w.write(b"CONNECT 127.0.0.1:19999 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
    await w.drain()
    head = await r.readuntil(b"\r\n\r\n")
    assert b" 200 " in head

    w.write(b"hi-conduit")
    await w.drain()
    echoed = await r.read(10)
    assert echoed == b"hi-conduit"

    await asyncio.sleep(0.05)
    snap = core.registry.snapshot()
    assert len(snap) == 1
    assert snap[0]["proto"] == "http"
    assert snap[0]["target"] == "127.0.0.1:19999"
    assert snap[0]["sent_bytes"] == 10
    assert snap[0]["recv_bytes"] == 10

    w.close()
    try:
        await w.wait_closed()
    except Exception:
        pass
    await asyncio.sleep(0.1)
    assert len(core.registry) == 0
