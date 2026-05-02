"""Tests for events_bus.EventBus."""
from __future__ import annotations

import asyncio

import pytest

from events_bus import EventBus

pytestmark = pytest.mark.asyncio


async def test_publish_to_one_subscriber():
    bus = EventBus()
    q = await bus.subscribe()
    bus.publish("hello", {"k": 1})
    evt = await asyncio.wait_for(q.get(), timeout=0.5)
    assert evt.type == "hello"
    assert evt.payload == {"k": 1}


async def test_fanout_to_many_subscribers():
    bus = EventBus()
    qs = [await bus.subscribe() for _ in range(3)]
    bus.publish("ping", {})
    for q in qs:
        evt = await asyncio.wait_for(q.get(), timeout=0.5)
        assert evt.type == "ping"


async def test_drop_oldest_on_overflow():
    bus = EventBus(queue_maxsize=2)
    q = await bus.subscribe()
    for i in range(5):
        bus.publish("e", {"i": i})
    drained = []
    while not q.empty():
        drained.append(q.get_nowait())
    assert len(drained) == 2
    assert drained[-1].payload["i"] == 4


async def test_unsubscribe_stops_delivery():
    bus = EventBus()
    q = await bus.subscribe()
    await bus.unsubscribe(q)
    bus.publish("e", {})
    with pytest.raises(asyncio.TimeoutError):
        await asyncio.wait_for(q.get(), timeout=0.1)
