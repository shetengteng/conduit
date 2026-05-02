"""Server PAC file parser for the Conduit client-app.

We do NOT execute the PAC JavaScript.  The client-app only needs the
*set of host patterns* that the server has declared as **must-go-via-server**
(i.e. anything that lands in a ``return PROXY;`` branch).  Those patterns
are pre-loaded into the local route cache as ``proxy`` entries so we
never waste a probe on hosts that are known to require the VPN.

Strategy
--------

1. Strip JavaScript comments (line + block) so commented-out hostnames
   don't leak into the result.
2. Walk the source character-by-character to locate top-level
   ``if (...) { ... }`` blocks.  We track parenthesis / brace depth so
   nested expressions are handled correctly.
3. For each block, check whether its body returns ``PROXY``.  If so,
   extract every literal hostname argument inside ``dnsDomainIs(host, "...")``
   and ``shExpMatch(host, "...")`` calls within the *condition*, and
   emit a normalised pattern.

The output patterns use the same convention understood by
``route_resolver._pattern_match``:

* ``dnsDomainIs(host, "zoom.us")``        →  ``*.zoom.us``
* ``shExpMatch(host, "*.zoom.us")``       →  ``*.zoom.us``
* ``shExpMatch(host, "git.zoom.us")``     →  ``git.zoom.us``  (exact)

Cross-references:
* design/2026-04-30-3-Conduit-Client-客户端可行性报告.md §3.5
  (cache pre-fill from PAC)
* server-app/core/proxy.pac (canonical fixture)
"""
from __future__ import annotations

import re
from dataclasses import dataclass

DNS_DOMAIN_RE = re.compile(
    r'dnsDomainIs\s*\(\s*host\s*,\s*"([^"]+)"\s*\)',
    re.IGNORECASE,
)
SH_EXP_RE = re.compile(
    r'shExpMatch\s*\(\s*host\s*,\s*"([^"]+)"\s*\)',
    re.IGNORECASE,
)


@dataclass(frozen=True)
class PacExtraction:
    proxy_patterns: list[str]
    direct_patterns: list[str]


def _strip_js_comments(src: str) -> str:
    """Remove ``// ...`` line and ``/* ... */`` block comments."""
    src = re.sub(r"/\*.*?\*/", "", src, flags=re.DOTALL)
    src = re.sub(r"//[^\n]*", "", src)
    return src


def _iter_if_blocks(src: str) -> list[tuple[str, str]]:
    """Yield ``(condition, body)`` for every top-level ``if (...) { ... }``.

    Implemented via a small character-level scan so nested parens / braces
    don't trip us up (regex with backreferences would be hard to read).
    """
    out: list[tuple[str, str]] = []
    i = 0
    n = len(src)
    while i < n:
        idx = src.find("if", i)
        if idx == -1:
            return out
        before = src[idx - 1] if idx > 0 else " "
        after = src[idx + 2] if idx + 2 < n else " "
        if (before.isalnum() or before == "_") or (after.isalnum() or after == "_"):
            i = idx + 2
            continue

        j = idx + 2
        while j < n and src[j].isspace():
            j += 1
        if j >= n or src[j] != "(":
            i = idx + 2
            continue

        depth = 0
        cond_start = j + 1
        cond_end = -1
        while j < n:
            c = src[j]
            if c == "(":
                depth += 1
            elif c == ")":
                depth -= 1
                if depth == 0:
                    cond_end = j
                    break
            j += 1
        if cond_end == -1:
            return out

        k = cond_end + 1
        while k < n and src[k].isspace():
            k += 1
        if k >= n or src[k] != "{":
            i = cond_end + 1
            continue

        body_start = k + 1
        depth = 1
        m = body_start
        while m < n and depth > 0:
            c = src[m]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
            m += 1
        if depth != 0:
            return out
        body_end = m - 1
        out.append((src[cond_start:cond_end], src[body_start:body_end]))
        i = m
    return out


def _block_returns(body: str) -> str | None:
    """Return ``'PROXY'`` / ``'DIRECT'`` for the first return inside body.

    PAC bodies typically use one of two styles:

    * ``return PROXY;``                       (where ``PROXY`` is a local var)
    * ``return "PROXY 192.168.1.3:8080";``    (string literal)

    Both must be detected, including bare ``DIRECT`` / ``"DIRECT"``.
    """
    m = re.search(
        r'return\s+(?:"(PROXY|DIRECT)[^"]*"|(PROXY|DIRECT))\s*;?',
        body,
        re.IGNORECASE,
    )
    if m:
        return (m.group(1) or m.group(2)).upper()
    return None


def _normalise_dns_domain(domain: str) -> str:
    """Convert ``zoom.us`` (dnsDomainIs semantics) into our ``*.zoom.us`` pattern.

    ``dnsDomainIs(host, X)`` matches both the apex ``X`` and any subdomain
    ``foo.X``, which is exactly what ``*.X`` means in our resolver.
    """
    domain = domain.lower().strip()
    if domain.startswith("*.") or domain.startswith("."):
        return domain
    return f"*.{domain}"


def extract_patterns(pac_source: str) -> PacExtraction:
    """Return PROXY / DIRECT host patterns referenced in the PAC source."""
    cleaned = _strip_js_comments(pac_source)
    proxy: list[str] = []
    direct: list[str] = []
    seen_proxy: set[str] = set()
    seen_direct: set[str] = set()

    for cond, body in _iter_if_blocks(cleaned):
        target = _block_returns(body)
        if target is None:
            continue
        bucket = proxy if target == "PROXY" else direct
        seen = seen_proxy if target == "PROXY" else seen_direct
        for raw in DNS_DOMAIN_RE.findall(cond):
            pat = _normalise_dns_domain(raw)
            if pat not in seen:
                seen.add(pat)
                bucket.append(pat)
        for raw in SH_EXP_RE.findall(cond):
            pat = raw.lower().strip()
            if pat and pat not in seen:
                seen.add(pat)
                bucket.append(pat)
    return PacExtraction(proxy_patterns=proxy, direct_patterns=direct)


def extract_proxy_hosts(pac_source: str) -> list[str]:
    """Convenience wrapper for client_main / cache pre-fill.

    Only the PROXY-bound patterns are useful for cache pre-fill — DIRECT
    hosts will be probed normally and end up cached by the resolver.
    """
    return extract_patterns(pac_source).proxy_patterns


__all__ = [
    "PacExtraction",
    "extract_patterns",
    "extract_proxy_hosts",
]
