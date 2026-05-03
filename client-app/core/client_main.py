"""Conduit Client smart-local-proxy entry point (S5 M1).

What this module does today
---------------------------

* Parses CLI args.
* Starts ``RouteCache`` + ``RouteResolver`` + ``LocalProxyServer`` wired
  to a single ``ServerEndpoint``.
* Optionally pre-fills the cache with the PAC patterns served by that
  endpoint (one shot, best-effort).
* Optionally points the macOS system SOCKS proxy at our local listener.
* Handles ``SIGINT`` / ``SIGTERM`` / parent-pid-disappeared by tearing
  everything down and restoring the system proxy.

M-α 增量（2026-05-01）：
* 启动时拉起最小 control HTTP API（仅 /healthz），让 Tauri 主进程能探活。

M-β.1 增量（2026-05-02）：
* 起 EventBus（进程内 pub/sub）
* 起 Discoverer（mDNS ``_conduit._tcp.local.`` + known-servers.json 持久化）
* control API 增 ``GET /api/servers`` 与 ``GET /api/events`` SSE

M-β.2 增量（2026-05-02 中）：
* connect/disconnect 状态机（idle / connecting / connected / failed）
* 5 步 connect_progress SSE（probe → fetch_pac → prefill → switch_endpoint → start_heartbeat）
* control API 增 ``POST /api/connect/{server_id}`` ``POST /api/disconnect`` ``GET /api/connection``
* 心跳协程 connectivity.Heartbeat：10s/次,1 失败 yellow / 3 失败 red

Not yet implemented (lands in M-γ / M-δ):
* heartbeat-driven global downgrade（红 → flush proxy cache + 强制 direct）
* route / cache / diagnose API
* 4 态托盘 / Settings 表单 / 打包

Cross-references:
* design/2026-04-30-3-Conduit-Client-客户端可行性报告.md §3.6 (start-up flow)
* design/2026-04-30-5-Conduit-开发TODO清单-进度S5M1-67.md §S6 客户端开发执行路线
"""
from __future__ import annotations

import argparse
import asyncio
import logging
import os
import signal
import socket
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Optional

from connectivity import Heartbeat, ProbeResult, probe as connectivity_probe
from discoverer import Discoverer, DiscoveredServer
from events_bus import EventBus
from local_proxy import (
    DEFAULT_DIRECT_CONNECT_TIMEOUT,
    DEFAULT_HANDSHAKE_TIMEOUT,
    DEFAULT_PROXY_CONNECT_TIMEOUT,
    LocalProxyServer,
    ServerEndpoint,
)
from pac_parser import extract_proxy_hosts
from route_cache import RouteCache, RouteEntry, _utcnow
from route_resolver import RouteResolver
from system_proxy import MacSystemProxy

DEFAULT_BIND_HOST = "127.0.0.1"
DEFAULT_BIND_PORT = 7890
DEFAULT_API_PORT = 8091
DEFAULT_SERVER_PORT = 8080
DEFAULT_PAC_PATH = "/proxy.pac"
PAC_FETCH_TIMEOUT = 3.0

logger = logging.getLogger("conduit.client.main")


# ---------------------------------------------------------------------------
# CLI parsing
# ---------------------------------------------------------------------------


@dataclass
class ClientConfig:
    bind_host: str
    bind_port: int
    api_port: int
    server_host: str | None
    server_port: int
    pac_path: str
    enable_system_proxy: bool
    log_level: str
    watchdog_ppid: int | None


