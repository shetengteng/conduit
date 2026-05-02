"""mDNS / Bonjour service advertiser for Conduit Server.

Broadcasts ``_conduit._tcp.local.`` so that Conduit Client instances on the
same LAN can discover this server without manual IP entry.

TXT record fields (per design `2026-04-30-2-...md` §3.5.2):

- ``name``: human-friendly instance name (defaults to the OS hostname)
- ``port``: HTTP proxy port (= service port for the SRV record)
- ``socks``: SOCKS5 port
- ``api``: control API port (loopback only on the server side, but the
  client can use this number to compose the management URL once it knows
  the host)
- ``vpn``: ``"on"`` / ``"off"`` snapshot at advertise time
- ``version``: server-app version
- ``pac``: PAC URL relative path

Falls back gracefully if zeroconf isn't installed (the server still works,
just without auto-discovery — clients must add by IP manually).
"""
from __future__ import annotations

import logging
import socket
from typing import Optional

log = logging.getLogger("mdns")

SERVICE_TYPE = "_conduit._tcp.local."


def _hostname_short() -> str:
    return socket.gethostname().split(".")[0]


def _to_txt(d: dict[str, str]) -> dict[bytes, bytes]:
    return {k.encode("ascii"): v.encode("utf-8") for k, v in d.items()}


class MdnsAdvertiser:
    """Owns a single ``zeroconf.ServiceInfo`` registration.

    Use as ``async with`` or call ``register`` / ``unregister`` explicitly.
    """

    def __init__(
        self,
        name: str,
        host_ip: str,
        http_port: int,
        socks_port: int,
        api_port: int,
        vpn_on: bool = False,
        version: str = "0.1.0",
    ) -> None:
        self.name = name or _hostname_short()
        self.host_ip = host_ip
        self.http_port = http_port
        self.socks_port = socks_port
        self.api_port = api_port
        self.vpn_on = vpn_on
        self.version = version
        self._zc = None
        self._info = None

    async def register(self) -> bool:
        """Begin advertising. Returns False if zeroconf is unavailable."""
        try:
            from zeroconf.asyncio import AsyncServiceInfo, AsyncZeroconf
        except ImportError:
            log.warning("zeroconf not installed; mDNS advertiser disabled")
            return False

        addresses: list[bytes] = []
        try:
            addresses.append(socket.inet_aton(self.host_ip))
        except OSError:
            log.warning("invalid mDNS host IP %s, falling back to all interfaces",
                        self.host_ip)

        instance_name = f"Conduit on {self.name}.{SERVICE_TYPE}"
        properties = _to_txt({
            "name": self.name,
            "port": str(self.http_port),
            "socks": str(self.socks_port),
            "api": str(self.api_port),
            "vpn": "on" if self.vpn_on else "off",
            "version": self.version,
            "pac": "/proxy.pac",
        })
        info = AsyncServiceInfo(
            SERVICE_TYPE,
            instance_name,
            addresses=addresses or None,
            port=self.http_port,
            properties=properties,
            server=f"{self.name}.local.",
        )

        zc = AsyncZeroconf()
        # allow_name_change=True 让 zeroconf 在重名(NonUniqueNameException)时
        # 自动追加 #2/#3 直到拿到唯一名 —— 应对上次进程没正常 unregister 留下的
        # 残留广播(常见于 dev 模式 pkill 后重启)。
        await zc.async_register_service(info, allow_name_change=True)
        self._zc = zc
        self._info = info
        log.info("mDNS advertised: %s @ %s:%d (vpn=%s)",
                 info.name, self.host_ip, self.http_port,
                 "on" if self.vpn_on else "off")
        return True

    async def update_vpn(self, vpn_on: bool) -> None:
        """Refresh TXT record when VPN status changes."""
        if self._zc is None or self._info is None:
            self.vpn_on = vpn_on
            return
        if self.vpn_on == vpn_on:
            return
        self.vpn_on = vpn_on
        new_props = _to_txt({
            "name": self.name,
            "port": str(self.http_port),
            "socks": str(self.socks_port),
            "api": str(self.api_port),
            "vpn": "on" if vpn_on else "off",
            "version": self.version,
            "pac": "/proxy.pac",
        })
        try:
            await self._zc.async_update_service(self._info)
            self._info.properties = new_props
            log.info("mDNS TXT updated: vpn=%s", "on" if vpn_on else "off")
        except Exception as exc:  # noqa: BLE001
            log.warning("mDNS update_service failed: %s", exc)

    async def unregister(self) -> None:
        if self._zc is None:
            return
        try:
            if self._info is not None:
                await self._zc.async_unregister_service(self._info)
            await self._zc.async_close()
        except Exception as exc:  # noqa: BLE001
            log.warning("mDNS unregister failed: %s", exc)
        finally:
            self._zc = None
            self._info = None

    async def __aenter__(self) -> "MdnsAdvertiser":
        await self.register()
        return self

    async def __aexit__(self, *exc_info) -> None:
        await self.unregister()
