"""Route cache for the Conduit client-app smart local proxy.

Stores per-host routing decisions (`direct` / `proxy`) with TTL + LRU eviction.

The cache plays three roles:

* **Pre-fill seed (source='pac')** — at startup we fetch the server's
  ``proxy.pac`` and pre-populate every "must-go-via-server" host as
  ``proxy``, so we never waste a probe on known VPN domains.

* **Probe memo (source='probe')** — once the resolver has run a TCP probe,
  the result is cached for ``DEFAULT_TTL`` (5 min) so the next request
  for the same host is essentially free.

* **Self-heal store** — when an in-flight ``direct`` connection actually
  fails (TCP refused / timeout), the SOCKS5 layer asks the cache to
  flip the entry to ``proxy`` and retries.  The next user gets the
  healed answer immediately.

Cross-references:
* design/2026-04-30-3-Conduit-Client-客户端可行性报告.md §3.5
"""
from __future__ import annotations

from collections import OrderedDict
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from threading import RLock
from typing import Iterable, Iterator, Literal

Direction = Literal["direct", "proxy"]
Source = Literal["pac", "probe", "manual"]


def _utcnow() -> datetime:
    return datetime.now(timezone.utc)


@dataclass
class RouteEntry:
    host: str
    direction: Direction
    expires_at: datetime
    source: Source
    hit_count: int = 0
    last_used: datetime = field(default_factory=_utcnow)

    def expired(self, *, now: datetime | None = None) -> bool:
        ref = now or _utcnow()
        return ref >= self.expires_at

    def touch(self) -> None:
        self.hit_count += 1
        self.last_used = _utcnow()

    def to_dict(self) -> dict[str, object]:
        return {
            "host": self.host,
            "direction": self.direction,
            "source": self.source,
            "hit_count": self.hit_count,
            "expires_at": self.expires_at.isoformat(),
            "last_used": self.last_used.isoformat(),
            "ttl_remaining_sec": max(
                0, int((self.expires_at - _utcnow()).total_seconds())
            ),
        }


@dataclass
class CacheStats:
    total: int
    direct_count: int
    proxy_count: int
    expired_count: int
    by_source: dict[Source, int]
    hits: int
    misses: int
    evictions: int


