"""单元测试：Discoverer mDNS + 历史持久化。

测试策略：
- 不真起 zeroconf（CI 没多播也能跑），改用单元注入：
    1. 直接构造一个假的 ServiceInfo（namedtuple-ish）喂 _service_info_to_server，
       验证 TXT 解析 / DiscoveredServer 字段映射。
    2. 直接调用 _on_service_added_or_updated 的内部分支：
       因为它依赖 AsyncServiceInfo.async_request，我们改为单独测纯函数 +
       通过 snapshot() 验证状态机。
    3. 持久化路径：直接调 _save_history / _load_history。
"""
from __future__ import annotations

import asyncio
import json
import socket
import time
from pathlib import Path

import pytest

from discoverer import (
    DiscoveredServer,
    Discoverer,
    SERVICE_TYPE,
    _load_history,
    _save_history,
    server_to_payload,
)
from events_bus import EventBus


# ---------------------------------------------------------------------------
# DiscoveredServer 数据类与序列化
# ---------------------------------------------------------------------------


def test_discovered_server_pac_url():
    ds = DiscoveredServer(
        server_id="alpha@10.0.0.1:8080",
        name="alpha",
        host="10.0.0.1",
        port=8080,
        socks=1080,
        api=9090,
        vpn=False,
        version="0.1.0",
        pac="/proxy.pac",
        source="mdns",
        last_seen_at=1714665600.0,
    )
    assert ds.pac_url == "http://10.0.0.1:8080/proxy.pac"


def test_server_to_payload_has_all_keys():
    ds = DiscoveredServer(
        server_id="x@1.2.3.4:80",
        name="x", host="1.2.3.4", port=80, socks=1, api=2,
        vpn=True, version="0.1.0", pac="/proxy.pac",
        source="mdns", last_seen_at=1.0, healthy=True,
    )
    payload = server_to_payload(ds)
    expected = {
        "server_id", "name", "host", "port", "socks", "api",
        "vpn", "version", "pac", "pac_url", "source", "last_seen_at", "healthy",
    }
    assert expected.issubset(payload.keys())
    assert payload["pac_url"] == "http://1.2.3.4:80/proxy.pac"


# ---------------------------------------------------------------------------
# 持久化（known-servers.json）
# ---------------------------------------------------------------------------


def test_save_and_load_history_roundtrip(tmp_path: Path):
    path = tmp_path / "known.json"
    items = [
        DiscoveredServer(
            server_id="a@1.1.1.1:80",
            name="a", host="1.1.1.1", port=80, socks=1, api=2,
            vpn=False, version="0.1.0", pac="/proxy.pac",
            source="mdns", last_seen_at=time.time(), healthy=True,
        ),
        DiscoveredServer(
            server_id="b@2.2.2.2:8080",
            name="b", host="2.2.2.2", port=8080, socks=1080, api=9090,
            vpn=True, version="0.1.1", pac="/proxy.pac",
            source="mdns", last_seen_at=time.time(), healthy=True,
        ),
    ]
    _save_history(path, items)
    assert path.exists()

    raw = json.loads(path.read_text(encoding="utf-8"))
    assert len(raw) == 2
    # 写入时 source 强制 history
    assert all(it["source"] == "history" for it in raw)

    loaded = _load_history(path)
    assert len(loaded) == 2
    assert {it.server_id for it in loaded} == {"a@1.1.1.1:80", "b@2.2.2.2:8080"}
    assert all(it.source == "history" for it in loaded)
    assert all(it.healthy is False for it in loaded)


def test_load_history_returns_empty_on_missing(tmp_path: Path):
    assert _load_history(tmp_path / "nope.json") == []


