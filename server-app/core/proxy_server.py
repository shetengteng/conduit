#!/usr/bin/env python3
"""LAN VPN sharing proxy entry point: HTTP (8080) + SOCKS5 (1080) + PAC + control API (8090).

Run on machine A (which owns the corporate VPN). Other LAN devices then point
their HTTP/SOCKS proxy or PAC URL at this machine to share the VPN egress.

The heavy lifting now lives in :class:`proxy_core.ProxyCore` so that the
Tauri shell can call the same ``start()`` / ``stop()`` semantics while the
CLI keeps its banner + interactive confirmation flow.
"""
from __future__ import annotations

import asyncio
import ipaddress
import logging
import os
import re
import signal
import subprocess
import sys

from config import Config, parse_args
from pac_engine import PacRules, load_rules
from proxy_core import ProxyCore

log = logging.getLogger("proxy")


def _setup_logging(cfg: Config) -> None:
    level = getattr(logging, cfg.log_level.upper(), logging.INFO)
    fmt = "%(asctime)s %(levelname)s %(name)s %(message)s"
    log_dir = os.path.dirname(cfg.log_file)
    if log_dir and not os.path.isdir(log_dir):
        try:
            os.makedirs(log_dir, exist_ok=True)
        except OSError:
            pass
    logging.basicConfig(
        level=level,
        format=fmt,
        handlers=[
            logging.StreamHandler(sys.stdout),
            logging.FileHandler(cfg.log_file, encoding="utf-8"),
        ],
    )
    logging.getLogger("asyncio").setLevel(logging.WARNING)


def _detect_default_route() -> tuple[str, str]:
    try:
        out = subprocess.check_output(
            ["route", "-n", "get", "default"], stderr=subprocess.DEVNULL, timeout=2
        ).decode("utf-8", errors="replace")
    except (FileNotFoundError, subprocess.SubprocessError):
        return "?", "?"
    iface, gw = "?", "?"
    for line in out.splitlines():
        line = line.strip()
        if line.startswith("interface:"):
            iface = line.split(":", 1)[1].strip()
        elif line.startswith("gateway:"):
            gw = line.split(":", 1)[1].strip()
    return iface, gw


_PHYSICAL_IFACE = re.compile(r"^(en|eth|wl|wlan|wifi)\d+$")
_VIRTUAL_IFACE_PREFIX = ("utun", "ppp", "tun", "tap", "gif", "stf",
                         "lo", "bridge", "awdl", "llw", "anpi", "vmenet")


def _list_local_ipv4() -> list[tuple[str, str]]:
    try:
        out = subprocess.check_output(["ifconfig"], stderr=subprocess.DEVNULL,
                                      timeout=2).decode("utf-8", errors="replace")
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


def _pick_lan_ip(candidates: list[tuple[str, str]]) -> str:
    physical_private: list[str] = []
    physical_other: list[str] = []
    other: list[str] = []
    for iface, ip in candidates:
        if iface.startswith(_VIRTUAL_IFACE_PREFIX):
            continue
        try:
            addr = ipaddress.ip_address(ip)
        except ValueError:
            continue
        if addr.is_loopback:
            continue
        is_phys = bool(_PHYSICAL_IFACE.match(iface))
        if is_phys and addr.is_private:
            physical_private.append(ip)
        elif is_phys:
            physical_other.append(ip)
        elif addr.is_private:
            other.append(ip)
    if physical_private:
        return physical_private[0]
    if physical_other:
        return physical_other[0]
    if other:
        return other[0]
    return "?"


