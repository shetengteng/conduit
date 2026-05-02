"""Lightweight health-check probes for the server-app sidecar.

Exposes :class:`HealthCheck` whose ``is_ready()`` / ``details()`` answer two
questions for the Tauri shell and the UI:

1. *Are my listening ports actually open?* (HTTP proxy + SOCKS5 + control API)
2. *Is the machine in a state where the proxy is useful?* (LAN IP detected,
   VPN tunnel available)

Designed to be cheap (< 50ms when ready) so the Tauri shell can poll
``/healthz`` aggressively at startup without slowing the UI.

Design ref: ``design/2026-04-30-2-...md`` §3.5.2 / §4.7.
"""
from __future__ import annotations

import asyncio
import ipaddress
import re
import socket
import subprocess
from dataclasses import dataclass

_PHYSICAL_IFACE = re.compile(r"^(en|eth|wl|wlan|wifi)\d+$")
_VPN_IFACE_PREFIX = ("utun", "ppp", "tun", "tap", "gpd")


@dataclass
class CheckResult:
    name: str
    ok: bool
    detail: str = ""


def _check_port(host: str, port: int, name: str, timeout: float = 0.3) -> CheckResult:
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return CheckResult(name, True, f"listening on {host}:{port}")
    except OSError as exc:
        return CheckResult(name, False, f"{exc.__class__.__name__}: {exc}")


def _list_ifaces() -> list[tuple[str, str]]:
    try:
        out = subprocess.check_output(
            ["ifconfig"], stderr=subprocess.DEVNULL, timeout=2
        ).decode("utf-8", errors="replace")
    except (FileNotFoundError, subprocess.SubprocessError):
        return []
    results: list[tuple[str, str]] = []
    iface = None
    for line in out.splitlines():
        if not line:
            continue
        if not line.startswith("\t") and not line.startswith(" "):
            iface = line.split(":", 1)[0].strip()
            continue
        if iface and "inet " in line:
            parts = line.strip().split()
            if len(parts) >= 2 and parts[0] == "inet":
                results.append((iface, parts[1]))
    return results


def _detect_default_iface() -> str | None:
    try:
        out = subprocess.check_output(
            ["route", "-n", "get", "default"],
            stderr=subprocess.DEVNULL, timeout=2,
        ).decode("utf-8", errors="replace")
    except (FileNotFoundError, subprocess.SubprocessError):
        return None
    for line in out.splitlines():
        line = line.strip()
        if line.startswith("interface:"):
            return line.split(":", 1)[1].strip()
    return None


def _check_lan_ip() -> CheckResult:
    ifaces = _list_ifaces()
    for iface, ip in ifaces:
        if not _PHYSICAL_IFACE.match(iface):
            continue
        try:
            if ipaddress.ip_address(ip).is_private:
                return CheckResult("lan_ip", True, f"{iface}={ip}")
        except ValueError:
            continue
    return CheckResult("lan_ip", False, "no physical private IPv4 interface")


def _check_vpn() -> CheckResult:
    ifaces = _list_ifaces()
    vpn_ifaces = [
        (iface, ip) for iface, ip in ifaces
        if any(iface.startswith(p) for p in _VPN_IFACE_PREFIX)
    ]
    if not vpn_ifaces:
        return CheckResult("vpn_tunnel", False, "no utun/ppp/tun interface")
    default_iface = _detect_default_iface()
    detail = ", ".join(f"{i}={ip}" for i, ip in vpn_ifaces)
    if default_iface and any(default_iface == i for i, _ in vpn_ifaces):
        return CheckResult("vpn_tunnel", True, f"{detail} (default route)")
    return CheckResult("vpn_tunnel", True, f"{detail} (not default route)")


class HealthCheck:
    """Aggregates port + network probes. Cheap to call.

    Caller passes the ports the server is supposed to bind. ``is_ready``
    requires ports to be live; LAN/VPN are advisory and reported separately.
    """

    def __init__(self, http_port: int, socks_port: int, api_port: int) -> None:
        self._http_port = http_port
        self._socks_port = socks_port
        self._api_port = api_port

    PORT_CHECK_NAMES = ("http_port", "socks5_port", "api_port")

    async def details(self) -> list[CheckResult]:
        loop = asyncio.get_running_loop()
        ports = [
            ("127.0.0.1", self._http_port, "http_port"),
            ("127.0.0.1", self._socks_port, "socks5_port"),
            ("127.0.0.1", self._api_port, "api_port"),
        ]
        port_results = await asyncio.gather(*[
            loop.run_in_executor(None, _check_port, h, p, n) for h, p, n in ports
        ])
        net_results = await asyncio.gather(
            loop.run_in_executor(None, _check_lan_ip),
            loop.run_in_executor(None, _check_vpn),
        )
        return list(port_results) + list(net_results)

    def _is_port_check(self, name: str) -> bool:
        return name in self.PORT_CHECK_NAMES

    async def is_ready(self) -> bool:
        results = await self.details()
        return all(r.ok for r in results if self._is_port_check(r.name))

    async def to_dict(self) -> dict:
        results = await self.details()
        return {
            "ready": all(r.ok for r in results if self._is_port_check(r.name)),
            "checks": [
                {"name": r.name, "ok": r.ok, "detail": r.detail}
                for r in results
            ],
        }
