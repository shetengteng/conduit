"""Route resolver for the Conduit client-app smart local proxy.

Given a target ``host:port`` decide whether the connection should go
``direct`` from this machine or be forwarded through the Conduit Server
(``proxy``).  The decision pipeline is:

1. **Global override** — if ``connectivity`` has marked the server
   unreachable, every host is downgraded to ``direct`` (we know the
   forward leg would fail anyway).
2. **Private-IP fast path** — RFC1918 / loopback never goes through
   the server.
3. **Cache lookup** — exact host match first, then wildcard / dotted
   prefix patterns pre-filled from the server's PAC.
4. **TCP connect probe** — 1.5 s connect, no application I/O, then
   memoise the result.
5. **Self-heal** — when the SOCKS5 layer reports that a cached
   ``direct`` answer actually failed, we flip it to ``proxy`` so the
   retry succeeds.

Cross-references:
* design/2026-04-30-3-Conduit-Client-客户端可行性报告.md §3.4
* design/2026-04-30-3-Conduit-Client-客户端可行性报告.md §3.10
"""
from __future__ import annotations

import asyncio
import ipaddress
import logging
import socket
from dataclasses import dataclass
from typing import Literal

from route_cache import Direction, RouteCache, RouteEntry

logger = logging.getLogger("conduit.client.resolver")

GlobalMode = Literal["normal", "a_unreachable"]

DNS_TIMEOUT = 0.5
DEFAULT_PROBE_TIMEOUT = 1.5


@dataclass(frozen=True)
class RouteDecision:
    direction: Direction
    source: str  # "global_override" / "private_ip" / "cache" / "pattern" / "probe"
    cache_entry: RouteEntry | None = None


def _is_private_ip(host: str) -> bool:
    """Return True for loopback, link-local, or RFC1918 addresses.

    Strings that fail to parse as IPs (i.e. domain names) return
    False — we always probe / look up domains.
    """
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        return False
    return ip.is_private or ip.is_loopback or ip.is_link_local


def _pattern_match(pattern: str, host: str) -> bool:
    """Match the limited subset of patterns we pre-fill from PAC.

    Supported forms:

    * ``*.zoom.us``  → matches ``foo.zoom.us`` (and ``zoom.us`` itself,
      mirroring the convention used by browsers when consuming PAC).
    * ``.zoom.us``   → same semantics as ``*.zoom.us``.
    * ``zoom.us``    → exact match (already covered by the cache).
    """
    pattern = pattern.lower().strip()
    host = host.lower().strip()
    if pattern == host:
        return True
    if pattern.startswith("*."):
        suffix = pattern[1:]   # ".zoom.us"
        return host.endswith(suffix) or host == suffix[1:]
    if pattern.startswith("."):
        return host.endswith(pattern) or host == pattern[1:]
    return False


async def tcp_probe(host: str, port: int, *, timeout: float = DEFAULT_PROBE_TIMEOUT) -> bool:
    """Single-shot TCP connect probe (no application data)."""
    try:
        try:
            ipaddress.ip_address(host)
            target_ip = host
        except ValueError:
            loop = asyncio.get_running_loop()
            infos = await asyncio.wait_for(
                loop.getaddrinfo(host, port, family=socket.AF_INET),
                timeout=DNS_TIMEOUT,
            )
            if not infos:
                return False
            target_ip = infos[0][4][0]

        reader, writer = await asyncio.wait_for(
            asyncio.open_connection(target_ip, port),
            timeout=timeout,
        )
        writer.close()
        try:
            await writer.wait_closed()
        except Exception:
            pass
        return True
    except (asyncio.TimeoutError, OSError, socket.gaierror):
        return False


