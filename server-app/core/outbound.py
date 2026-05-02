"""Smart outbound connector — DIRECT first, VPN fallback.

Why this exists:
    On split-tunnel VPN setups (e.g. GlobalProtect with two ``default`` routes
    via ``utun4`` and ``en0``), unbound sockets follow the VPN default. Binding
    a socket to the physical-LAN source IP forces the kernel to use the
    interface-scoped (``Ig``) route, which goes out through the ISP instead.

    This module wraps ``asyncio.open_connection`` so that *every* outbound
    request first tries the ISP path; if it does not establish quickly, the
    VPN path is also raced. The winner is used and remembered for a short
    TTL, so subsequent connections to the same host skip the race.

Public API:
    ``open_with_fallback(host, port, cfg) -> (reader, writer, route_label)``

Behaviour summary:
    1. Cache hit → use cached route only.
    2. Cache miss → start DIRECT (bind to ``cfg.physical_iface_ip``) immediately.
    3. After ``cfg.direct_first_timeout_s`` (default 1.5s), if DIRECT is still
       pending or has just failed, also start the VPN path. First success wins.
    4. Cache the winner for ``cfg.direct_cache_ttl_s`` seconds.
    5. If a cached path errors out at connect time, the cache entry is dropped
       and a fresh race is run (single retry).
"""
from __future__ import annotations

import asyncio
import logging
import time
from dataclasses import dataclass
from typing import Optional

log = logging.getLogger("outbound")

ROUTE_DIRECT = "direct"
ROUTE_VPN = "vpn"
ROUTE_DIRECT_ONLY = "direct-only"
ROUTE_VPN_ONLY = "vpn-only"

POLICY_AUTO = "auto"
POLICY_DIRECT = "direct"
POLICY_VPN = "vpn"


@dataclass
class _CacheEntry:
    route: str
    expires_at: float


class RouteCache:
    """Per-host outbound route cache. Process-local, asyncio-safe."""

    def __init__(self) -> None:
        self._data: dict[str, _CacheEntry] = {}

    def get(self, host: str) -> Optional[str]:
        e = self._data.get(host.lower())
        if not e:
            return None
        if e.expires_at < time.time():
            self._data.pop(host.lower(), None)
            return None
        return e.route

    def put(self, host: str, route: str, ttl_s: float) -> None:
        self._data[host.lower()] = _CacheEntry(route, time.time() + ttl_s)

    def invalidate(self, host: str) -> None:
        self._data.pop(host.lower(), None)

    def snapshot(self) -> list[dict]:
        now = time.time()
        return [
            {
                "host": h,
                "route": e.route,
                "ttl_remaining_s": max(0, int(e.expires_at - now)),
            }
            for h, e in self._data.items()
            if e.expires_at >= now
        ]

    def clear(self) -> int:
        n = len(self._data)
        self._data.clear()
        return n


cache = RouteCache()


async def _connect_direct(host: str, port: int, src_ip: str, timeout_s: float):
    return await asyncio.wait_for(
        asyncio.open_connection(host, port, local_addr=(src_ip, 0)),
        timeout=timeout_s,
    )


async def _connect_vpn(host: str, port: int, timeout_s: float):
    return await asyncio.wait_for(
        asyncio.open_connection(host, port),
        timeout=timeout_s,
    )


async def _close_writer(writer: Optional[asyncio.StreamWriter]) -> None:
    if writer is None:
        return
    try:
        writer.close()
        await writer.wait_closed()
    except Exception:
        pass


