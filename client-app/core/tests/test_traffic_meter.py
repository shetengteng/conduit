"""traffic_meter.py 单元测试 —— M-γ。"""
from __future__ import annotations

import asyncio

import pytest

from events_bus import EventBus
from traffic_meter import TrafficMeter


@pytest.mark.asyncio
async def test_meter_accumulates_and_emits_tick():
    bus = EventBus()
    q = await bus.subscribe()
    meter = TrafficMeter(bus, tick_interval=0.1)
    await meter.start()
    try:
        await meter.on_chunk(uplink=100, downlink=0)
        await meter.on_chunk(uplink=0, downlink=300)
        await meter.on_chunk(uplink=50, downlink=0)
        await asyncio.sleep(0.18)
    finally:
        await meter.stop()

    ticks = []
    while True:
        try:
            evt = q.get_nowait()
        except asyncio.QueueEmpty:
            break
        if evt.type == "traffic_tick":
            ticks.append(evt.payload)

    assert len(ticks) >= 1
    last = ticks[-1]
    assert last["total_uplink"] == 150
    assert last["total_downlink"] == 300
    assert last["uplink_bytes"] >= 0
    assert last["downlink_bytes"] >= 0


@pytest.mark.asyncio
async def test_meter_snapshot_returns_cumulative():
    bus = EventBus()
    meter = TrafficMeter(bus, tick_interval=10.0)
    await meter.start()
    try:
        await meter.on_chunk(uplink=42, downlink=0)
        await meter.on_chunk(uplink=0, downlink=58)
        snap = meter.snapshot()
        assert snap["total_uplink"] == 42
        assert snap["total_downlink"] == 58
        assert snap["uplink_bytes"] == 42
        assert snap["downlink_bytes"] == 58
    finally:
        await meter.stop()


@pytest.mark.asyncio
async def test_meter_stop_is_idempotent_and_releases_task():
    bus = EventBus()
    meter = TrafficMeter(bus, tick_interval=0.05)
    await meter.start()
    await meter.stop()
    await meter.stop()
    assert meter._task is None