def parse_args(argv: list[str] | None = None) -> ClientConfig:
    p = argparse.ArgumentParser(
        prog="conduit-client",
        description="Conduit client-app smart local proxy (macOS only, v0.1).",
    )
    p.add_argument("--bind-host", default=DEFAULT_BIND_HOST)
    p.add_argument("--bind-port", type=int, default=DEFAULT_BIND_PORT)
    p.add_argument(
        "--api-port",
        type=int,
        default=DEFAULT_API_PORT,
        help="Control HTTP API port (loopback only). Tauri 主进程通过 healthz 探活。",
    )
    p.add_argument(
        "--server-host",
        default=None,
        help="Conduit Server LAN IP (omit to start without an upstream — every "
             "request will go direct, useful for offline development).",
    )
    p.add_argument("--server-port", type=int, default=DEFAULT_SERVER_PORT)
    p.add_argument("--pac-path", default=DEFAULT_PAC_PATH)
    p.add_argument(
        "--no-system-proxy",
        action="store_true",
        help="Don't flip macOS networksetup; just expose the SOCKS5 listener.",
    )
    p.add_argument("--log-level", default="INFO", choices=["DEBUG", "INFO", "WARN", "WARNING", "ERROR"])
    p.add_argument(
        "--watchdog-ppid",
        type=int,
        default=None,
        help="If set, exit when this PID disappears (Tauri orphan watchdog).",
    )
    args = p.parse_args(argv)
    return ClientConfig(
        bind_host=args.bind_host,
        bind_port=args.bind_port,
        api_port=args.api_port,
        server_host=args.server_host,
        server_port=args.server_port,
        pac_path=args.pac_path,
        enable_system_proxy=not args.no_system_proxy,
        log_level=args.log_level,
        watchdog_ppid=args.watchdog_ppid,
    )


# ---------------------------------------------------------------------------
# PAC pre-fill
# ---------------------------------------------------------------------------


async def _fetch_pac_once(server: ServerEndpoint, pac_path: str) -> str | None:
    """Best-effort PAC fetch — returns text on success, None on any failure."""
    url = f"http://{server.host}:{server.port}{pac_path}"

    def _do_fetch() -> str:
        with urllib.request.urlopen(url, timeout=PAC_FETCH_TIMEOUT) as resp:
            return resp.read().decode("utf-8", errors="replace")

    try:
        return await asyncio.to_thread(_do_fetch)
    except (urllib.error.URLError, OSError, TimeoutError) as exc:
        logger.warning("pac fetch %s failed: %s", url, exc)
        return None


def _prefill_cache_from_pac(cache: RouteCache, pac_source: str) -> int:
    """Insert every PROXY-bound pattern as ``proxy`` / source='pac'."""
    patterns = extract_proxy_hosts(pac_source)
    from datetime import timedelta

    expires = _utcnow() + timedelta(minutes=5)
    for pattern in patterns:
        cache.set(
            pattern,
            RouteEntry(
                host=pattern,
                direction="proxy",
                expires_at=expires,
                source="pac",
            ),
        )
    return len(patterns)


# ---------------------------------------------------------------------------
# main runtime
# ---------------------------------------------------------------------------