async def _race(
    host: str,
    port: int,
    cfg,
) -> tuple[asyncio.StreamReader, asyncio.StreamWriter, str]:
    """Happy-eyeballs style race: DIRECT first, VPN joins after head-start."""
    src_ip = cfg.physical_iface_ip
    head_start = max(0.0, cfg.direct_first_timeout_s)
    overall_timeout = cfg.connect_timeout_s

    direct_task = asyncio.create_task(
        _connect_direct(host, port, src_ip, overall_timeout)
    )

    try:
        result = await asyncio.wait_for(asyncio.shield(direct_task), head_start)
        reader, writer = result
        return reader, writer, ROUTE_DIRECT
    except asyncio.TimeoutError:
        pass
    except (OSError, asyncio.CancelledError) as exc:
        log.debug("direct head-start failed for %s:%d (%s); going VPN-first", host, port, exc)

    vpn_task = asyncio.create_task(_connect_vpn(host, port, overall_timeout))

    pending = {t for t in (direct_task, vpn_task) if not t.done()}
    finished_tasks: list[asyncio.Task] = list({direct_task, vpn_task} - pending)

    while pending:
        done, pending = await asyncio.wait(
            pending, return_when=asyncio.FIRST_COMPLETED
        )
        finished_tasks.extend(done)
        winner = None
        for t in done:
            try:
                t.result()
            except BaseException:
                continue
            winner = t
            break
        if winner is None:
            continue

        for t in pending:
            t.cancel()
        if pending:
            results = await asyncio.gather(*pending, return_exceptions=True)
            for r in results:
                if isinstance(r, tuple) and len(r) == 2:
                    await _close_writer(r[1])

        for other in finished_tasks:
            if other is winner:
                continue
            try:
                r, w = other.result()
                await _close_writer(w)
            except BaseException:
                pass

        reader, writer = winner.result()
        route = ROUTE_DIRECT if winner is direct_task else ROUTE_VPN
        return reader, writer, route

    errors: list[str] = []
    for t in (direct_task, vpn_task):
        try:
            t.result()
        except BaseException as exc:
            errors.append(f"{type(exc).__name__}: {exc}")
    raise OSError(f"both routes failed for {host}:{port}: {'; '.join(errors)}")


async def open_with_fallback(
    host: str,
    port: int,
    cfg,
    policy: str = POLICY_AUTO,
) -> tuple[asyncio.StreamReader, asyncio.StreamWriter, str]:
    """Open a TCP connection to ``host:port`` according to ``policy``.

    policy:
        ``"auto"``    — default; cache → race (DIRECT first, VPN fallback).
        ``"direct"``  — bind to ``cfg.physical_iface_ip`` only; raises if it fails.
        ``"vpn"``     — default-route only; never bind to physical iface.

    The master switch ``cfg.direct_first`` (default ``True``) only affects
    ``"auto"``. Setting ``cfg.direct_first = False`` (or running with
    ``--no-direct-first``) is equivalent to forcing ``"vpn"`` policy on every
    call — every outbound connection goes through the default route.
    """
    if policy == POLICY_VPN or not getattr(cfg, "direct_first", True) \
            or not cfg.physical_iface_ip:
        reader, writer = await _connect_vpn(host, port, cfg.connect_timeout_s)
        return reader, writer, ROUTE_VPN_ONLY

    if policy == POLICY_DIRECT:
        reader, writer = await _connect_direct(
            host, port, cfg.physical_iface_ip, cfg.connect_timeout_s
        )
        return reader, writer, ROUTE_DIRECT_ONLY

    cached = cache.get(host)
    if cached == ROUTE_DIRECT:
        try:
            reader, writer = await _connect_direct(
                host, port, cfg.physical_iface_ip, cfg.connect_timeout_s
            )
            return reader, writer, ROUTE_DIRECT
        except (OSError, asyncio.TimeoutError) as exc:
            log.info("cached DIRECT failed for %s:%d (%s); invalidating + re-racing",
                     host, port, type(exc).__name__)
            cache.invalidate(host)
    elif cached == ROUTE_VPN:
        try:
            reader, writer = await _connect_vpn(host, port, cfg.connect_timeout_s)
            return reader, writer, ROUTE_VPN
        except (OSError, asyncio.TimeoutError) as exc:
            log.info("cached VPN failed for %s:%d (%s); invalidating + re-racing",
                     host, port, type(exc).__name__)
            cache.invalidate(host)

    reader, writer, route = await _race(host, port, cfg)
    cache.put(host, route, cfg.direct_cache_ttl_s)
    log.debug("route decided for %s:%d -> %s (cached %ds)",
              host, port, route, cfg.direct_cache_ttl_s)
    return reader, writer, route


def policy_from_pac_section(matched_section: str) -> str:
    """Map a PAC engine ``Decision.matched_section`` to an outbound policy.

    PAC section labels (defined in pac_engine.find_proxy):
        "1. local/private"                                          → DIRECT only
        "2. internal (must use VPN)"                                → VPN only
        "3. may need VPN (proxy first, DIRECT fallback)"            → VPN only
            (using VPN avoids the "TCP-handshakes-but-TLS-RST" trap)
        "4. CN direct"                                              → DIRECT only
        "5. default"                                                → AUTO race
    """
    section = (matched_section or "").strip().lower()
    if section.startswith("1."):
        return POLICY_DIRECT
    if section.startswith("2.") or section.startswith("3."):
        return POLICY_VPN
    if section.startswith("4."):
        return POLICY_DIRECT
    return POLICY_AUTO
