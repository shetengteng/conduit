"""RouteResolver 决策事件测试 —— M-γ。

确认 set_event_publisher / set_event_publisher(None) 正常 fire / silence。
"""
from __future__ import annotations

import asyncio
import socket

import pytest

from route_cache import RouteCache
from route_resolver import RouteResolver


def _free_port() -> int:
    s = socket.socket()
    try:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]
    finally:
        s.close()


@pytest.mark.asyncio
async def test_publisher_called_on_each_resolve_with_private_ip():
    cache = RouteCache()
    resolver = RouteResolver(cache)
    events: list[tuple[str, int, str, str]] = []
    resolver.set_event_publisher(lambda h, p, d: events.append((h, p, d.direction, d.source)))

    await resolver.resolve("192.168.1.50", 80)
    assert len(events) == 1
    assert events[0] == ("192.168.1.50", 80, "direct", "private_ip")


@pytest.mark.asyncio
async def test_publisher_called_on_global_override_mode():
    cache = RouteCache()
    resolver = RouteResolver(cache)
    events: list = []
    resolver.set_event_publisher(lambda h, p, d: events.append(d))

    resolver.set_global_mode("a_unreachable")
    await resolver.resolve("api.example.com", 443)
    assert events[0].direction == "direct"
    assert events[0].source == "global_override"


@pytest.mark.asyncio
async def test_publisher_can_be_unset_silences():
    cache = RouteCache()
    resolver = RouteResolver(cache)
    events: list = []
    resolver.set_event_publisher(lambda h, p, d: events.append(d))
    await resolver.resolve("192.168.1.1", 80)
    assert len(events) == 1

    resolver.set_event_publisher(None)
    await resolver.resolve("10.0.0.1", 22)
    assert len(events) == 1


@pytest.mark.asyncio
async def test_publisher_failure_does_not_break_resolve():
    cache = RouteCache()
    resolver = RouteResolver(cache)

    def bad_pub(h, p, d):
        raise RuntimeError("boom")

    resolver.set_event_publisher(bad_pub)
    decision = await resolver.resolve("192.168.1.1", 80)
    assert decision.direction == "direct"
