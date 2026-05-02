"""Connection registry + traffic ring buffer for Conduit server.

Lives entirely in-process (single asyncio loop). No external storage.

Two collaborating objects:

- :class:`ConnectionRegistry` — every accepted CONNECT/SOCKS session calls
  ``add``/``update_bytes``/``remove`` on this. Supplies ``snapshot()`` to
  the HTTP control API.
- :class:`TrafficSampler` — runs a 1-second ring buffer keyed by client IP,
  deriving ``in_bps`` / ``out_bps`` from the registry's running totals.
  ``snapshot_tick()`` is what the SSE channel pushes to UI every second.

Design ref: ``design/2026-04-30-2-...md`` §3.5.7 (snake_case wire format),
§4.10 (this module's spec) and §4.11 (relay's ``on_progress`` contract).
"""
from __future__ import annotations

import asyncio
import time
from collections import defaultdict, deque
from dataclasses import dataclass
from typing import Any, Callable, Iterable, Optional

PublishFn = Callable[[str, dict[str, Any]], None]


@dataclass
class ConnectionInfo:
    session_id: str
    peer_ip: str
    proto: str  # "http" | "socks5"
    target: str  # "host:port"
    since: float
    last_seen: float
    sent_bytes: int = 0
    recv_bytes: int = 0


class ConnectionRegistry:
    """Process-wide registry of in-flight proxy sessions.

    Single asyncio loop assumption. ``add`` and ``remove`` need a lock
    only for the monotonic counter; the per-session byte-count update
    is hot-path and intentionally lock-free.
    """

    def __init__(self, publish: Optional[PublishFn] = None) -> None:
        self._sessions: dict[str, ConnectionInfo] = {}
        self._next_id = 0
        self._lock = asyncio.Lock()
        self._publish = publish

    def set_publisher(self, publish: PublishFn) -> None:
        self._publish = publish

    async def add(self, peer_ip: str, proto: str, target: str) -> str:
        async with self._lock:
            self._next_id += 1
            sid = f"s{self._next_id}"
        now = time.time()
        self._sessions[sid] = ConnectionInfo(sid, peer_ip, proto, target, now, now)
        if self._publish is not None:
            self._publish("client_connected", {
                "session_id": sid,
                "peer_ip": peer_ip,
                "proto": proto,
                "target": target,
                "since": now,
            })
        return sid

    async def update_bytes(self, sid: str, sent_delta: int, recv_delta: int) -> None:
        s = self._sessions.get(sid)
        if not s:
            return
        s.sent_bytes += sent_delta
        s.recv_bytes += recv_delta
        s.last_seen = time.time()

    async def remove(self, sid: str) -> None:
        s = self._sessions.pop(sid, None)
        if s is not None and self._publish is not None:
            self._publish("client_disconnected", {
                "session_id": sid,
                "peer_ip": s.peer_ip,
                "sent_bytes": s.sent_bytes,
                "recv_bytes": s.recv_bytes,
                "duration_sec": time.time() - s.since,
            })

    def snapshot(self) -> list[dict]:
        return [
            {
                "session_id": s.session_id,
                "peer_ip": s.peer_ip,
                "proto": s.proto,
                "target": s.target,
                "since": s.since,
                "last_seen": s.last_seen,
                "sent_bytes": s.sent_bytes,
                "recv_bytes": s.recv_bytes,
            }
            for s in self._sessions.values()
        ]

    def __len__(self) -> int:
        return len(self._sessions)

    def values(self) -> Iterable[ConnectionInfo]:
        return self._sessions.values()


class TrafficSampler:
    """Per-IP 1-second ring buffer driving the live traffic chart.

    Each tick computes ``in_bps`` / ``out_bps`` as the delta of cumulative
    bytes since the last tick — ie. flat counters in the registry are
    converted into rate samples here.
    """

    DEFAULT_WINDOW = 600  # seconds

    def __init__(
        self,
        registry: ConnectionRegistry,
        window: int = DEFAULT_WINDOW,
        publish: Optional[PublishFn] = None,
    ) -> None:
        self._registry = registry
        self._window = window
        self._series: dict[str, deque[tuple[float, int, int]]] = defaultdict(
            lambda: deque(maxlen=self._window)
        )
        self._last_totals: dict[str, tuple[int, int]] = {}
        self._task: asyncio.Task | None = None
        self._publish = publish

    def set_publisher(self, publish: PublishFn) -> None:
        self._publish = publish

    def start(self) -> None:
        if self._task is None or self._task.done():
            self._task = asyncio.create_task(self._run_forever())

    async def stop(self) -> None:
        if self._task is None:
            return
        self._task.cancel()
        try:
            await self._task
        except (asyncio.CancelledError, Exception):
            pass
        self._task = None

    async def _run_forever(self) -> None:
        while True:
            await asyncio.sleep(1.0)
            self._sample_once()

    def _sample_once(self) -> None:
        now = time.time()
        totals: dict[str, tuple[int, int]] = defaultdict(lambda: (0, 0))
        for s in self._registry.values():
            sent, recv = totals[s.peer_ip]
            totals[s.peer_ip] = (sent + s.sent_bytes, recv + s.recv_bytes)

        seen_ips: set[str] = set(totals.keys())
        tick_payload: dict[str, dict[str, int]] = {}
        for ip, (sent_total, recv_total) in totals.items():
            prev_sent, prev_recv = self._last_totals.get(ip, (sent_total, recv_total))
            sent_bps = max(0, sent_total - prev_sent)
            recv_bps = max(0, recv_total - prev_recv)
            self._series[ip].append((now, sent_bps, recv_bps))
            self._last_totals[ip] = (sent_total, recv_total)
            tick_payload[ip] = {"sent_bps": sent_bps, "recv_bps": recv_bps}

        for ip in list(self._last_totals.keys()):
            if ip in seen_ips:
                continue
            self._series[ip].append((now, 0, 0))
            tick_payload.setdefault(ip, {"sent_bps": 0, "recv_bps": 0})

        if self._publish is not None and tick_payload:
            self._publish("traffic_tick", {"ts": now, "per_peer": tick_payload})

    def series(self, peer_ip: str, window_sec: int) -> list[tuple[float, int, int]]:
        cutoff = time.time() - window_sec
        return [t for t in self._series.get(peer_ip, ()) if t[0] >= cutoff]

    def all_series(self, window_sec: int) -> dict[str, list[tuple[float, int, int]]]:
        cutoff = time.time() - window_sec
        return {
            ip: [t for t in dq if t[0] >= cutoff]
            for ip, dq in self._series.items()
        }

    def snapshot_tick(self) -> dict[str, tuple[float, int, int]]:
        return {ip: dq[-1] for ip, dq in self._series.items() if dq}