def test_load_history_skips_malformed(tmp_path: Path):
    path = tmp_path / "broken.json"
    path.write_text(
        json.dumps([
            {"server_id": "ok@1.1.1.1:80", "name": "ok", "host": "1.1.1.1",
             "port": 80, "socks": 1, "api": 2, "vpn": False,
             "version": "0.1.0", "pac": "/proxy.pac", "last_seen_at": 1.0},
            {"missing": "fields"},  # 这一条该被 skip
        ]),
        encoding="utf-8",
    )
    loaded = _load_history(path)
    assert len(loaded) == 1
    assert loaded[0].server_id == "ok@1.1.1.1:80"


# ---------------------------------------------------------------------------
# _service_info_to_server: TXT 解析正确性
# ---------------------------------------------------------------------------


class _FakeServiceInfo:
    """zeroconf.ServiceInfo 的 duck-typed 替身（仅含 _service_info_to_server 用到的字段）。"""

    def __init__(self, name, addresses, port, properties):
        self.name = name
        self.addresses = addresses
        self.port = port
        self.properties = properties


@pytest.fixture
def discoverer(tmp_path):
    bus = EventBus()
    return Discoverer(bus, storage_path=tmp_path / "known.json")


def test_service_info_to_server_parses_txt(discoverer):
    info = _FakeServiceInfo(
        name="Conduit on alpha._conduit._tcp.local.",
        addresses=[socket.inet_aton("192.168.1.20")],
        port=8080,
        properties={
            b"name": b"alpha",
            b"port": b"8080",
            b"socks": b"1080",
            b"api": b"9090",
            b"vpn": b"on",
            b"version": b"0.1.0",
            b"pac": b"/proxy.pac",
        },
    )
    ds = discoverer._service_info_to_server(info)
    assert ds is not None
    assert ds.server_id == "alpha@192.168.1.20:8080"
    assert ds.name == "alpha"
    assert ds.host == "192.168.1.20"
    assert ds.port == 8080
    assert ds.socks == 1080
    assert ds.api == 9090
    assert ds.vpn is True
    assert ds.version == "0.1.0"
    assert ds.pac == "/proxy.pac"
    assert ds.source == "mdns"


def test_service_info_to_server_skips_no_address(discoverer):
    info = _FakeServiceInfo(
        name="x._conduit._tcp.local.",
        addresses=[],
        port=80,
        properties={b"port": b"80"},
    )
    assert discoverer._service_info_to_server(info) is None


def test_service_info_to_server_skips_invalid_port(discoverer):
    info = _FakeServiceInfo(
        name="x._conduit._tcp.local.",
        addresses=[socket.inet_aton("10.0.0.1")],
        port=80,
        properties={b"port": b"not-a-number"},
    )
    assert discoverer._service_info_to_server(info) is None


def test_service_info_to_server_handles_ipv6_only_skips(discoverer):
    info = _FakeServiceInfo(
        name="x._conduit._tcp.local.",
        addresses=[b"\x00" * 16],  # 长度 16 = IPv6
        port=80,
        properties={b"port": b"80"},
    )
    assert discoverer._service_info_to_server(info) is None


def test_service_info_prefers_lan_over_loopback_in_same_record(discoverer):
    """单个 ServiceInfo 同时携带 127.x + 192.168.x 时,选 LAN。

    回归保护:zeroconf 在 multi-iface 场景下偶尔会把多个 IPv4 一次性塞进
    addresses;旧实现 break first 可能选到 127.0.0.1。
    """
    info = _FakeServiceInfo(
        name="Conduit on dual._conduit._tcp.local.",
        addresses=[
            socket.inet_aton("127.0.0.1"),       # loopback
            socket.inet_aton("192.168.1.20"),    # LAN, 应当优先
        ],
        port=8080,
        properties={
            b"name": b"dual",
            b"port": b"8080",
            b"socks": b"1080",
            b"api": b"9090",
            b"vpn": b"on",
            b"version": b"0.1.0",
        },
    )
    ds = discoverer._service_info_to_server(info)
    assert ds is not None
    assert ds.host == "192.168.1.20"
    assert ds.server_id == "dual@192.168.1.20:8080"


