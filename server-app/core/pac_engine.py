"""Tiny PAC rules evaluator.

Parses ``proxy.pac`` for the section pattern used in this project (5 numbered
sections delimited by ``// ---------- N. xxx ----------`` comments) and exposes
a Python-side ``find_proxy(host)`` helper. Used by the ``/check?host=xxx``
diagnostics endpoint so users can verify routing decisions without touching a
browser.

The grammar handled is intentionally narrow — only the helpers used in
``proxy.pac``: ``shExpMatch(host, "...")`` and ``isInNet(host, "ip", "mask")``
plus the ``isPlainHostName`` / ``localhost`` short-circuit. If the PAC file is
later rewritten with extra logic, this engine falls back to ``UNKNOWN`` for
that section so the runtime keeps working.
"""
from __future__ import annotations

import fnmatch
import ipaddress
import re
from dataclasses import dataclass, field
from pathlib import Path

SECTION_RE = re.compile(
    r"^\s*//\s*-{2,}\s*(\d+)\.\s*(.+?)\s*-{2,}\s*$",
    re.IGNORECASE | re.MULTILINE,
)
SHEXP_RE = re.compile(r'shExpMatch\s*\(\s*host\s*,\s*"([^"]+)"\s*\)')
DNSDOMAIN_RE = re.compile(r'dnsDomainIs\s*\(\s*host\s*,\s*"([^"]+)"\s*\)')
ISINNET_RE = re.compile(
    r'isInNet\s*\(\s*host\s*,\s*"([\d.]+)"\s*,\s*"([\d.]+)"\s*\)'
)


@dataclass
class PacRules:
    local_globs: list[str] = field(default_factory=list)
    local_domains: list[str] = field(default_factory=list)
    local_nets: list[tuple[str, str]] = field(default_factory=list)
    internal_globs: list[str] = field(default_factory=list)
    internal_domains: list[str] = field(default_factory=list)
    fallback_globs: list[str] = field(default_factory=list)
    fallback_domains: list[str] = field(default_factory=list)
    cn_direct_globs: list[str] = field(default_factory=list)
    cn_direct_domains: list[str] = field(default_factory=list)
    proxy_target: str = "PROXY 192.168.1.3:8080"
    proxy_target_with_fallback: str = "PROXY 192.168.1.3:8080; DIRECT"
    source_path: str = ""

    def update_proxy_target(self, host: str, http_port: int) -> None:
        self.proxy_target = f"PROXY {host}:{http_port}"
        self.proxy_target_with_fallback = f"PROXY {host}:{http_port}; DIRECT"


def _split_sections(text: str) -> dict[int, str]:
    matches = list(SECTION_RE.finditer(text))
    if not matches:
        return {}
    out: dict[int, str] = {}
    for i, m in enumerate(matches):
        idx = int(m.group(1))
        start = m.end()
        end = matches[i + 1].start() if i + 1 < len(matches) else len(text)
        out[idx] = text[start:end]
    return out


def load_rules(pac_path: str | Path) -> PacRules:
    rules = PacRules()
    p = Path(pac_path)
    if not p.is_absolute():
        p = Path(__file__).resolve().parent / p
    rules.source_path = str(p)
    try:
        text = p.read_text(encoding="utf-8")
    except OSError:
        return rules
    sections = _split_sections(text)
    for idx, body in sections.items():
        globs = SHEXP_RE.findall(body)
        domains = DNSDOMAIN_RE.findall(body)
        nets = ISINNET_RE.findall(body)
        if idx == 1:
            rules.local_globs = globs
            rules.local_domains = domains
            rules.local_nets = nets
        elif idx == 2:
            rules.internal_globs = globs
            rules.internal_domains = domains
        elif idx == 3:
            rules.fallback_globs = globs
            rules.fallback_domains = domains
        elif idx == 4:
            rules.cn_direct_globs = globs
            rules.cn_direct_domains = domains
    return rules


def _is_plain_host(host: str) -> bool:
    return "." not in host


def _ip_in_net(ip_str: str, net_ip: str, net_mask: str) -> bool:
    try:
        addr = ipaddress.IPv4Address(ip_str)
        cidr = ipaddress.IPv4Network(f"{net_ip}/{net_mask}", strict=False)
    except (ValueError, ipaddress.AddressValueError):
        return False
    return addr in cidr


def _looks_like_ipv4(host: str) -> bool:
    try:
        ipaddress.IPv4Address(host)
        return True
    except (ValueError, ipaddress.AddressValueError):
        return False


def _glob_any(host: str, globs: list[str]) -> str | None:
    for g in globs:
        if fnmatch.fnmatchcase(host, g):
            return g
    return None


def _domain_any(host: str, domains: list[str]) -> str | None:
    for d in domains:
        d_norm = d.lstrip(".").lower()
        if host == d_norm or host.endswith("." + d_norm):
            return d
    return None


@dataclass
class Decision:
    proxy: str
    matched_section: str
    matched_pattern: str

    def to_dict(self) -> dict:
        return {
            "proxy": self.proxy,
            "matched_section": self.matched_section,
            "matched_pattern": self.matched_pattern,
        }


def find_proxy(host: str, rules: PacRules) -> Decision:
    host = host.lower().strip()
    if not host:
        return Decision("DIRECT", "5. default", "(empty host)")

    if _is_plain_host(host) or host in {"localhost"}:
        return Decision("DIRECT", "1. local/private", "isPlainHostName / localhost")
    g = _glob_any(host, rules.local_globs)
    if g:
        return Decision("DIRECT", "1. local/private", g)
    d = _domain_any(host, rules.local_domains)
    if d:
        return Decision("DIRECT", "1. local/private", f"dnsDomainIs({d})")
    if _looks_like_ipv4(host):
        for net_ip, mask in rules.local_nets:
            if _ip_in_net(host, net_ip, mask):
                return Decision("DIRECT", "1. local/private",
                                f"isInNet({net_ip}/{mask})")

    g = _glob_any(host, rules.internal_globs)
    if g:
        return Decision(rules.proxy_target, "2. internal (must use VPN)", g)
    d = _domain_any(host, rules.internal_domains)
    if d:
        return Decision(rules.proxy_target,
                        "2. internal (must use VPN)", f"dnsDomainIs({d})")

    g = _glob_any(host, rules.fallback_globs)
    if g:
        return Decision(rules.proxy_target,
                        "3. may need VPN (proxy, no DIRECT fallback)", g)
    d = _domain_any(host, rules.fallback_domains)
    if d:
        return Decision(rules.proxy_target,
                        "3. may need VPN (proxy, no DIRECT fallback)",
                        f"dnsDomainIs({d})")

    g = _glob_any(host, rules.cn_direct_globs)
    if g:
        return Decision("DIRECT", "4. CN direct", g)
    d = _domain_any(host, rules.cn_direct_domains)
    if d:
        return Decision("DIRECT", "4. CN direct", f"dnsDomainIs({d})")

    return Decision("DIRECT", "5. default", "(no rule matched)")
