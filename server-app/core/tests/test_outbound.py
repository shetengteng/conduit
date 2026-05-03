"""出站连接策略 + RouteCache 单元测试 —— outbound.py。

不触发真实网络:
  - RouteCache 全部用模拟时间或瞬时 ttl
  - policy_from_pac_section 是纯映射函数
  - open_with_fallback 跑 policy=direct/vpn 路径,_connect_direct/_connect_vpn 用 monkeypatch 替换
"""
from __future__ import annotations

import asyncio
import time
from dataclasses import dataclass

import pytest

from outbound import (
    POLICY_AUTO,
    POLICY_DIRECT,
    POLICY_VPN,
    ROUTE_DIRECT,
    ROUTE_DIRECT_ONLY,
    ROUTE_VPN,
    ROUTE_VPN_ONLY,
    RouteCache,
    open_with_fallback,
    policy_from_pac_section,
)


# ---------- RouteCache ----------

def test_route_cache_get_miss_returns_none():
    c = RouteCache()
    assert c.get("example.com") is None


def test_route_cache_put_then_get_hits():
    c = RouteCache()
    c.put("example.com", ROUTE_DIRECT, ttl_s=10)
    assert c.get("example.com") == ROUTE_DIRECT


def test_route_cache_get_lower_cases_host():
    c = RouteCache()
    c.put("Example.COM", ROUTE_VPN, ttl_s=10)
    assert c.get("example.com") == ROUTE_VPN


def test_route_cache_expired_entries_are_dropped(monkeypatch):
    c = RouteCache()
    base = 1000.0
    monkeypatch.setattr(time, "time", lambda: base)
    c.put("example.com", ROUTE_DIRECT, ttl_s=5)
    monkeypatch.setattr(time, "time", lambda: base + 6)
    assert c.get("example.com") is None
    assert "example.com" not in c._data


def test_route_cache_invalidate_removes_entry():
    c = RouteCache()
    c.put("a.com", ROUTE_DIRECT, ttl_s=10)
    c.invalidate("A.COM")
    assert c.get("a.com") is None


def test_route_cache_snapshot_excludes_expired(monkeypatch):
    c = RouteCache()
    base = 1000.0
    monkeypatch.setattr(time, "time", lambda: base)
    c.put("alive.com", ROUTE_DIRECT, ttl_s=10)
    c.put("dead.com", ROUTE_VPN, ttl_s=1)
    monkeypatch.setattr(time, "time", lambda: base + 5)
    snap = c.snapshot()
    hosts = {row["host"] for row in snap}
    assert hosts == {"alive.com"}
    assert snap[0]["route"] == ROUTE_DIRECT
    assert snap[0]["ttl_remaining_s"] >= 0


def test_route_cache_clear_returns_count_and_empties():
    c = RouteCache()
    c.put("a.com", ROUTE_DIRECT, 10)
    c.put("b.com", ROUTE_VPN, 10)
    assert c.clear() == 2
    assert c.snapshot() == []


# ---------- policy_from_pac_section ----------

@pytest.mark.parametrize("section,expected", [
    ("1. local/private", POLICY_DIRECT),
    ("2. internal (must use VPN)", POLICY_VPN),
    ("3. may need VPN (proxy first, DIRECT fallback)", POLICY_VPN),
    ("4. CN direct", POLICY_DIRECT),
    ("5. default", POLICY_AUTO),
    ("", POLICY_AUTO),
    (None, POLICY_AUTO),
    ("99. unknown", POLICY_AUTO),
])
def test_policy_from_pac_section(section, expected):
    assert policy_from_pac_section(section) == expected


# ---------- open_with_fallback ----------

@dataclass
class _FakeCfg:
    physical_iface_ip: str = "127.0.0.1"
    direct_first_timeout_s: float = 1.5
    direct_cache_ttl_s: int = 30
    connect_timeout_s: float = 5
    direct_first: bool = True


class _FakeReader:
    pass


class _FakeWriter:
    def __init__(self) -> None:
        self.closed = False

    def close(self) -> None:
        self.closed = True

    async def wait_closed(self) -> None:
        return None