def test_prefer_host_ranks_lan_above_loopback(discoverer):
    """同名 + 同端口的 LAN entry 优先于 loopback entry。"""
    lan = DiscoveredServer(
        server_id="x@192.168.1.14:8080",
        name="x", host="192.168.1.14", port=8080, socks=1, api=2,
        vpn=True, version="0.1.0", pac="/proxy.pac",
        source="mdns", last_seen_at=1.0, healthy=True,
    )
    loop = DiscoveredServer(
        server_id="x@127.0.0.1:8080",
        name="x", host="127.0.0.1", port=8080, socks=1, api=2,
        vpn=True, version="0.1.0", pac="/proxy.pac",
        source="mdns", last_seen_at=2.0, healthy=True,
    )
    # _prefer_host(lan, loop) → lan(LAN 排第一档)
    assert Discoverer._prefer_host(lan, loop) is lan
    # _prefer_host(loop, lan) → lan
    assert Discoverer._prefer_host(loop, lan) is lan
    # 同档时取 b(更新)
    other_lan = DiscoveredServer(
        server_id="x@10.0.0.5:8080",
        name="x", host="10.0.0.5", port=8080, socks=1, api=2,
        vpn=True, version="0.1.0", pac="/proxy.pac",
        source="mdns", last_seen_at=3.0, healthy=True,
    )
    assert Discoverer._prefer_host(lan, other_lan) is other_lan


def test_prefer_host_172_private_range(discoverer):
    """172.16.0.0/12 是 RFC1918 私网,应当排在第 0 档。172.32+ 不是,排第 1 档。"""
    private_172 = DiscoveredServer(
        server_id="x@172.20.5.5:80", name="x", host="172.20.5.5", port=80,
        socks=0, api=0, vpn=False, version="", pac="/", source="mdns",
        last_seen_at=0, healthy=True,
    )
    public_172 = DiscoveredServer(
        server_id="x@172.40.5.5:80", name="x", host="172.40.5.5", port=80,
        socks=0, api=0, vpn=False, version="", pac="/", source="mdns",
        last_seen_at=0, healthy=True,
    )
    loopback = DiscoveredServer(
        server_id="x@127.0.0.1:80", name="x", host="127.0.0.1", port=80,
        socks=0, api=0, vpn=False, version="", pac="/", source="mdns",
        last_seen_at=0, healthy=True,
    )
    # 私网 172 优先于 loopback
    assert Discoverer._prefer_host(private_172, loopback) is private_172
    # 公网 172 优先于 loopback
    assert Discoverer._prefer_host(public_172, loopback) is public_172
    # 私网 172 优先于公网 172
    assert Discoverer._prefer_host(public_172, private_172) is private_172


# ---------------------------------------------------------------------------
# snapshot 合并 online / history 的排序
# ---------------------------------------------------------------------------


def test_snapshot_online_overrides_history_same_id(discoverer):
    older_history = DiscoveredServer(
        server_id="a@1.1.1.1:80",
        name="a", host="1.1.1.1", port=80, socks=1, api=2,
        vpn=False, version="0.0.9", pac="/proxy.pac",
        source="history", last_seen_at=1.0, healthy=False,
    )
    discoverer._state.history = [older_history]
    discoverer._state.online["a@1.1.1.1:80"] = DiscoveredServer(
        server_id="a@1.1.1.1:80",
        name="a", host="1.1.1.1", port=80, socks=1, api=2,
        vpn=True, version="0.1.0", pac="/proxy.pac",
        source="mdns", last_seen_at=2.0, healthy=True,
    )
    snap = discoverer.snapshot()
    assert len(snap) == 1
    assert snap[0].source == "mdns"
    assert snap[0].vpn is True
    assert snap[0].version == "0.1.0"


