"""mDNS 服务发现 —— 监听 ``_conduit._tcp.local.`` 并维护 server 列表。

设计要点：

* **契约对齐**：service-type / TXT 字段严格对齐 server-app/core/mdns_advertiser.py
  （name / port / socks / api / vpn / version / pac），任何字段名变化必须双向
  同步更新。
* **持久化**：发现的 server 写到 ``~/Library/Application Support/Conduit/known-servers.json``，
  下次启动可作为"曾经看到过"的备选；mDNS 在线列表 + 历史合并由 api/discovery.py 完成。
* **EventBus**：发现 / 失联 → publish 到 EventBus，UI 通过 SSE 实时刷新。
* **健壮性**：zeroconf 未安装时不抛 —— 直接 disable，UI 提示用户手动添加（M-δ）。

实现选择：AsyncServiceBrowser + AsyncServiceInfo，全程在 asyncio loop 中跑，
避免线程回调的复杂同步。
"""
from __future__ import annotations

import asyncio
import json
import logging
import os
import socket
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Optional

log = logging.getLogger("conduit.client.discoverer")

SERVICE_TYPE = "_conduit._tcp.local."

# ---------------------------------------------------------------------------
# 数据模型
# ---------------------------------------------------------------------------


@dataclass
class DiscoveredServer:
    """单个发现到（或历史上见过）的 Conduit Server 记录。

    与 UI 层 `types/client.ts` 的 `DiscoveredServer` 严格一致（snake_case）。
    """
    server_id: str           # name@host:port — 跨 session 稳定的标识
    name: str
    host: str
    port: int                # HTTP proxy / PAC 端口
    socks: int
    api: int
    vpn: bool
    version: str
    pac: str                 # PAC URL 相对路径
    source: str              # "mdns" | "history" | "manual"
    last_seen_at: float      # epoch seconds
    healthy: bool = True     # 当前是否在线（M-β.2 接 connectivity 后才有意义）

    @property
    def pac_url(self) -> str:
        return f"http://{self.host}:{self.port}{self.pac}"


def _make_server_id(name: str, host: str, port: int) -> str:
    return f"{name}@{host}:{port}"


# ---------------------------------------------------------------------------
# 持久化（known-servers.json）
# ---------------------------------------------------------------------------


def _default_storage_path() -> Path:
    """macOS: ~/Library/Application Support/Conduit/known-servers.json
    其它平台 fallback 到 ~/.conduit/known-servers.json
    """
    home = Path.home()
    if os.name == "posix" and (home / "Library" / "Application Support").is_dir():
        base = home / "Library" / "Application Support" / "Conduit"
    else:
        base = home / ".conduit"
    return base / "known-servers.json"


def _load_history(path: Path) -> list[DiscoveredServer]:
    if not path.exists():
        return []
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        log.warning("known-servers.json unreadable: %s", exc)
        return []
    out: list[DiscoveredServer] = []
    for item in raw if isinstance(raw, list) else []:
        try:
            out.append(DiscoveredServer(
                server_id=item["server_id"],
                name=item.get("name", ""),
                host=item.get("host", ""),
                port=int(item.get("port", 0)),
                socks=int(item.get("socks", 0)),
                api=int(item.get("api", 0)),
                vpn=bool(item.get("vpn", False)),
                version=item.get("version", ""),
                pac=item.get("pac", "/proxy.pac"),
                source="history",
                last_seen_at=float(item.get("last_seen_at", 0.0)),
                healthy=False,
            ))
        except (KeyError, ValueError, TypeError) as exc:
            log.warning("skipping malformed history entry %r: %s", item, exc)
    return out


def _save_history(path: Path, items: list[DiscoveredServer]) -> None:
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        snapshot = [
            {**asdict(it), "source": "history"}  # 写入时统一 source=history
            for it in items
        ]
        path.write_text(json.dumps(snapshot, ensure_ascii=False, indent=2), encoding="utf-8")
    except OSError as exc:
        log.warning("known-servers.json save failed: %s", exc)


# ---------------------------------------------------------------------------
# Discoverer 主类
# ---------------------------------------------------------------------------


