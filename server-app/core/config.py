"""Configuration for the LAN VPN proxy server.

Loaded from CLI args (argparse) on top of dataclass defaults. No file IO.
"""
from __future__ import annotations

import argparse
import ipaddress
from dataclasses import dataclass, field


@dataclass
class Config:
    bind: str = "0.0.0.0"
    http_port: int = 8080
    socks_port: int = 1080

    api_port: int = 8090
    api_bind_loopback_only: bool = True

    mdns_enabled: bool = True
    mdns_service_name: str = ""

    allowed_cidrs: list[str] = field(default_factory=lambda: [
        "192.168.0.0/16",
        "10.0.0.0/8",
        "172.16.0.0/12",
        "127.0.0.0/8",
    ])
    allowed_connect_ports: set[int] = field(default_factory=lambda: {
        80, 443, 22, 8080, 8443, 8118, 8888, 9000, 9443,
    })

    pac_file_path: str = "proxy.pac"
    pac_endpoints: tuple[str, ...] = ("/proxy.pac", "/wpad.dat")
    pac_advertised_host: str = ""

    log_file: str = "log/proxy.log"
    log_level: str = "INFO"
    redact_query: bool = True

    handshake_timeout_s: float = 10.0
    connect_timeout_s: float = 10.0
    skip_banner: bool = False

    direct_first: bool = True
    direct_first_timeout_s: float = 1.5
    direct_cache_ttl_s: float = 300.0
    physical_iface_ip: str = ""

    traffic_sample_window_sec: int = 600

    watchdog_ppid: int | None = None

    def is_client_allowed(self, peer_ip: str) -> bool:
        try:
            ip = ipaddress.ip_address(peer_ip)
        except ValueError:
            return False
        for cidr in self.allowed_cidrs:
            try:
                if ip in ipaddress.ip_network(cidr, strict=False):
                    return True
            except ValueError:
                continue
        return False

    def is_connect_port_allowed(self, port: int) -> bool:
        return port in self.allowed_connect_ports


def parse_args(argv: list[str] | None = None) -> Config:
    cfg = Config()
    p = argparse.ArgumentParser(description="LAN VPN sharing proxy (HTTP + SOCKS5)")
    p.add_argument("--bind", default=cfg.bind, help="Bind address (default: 0.0.0.0)")
    p.add_argument("--http-port", type=int, default=cfg.http_port)
    p.add_argument("--socks-port", type=int, default=cfg.socks_port)
    p.add_argument(
        "--allow-cidr",
        action="append",
        default=None,
        help="Replace default LAN CIDR allowlist; pass multiple times.",
    )
    p.add_argument(
        "--allow-port",
        action="append",
        type=int,
        default=None,
        help="Replace default CONNECT port allowlist; pass multiple times.",
    )
    p.add_argument("--pac-file", default=cfg.pac_file_path)
    p.add_argument(
        "--pac-host",
        default="",
        help="Hostname or IP injected into PAC's PROXY directive "
             "(e.g. HW0023148.local). Defaults to auto-detected LAN IP.",
    )
    p.add_argument("--log-file", default=cfg.log_file)
    p.add_argument("--log-level", default=cfg.log_level,
                   choices=["DEBUG", "INFO", "WARNING", "ERROR"])
    p.add_argument("--yes", action="store_true",
                   help="Skip the risk-confirmation banner.")

    p.add_argument("--no-direct-first", dest="direct_first", action="store_false",
                   help="Disable DIRECT-first; route every outbound connection "
                        "through the default route (i.e. VPN). Default: enabled.")
    p.set_defaults(direct_first=True)
    p.add_argument("--direct-timeout", type=float, default=cfg.direct_first_timeout_s,
                   help="DIRECT head-start window in seconds before VPN joins "
                        "the race (default: 1.5).")
    p.add_argument("--direct-cache-ttl", type=float, default=cfg.direct_cache_ttl_s,
                   help="Per-host route cache TTL in seconds (default: 300).")
    p.add_argument("--physical-iface-ip", default="",
                   help="Force the source IPv4 used for DIRECT attempts "
                        "(default: auto-detected from physical interfaces).")

    p.add_argument("--api-port", type=int, default=cfg.api_port,
                   help="Local control API port (default: 8090, loopback-only).")
    p.add_argument("--no-mdns", dest="mdns_enabled", action="store_false",
                   help="Disable mDNS service advertisement (_conduit._tcp.local.).")
    p.set_defaults(mdns_enabled=True)
    p.add_argument("--mdns-name", default="",
                   help="Service instance name for mDNS (default: <hostname>).")
    p.add_argument("--watchdog-ppid", type=int, default=None,
                   help="If set, sidecar self-exits when its parent process "
                        "(this PID) dies. Used by the Tauri shell.")

    ns = p.parse_args(argv)

    cfg.bind = ns.bind
    cfg.http_port = ns.http_port
    cfg.socks_port = ns.socks_port
    if ns.allow_cidr:
        cfg.allowed_cidrs = list(ns.allow_cidr)
    if ns.allow_port:
        cfg.allowed_connect_ports = set(ns.allow_port)
    cfg.pac_file_path = ns.pac_file
    cfg.pac_advertised_host = ns.pac_host
    cfg.log_file = ns.log_file
    cfg.log_level = ns.log_level
    cfg.skip_banner = ns.yes
    cfg.direct_first = ns.direct_first
    cfg.direct_first_timeout_s = ns.direct_timeout
    cfg.direct_cache_ttl_s = ns.direct_cache_ttl
    if ns.physical_iface_ip:
        cfg.physical_iface_ip = ns.physical_iface_ip
    cfg.api_port = ns.api_port
    cfg.mdns_enabled = ns.mdns_enabled
    cfg.mdns_service_name = ns.mdns_name
    cfg.watchdog_ppid = ns.watchdog_ppid
    return cfg
