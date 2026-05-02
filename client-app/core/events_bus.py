"""EventBus —— 进程内 pub/sub，给客户端 control API SSE 用。

设计与 server-app/core/events_bus.py 同构（事实上代码一致），独立放在
client-app 下避免跨模块依赖。

事件类型（M-β 阶段）：

- ``server_discovered``     payload: ``{server_id, name, host, port, socks, api, vpn, version, source}``
- ``server_lost``           payload: ``{server_id, name}``
- (M-β.2 增) ``connect_progress`` ``connect_done`` ``heartbeat_changed``
- (M-γ 增) ``cache_hit`` ``probe_completed`` ``mode_changed``

每个订阅方拥有独立的有界 ``asyncio.Queue``。慢消费者会被丢弃旧事件而不是
阻塞 publisher。
"""
from __future__ import annotations

import asyncio
import logging
import time
from dataclasses import dataclass
from typing import Any, AsyncIterator, Dict

log = logging.getLogger("client.events")


@dataclass
class Event:
    type: str
    payload: Dict[str, Any]
    ts: float


class EventBus:
    """进程内广播器。订阅者队列满时丢最旧事件。"""

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