class RouteResolver:
    """Per-process resolver. Owns a single ``RouteCache``."""

    def __init__(
        self,
        cache: RouteCache,
        *,
        probe_timeout: float = DEFAULT_PROBE_TIMEOUT,
        event_publisher=None,
    ) -> None:
        self.cache = cache
        self.probe_timeout = probe_timeout
        self._global_mode: GlobalMode = "normal"
        # M-γ:每次决策后调 event_publisher(host, decision_dict);ClientRuntime
        # 注入闭包,把 publish 转成 EventBus.publish("route_decision", ...)。
        # 留 None 时静默(向后兼容,server-app 也不需要事件)。
        self._event_publisher = event_publisher

    def set_event_publisher(self, publisher) -> None:
        """连接 / 断开时由 ClientRuntime 切换。None = 不广播。"""
        self._event_publisher = publisher

    def _emit(self, host: str, port: int, decision: RouteDecision) -> None:
        if self._event_publisher is None:
            return
        try:
            self._event_publisher(host, port, decision)
        except Exception as exc:  # noqa: BLE001
            logger.warning("route_decision publisher failed: %s", exc)

    # ------------------------------------------------------------------
    # global mode (driven by connectivity heartbeat)
    # ------------------------------------------------------------------

    @property
    def global_mode(self) -> GlobalMode:
        return self._global_mode

    def set_global_mode(self, mode: GlobalMode) -> None:
        if mode == self._global_mode:
            return
        logger.info("route_resolver: global mode -> %s", mode)
        self._global_mode = mode
        if mode == "a_unreachable":
            removed = self.cache.flush_proxy_entries()
            logger.info(
                "route_resolver: global downgrade flushed %d proxy entries", removed
            )

    # ------------------------------------------------------------------
    # core decision
    # ------------------------------------------------------------------

    async def resolve(self, host: str, port: int) -> RouteDecision:
        host_norm = host.strip().lower()

        if self._global_mode == "a_unreachable":
            decision = RouteDecision("direct", source="global_override")
            self._emit(host_norm, port, decision)
            return decision

        if _is_private_ip(host_norm):
            decision = RouteDecision("direct", source="private_ip")
            self._emit(host_norm, port, decision)
            return decision

        entry = self.cache.get(host_norm)
        if entry is not None:
            decision = RouteDecision(entry.direction, source="cache", cache_entry=entry)
            self._emit(host_norm, port, decision)
            return decision

        for pattern, pat_entry in self.cache.iter_patterns():
            if _pattern_match(pattern, host_norm):
                copied = self.cache.set_direction(
                    host_norm,
                    pat_entry.direction,
                    source=pat_entry.source,
                )
                decision = RouteDecision(
                    pat_entry.direction, source="pattern", cache_entry=copied
                )
                self._emit(host_norm, port, decision)
                return decision

        ok = await tcp_probe(host_norm, port, timeout=self.probe_timeout)
        direction: Direction = "direct" if ok else "proxy"
        memo = self.cache.set_direction(host_norm, direction, source="probe")
        logger.debug(
            "route_resolver: probe %s:%d -> %s (cache memoised)",
            host_norm, port, direction,
        )
        decision = RouteDecision(direction, source="probe", cache_entry=memo)
        self._emit(host_norm, port, decision)
        return decision

    # ------------------------------------------------------------------
    # self-heal hooks (called by local_proxy on actual connect failure)
    # ------------------------------------------------------------------

    def mark_direct_failed(self, host: str, port: int) -> RouteDecision:
        """Flip a stale ``direct`` cache entry to ``proxy`` and retry."""
        memo = self.cache.set_direction(host, "proxy", source="probe")
        logger.info(
            "route_resolver: self-heal %s:%d direct->proxy", host, port,
        )
        return RouteDecision("proxy", source="self_heal", cache_entry=memo)

    def mark_proxy_failed(self, host: str, port: int) -> None:
        """Last-resort: even ``proxy`` failed.  We invalidate so the
        next request re-probes from scratch."""
        self.cache.invalidate(host)
        logger.warning(
            "route_resolver: proxy path failed for %s:%d, cache invalidated",
            host, port,
        )


__all__ = [
    "DEFAULT_PROBE_TIMEOUT",
    "GlobalMode",
    "RouteDecision",
    "RouteResolver",
    "tcp_probe",
]