def test_snapshot_orders_mdns_first_then_by_recency(discoverer):
    discoverer._state.history = [
        DiscoveredServer(
            server_id="hist1@10.0.0.1:80",
            name="hist1", host="10.0.0.1", port=80, socks=1, api=2,
            vpn=False, version="0.0.1", pac="/proxy.pac",
            source="history", last_seen_at=100.0,
        ),
        DiscoveredServer(
            server_id="hist2@10.0.0.2:80",
            name="hist2", host="10.0.0.2", port=80, socks=1, api=2,
            vpn=False, version="0.0.1", pac="/proxy.pac",
            source="history", last_seen_at=50.0,
        ),
    ]
    discoverer._state.online["live@10.0.0.3:80"] = DiscoveredServer(
        server_id="live@10.0.0.3:80",
        name="live", host="10.0.0.3", port=80, socks=1, api=2,
        vpn=False, version="0.1.0", pac="/proxy.pac",
        source="mdns", last_seen_at=10.0, healthy=True,
    )
    snap = discoverer.snapshot()
    # mdns 在最前
    assert [it.server_id for it in snap] == [
        "live@10.0.0.3:80",       # mdns 优先
        "hist1@10.0.0.1:80",      # history 内按 last_seen_at desc
        "hist2@10.0.0.2:80",
    ]


# ---------------------------------------------------------------------------
# EventBus 联动：手动构造 server 进入 online，验证 snapshot 与持久化
# ---------------------------------------------------------------------------


async def test_persist_merged_history_writes_online_to_disk(tmp_path):
    bus = EventBus()
    path = tmp_path / "known.json"
    d = Discoverer(bus, storage_path=path)
    d._state.online["a@1.1.1.1:80"] = DiscoveredServer(
        server_id="a@1.1.1.1:80",
        name="a", host="1.1.1.1", port=80, socks=1, api=2,
        vpn=False, version="0.1.0", pac="/proxy.pac",
        source="mdns", last_seen_at=time.time(), healthy=True,
    )
    d._persist_merged_history()
    assert path.exists()
    loaded = _load_history(path)
    assert len(loaded) == 1
    assert loaded[0].server_id == "a@1.1.1.1:80"


async def test_start_without_zeroconf_returns_gracefully(tmp_path, monkeypatch):
    """如果 zeroconf 缺包，start 应不抛、available=False、snapshot 仅含 history。"""
    import builtins
    real_import = builtins.__import__

    def fake_import(name, *a, **kw):
        if name.startswith("zeroconf"):
            raise ImportError("simulated missing zeroconf")
        return real_import(name, *a, **kw)

    monkeypatch.setattr(builtins, "__import__", fake_import)

    bus = EventBus()
    d = Discoverer(bus, storage_path=tmp_path / "known.json")
    await d.start()
    assert d.available is False
    assert d.snapshot() == []
    await d.stop()


async def test_start_loads_history_on_startup(tmp_path, monkeypatch):
    """启动时把 known-servers.json 灌进 snapshot。"""
    import builtins
    real_import = builtins.__import__

    def fake_import(name, *a, **kw):
        if name.startswith("zeroconf"):
            raise ImportError("simulated missing zeroconf")
        return real_import(name, *a, **kw)

    monkeypatch.setattr(builtins, "__import__", fake_import)

    path = tmp_path / "known.json"
    _save_history(path, [DiscoveredServer(
        server_id="prev@1.1.1.1:80",
        name="prev", host="1.1.1.1", port=80, socks=1, api=2,
        vpn=False, version="0.1.0", pac="/proxy.pac",
        source="mdns", last_seen_at=time.time(), healthy=True,
    )])

    bus = EventBus()
    d = Discoverer(bus, storage_path=path)
    await d.start()
    snap = d.snapshot()
    assert len(snap) == 1
    assert snap[0].server_id == "prev@1.1.1.1:80"
    assert snap[0].source == "history"
    await d.stop()
