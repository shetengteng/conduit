"""ProxyCore — composes everything that makes the server-app sidecar work.

Owns the lifecycle of:

- the HTTP proxy listener (CONNECT + absolute-URI + PAC + /check + /status)
- the SOCKS5 listener
- the connection registry and traffic sampler
- the mDNS advertiser (best-effort; falls back if zeroconf is unavailable)

Provides ``start() / stop() / status()`` for the control API and the Tauri
shell. Designed so it can be embedded inside the same event loop as the
aiohttp control API server (see ``api/server.py``).

Design ref: ``design/2026-04-30-2-...md`` §3.5.2.
"""
from __future__ import annotations

import asyncio
import logging
import time
from typing import Optional

from active_connections import ConnectionRegistry, PassiveClientRegistry, TrafficSampler
from config import Config
from events_bus import EventBus
from healthcheck import HealthCheck
from http_proxy import handle_http
from mdns_advertiser import MdnsAdvertiser
from pac_engine import PacRules, load_rules
from socks5_proxy import handle_socks5

from api.server import ApiServer  # local-relative import is fine, package is on sys.path
from _version import VERSION  # 单一来源,见 _version.py 注释

log = logging.getLogger("core")


class ProxyCore:
    """Single-instance composition root for the proxy engine.

    Typical usage::

        cfg = parse_args(argv)
        core = ProxyCore(cfg)
        await core.start()
        # ... main service loop / API layer ...
        await core.stop()
    """

    def __init__(self, cfg: Config, rules: Optional[PacRules] = None) -> None:
        self.cfg = cfg
        self.rules = rules
        self.bus = EventBus()
        self.registry = ConnectionRegistry(publish=self.bus.publish)
        self.passive_clients = PassiveClientRegistry(publish=self.bus.publish)
        self.sampler = TrafficSampler(
            self.registry, cfg.traffic_sample_window_sec, publish=self.bus.publish
        )
        self.health = HealthCheck(cfg.http_port, cfg.socks_port, cfg.api_port)
        self._http_srv: Optional[asyncio.base_events.Server] = None
        self._socks_srv: Optional[asyncio.base_events.Server] = None
        self._mdns: Optional[MdnsAdvertiser] = None
        self._started_at: Optional[float] = None
        self._lock = asyncio.Lock()
        self._vpn_watch_task: Optional[asyncio.Task] = None
        self._last_vpn_state: Optional[bool] = None
        self.api = ApiServer(self)

    @property
    def running(self) -> bool:
        return self._http_srv is not None and self._socks_srv is not None

    @property
    def uptime_sec(self) -> int:
        if self._started_at is None:
            return 0
        return int(time.time() - self._started_at)

    async def start(self) -> None:
        async with self._lock:
            if self.running:
                return
            if self.rules is None:
                self.rules = load_rules(self.cfg.pac_file_path)
                self.rules.update_proxy_target(
                    self.cfg.pac_advertised_host or self.cfg.bind,
                    self.cfg.http_port,
                )

            self._http_srv = await asyncio.start_server(
                lambda r, w: handle_http(
                    r, w, self.cfg, self.rules, self.registry, self.passive_clients
                ),
                self.cfg.bind, self.cfg.http_port,
            )
            self._socks_srv = await asyncio.start_server(
                lambda r, w: handle_socks5(r, w, self.cfg, self.rules, self.registry),
                self.cfg.bind, self.cfg.socks_port,
            )
            log.info("HTTP listening on %s:%d", self.cfg.bind, self.cfg.http_port)
            log.info("SOCKS5 listening on %s:%d", self.cfg.bind, self.cfg.socks_port)

            self.sampler.start()
            self.passive_clients.start()
            self._vpn_watch_task = asyncio.create_task(self._watch_vpn_state())
            self._started_at = time.time()

            await self.api.start()

            if self.cfg.mdns_enabled and self.cfg.pac_advertised_host:
                self._mdns = MdnsAdvertiser(
                    name=self.cfg.mdns_service_name,
                    host_ip=self.cfg.pac_advertised_host,
                    http_port=self.cfg.http_port,
                    socks_port=self.cfg.socks_port,
                    api_port=self.cfg.api_port,
                    vpn_on=self._vpn_on_quick_check(),
                    version=VERSION,
                )
                await self._mdns.register()

    async def stop(self) -> None:
        async with self._lock:
            if not self.running:
                return
            log.info("shutdown signal received, draining...")
            await self.api.stop()
            if self._mdns is not None:
                await self._mdns.unregister()
                self._mdns = None
            for srv in (self._http_srv, self._socks_srv):
                if srv is not None:
                    srv.close()
            await asyncio.gather(
                *[s.wait_closed() for s in (self._http_srv, self._socks_srv) if s],
                return_exceptions=True,
            )
            await self.sampler.stop()
            await self.passive_clients.stop()
            if self._vpn_watch_task is not None:
                self._vpn_watch_task.cancel()
                try:
                    await self._vpn_watch_task
                except (asyncio.CancelledError, Exception):
                    pass
                self._vpn_watch_task = None
            self._http_srv = None
            self._socks_srv = None
            self._started_at = None

    async def serve_forever(self) -> None:
        if not self.running:
            await self.start()
        await asyncio.gather(
            self._http_srv.serve_forever(),  # type: ignore[union-attr]
            self._socks_srv.serve_forever(),  # type: ignore[union-attr]
            return_exceptions=True,
        )

    def _vpn_on_quick_check(self) -> bool:
        from healthcheck import _check_vpn  # local import to avoid cycles
        return _check_vpn().ok

    async def _watch_vpn_state(self) -> None:
        from healthcheck import _check_vpn
        while True:
            try:
                await asyncio.sleep(5.0)
                check = _check_vpn()
                cur = bool(check.ok)
                if self._last_vpn_state is None:
                    self._last_vpn_state = cur
                    continue
                if cur != self._last_vpn_state:
                    self._last_vpn_state = cur
                    self.bus.publish("vpn_state_changed", {
                        "available": cur,
                        "iface": check.detail,
                    })
                    log.info("VPN state changed: available=%s detail=%s", cur, check.detail)
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                log.warning("VPN watcher error: %s", exc)

    async def status(self) -> dict:
        health_dict = await self.health.to_dict()
        vpn_check = next(
            (c for c in health_dict["checks"] if c["name"] == "vpn_tunnel"),
            None,
        )
        lan_check = next(
            (c for c in health_dict["checks"] if c["name"] == "lan_ip"),
            None,
        )
        return {
            "running": self.running,
            "version": VERSION,
            "http_port": self.cfg.http_port,
            "socks5_port": self.cfg.socks_port,
            "api_port": self.cfg.api_port,
            "pac_url": self._pac_url(),
            "mdns": {
                "enabled": self.cfg.mdns_enabled,
                "name": self._effective_mdns_name(),
                "service_type": "_conduit._tcp.local.",
            },
            "vpn": {
                "available": bool(vpn_check and vpn_check["ok"]),
                "iface": vpn_check["detail"] if vpn_check else None,
                "default_route_via_vpn": bool(
                    vpn_check and "default route" in vpn_check["detail"]
                ),
            },
            "lan": {
                "available": bool(lan_check and lan_check["ok"]),
                "detail": lan_check["detail"] if lan_check else None,
            },
            "clients_count": len(self.registry),
            "passive_clients_count": len(self.passive_clients),
            "uptime_sec": self.uptime_sec,
            "ready": health_dict["ready"],
        }

    def _effective_mdns_name(self) -> str:
        """返回 mDNS 实际广播用的 instance name:用户传 --mdns-name 时用之,
        否则与 mdns_advertiser._hostname_short 同步,显示系统短主机名。
        """
        from mdns_advertiser import _hostname_short
        return self.cfg.mdns_service_name or _hostname_short()

    def _pac_url(self) -> Optional[str]:
        host = self.cfg.pac_advertised_host or self.cfg.bind
        if not host or host == "0.0.0.0":
            return None
        return f"http://{host}:{self.cfg.http_port}/proxy.pac"