class ClientRuntime:
    def __init__(self, cfg: ClientConfig) -> None:
        self.cfg = cfg
        self.cache = RouteCache()
        self.route_cache = self.cache  # api/cache.py 用的别名,保留 self.cache 不破坏旧调用
        self.bus = EventBus()
        self.resolver = RouteResolver(self.cache)
        self.proxy = LocalProxyServer(
            self.resolver,
            bind_host=cfg.bind_host,
            bind_port=cfg.bind_port,
            server_endpoint=self._initial_endpoint(),
            handshake_timeout=DEFAULT_HANDSHAKE_TIMEOUT,
            direct_connect_timeout=DEFAULT_DIRECT_CONNECT_TIMEOUT,
            proxy_connect_timeout=DEFAULT_PROXY_CONNECT_TIMEOUT,
        )
        self.system_proxy = MacSystemProxy() if MacSystemProxy.is_supported() else None
        self.discoverer = Discoverer(self.bus)
        # M-γ:traffic_meter 仅在 connected 时活;空 endpoint 也保持 None
        from traffic_meter import TrafficMeter
        self.traffic_meter: Optional[TrafficMeter] = None
        self._TrafficMeter = TrafficMeter
        from api import ApiServer
        self.api: ApiServer = ApiServer(self, port=cfg.api_port, loopback_only=True)
        self._stop_event = asyncio.Event()
        self._system_proxy_active = False
        # 记录最近一次 system_proxy.enable() 的错误信息,供 diagnose 显示
        self._system_proxy_last_error: Optional[str] = None

        # M-β.2 连接状态机
        # state: "idle" | "connecting" | "connected" | "failed" | "disconnecting"
        self.connection_state: str = "idle"
        self.connected_server: Optional[DiscoveredServer] = None
        self.connected_since: Optional[float] = None
        self.last_connect_error: Optional[str] = None
        self.heartbeat: Optional[Heartbeat] = None
        self._connect_lock = asyncio.Lock()
        self._started_at = time.time()  # M-δ diagnose 用

    def _initial_endpoint(self) -> ServerEndpoint | None:
        if self.cfg.server_host is None:
            return None
        return ServerEndpoint(host=self.cfg.server_host, port=self.cfg.server_port)

    async def start(self) -> None:
        if self.system_proxy is not None:
            cleaned = self.system_proxy.cleanup_if_pointing_to_us(
                host=self.cfg.bind_host, port=self.cfg.bind_port,
            )
            if cleaned:
                logger.info("startup cleanup: stale system proxy disabled")

        await self.proxy.start()

        endpoint = self.proxy.server_endpoint
        if endpoint is not None:
            pac_text = await _fetch_pac_once(endpoint, self.cfg.pac_path)
            if pac_text is not None:
                n = _prefill_cache_from_pac(self.cache, pac_text)
                logger.info("pac prefill: %d patterns inserted", n)

        if self.cfg.enable_system_proxy and self.system_proxy is not None:
            try:
                self.system_proxy.enable(
                    host=self.cfg.bind_host, port=self.proxy.actual_port,
                )
                self._system_proxy_active = True
                self._system_proxy_last_error = None
            except RuntimeError as exc:
                self._system_proxy_last_error = str(exc)
                logger.warning("system proxy enable failed: %s", exc)

        # mDNS 发现：失败不阻断启动（zeroconf 缺包 / 沙箱无网卡都允许降级到手动添加）。
        try:
            await self.discoverer.start()
        except Exception as exc:  # noqa: BLE001
            logger.warning("discoverer start failed (continuing without mDNS): %s", exc)

        await self.api.start()

    async def stop(self) -> None:
        await self.api.stop()
        if self.heartbeat is not None:
            try:
                await self.heartbeat.stop()
            except Exception as exc:  # noqa: BLE001
                logger.warning("heartbeat stop failed: %s", exc)
            self.heartbeat = None
        try:
            await self.discoverer.stop()
        except Exception as exc:  # noqa: BLE001
            logger.warning("discoverer stop failed: %s", exc)
        if self._system_proxy_active and self.system_proxy is not None:
            try:
                self.system_proxy.disable()
            except RuntimeError as exc:
                logger.warning("system proxy disable failed: %s", exc)
            self._system_proxy_active = False
        await self.proxy.stop()

    # ----- M-β.2 连接编排 -----

    # 5 步常量,与 ConnectingProgress.vue 对齐
    CONNECT_STEPS = [
        ("probe",            "可达性检查"),
        ("fetch_pac",        "拉取 PAC"),
        ("prefill_cache",    "解析 PAC 预填路由"),
        ("switch_endpoint",  "切换上游 server"),
        ("start_heartbeat",  "启动心跳与系统代理"),
    ]

    def _publish_progress(self, *, step: int, key: str, status: str, detail: str = "", server_id: str = "") -> None:
        self.bus.publish("connect_progress", {
            "step": step,
            "total": len(self.CONNECT_STEPS),
            "key": key,
            "label": dict(self.CONNECT_STEPS)[key],
            "status": status,         # "running" | "ok" | "failed"
            "detail": detail,
            "server_id": server_id,
        })

    async def connect_to(self, server: DiscoveredServer) -> dict:
        """主流程:5 步执行;每步 publish 进度。返回最终状态。

        互斥:同一时刻只允许一个 connect / disconnect 在跑,后来者 409。
        """
        if self._connect_lock.locked():
            return {"ok": False, "error": "BUSY", "message": "另一个连接/断开操作正在进行"}

        async with self._connect_lock:
            if self.connection_state == "connected":
                # 同一 server 重复点 -> 视为成功
                if self.connected_server and self.connected_server.server_id == server.server_id:
                    return self.connection_snapshot()
                # 不同 server -> 先断开再接(简化:返回 409 让用户先点断开)
                return {"ok": False, "error": "ALREADY_CONNECTED",
                        "message": f"已连接到 {self.connected_server.name if self.connected_server else '?'},请先断开"}

            self.connection_state = "connecting"
            self.last_connect_error = None
            self.bus.publish("connection_state_changed", {"state": "connecting", "server_id": server.server_id})

            # ---- step 1: probe ----
            self._publish_progress(step=1, key="probe", status="running", server_id=server.server_id)
            result: ProbeResult = await connectivity_probe(
                host=server.host, http_port=server.port, socks_port=server.socks,
            )
            if not result.ok:
                return await self._connect_failed(
                    step=1, key="probe", error=result.error or "可达性检查失败", server_id=server.server_id,
                )
            self._publish_progress(
                step=1, key="probe", status="ok",
                detail=f"延迟 {int(result.latency_ms)} ms", server_id=server.server_id,
            )

            # ---- step 2: fetch PAC ----
            self._publish_progress(step=2, key="fetch_pac", status="running", server_id=server.server_id)
            endpoint = ServerEndpoint(host=server.host, port=server.port)
            pac_text = await _fetch_pac_once(endpoint, server.pac)
            if pac_text is None:
                return await self._connect_failed(
                    step=2, key="fetch_pac",
                    error=f"无法从 {endpoint.label()} 拉取 PAC",
                    server_id=server.server_id,
                )
            self._publish_progress(
                step=2, key="fetch_pac", status="ok",
                detail=f"{len(pac_text)} 字节", server_id=server.server_id,
            )

            # ---- step 3: prefill cache ----
            self._publish_progress(step=3, key="prefill_cache", status="running", server_id=server.server_id)
            n = _prefill_cache_from_pac(self.cache, pac_text)
            self._publish_progress(
                step=3, key="prefill_cache", status="ok",
                detail=f"预填 {n} 条规则", server_id=server.server_id,
            )

            # ---- step 4: switch endpoint + system proxy ----
            # endpoint 切换是关键路径,失败 => 整体连接失败(因为没有上游 PAC 没法路由)
            # system_proxy 切换是"锦上添花",失败时只发警告,不让整体连接 fail
            # (用户仍可手动在系统设置里指 SOCKS5 到我们的 :PORT)
            self._publish_progress(step=4, key="switch_endpoint", status="running", server_id=server.server_id)
            try:
                self.proxy.set_server_endpoint(endpoint)
            except (RuntimeError, OSError) as exc:
                return await self._connect_failed(
                    step=4, key="switch_endpoint",
                    error=f"切换上游 endpoint 失败: {exc}",
                    server_id=server.server_id,
                )

            system_proxy_warning: str | None = None
            if self.cfg.enable_system_proxy and self.system_proxy is not None:
                try:
                    self.system_proxy.enable(host=self.cfg.bind_host, port=self.proxy.actual_port)
                    self._system_proxy_active = True
                    self._system_proxy_last_error = None
                except (RuntimeError, OSError) as exc:
                    # 不让整体连接失败,只在 progress detail 里附带警告
                    system_proxy_warning = str(exc)
                    self._system_proxy_last_error = str(exc)
                    logger.warning("system proxy switch failed (continuing anyway): %s", exc)

            if self._system_proxy_active:
                sp_state = "已切换"
            elif system_proxy_warning:
                sp_state = f"切换失败,需手动配置 SOCKS5 :{self.proxy.actual_port}"
            else:
                sp_state = "未启用(由用户手动配 SOCKS5)"
            self._publish_progress(
                step=4, key="switch_endpoint", status="ok",
                detail=f"上游 {endpoint.label()} · 本机 SOCKS5 :{self.proxy.actual_port} · 系统代理 {sp_state}",
                server_id=server.server_id,
            )
            if system_proxy_warning:
                # 单独发一个 warning event,前端可以 toast 显示给用户
                self.bus.publish("system_proxy_warning", {
                    "server_id": server.server_id,
                    "message": system_proxy_warning,
                    "manual_socks_port": self.proxy.actual_port,
                })

            # ---- step 5: start heartbeat ----
            self._publish_progress(step=5, key="start_heartbeat", status="running", server_id=server.server_id)
            try:
                hostname = socket.gethostname().split(".")[0] or "Conduit Client"
            except Exception:
                hostname = "Conduit Client"
            self.heartbeat = Heartbeat(
                self.bus,
                host=server.host, http_port=server.port, socks_port=server.socks,
                client_name=hostname,
                client_version="0.1.0",
            )
            await self.heartbeat.start()
            # M-γ:启动 traffic_meter,挂上 LocalProxyServer 的进度回调
            self.traffic_meter = self._TrafficMeter(self.bus)
            await self.traffic_meter.start()
            self.proxy.set_progress_callback(self.traffic_meter.on_chunk)
            # M-γ:resolver 决策 -> route_decision 事件,UI 表实时增长
            self.resolver.set_event_publisher(self._publish_route_decision)
            self._publish_progress(
                step=5, key="start_heartbeat", status="ok",
                detail="心跳每 10 秒一次 · 流量统计已启用", server_id=server.server_id,
            )

            # ---- 完成 ----
            self.connection_state = "connected"
            self.connected_server = server
            self.connected_since = time.time()
            self.last_connect_error = None
            snapshot = self.connection_snapshot()
            self.bus.publish("connect_done", {**snapshot, "server_id": server.server_id})
            self.bus.publish("connection_state_changed", {"state": "connected", "server_id": server.server_id})
            logger.info("connected to %s (%s)", server.name, endpoint.label())
            return snapshot

    def _publish_route_decision(self, host: str, port: int, decision) -> None:
        """RouteResolver 每次决策完调本方法 -> 发 SSE event。"""
        cache_entry = decision.cache_entry
        self.bus.publish("route_decision", {
            "host": host,
            "port": port,
            "direction": decision.direction,    # "direct" | "proxy"
            "source": decision.source,          # "cache" | "probe" | "pattern" | "private_ip" | "global_override" | "self_heal"
            "hit_count": cache_entry.hit_count if cache_entry else 0,
        })

    async def _connect_failed(self, *, step: int, key: str, error: str, server_id: str) -> dict:
        """connect 任一步失败的统一回滚:把 endpoint / system_proxy / heartbeat / traffic_meter 全部回到 idle。"""
        self._publish_progress(step=step, key=key, status="failed", detail=error, server_id=server_id)
        self.connection_state = "failed"
        self.last_connect_error = error
        # 回滚:本次连接如果已经 partial 改了 endpoint / system_proxy / traffic / publisher,撤销
        try:
            self.proxy.set_server_endpoint(None)
            self.proxy.set_progress_callback(None)
        except Exception:
            pass
        try:
            self.resolver.set_event_publisher(None)
        except Exception:
            pass
        if self.traffic_meter is not None:
            try:
                await self.traffic_meter.stop()
            except Exception:
                pass
            self.traffic_meter = None
        if self._system_proxy_active and self.system_proxy is not None:
            try:
                self.system_proxy.disable()
            except RuntimeError as exc:
                logger.warning("rollback system_proxy failed: %s", exc)
            self._system_proxy_active = False
        if self.heartbeat is not None:
            try:
                await self.heartbeat.stop()
            except Exception:
                pass
            self.heartbeat = None
        self.bus.publish("connection_state_changed", {"state": "failed", "server_id": server_id, "error": error})
        return {"ok": False, "error": "CONNECT_FAILED", "message": error, "step": step, "step_key": key}

    async def disconnect(self) -> dict:
        if self._connect_lock.locked():
            return {"ok": False, "error": "BUSY", "message": "另一个连接/断开操作正在进行"}

        async with self._connect_lock:
            if self.connection_state == "idle":
                return {"ok": True, "state": "idle", "message": "本来就未连接"}

            self.connection_state = "disconnecting"
            self.bus.publish("connection_state_changed", {"state": "disconnecting"})

            # 停 heartbeat
            if self.heartbeat is not None:
                try:
                    await self.heartbeat.stop()
                except Exception as exc:  # noqa: BLE001
                    logger.warning("heartbeat stop failed: %s", exc)
                self.heartbeat = None

            # 停 traffic_meter + 解绑回调(M-γ)
            if self.traffic_meter is not None:
                try:
                    await self.traffic_meter.stop()
                except Exception as exc:  # noqa: BLE001
                    logger.warning("traffic_meter stop failed: %s", exc)
                self.traffic_meter = None
            try:
                self.proxy.set_progress_callback(None)
                self.resolver.set_event_publisher(None)
            except Exception:  # noqa: BLE001
                pass

            # 还系统代理
            if self._system_proxy_active and self.system_proxy is not None:
                try:
                    self.system_proxy.disable()
                except RuntimeError as exc:
                    logger.warning("system proxy disable failed: %s", exc)
                self._system_proxy_active = False

            # 切回 endpoint=None(后续访问全部直连)
            self.proxy.set_server_endpoint(None)

            self.connection_state = "idle"
            self.connected_server = None
            self.connected_since = None
            self.bus.publish("connection_state_changed", {"state": "idle"})
            return {"ok": True, "state": "idle"}

    def connection_snapshot(self) -> dict:
        """`GET /api/connection` 用,UI 也可初始拉一次校准状态。"""
        srv = self.connected_server
        hb = self.heartbeat
        return {
            "ok": True,
            "state": self.connection_state,
            "server": None if srv is None else {
                "server_id": srv.server_id,
                "name": srv.name,
                "host": srv.host,
                "port": srv.port,
                "socks": srv.socks,
                "api": srv.api,
                "vpn": srv.vpn,
                "version": srv.version,
            },
            "connected_since": self.connected_since,
            "system_proxy_active": self._system_proxy_active,
            "heartbeat": None if hb is None else {
                "tone": hb.state.tone,
                "consecutive_failures": hb.state.consecutive_failures,
                "last_check_at": hb.state.last_check_at,
                "last_error": hb.state.last_error,
            },
            "last_error": self.last_connect_error,
        }

    def request_stop(self) -> None:
        self._stop_event.set()

    async def serve_forever(self) -> None:
        if self.cfg.watchdog_ppid is not None:
            asyncio.create_task(self._watch_parent_pid(self.cfg.watchdog_ppid))

        await self._stop_event.wait()


    async def _watch_parent_pid(self, ppid: int) -> None:
        while not self._stop_event.is_set():
            try:
                os.kill(ppid, 0)
            except ProcessLookupError:
                logger.warning("parent pid %d disappeared, shutting down", ppid)
                self._stop_event.set()
                return
            except PermissionError:
                pass
            await asyncio.sleep(1.0)


