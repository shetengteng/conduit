"""macOS system proxy switch.

Wraps the four ``networksetup`` calls we need:

* ``-listallnetworkservices``         — enumerate visible services
* ``-getsocksfirewallproxy <svc>``    — read current SOCKS proxy
* ``-setsocksfirewallproxy <svc> ...``— point SOCKS proxy at host:port
* ``-setsocksfirewallproxystate``     — toggle on/off

We deliberately avoid subprocess sugar libraries and shell interpolation
so this module remains trivial to package with PyInstaller / Nuitka.

A ``ProcessRunner`` indirection makes the module testable on a CI box
where ``networksetup`` does not exist (Linux runner) — production code
uses the default ``_run_real_subprocess`` implementation.

Cross-references:
* design/2026-04-30-3-Conduit-Client-客户端可行性报告.md §3.7
"""
from __future__ import annotations

import logging
import shutil
import subprocess
from dataclasses import dataclass
from typing import Callable, Sequence

logger = logging.getLogger("conduit.client.system_proxy")

NETWORKSETUP = "/usr/sbin/networksetup"
DEFAULT_HOST = "127.0.0.1"
DEFAULT_PORT = 7890


# ---------------------------------------------------------------------------
# subprocess indirection (so tests can inject a fake)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ProcessResult:
    args: list[str]
    returncode: int
    stdout: str
    stderr: str


ProcessRunner = Callable[[Sequence[str]], ProcessResult]


def _run_real_subprocess(args: Sequence[str]) -> ProcessResult:
    completed = subprocess.run(
        list(args),
        capture_output=True,
        text=True,
        check=False,
    )
    return ProcessResult(
        args=list(args),
        returncode=completed.returncode,
        stdout=completed.stdout,
        stderr=completed.stderr,
    )


# ---------------------------------------------------------------------------
# state object returned by getsocksfirewallproxy
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class SocksProxyState:
    enabled: bool
    server: str
    port: int

    def points_to(self, host: str, port: int) -> bool:
        return self.enabled and self.server == host and self.port == port


# ---------------------------------------------------------------------------
# main API
# ---------------------------------------------------------------------------


class MacSystemProxy:
    """Thin imperative wrapper for macOS ``networksetup`` SOCKS settings."""

    def __init__(self, runner: ProcessRunner | None = None) -> None:
        self._run = runner or _run_real_subprocess

    # ------------------------------------------------------------------
    # availability check
    # ------------------------------------------------------------------

    @staticmethod
    def is_supported() -> bool:
        return shutil.which(NETWORKSETUP) is not None

    # ------------------------------------------------------------------
    # service discovery
    # ------------------------------------------------------------------

    def list_services(self) -> list[str]:
        out = self._run([NETWORKSETUP, "-listallnetworkservices"])
        if out.returncode != 0:
            raise RuntimeError(
                f"networksetup -listallnetworkservices failed: {out.stderr.strip()}"
            )
        lines = out.stdout.splitlines()
        services: list[str] = []
        for raw in lines:
            line = raw.strip()
            if not line:
                continue
            low = line.lower()
            if "denotes that" in low or low.startswith("an asterisk"):
                continue
            if line.startswith("*"):
                continue
            services.append(line)
        return services

    def active_service(self) -> str:
        services = self.list_services()
        if not services:
            raise RuntimeError("no active network services found")
        for preferred in ("Wi-Fi", "Wi‑Fi", "WiFi", "Ethernet", "USB 10/100/1000 LAN"):
            if preferred in services:
                return preferred
        return services[0]

    # ------------------------------------------------------------------
    # read current SOCKS state
    # ------------------------------------------------------------------

    def get_socks_proxy(self, service: str | None = None) -> SocksProxyState:
        svc = service or self.active_service()
        out = self._run([NETWORKSETUP, "-getsocksfirewallproxy", svc])
        if out.returncode != 0:
            raise RuntimeError(
                f"-getsocksfirewallproxy {svc!r} failed: {out.stderr.strip()}"
            )
        enabled = False
        server = ""
        port = 0
        for raw in out.stdout.splitlines():
            line = raw.strip()
            if not line:
                continue
            key, _, value = line.partition(":")
            key = key.strip().lower()
            value = value.strip()
            if key == "enabled":
                enabled = value.lower() in ("yes", "true", "1")
            elif key == "server":
                server = value
            elif key == "port":
                try:
                    port = int(value)
                except ValueError:
                    port = 0
        return SocksProxyState(enabled=enabled, server=server, port=port)

    def is_set_to_us(
        self,
        *,
        host: str = DEFAULT_HOST,
        port: int = DEFAULT_PORT,
        service: str | None = None,
    ) -> bool:
        try:
            state = self.get_socks_proxy(service)
        except RuntimeError as exc:
            logger.debug("is_set_to_us: %s", exc)
            return False
        return state.points_to(host, port)

    # ------------------------------------------------------------------
    # mutators
    # ------------------------------------------------------------------

    def enable(
        self,
        *,
        host: str = DEFAULT_HOST,
        port: int = DEFAULT_PORT,
        service: str | None = None,
    ) -> str:
        svc = service or self.active_service()
        a = self._run([NETWORKSETUP, "-setsocksfirewallproxy", svc, host, str(port)])
        if a.returncode != 0:
            raise RuntimeError(
                f"-setsocksfirewallproxy {svc!r} failed: {a.stderr.strip()}"
            )
        b = self._run([NETWORKSETUP, "-setsocksfirewallproxystate", svc, "on"])
        if b.returncode != 0:
            raise RuntimeError(
                f"-setsocksfirewallproxystate {svc!r} on failed: {b.stderr.strip()}"
            )
        logger.info("system proxy enabled: %s -> %s:%d", svc, host, port)
        return svc

    def disable(self, *, service: str | None = None) -> str:
        svc = service or self.active_service()
        out = self._run([NETWORKSETUP, "-setsocksfirewallproxystate", svc, "off"])
        if out.returncode != 0:
            raise RuntimeError(
                f"-setsocksfirewallproxystate {svc!r} off failed: {out.stderr.strip()}"
            )
        logger.info("system proxy disabled: %s", svc)
        return svc

    # ------------------------------------------------------------------
    # post-crash cleanup helper
    # ------------------------------------------------------------------

    def cleanup_if_pointing_to_us(
        self,
        *,
        host: str = DEFAULT_HOST,
        port: int = DEFAULT_PORT,
    ) -> bool:
        """Disable system SOCKS if it's still pointing at our (now dead) proxy.

        Run from ``client_main`` at startup so a previous crash that left
        the system pointing at ``127.0.0.1:7890`` (with no listener)
        doesn't break the user's networking.
        """
        try:
            services = self.list_services()
        except RuntimeError as exc:
            logger.warning("cleanup_if_pointing_to_us: list_services failed: %s", exc)
            return False
        cleaned = False
        for svc in services:
            try:
                state = self.get_socks_proxy(svc)
            except RuntimeError:
                continue
            if state.points_to(host, port):
                try:
                    self.disable(service=svc)
                    cleaned = True
                except RuntimeError as exc:
                    logger.warning(
                        "cleanup_if_pointing_to_us: disable %s failed: %s", svc, exc,
                    )
        return cleaned


__all__ = [
    "DEFAULT_HOST",
    "DEFAULT_PORT",
    "MacSystemProxy",
    "ProcessResult",
    "ProcessRunner",
    "SocksProxyState",
]