@pytest.mark.asyncio
async def test_open_with_fallback_policy_direct_uses_local_addr(monkeypatch):
    seen: dict = {}

    async def fake_direct(host, port, src_ip, timeout):
        seen["direct"] = (host, port, src_ip, timeout)
        return _FakeReader(), _FakeWriter()

    async def fake_vpn(host, port, timeout):
        pytest.fail("policy=direct should never call vpn connector")

    import outbound as out
    monkeypatch.setattr(out, "_connect_direct", fake_direct)
    monkeypatch.setattr(out, "_connect_vpn", fake_vpn)

    cfg = _FakeCfg(physical_iface_ip="127.0.0.1")
    r, w, route = await open_with_fallback("h", 80, cfg, policy=POLICY_DIRECT)
    assert route == ROUTE_DIRECT_ONLY
    assert seen["direct"] == ("h", 80, "127.0.0.1", cfg.connect_timeout_s)


@pytest.mark.asyncio
async def test_open_with_fallback_policy_vpn_skips_local_addr(monkeypatch):
    async def fake_vpn(host, port, timeout):
        return _FakeReader(), _FakeWriter()

    async def fake_direct(*a, **kw):
        pytest.fail("policy=vpn should never call direct connector")

    import outbound as out
    monkeypatch.setattr(out, "_connect_direct", fake_direct)
    monkeypatch.setattr(out, "_connect_vpn", fake_vpn)

    cfg = _FakeCfg()
    r, w, route = await open_with_fallback("h", 80, cfg, policy=POLICY_VPN)
    assert route == ROUTE_VPN_ONLY


@pytest.mark.asyncio
async def test_open_with_fallback_no_physical_ip_falls_back_to_vpn(monkeypatch):
    async def fake_vpn(host, port, timeout):
        return _FakeReader(), _FakeWriter()

    import outbound as out
    monkeypatch.setattr(out, "_connect_vpn", fake_vpn)

    cfg = _FakeCfg(physical_iface_ip="")
    r, w, route = await open_with_fallback("h", 80, cfg, policy=POLICY_AUTO)
    assert route == ROUTE_VPN_ONLY


@pytest.mark.asyncio
async def test_open_with_fallback_disabled_direct_first_falls_back_to_vpn(monkeypatch):
    async def fake_vpn(host, port, timeout):
        return _FakeReader(), _FakeWriter()

    import outbound as out
    monkeypatch.setattr(out, "_connect_vpn", fake_vpn)

    cfg = _FakeCfg(direct_first=False)
    r, w, route = await open_with_fallback("h", 80, cfg, policy=POLICY_AUTO)
    assert route == ROUTE_VPN_ONLY


@pytest.mark.asyncio
async def test_open_with_fallback_cached_direct_path_is_used(monkeypatch):
    seen: dict = {}

    async def fake_direct(host, port, src_ip, timeout):
        seen["used"] = "direct"
        return _FakeReader(), _FakeWriter()

    async def fake_vpn(*a, **kw):
        seen["used"] = "vpn"
        return _FakeReader(), _FakeWriter()

    import outbound as out
    monkeypatch.setattr(out, "_connect_direct", fake_direct)
    monkeypatch.setattr(out, "_connect_vpn", fake_vpn)
    out.cache.put("cached.example.com", ROUTE_DIRECT, ttl_s=10)
    try:
        cfg = _FakeCfg()
        r, w, route = await open_with_fallback("cached.example.com", 80, cfg, policy=POLICY_AUTO)
        assert route == ROUTE_DIRECT
        assert seen["used"] == "direct"
    finally:
        out.cache.clear()


@pytest.mark.asyncio
async def test_open_with_fallback_cached_direct_failure_invalidates_and_races(monkeypatch):
    """缓存的 DIRECT 失败时,应当 invalidate 然后回到 race 路径(并能成功)。"""
    direct_calls = {"n": 0}

    async def fake_direct(host, port, src_ip, timeout):
        direct_calls["n"] += 1
        if direct_calls["n"] == 1:
            raise OSError("simulated direct failure")
        return _FakeReader(), _FakeWriter()

    async def fake_vpn(host, port, timeout):
        # 在 race 阶段,delay 一下让 direct 先成功
        await asyncio.sleep(10)
        return _FakeReader(), _FakeWriter()

    import outbound as out
    monkeypatch.setattr(out, "_connect_direct", fake_direct)
    monkeypatch.setattr(out, "_connect_vpn", fake_vpn)
    out.cache.put("flaky.example.com", ROUTE_DIRECT, ttl_s=10)
    try:
        cfg = _FakeCfg(direct_first_timeout_s=0.05)
        r, w, route = await open_with_fallback("flaky.example.com", 80, cfg, policy=POLICY_AUTO)
        assert route == ROUTE_DIRECT
        # 失败一次后被 invalidate,然后 _race 又跑了一次 direct -> 总共 2 次
        assert direct_calls["n"] == 2
    finally:
        out.cache.clear()
