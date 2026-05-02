"""EventBus — process-local pub/sub for the control API SSE stream.

Each subscriber owns a bounded asyncio.Queue. Slow consumers are dropped
events-first instead of stalling the publishers. Events are plain dicts,
serialised by the SSE handler.

Event types (see design §4.6):

- ``client_connected``     payload: ``{"id", "remote", "target", "proto"}``
- ``client_disconnected``  payload: ``{"id"}``
- ``traffic_tick``         payload: ``{"sent_bps", "recv_bps", "clients"}`` (1Hz)
- ``vpn_state_changed``    payload: ``{"available", "iface"}``
- ``log``                  payload: ``{"level", "logger", "message"}`` (optional)
"""
from __future__ import annotations

import asyncio
import logging
import time
from dataclasses import dataclass
from typing import Any, AsyncIterator, Dict

log = logging.getLogger("core.events")


@dataclass
class Event:
    type: str
    payload: Dict[str, Any]
    ts: float


class EventBus:
    """In-process broadcaster. Drop-on-overflow per subscriber."""

    def __init__(self, queue_maxsize: int = 256) -> None:
        self._subs: list[asyncio.Queue[Event]] = []
        self._lock = asyncio.Lock()
        self._maxsize = queue_maxsize

    async def subscribe(self) -> asyncio.Queue[Event]:
        q: asyncio.Queue[Event] = asyncio.Queue(maxsize=self._maxsize)
        async with self._lock:
            self._subs.append(q)
        return q

    async def unsubscribe(self, q: asyncio.Queue[Event]) -> None:
        async with self._lock:
            try:
                self._subs.remove(q)
            except ValueError:
                pass

    def publish(self, type_: str, payload: Dict[str, Any]) -> None:
        if not self._subs:
            return
        evt = Event(type=type_, payload=payload, ts=time.time())
        for q in list(self._subs):
            try:
                q.put_nowait(evt)
            except asyncio.QueueFull:
                try:
                    q.get_nowait()
                    q.put_nowait(evt)
                except Exception:
                    pass

    async def stream(self, q: asyncio.Queue[Event]) -> AsyncIterator[Event]:
        while True:
            evt = await q.get()
            yield evt