def _print_banner_with_state(cfg: Config, candidates: list[tuple[str, str]],
                             lan_ip: str, rules: PacRules) -> bool:
    iface, gw = _detect_default_route()
    advertised = cfg.pac_advertised_host or lan_ip

    border = "=" * 70
    print(border)
    print("  Conduit Server  (HTTP + SOCKS5 + PAC + control API)")
    print(border)
    print(f"  Default route via : {iface}  (gateway {gw})  [VPN tunnel]")
    print(f"  LAN IP (for share): {lan_ip}")
    print( "  All local IPv4    :")
    if candidates:
        for cand_iface, cand_ip in candidates:
            tag = ""
            if cand_iface.startswith(("utun", "ppp", "tun", "tap")):
                tag = "  (VPN/virtual — DO NOT advertise to LAN)"
            elif cand_iface.startswith("lo"):
                tag = "  (loopback)"
            elif _PHYSICAL_IFACE.match(cand_iface):
                tag = "  (physical — share this to LAN devices)"
            print(f"    - {cand_iface:<10} {cand_ip}{tag}")
    else:
        print("    - (ifconfig unavailable)")
    print(f"  HTTP  proxy       : http://{advertised}:{cfg.http_port}")
    print(f"  SOCKS5 proxy      : socks5://{advertised}:{cfg.socks_port}")
    print(f"  PAC URL           : http://{advertised}:{cfg.http_port}/proxy.pac")
    print(f"  Control API       : http://127.0.0.1:{cfg.api_port}/  (loopback only)")
    print(f"  mDNS broadcast    : {'enabled (_conduit._tcp.local.)' if cfg.mdns_enabled else 'disabled'}")
    print(f"  Allowed CIDRs     : {', '.join(cfg.allowed_cidrs)}")
    print(f"  Allowed CONNECT   : {sorted(cfg.allowed_connect_ports)}")
    print(f"  PAC rules loaded  : {len(rules.internal_domains)} internal | "
          f"{len(rules.fallback_domains)} fallback | "
          f"{len(rules.cn_direct_domains)} CN-direct")
    if cfg.direct_first and cfg.physical_iface_ip:
        print(f"  Outbound mode     : DIRECT-first (bind {cfg.physical_iface_ip}, "
              f"head-start {cfg.direct_first_timeout_s}s) → VPN fallback")
        print(f"  Route cache TTL   : {int(cfg.direct_cache_ttl_s)}s")
    elif cfg.direct_first:
        print( "  Outbound mode     : VPN-only (no physical IP detected, "
               "DIRECT-first disabled)")
    else:
        print( "  Outbound mode     : VPN-only (--no-direct-first)")
    print(border)
    print("  WARNING: This service forwards LAN traffic through your corporate")
    print("           VPN. Most company IT policies forbid this. Use only on")
    print("           private/home WiFi at your own risk.")
    print(border)
    if cfg.skip_banner:
        return True
    try:
        ans = input("Type 'y' to continue, anything else to abort: ").strip().lower()
    except EOFError:
        return False
    return ans == "y"


async def _orphan_watchdog(stop: asyncio.Event, parent_pid: int) -> None:
    """Self-terminate when our parent process dies (PPID becomes 1).

    Used by the Tauri shell to make sure the python sidecar gets cleaned up
    even when the main process is force-killed (SIGKILL / OOM).
    """
    while not stop.is_set():
        try:
            current_ppid = os.getppid()
            if current_ppid in (1, 0) or current_ppid != parent_pid:
                log.warning(
                    "watchdog: parent %d gone (now ppid=%d), exiting", parent_pid, current_ppid
                )
                stop.set()
                return
        except Exception as exc:
            log.debug("watchdog tick error: %s", exc)
        try:
            await asyncio.wait_for(stop.wait(), timeout=2.0)
        except asyncio.TimeoutError:
            continue


async def _serve(core: ProxyCore, watchdog_ppid: int | None = None) -> None:
    await core.start()
    log.info(
        "PAC rules: %d internal domains, %d fallback, %d CN-direct (from %s)",
        len(core.rules.internal_domains),  # type: ignore[union-attr]
        len(core.rules.fallback_domains),  # type: ignore[union-attr]
        len(core.rules.cn_direct_domains),  # type: ignore[union-attr]
        core.rules.source_path,  # type: ignore[union-attr]
    )

    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, stop.set)
        except NotImplementedError:
            pass

    watchdog_task: asyncio.Task | None = None
    if watchdog_ppid is not None and watchdog_ppid > 1:
        watchdog_task = asyncio.create_task(_orphan_watchdog(stop, watchdog_ppid))
        log.info("orphan watchdog enabled (parent pid=%d)", watchdog_ppid)

    serve_task = asyncio.create_task(core.serve_forever())
    stop_task = asyncio.create_task(stop.wait())
    await asyncio.wait(
        {serve_task, stop_task}, return_when=asyncio.FIRST_COMPLETED
    )
    await core.stop()
    if watchdog_task is not None and not watchdog_task.done():
        watchdog_task.cancel()
        try:
            await watchdog_task
        except (asyncio.CancelledError, Exception):
            pass
    if not serve_task.done():
        serve_task.cancel()
        try:
            await serve_task
        except (asyncio.CancelledError, Exception):
            pass


def main() -> int:
    cfg = parse_args(sys.argv[1:])
    _setup_logging(cfg)

    candidates = _list_local_ipv4()
    lan_ip = _pick_lan_ip(candidates)
    if not cfg.pac_advertised_host or cfg.pac_advertised_host == "":
        cfg.pac_advertised_host = lan_ip if lan_ip != "?" else cfg.bind
    if not cfg.physical_iface_ip and lan_ip != "?":
        cfg.physical_iface_ip = lan_ip

    rules = load_rules(cfg.pac_file_path)
    rules.update_proxy_target(cfg.pac_advertised_host, cfg.http_port)

    if not _print_banner_with_state(cfg, candidates, lan_ip, rules):
        print("aborted.")
        return 1

    core = ProxyCore(cfg, rules)
    try:
        asyncio.run(_serve(core, watchdog_ppid=cfg.watchdog_ppid))
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