# ---------------------------------------------------------------------------
# entry point
# ---------------------------------------------------------------------------


def _install_signal_handlers(loop: asyncio.AbstractEventLoop, runtime: ClientRuntime) -> None:
    def _handler():
        logger.info("signal received, requesting shutdown")
        runtime.request_stop()

    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, _handler)
        except (NotImplementedError, RuntimeError):
            pass


async def amain(cfg: ClientConfig) -> int:
    runtime = ClientRuntime(cfg)
    loop = asyncio.get_running_loop()
    _install_signal_handlers(loop, runtime)

    try:
        await runtime.start()
    except Exception as exc:
        logger.exception("startup failed: %s", exc)
        await runtime.stop()
        return 1

    logger.info(
        "conduit-client ready on socks5://%s:%d  api=http://127.0.0.1:%d  (server=%s, system_proxy=%s)",
        cfg.bind_host, runtime.proxy.actual_port, cfg.api_port,
        runtime.proxy.server_endpoint.label() if runtime.proxy.server_endpoint else "<none>",
        runtime._system_proxy_active,
    )
    try:
        await runtime.serve_forever()
    finally:
        await runtime.stop()
        logger.info("conduit-client stopped")
    return 0


def main() -> int:
    cfg = parse_args(sys.argv[1:])
    logging.basicConfig(
        level=getattr(logging, cfg.log_level.upper().replace("WARN", "WARNING")),
        format="%(asctime)s %(levelname)-7s %(name)s: %(message)s",
    )
    try:
        return asyncio.run(amain(cfg))
    except KeyboardInterrupt:
        return 0


if __name__ == "__main__":
    sys.exit(main())
