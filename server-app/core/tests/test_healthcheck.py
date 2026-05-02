"""Tests for healthcheck.HealthCheck."""
from __future__ import annotations

import asyncio
import socket

import pytest

from healthcheck import HealthCheck

pytestmark = pytest.mark.asyncio


async def _free_port_listening() -> tuple[int, asyncio.base_events.Server]:
    async def noop(reader, writer):
        writer.close()

    srv = await asyncio.start_server(noop, "127.0.0.1", 0)
    port = srv.sockets[0].getsockname()[1]
    return port, srv


async def test_port_check_pass_when_listening():
    port, srv = await _free_port_listening()
    try:
        hc = HealthCheck(http_port=port, socks_port=port, api_port=port)
        details = await hc.to_dict()
        names = {c["name"]: c for c in details["checks"]}
        assert names["http_port"]["ok"] is True
        assert names["socks5_port"]["ok"] is True
        assert names["api_port"]["ok"] is True
    finally:
        srv.close()
        await srv.wait_closed()


async def test_port_check_fail_when_nothing_listens():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]

    hc = HealthCheck(http_port=port, socks_port=port, api_port=port)
    details = await hc.to_dict()
    names = {c["name"]: c for c in details["checks"]}
    assert names["http_port"]["ok"] is False
    assert "ready" in details
    assert details["ready"] in (True, False)


async def test_lan_ip_check_returns_something():
    hc = HealthCheck(80, 1080, 8090)
    details = await hc.to_dict()
    lan = next(c for c in details["checks"] if c["name"] == "lan_ip")
    assert isinstance(lan["ok"], bool)
    assert isinstance(lan["detail"], str)
