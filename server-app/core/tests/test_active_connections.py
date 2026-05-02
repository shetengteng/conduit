"""Tests for active_connections.ConnectionRegistry + TrafficSampler."""
from __future__ import annotations

import asyncio

import pytest

from active_connections import ConnectionRegistry, TrafficSampler


pytestmark = pytest.mark.asyncio


async def test_register_lifecycle_publishes_events():
    events: list[tuple[str, dict]] = []
    reg = ConnectionRegistry(publish=lambda t, p: events.append((t, p)))

    sid = await reg.add("10.0.0.1", "http", "example.com:443")
    assert sid.startswith("s")
    assert len(reg) == 1

    await reg.update_bytes(sid, sent_delta=100, recv_delta=200)
    snap = reg.snapshot()[0]
    assert snap["sent_bytes"] == 100
    assert snap["recv_bytes"] == 200

    await reg.remove(sid)
    assert len(reg) == 0

    types = [e[0] for e in events]
    assert types == ["client_connected", "client_disconnected"]
    payload = events[1][1]
    assert payload["session_id"] == sid
    assert payload["sent_bytes"] == 100
    assert payload["recv_bytes"] == 200
    assert payload["duration_sec"] >= 0


async def test_remove_nonexistent_is_noop():
    reg = ConnectionRegistry()
    await reg.remove("ghost")
    assert len(reg) == 0


async def test_traffic_sampler_emits_per_peer_tick():
    events: list[tuple[str, dict]] = []
    reg = ConnectionRegistry()
    sampler = TrafficSampler(reg, window=5, publish=lambda t, p: events.append((t, p)))

    sid = await reg.add("10.0.0.5", "http", "host:80")

    sampler.start()
    await asyncio.sleep(1.05)
    await reg.update_bytes(sid, sent_delta=500, recv_delta=1500)
    await asyncio.sleep(1.1)
    await sampler.stop()
    await reg.remove(sid)

    ticks = [p for t, p in events if t == "traffic_tick"]
    assert ticks, "expected at least one traffic_tick"
    rates = [p["per_peer"].get("10.0.0.5") for p in ticks]
    assert any(r and r["sent_bps"] >= 500 for r in rates), (
        f"expected at least one tick with sent_bps>=500, got {rates}"
    )
    assert any(r and r["recv_bps"] >= 1500 for r in rates)


async def test_concurrent_id_allocation_unique():
    reg = ConnectionRegistry()
    sids = await asyncio.gather(*(reg.add("1.2.3.4", "http", "h:1") for _ in range(50)))
    assert len(set(sids)) == 50
