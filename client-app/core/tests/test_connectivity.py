"""单元测试:connectivity.probe + Heartbeat 状态机。"""
from __future__ import annotations

import asyncio
import socket

import pytest

from connectivity import (
    HEARTBEAT_RED_AT,
    HEARTBEAT_YELLOW_AT,
    Heartbeat,
    ProbeResult,
    probe,
)
from events_bus import EventBus


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


async def _accept_and_close(reader, writer):
    """假 server handler:接受连接后立刻关。"""
    try:
        writer.close()
        await writer.wait_closed()
    except Exception:
        pass


async def _start_dummy_server(port: int = 0):
    """启 TCP 监听器(handler 立刻关连接,避免 wait_closed 卡死)。"""
    return await asyncio.start_server(_accept_and_close, "127.0.0.1", port)


# ---------------------------------------------------------------------------
# probe
# ---------------------------------------------------------------------------


async def test_probe_returns_ok_when_both_ports_open():
    """开两个 TCP 监听器作为假 server。"""
    srv1 = await _start_dummy_server()
    srv2 = await _start_dummy_server()
    try:
        host, p1 = srv1.sockets[0].getsockname()[:2]
        _, p2 = srv2.sockets[0].getsockname()[:2]
        result = await probe(host=host, http_port=p1, socks_port=p2, timeout=2.0)
        assert isinstance(result, ProbeResult)
        assert result.ok is True
        assert result.socks_reachable is True
        assert result.http_reachable is True
        assert result.error is None
        assert result.latency_ms < 2000
    finally:
        srv1.close()
        srv2.close()
        await srv1.wait_closed()
        await srv2.wait_closed()


async def test_probe_returns_failure_when_socks_closed():
    """HTTP open, SOCKS 关 -> ok=False, 错误说 SOCKS 不可达。"""
    srv = await _start_dummy_server()
    try:
        host, http_port = srv.sockets[0].getsockname()[:2]
        socks_port = _free_port()  # 该端口当前没人 listen
        result = await probe(host=host, http_port=http_port, socks_port=socks_port, timeout=1.0)
        assert result.ok is False
        assert result.http_reachable is True
        assert result.socks_reachable is False
        assert "SOCKS" in result.error
        assert str(socks_port) in result.error
    finally:
        srv.close()
        await srv.wait_closed()


async def test_probe_returns_failure_when_both_closed():
    p1 = _free_port()
    p2 = _free_port()
    result = await probe(host="127.0.0.1", http_port=p1, socks_port=p2, timeout=1.0)
    assert result.ok is False
    assert "都不通" in result.error or "都不通" in (result.error or "")


# ---------------------------------------------------------------------------
# Heartbeat 状态机
# ---------------------------------------------------------------------------


async def test_heartbeat_publishes_yellow_after_first_failure():
    """server 一直关着 -> 第 1 次 fail -> 应 publish tone=yellow。"""
    bus = EventBus()
    q = await bus.subscribe()
    closed_port = _free_port()

    hb = Heartbeat(bus, host="127.0.0.1", http_port=closed_port, socks_port=closed_port,
                   interval=0.05, timeout=0.3)
    await hb.start()
    try:
        # 等 1 次 tick + publish
        evt = await asyncio.wait_for(q.get(), timeout=2.0)
        assert evt.type == "heartbeat_changed"
        assert evt.payload["tone"] == "yellow"
        assert evt.payload["consecutive_failures"] >= HEARTBEAT_YELLOW_AT
    finally:
        await hb.stop()


async def test_heartbeat_escalates_to_red_after_three_failures():
    bus = EventBus()
    q = await bus.subscribe()
    closed_port = _free_port()

    hb = Heartbeat(bus, host="127.0.0.1", http_port=closed_port, socks_port=closed_port,
                   interval=0.05, timeout=0.3)
    await hb.start()
    try:
        # 收集所有事件,直到看到 red 或超时
        seen_tones = []
        deadline = asyncio.get_running_loop().time() + 5.0
        while asyncio.get_running_loop().time() < deadline:
            try:
                evt = await asyncio.wait_for(q.get(), timeout=1.0)
            except asyncio.TimeoutError:
                continue
            if evt.type == "heartbeat_changed":
                seen_tones.append(evt.payload["tone"])
                if evt.payload["tone"] == "red":
                    assert evt.payload["consecutive_failures"] >= HEARTBEAT_RED_AT
                    break
        assert "yellow" in seen_tones
        assert "red" in seen_tones
    finally:
        await hb.stop()


async def test_heartbeat_recovers_to_green_when_server_back():
    """先关后开 socket -> tone yellow -> green(recovered=True)。"""
    bus = EventBus()
    q = await bus.subscribe()
    port = _free_port()

    hb = Heartbeat(bus, host="127.0.0.1", http_port=port, socks_port=port,
                   interval=0.05, timeout=0.3)
    await hb.start()
    try:
        # 等 yellow
        async def wait_for_tone(target):
            while True:
                evt = await asyncio.wait_for(q.get(), timeout=2.0)
                if evt.type == "heartbeat_changed" and evt.payload["tone"] == target:
                    return evt

        await wait_for_tone("yellow")

        # 起 server 让它恢复
        srv = await _start_dummy_server(port)
        try:
            evt = await wait_for_tone("green")
            assert evt.payload["recovered"] is True
            assert evt.payload["consecutive_failures"] == 0
        finally:
            srv.close()
            await srv.wait_closed()
    finally:
        await hb.stop()
