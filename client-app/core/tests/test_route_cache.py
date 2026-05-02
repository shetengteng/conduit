"""Tests for client-app route_cache.

Covers the contracts spelled out in
design/2026-04-30-3-Conduit-Client-客户端可行性报告.md §3.5:

* ``get`` returns the entry, bumps ``hit_count`` / ``last_used``, and
  refreshes LRU ordering;
* expired entries are dropped on access (and not counted as hits);
* ``flush_proxy_entries`` removes *only* ``proxy`` rows so we can
  cleanly downgrade to direct mode without losing useful direct hints;
* the LRU cap is honoured;
* pattern-style hosts are exposed via ``iter_patterns()``.
"""
from __future__ import annotations

import time
from datetime import timedelta

from route_cache import RouteCache, RouteEntry, _utcnow, build_default_cache


def _entry(host: str, direction: str, *, ttl_sec: int = 300, source: str = "probe") -> RouteEntry:
    return RouteEntry(
        host=host,
        direction=direction,  # type: ignore[arg-type]
        expires_at=_utcnow() + timedelta(seconds=ttl_sec),
        source=source,  # type: ignore[arg-type]
    )


def test_set_then_get_returns_entry_and_bumps_hit_count():
    c = RouteCache()
    c.set("baidu.com", _entry("baidu.com", "direct"))

    e1 = c.get("baidu.com")
    assert e1 is not None
    assert e1.direction == "direct"
    assert e1.hit_count == 1

    e2 = c.get("baidu.com")
    assert e2 is e1
    assert e2.hit_count == 2


def test_get_miss_returns_none_and_increments_miss_counter():
    c = RouteCache()
    assert c.get("nope.example") is None
    s = c.stats()
    assert s.hits == 0 and s.misses == 1


def test_expired_entry_is_dropped_on_access():
    c = RouteCache()
    e = _entry("git.zoom.us", "proxy", ttl_sec=0)
    c.set("git.zoom.us", e)

    assert c.get("git.zoom.us") is None
    assert "git.zoom.us" not in c
    assert len(c) == 0


def test_evict_expired_bulk():
    c = RouteCache()
    c.set("a", _entry("a", "direct", ttl_sec=0))
    c.set("b", _entry("b", "proxy", ttl_sec=0))
    c.set("c", _entry("c", "direct", ttl_sec=600))

    n = c.evict_expired()
    assert n == 2
    assert len(c) == 1


def test_lru_eviction_when_over_capacity():
    c = RouteCache(max_entries=3)
    c.set("a", _entry("a", "direct"))
    c.set("b", _entry("b", "direct"))
    c.set("c", _entry("c", "direct"))
    c.get("a")  # bump 'a' to most-recent
    c.set("d", _entry("d", "direct"))  # should evict 'b' (now oldest)

    assert "a" in c
    assert "b" not in c
    assert "c" in c
    assert "d" in c
    assert c.stats().evictions == 1


def test_invalidate_removes_entry():
    c = RouteCache()
    c.set("baidu.com", _entry("baidu.com", "direct"))
    assert c.invalidate("baidu.com") is True
    assert c.invalidate("baidu.com") is False
    assert c.get("baidu.com") is None


def test_flush_proxy_entries_only_removes_proxy():
    c = RouteCache()
    c.set("git.zoom.us", _entry("git.zoom.us", "proxy"))
    c.set("source.zoom.us", _entry("source.zoom.us", "proxy"))
    c.set("baidu.com", _entry("baidu.com", "direct"))

    n = c.flush_proxy_entries()
    assert n == 2
    assert "baidu.com" in c
    assert "git.zoom.us" not in c


def test_flush_all_clears_everything():
    c = RouteCache()
    c.set("a", _entry("a", "direct"))
    c.set("b", _entry("b", "proxy"))
    assert c.flush_all() == 2
    assert len(c) == 0


def test_iter_patterns_only_yields_wildcards():
    c = RouteCache()
    c.set("baidu.com", _entry("baidu.com", "direct"))
    c.set("*.zoom.us", _entry("*.zoom.us", "proxy", source="pac"))
    c.set(".zoomdev.us", _entry(".zoomdev.us", "proxy", source="pac"))

    patterns = sorted(k for k, _ in c.iter_patterns())
    assert patterns == ["*.zoom.us", ".zoomdev.us"]


def test_set_direction_helper_persists_entry():
    c = RouteCache()
    e = c.set_direction("example.com", "proxy", source="manual")
    assert c.get("example.com") is not None
    assert e.source == "manual"
    assert e.direction == "proxy"


def test_keys_are_normalised_lowercase():
    c = RouteCache()
    c.set("Baidu.COM", _entry("Baidu.COM", "direct"))
    assert c.get("baidu.com") is not None
    assert c.get("BAIDU.com") is not None


def test_stats_reflects_cache_contents():
    c = RouteCache()
    c.set("a", _entry("a", "direct", source="probe"))
    c.set("b", _entry("b", "proxy", source="pac"))
    c.set("c", _entry("c", "proxy", source="manual"))

    c.get("a")           # +1 hit
    c.get("missing-x")   # +1 miss
    s = c.stats()

    assert s.total == 3
    assert s.direct_count == 1
    assert s.proxy_count == 2
    assert s.by_source["probe"] == 1
    assert s.by_source["pac"] == 1
    assert s.by_source["manual"] == 1
    assert s.hits == 1
    assert s.misses == 1


def test_default_factory_returns_independent_caches():
    a = build_default_cache()
    b = build_default_cache()
    a.set("x", _entry("x", "direct"))
    assert "x" in a
    assert "x" not in b


def test_short_ttl_entry_expires_after_real_sleep():
    c = RouteCache()
    c.set("flaky.example", _entry("flaky.example", "direct", ttl_sec=1))
    assert c.get("flaky.example") is not None
    time.sleep(1.05)
    assert c.get("flaky.example") is None