class RouteCache:
    """Thread-safe LRU + TTL cache mapping ``host -> RouteEntry``.

    The implementation uses a single ``RLock`` rather than asyncio
    primitives because:

    * the cache is touched both from the SOCKS5 event loop and from
      the control HTTP API request handlers;
    * every operation is O(1) and finishes well under a microsecond,
      so contention is not a concern.
    """

    DEFAULT_TTL = timedelta(minutes=5)
    MAX_ENTRIES = 5000

    def __init__(
        self,
        *,
        default_ttl: timedelta | None = None,
        max_entries: int | None = None,
    ) -> None:
        self._ttl = default_ttl or self.DEFAULT_TTL
        self._max = max_entries or self.MAX_ENTRIES
        self._store: "OrderedDict[str, RouteEntry]" = OrderedDict()
        self._lock = RLock()
        self._hits = 0
        self._misses = 0
        self._evictions = 0

    # ------------------------------------------------------------------
    # core operations
    # ------------------------------------------------------------------

    def get(self, host: str) -> RouteEntry | None:
        """Return a non-expired entry, bumping LRU + hit_count.

        Expired entries are silently removed.
        """
        host = self._key(host)
        with self._lock:
            entry = self._store.get(host)
            if entry is None:
                self._misses += 1
                return None
            if entry.expired():
                self._store.pop(host, None)
                self._misses += 1
                return None
            entry.touch()
            self._store.move_to_end(host)
            self._hits += 1
            return entry

    def set(self, host: str, entry: RouteEntry) -> None:
        """Insert / overwrite an entry. Evicts oldest if over capacity."""
        host = self._key(host)
        if entry.host != host:
            entry.host = host
        with self._lock:
            if host in self._store:
                self._store.pop(host)
            self._store[host] = entry
            self._evict_if_needed()

    def set_direction(
        self,
        host: str,
        direction: Direction,
        *,
        source: Source = "probe",
        ttl: timedelta | None = None,
    ) -> RouteEntry:
        """Convenience wrapper to build + insert an entry in one call."""
        ttl = ttl or self._ttl
        entry = RouteEntry(
            host=self._key(host),
            direction=direction,
            expires_at=_utcnow() + ttl,
            source=source,
        )
        self.set(host, entry)
        return entry

    def invalidate(self, host: str) -> bool:
        host = self._key(host)
        with self._lock:
            return self._store.pop(host, None) is not None

    def set_pattern(self, host_pattern: str, entry: RouteEntry) -> None:
        """Store a wildcard-style pattern (``*.zoom.us`` / ``zoom.us``).

        The cache is *exact-match* — patterns live next to literal hosts
        but lookups never wildcard-match here.  Pattern matching is done
        upstream by ``route_resolver`` against ``iter_patterns()``.
        """
        self.set(host_pattern, entry)

    # ------------------------------------------------------------------
    # bulk operations
    # ------------------------------------------------------------------

    def flush_all(self) -> int:
        with self._lock:
            n = len(self._store)
            self._store.clear()
            return n

    def flush_proxy_entries(self) -> int:
        """Remove every cached ``proxy`` decision (used on global downgrade).

        Called by ``connectivity.py`` when the server has been unreachable
        for >= 3 heartbeats — at that point any future attempt to forward
        through it would just hang, so we wipe the proxy hints and let
        new requests fall back to ``direct``.
        """
        with self._lock:
            removed = [k for k, v in self._store.items() if v.direction == "proxy"]
            for k in removed:
                self._store.pop(k, None)
            return len(removed)

    def evict_expired(self) -> int:
        with self._lock:
            now = _utcnow()
            removed = [k for k, v in self._store.items() if v.expired(now=now)]
            for k in removed:
                self._store.pop(k, None)
            return len(removed)

    # ------------------------------------------------------------------
    # introspection
    # ------------------------------------------------------------------

    def __len__(self) -> int:
        with self._lock:
            return len(self._store)

    def __contains__(self, host: object) -> bool:
        if not isinstance(host, str):
            return False
        with self._lock:
            entry = self._store.get(self._key(host))
            return entry is not None and not entry.expired()

    def items(self) -> list[tuple[str, RouteEntry]]:
        with self._lock:
            return list(self._store.items())

    def iter_patterns(self) -> Iterator[tuple[str, RouteEntry]]:
        """Yield entries whose key looks like a wildcard pattern.

        The resolver consults this when an exact ``get(host)`` misses,
        so PAC pre-fills like ``*.zoom.us`` are still honoured.
        """
        with self._lock:
            for host, entry in list(self._store.items()):
                if "*" in host or host.startswith("."):
                    yield host, entry

    def stats(self) -> CacheStats:
        with self._lock:
            direct = sum(1 for e in self._store.values() if e.direction == "direct")
            proxy = sum(1 for e in self._store.values() if e.direction == "proxy")
            expired = sum(1 for e in self._store.values() if e.expired())
            by_source: dict[Source, int] = {"pac": 0, "probe": 0, "manual": 0}
            for e in self._store.values():
                by_source[e.source] = by_source.get(e.source, 0) + 1
            return CacheStats(
                total=len(self._store),
                direct_count=direct,
                proxy_count=proxy,
                expired_count=expired,
                by_source=by_source,
                hits=self._hits,
                misses=self._misses,
                evictions=self._evictions,
            )

    # ------------------------------------------------------------------
    # internals
    # ------------------------------------------------------------------

    @staticmethod
    def _key(host: str) -> str:
        return host.strip().lower()

    def _evict_if_needed(self) -> None:
        while len(self._store) > self._max:
            self._store.popitem(last=False)
            self._evictions += 1


def build_default_cache() -> RouteCache:
    """Factory used by ``client_main`` and tests."""
    return RouteCache()


__all__ = [
    "Direction",
    "Source",
    "RouteEntry",
    "RouteCache",
    "CacheStats",
    "build_default_cache",
]
