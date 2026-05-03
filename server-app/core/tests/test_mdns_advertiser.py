"""mDNS 广播器单元测试 —— mdns_advertiser.py。

不依赖真实 zeroconf 注册:用 monkeypatch 假冒 zeroconf.asyncio,
只验证:
  - 构造参数被正确编码为 TXT 记录
  - register 在 zeroconf 缺失时返回 False(降级行为)
  - update_vpn 同状态时短路、变更时调用 async_update_service
  - unregister 把内部句柄清空,异常被吞
"""
from __future__ import annotations

import builtins
import sys

import pytest


pytestmark = pytest.mark.asyncio


# ---------- 假冒 zeroconf 模块 ----------

class _FakeAsyncServiceInfo:
    last_instance = None

    def __init__(
        self,
        type_,
        name,
        addresses=None,
        port=None,
        properties=None,
        server=None,
    ):
        self.type_ = type_
        self.name = name
        self.addresses = addresses
        self.port = port
        self.properties = properties
        self.server = server
        _FakeAsyncServiceInfo.last_instance = self


class _FakeAsyncZeroconf:
    instances: list["_FakeAsyncZeroconf"] = []

    def __init__(self):
        self.registered: list = []
        self.updated: list = []
        self.closed = False
        _FakeAsyncZeroconf.instances.append(self)

    async def async_register_service(self, info, allow_name_change=False):
        self.registered.append((info, allow_name_change))

    async def async_update_service(self, info):
        self.updated.append(info)

    async def async_unregister_service(self, info):
        self.unregistered = info

    async def async_close(self):
        self.closed = True


class _FailingAsyncZeroconf(_FakeAsyncZeroconf):
    async def async_unregister_service(self, info):
        raise RuntimeError("simulated unregister error")


class _FakeZeroconfModule:
    asyncio = type("asyncio", (), {
        "AsyncServiceInfo": _FakeAsyncServiceInfo,
        "AsyncZeroconf": _FakeAsyncZeroconf,
    })


@pytest.fixture
def fake_zeroconf(monkeypatch):
    fake_pkg = _FakeZeroconfModule()
    fake_async_pkg = fake_pkg.asyncio
    monkeypatch.setitem(sys.modules, "zeroconf", fake_pkg)
    monkeypatch.setitem(sys.modules, "zeroconf.asyncio", fake_async_pkg)
    _FakeAsyncZeroconf.instances.clear()
    yield fake_pkg


# ---------- 测试 ----------

async def test_register_returns_false_when_zeroconf_missing(monkeypatch):
    """zeroconf 包不可导入时,register 不抛异常,返回 False(降级)。"""
    real_import = builtins.__import__

    def fake_import(name, *a, **kw):
        if name == "zeroconf" or name.startswith("zeroconf."):
            raise ImportError("simulated missing zeroconf")
        return real_import(name, *a, **kw)

    monkeypatch.setattr(builtins, "__import__", fake_import)

    from mdns_advertiser import MdnsAdvertiser
    adv = MdnsAdvertiser(
        name="x", host_ip="127.0.0.1",
        http_port=1, socks_port=2, api_port=3,
    )
    ok = await adv.register()
    assert ok is False
    assert adv._zc is None


async def test_register_encodes_txt_record_fields(fake_zeroconf):
    from mdns_advertiser import MdnsAdvertiser, SERVICE_TYPE
    adv = MdnsAdvertiser(
        name="conduit-host",
        host_ip="192.168.1.10",
        http_port=24807,
        socks_port=23326,
        api_port=19883,
        vpn_on=True,
        version="0.1.0",
    )
    ok = await adv.register()
    assert ok is True
    info = _FakeAsyncServiceInfo.last_instance
    assert info is not None
    assert info.type_ == SERVICE_TYPE
    assert info.name.startswith("Conduit on conduit-host")
    assert info.port == 24807
    # 解码 TXT
    props = {k.decode(): v.decode() for k, v in info.properties.items()}
    assert props["name"] == "conduit-host"
    assert props["port"] == "24807"
    assert props["socks"] == "23326"
    assert props["api"] == "19883"
    assert props["vpn"] == "on"
    assert props["version"] == "0.1.0"
    assert props["pac"] == "/proxy.pac"
    # allow_name_change 必须传 True,否则重启时会撞 NonUniqueNameException
    zc = _FakeAsyncZeroconf.instances[-1]
    assert zc.registered[0][1] is True


async def test_register_invalid_host_ip_falls_back_to_all_interfaces(fake_zeroconf, caplog):
    from mdns_advertiser import MdnsAdvertiser
    adv = MdnsAdvertiser(
        name="x", host_ip="not.an.ip",
        http_port=1, socks_port=2, api_port=3,
    )
    ok = await adv.register()
    assert ok is True
    info = _FakeAsyncServiceInfo.last_instance
    # addresses 列表应该为 None 或空(无效 IP 时不放进去),这样 zeroconf 自己选所有接口
    assert info.addresses is None


async def test_update_vpn_no_change_short_circuits(fake_zeroconf):
    from mdns_advertiser import MdnsAdvertiser
    adv = MdnsAdvertiser(
        name="x", host_ip="127.0.0.1",
        http_port=1, socks_port=2, api_port=3,
        vpn_on=False,
    )
    await adv.register()
    zc = _FakeAsyncZeroconf.instances[-1]
    await adv.update_vpn(False)
    assert zc.updated == []


async def test_update_vpn_change_triggers_async_update(fake_zeroconf):
    from mdns_advertiser import MdnsAdvertiser
    adv = MdnsAdvertiser(
        name="x", host_ip="127.0.0.1",
        http_port=1, socks_port=2, api_port=3,
        vpn_on=False,
    )
    await adv.register()
    zc = _FakeAsyncZeroconf.instances[-1]
    await adv.update_vpn(True)
    assert len(zc.updated) == 1
    assert adv.vpn_on is True
    # TXT 也被刷新
    props = {k.decode(): v.decode() for k, v in adv._info.properties.items()}
    assert props["vpn"] == "on"


async def test_update_vpn_before_register_only_updates_state(fake_zeroconf):
    from mdns_advertiser import MdnsAdvertiser
    adv = MdnsAdvertiser(
        name="x", host_ip="127.0.0.1",
        http_port=1, socks_port=2, api_port=3,
        vpn_on=False,
    )
    # 没 register 直接 update,不应抛
    await adv.update_vpn(True)
    assert adv.vpn_on is True


async def test_unregister_clears_handles_and_swallows_errors(monkeypatch):
    monkeypatch.setitem(sys.modules, "zeroconf", _FakeZeroconfModule())
    monkeypatch.setitem(
        sys.modules, "zeroconf.asyncio",
        type("a", (), {
            "AsyncServiceInfo": _FakeAsyncServiceInfo,
            "AsyncZeroconf": _FailingAsyncZeroconf,
        }),
    )
    from mdns_advertiser import MdnsAdvertiser
    adv = MdnsAdvertiser(
        name="x", host_ip="127.0.0.1",
        http_port=1, socks_port=2, api_port=3,
    )
    await adv.register()
    # unregister 内部 raise,但调用方不应感知
    await adv.unregister()
    assert adv._zc is None
    assert adv._info is None


async def test_async_context_manager(fake_zeroconf):
    from mdns_advertiser import MdnsAdvertiser
    async with MdnsAdvertiser(
        name="x", host_ip="127.0.0.1",
        http_port=1, socks_port=2, api_port=3,
    ) as adv:
        assert adv._zc is not None
    # 出 context 后被清理
    assert adv._zc is None