@dataclass
class _DiscovererState:
    online: dict[str, DiscoveredServer] = field(default_factory=dict)   # 当前在线（mDNS 看得见）
    history: list[DiscoveredServer] = field(default_factory=list)       # 持久化（历史 + 离线）


class Discoverer:
    """mDNS 服务发现 + 历史持久化 + EventBus 推送。

    用法（在 ClientRuntime.start 中）::

        self.discoverer = Discoverer(self.bus)
        await self.discoverer.start()
        # ...
        await self.discoverer.stop()
    """

    def __init__(
        self,
        bus,                                   # EventBus（避免循环 import 用 duck typing）
        *,
        storage_path: Optional[Path] = None,
    ) -> None:
        self.bus = bus
        self.storage_path = storage_path or _default_storage_path()
        self._state = _DiscovererState()
        self._zc = None
        self._browser = None
        self._available = False  # zeroconf 是否可用

    # ----- 生命周期 -----

    async def start(self) -> None:
        # 先把历史灌进 state（UI 启动后立刻能看到"上次用过"）
        self._state.history = _load_history(self.storage_path)
        log.info("loaded %d historical server(s) from %s",
                 len(self._state.history), self.storage_path)

        try:
            from zeroconf import ServiceStateChange
            from zeroconf.asyncio import AsyncServiceBrowser, AsyncServiceInfo, AsyncZeroconf
        except ImportError:
            log.warning("zeroconf not installed; mDNS discovery disabled")
            self._available = False
            return

        self._zc = AsyncZeroconf()
        self._available = True

        loop = asyncio.get_running_loop()

        # zeroconf 的 ServiceBrowser handler 是**同步**回调（运行在 zeroconf 线程），
        # 我们这里转交给 asyncio loop 的协程。注意：handler 不能是 async def，
        # 否则 zeroconf 不 await 它,只会得到一个未 await 的 coroutine。
        def _on_service_state_change(zeroconf, service_type, name, state_change):
            if state_change is ServiceStateChange.Removed:
                coro = self._on_service_removed(name)
            elif state_change in (ServiceStateChange.Added, ServiceStateChange.Updated):
                coro = self._on_service_added_or_updated(zeroconf, service_type, name)
            else:
                return
            asyncio.run_coroutine_threadsafe(coro, loop)

        self._browser = AsyncServiceBrowser(
            self._zc.zeroconf,
            [SERVICE_TYPE],
            handlers=[_on_service_state_change],
        )
        log.info("mDNS browser listening on %s", SERVICE_TYPE)

    async def stop(self) -> None:
        # 持久化最近一次在线视图（合并到 history，按 server_id 去重）
        self._persist_merged_history()

        if self._browser is not None:
            try:
                await self._browser.async_cancel()
            except Exception as exc:  # noqa: BLE001
                log.warning("browser cancel failed: %s", exc)
            self._browser = None

        if self._zc is not None:
            try:
                await self._zc.async_close()
            except Exception as exc:  # noqa: BLE001
                log.warning("zeroconf close failed: %s", exc)
            self._zc = None

        self._available = False
        log.info("discoverer stopped")

    # ----- 同步快照接口（API 端用） -----

    @property
    def available(self) -> bool:
        return self._available

    def snapshot(self) -> list[DiscoveredServer]:
        """合并 online + history，按 (online 优先, last_seen_at desc) 排序。

        相同 server_id：online 覆盖 history（在线更新鲜）。
        """
        merged: dict[str, DiscoveredServer] = {}
        for it in self._state.history:
            merged[it.server_id] = it
        for it in self._state.online.values():
            merged[it.server_id] = it  # 覆盖

        items = list(merged.values())
        items.sort(key=lambda x: (0 if x.source == "mdns" else 1, -x.last_seen_at))
        return items

    # ----- 内部回调 -----

    async def _on_service_added_or_updated(self, zeroconf, service_type: str, name: str) -> None:
        from zeroconf.asyncio import AsyncServiceInfo
        info = AsyncServiceInfo(service_type, name)
        # 默认 3 秒内拿不到 TXT 就算了，否则会拖住 callback
        ok = await info.async_request(zeroconf, timeout=3000)
        if not ok:
            log.debug("service info request failed for %s (timeout)", name)
            return
        ds = self._service_info_to_server(info)
        if ds is None:
            return
        # 同名 + 同端口 去重 —— zeroconf 在 client 和 server 跑在同一台 mac
        # 时,可能通过 loopback (127.x) 和 LAN (192.168.x) 两条路径分别推
        # 一次 add_service 回调,各自的 ServiceInfo 只带一个 host。结果就是
        # online dict 出现两条 "HW0023148@127.0.0.1:18367" + "HW0023148@192.168.1.14:18367"。
        # 这里做"同 name+port"折叠,LAN > 非 loopback > loopback,保留最优。
        existing = self._find_same_endpoint(ds.name, ds.port)
        if existing is not None and existing.server_id != ds.server_id:
            keep = self._prefer_host(existing, ds)
            drop = ds if keep is existing else existing
            # 把 dict 里 drop 的那条删掉,确保 online 里同 name+port 永远只剩 1 条
            self._state.online.pop(drop.server_id, None)
            log.debug(
                "discoverer dedup: keep %s, drop %s",
                keep.server_id, drop.server_id,
            )
            if keep is existing:
                # 远端送来的是次优 host(比如 127.x),existing 已经是优选,直接返回
                return
        self._state.online[ds.server_id] = ds
        log.info("server discovered: %s (vpn=%s, version=%s)",
                 ds.server_id, ds.vpn, ds.version)
        self.bus.publish("server_discovered", _server_to_payload(ds))

    def _find_same_endpoint(
        self, name: str, port: int,
    ) -> Optional[DiscoveredServer]:
        for ds in self._state.online.values():
            if ds.name == name and ds.port == port:
                return ds
        return None

    @staticmethod
    def _prefer_host(a: DiscoveredServer, b: DiscoveredServer) -> DiscoveredServer:
        """优选 LAN IP > 其它 > loopback。两侧同档时取后者(更新鲜)。"""
        def _rank(ip: str) -> int:
            if ip.startswith("192.168.") or ip.startswith("10."):
                return 0
            if ip.startswith("172."):
                try:
                    second = int(ip.split(".")[1])
                    if 16 <= second <= 31:
                        return 0
                except (IndexError, ValueError):
                    pass
            if ip.startswith("127."):
                return 2
            return 1
        ra, rb = _rank(a.host), _rank(b.host)
        if ra < rb:
            return a
        if rb < ra:
            return b
        return b

    async def _on_service_removed(self, name: str) -> None:
        # name 是完整 instance name（例如 "Conduit on host._conduit._tcp.local."）
        # 我们没在 state 里按 instance name 索引，只能反查 host:port
        # 简化：扫一遍 online dict 看有没有 instance_name 命中
        gone: list[str] = []
        for sid, ds in list(self._state.online.items()):
            if name.startswith(f"Conduit on {ds.name}."):
                gone.append(sid)
        for sid in gone:
            ds = self._state.online.pop(sid)
            log.info("server lost: %s", sid)
            self.bus.publish("server_lost", {
                "server_id": sid,
                "name": ds.name,
            })

    # ----- 工具 -----

    def _service_info_to_server(self, info) -> Optional[DiscoveredServer]:
        if not info.addresses:
            log.debug("ignoring %s: no addresses", info.name)
            return None
        # IPv4 优先 + 偏向 LAN —— zeroconf 在多网口同机器场景下,addresses
        # 可能同时包含 loopback (127.x) 和 LAN (192.168.x / 10.x / 172.16-31)。
        # 我们偏向真正的 LAN 地址,这样从同一台 mac 跑 client + server 时,
        # 不会把 server discover 成两条 (一条 127.0.0.1 + 一条 192.168.x)。
        candidates: list[str] = []
        for addr in info.addresses:
            if len(addr) == 4:
                candidates.append(socket.inet_ntoa(addr))
        if not candidates:
            log.debug("ignoring %s: no IPv4 address", info.name)
            return None
        # 优先级:LAN > 其它非 loopback > loopback
        def _ip_priority(ip: str) -> int:
            if ip.startswith("192.168.") or ip.startswith("10."):
                return 0
            if ip.startswith("172."):
                try:
                    second = int(ip.split(".")[1])
                    if 16 <= second <= 31:
                        return 0
                except (IndexError, ValueError):
                    pass
            if ip.startswith("127."):
                return 2
            return 1
        candidates.sort(key=_ip_priority)
        host = candidates[0]

        props = info.properties or {}

        def _txt(key: str, default: str = "") -> str:
            raw = props.get(key.encode("ascii"))
            if raw is None:
                return default
            try:
                return raw.decode("utf-8")
            except Exception:
                return default

        try:
            port = int(_txt("port", str(info.port or 0)))
            socks = int(_txt("socks", "0"))
            api = int(_txt("api", "0"))
        except ValueError as exc:
            log.warning("invalid TXT port for %s: %s", info.name, exc)
            return None

        name = _txt("name") or info.name.split(".")[0]
        return DiscoveredServer(
            server_id=_make_server_id(name, host, port),
            name=name,
            host=host,
            port=port,
            socks=socks,
            api=api,
            vpn=_txt("vpn", "off") == "on",
            version=_txt("version", ""),
            pac=_txt("pac", "/proxy.pac"),
            source="mdns",
            last_seen_at=time.time(),
            healthy=True,
        )

    def _persist_merged_history(self) -> None:
        # 取并集：history 里没出现过的 online server 加进去
        merged: dict[str, DiscoveredServer] = {it.server_id: it for it in self._state.history}
        for sid, ds in self._state.online.items():
            merged[sid] = ds  # online 覆盖
        items = list(merged.values())
        # 简单容量控制：最多保留 32 条最近的
        items.sort(key=lambda x: -x.last_seen_at)
        items = items[:32]
        _save_history(self.storage_path, items)

    # ----- forget API（M-δ 验收期补丁:用户可以主动清理"曾见过") -----

    def forget(self, server_id: str) -> bool:
        """从历史记录中移除指定 server。返回是否真的删除了一条(False = 不存在)。

        注意:若该 server 当前 mDNS 在线,本方法 *只清历史记录*,不影响当前 online 列表
        (因为 zeroconf 还在广播,任何刷新都会再加回来)。要彻底"踢掉"必须同时让对方
        停止广播,或者关闭本机的 mDNS。
        """
        before = len(self._state.history)
        self._state.history = [it for it in self._state.history if it.server_id != server_id]
        removed = len(self._state.history) < before
        if removed:
            _save_history(self.storage_path, self._state.history)
            log.info("forget(%s): removed from known-servers.json", server_id)
            try:
                self.bus.publish("server_forgotten", {"server_id": server_id})
            except Exception:  # noqa: BLE001
                pass
        return removed

    def forget_all_history(self) -> int:
        """清空全部"曾见过"。返回清掉的条数。online 列表不动。"""
        n = len(self._state.history)
        self._state.history = []
        _save_history(self.storage_path, [])
        if n > 0:
            log.info("forget_all_history(): wiped %d historical server(s)", n)
            try:
                self.bus.publish("server_forgotten", {"server_id": None, "removed_count": n})
            except Exception:  # noqa: BLE001
                pass
        return n


# ---------------------------------------------------------------------------
# JSON 序列化辅助
# ---------------------------------------------------------------------------


def _server_to_payload(ds: DiscoveredServer) -> dict:
    """把 dataclass 转成 SSE / API 友好的 dict。"""
    return {
        "server_id": ds.server_id,
        "name": ds.name,
        "host": ds.host,
        "port": ds.port,
        "socks": ds.socks,
        "api": ds.api,
        "vpn": ds.vpn,
        "version": ds.version,
        "pac": ds.pac,
        "pac_url": ds.pac_url,
        "source": ds.source,
        "last_seen_at": ds.last_seen_at,
        "healthy": ds.healthy,
    }


def server_to_payload(ds: DiscoveredServer) -> dict:
    """公开版，供 api/discovery.py 调用。"""
    return _server_to_payload(ds)
