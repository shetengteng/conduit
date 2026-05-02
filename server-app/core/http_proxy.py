"""HTTP/HTTPS proxy handler with PAC file fast-path on the same listening port."""
from __future__ import annotations

import asyncio
import json
import logging
import os
import re
from urllib.parse import parse_qs, urlsplit

from active_connections import ConnectionRegistry
from config import Config
from outbound import POLICY_AUTO, open_with_fallback, policy_from_pac_section
from pac_engine import PacRules, find_proxy
from relay import bidirectional_relay, safe_close

log = logging.getLogger("http")

MAX_REQUEST_LINE = 8192
MAX_HEADERS = 64 * 1024
HOP_BY_HOP = {
    "connection",
    "proxy-connection",
    "keep-alive",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "proxy-authenticate",
    "proxy-authorization",
}

REDACT_KEYS = re.compile(
    r"(?i)(token|access[_-]?key|api[_-]?key|password|secret|authorization)=([^&\s]+)"
)


def _redact(s: str) -> str:
    return REDACT_KEYS.sub(lambda m: f"{m.group(1)}=***", s)


async def _read_until_double_crlf(reader: asyncio.StreamReader, limit: int) -> bytes:
    buf = bytearray()
    while b"\r\n\r\n" not in buf:
        if len(buf) > limit:
            raise ValueError("headers too large")
        chunk = await reader.read(1024)
        if not chunk:
            break
        buf.extend(chunk)
    return bytes(buf)


def _parse_request_line(line: bytes) -> tuple[str, str, str]:
    s = line.rstrip(b"\r\n").decode("iso-8859-1", errors="replace")
    parts = s.split(" ", 2)
    if len(parts) != 3:
        raise ValueError(f"bad request line: {s!r}")
    return parts[0].upper(), parts[1], parts[2]


def _parse_authority(authority: str) -> tuple[str, int]:
    if authority.startswith("["):
        host, _, port_s = authority[1:].partition("]")
        port_s = port_s.lstrip(":")
    else:
        host, _, port_s = authority.partition(":")
    if not host:
        raise ValueError(f"bad authority: {authority!r}")
    return host, int(port_s) if port_s else 443


async def _send_simple(writer: asyncio.StreamWriter, status: str, body: bytes = b"",
                       extra_headers: dict[str, str] | None = None) -> None:
    headers = [f"HTTP/1.1 {status}", f"Content-Length: {len(body)}",
               "Connection: close", "Cache-Control: no-store"]
    if extra_headers:
        for k, v in extra_headers.items():
            headers.append(f"{k}: {v}")
    raw = ("\r\n".join(headers) + "\r\n\r\n").encode("ascii") + body
    try:
        writer.write(raw)
        await writer.drain()
    except (ConnectionResetError, BrokenPipeError, OSError):
        pass


async def _drain_request(reader: asyncio.StreamReader, timeout: float) -> None:
    try:
        await asyncio.wait_for(
            _read_until_double_crlf(reader, MAX_HEADERS), timeout=timeout
        )
    except (asyncio.TimeoutError, ValueError, asyncio.IncompleteReadError):
        pass


async def _serve_check(reader: asyncio.StreamReader, writer: asyncio.StreamWriter,
                       target: str, rules: PacRules | None, peer_ip: str) -> None:
    await _drain_request(reader, 2.0)
    qs = target.partition("?")[2]
    params = parse_qs(qs)
    host_q = (params.get("host") or [""])[0].strip().lower()
    if not host_q:
        await _send_simple(writer, "400 Bad Request",
                           b'{"error": "missing host parameter, use /check?host=foo.com"}\n',
                           extra_headers={"Content-Type": "application/json"})
        return
    if rules is None:
        await _send_simple(writer, "503 Service Unavailable",
                           b'{"error": "PAC rules not loaded on server"}\n',
                           extra_headers={"Content-Type": "application/json"})
        return
    decision = find_proxy(host_q, rules)
    payload = {
        "host": host_q,
        "proxy": decision.proxy,
        "matched_section": decision.matched_section,
        "matched_pattern": decision.matched_pattern,
        "explanation": _explain_decision(decision.proxy),
    }
    body = (json.dumps(payload, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
    log.info("check from %s host=%s -> %s", peer_ip, host_q, decision.proxy)
    await _send_simple(writer, "200 OK", body,
                       extra_headers={"Content-Type": "application/json; charset=utf-8"})


def _explain_decision(proxy: str) -> str:
    if proxy == "DIRECT":
        return "Client B will connect to the target directly via its own ISP/network."
    if "; DIRECT" in proxy:
        return ("Client B tries the proxy first (B -> A -> VPN -> target); "
                "if proxy is unreachable, browser/OS auto-fallbacks to DIRECT.")
    return "Client B routes via proxy (B -> A -> VPN -> target). No fallback."


async def _serve_status(reader: asyncio.StreamReader, writer: asyncio.StreamWriter,
                        rules: PacRules | None, cfg: Config, peer_ip: str) -> None:
    await _drain_request(reader, 2.0)
    payload = {
        "ok": True,
        "http_port": cfg.http_port,
        "socks_port": cfg.socks_port,
        "allowed_cidrs": cfg.allowed_cidrs,
        "allowed_connect_ports": sorted(cfg.allowed_connect_ports),
        "pac_endpoints": list(cfg.pac_endpoints),
        "pac_rules": {
            "loaded_from": rules.source_path if rules else None,
            "internal_domains": rules.internal_domains if rules else [],
            "fallback_domains": rules.fallback_domains if rules else [],
            "cn_direct_domains": rules.cn_direct_domains if rules else [],
            "internal_globs": rules.internal_globs if rules else [],
            "fallback_globs": rules.fallback_globs if rules else [],
            "cn_direct_globs": rules.cn_direct_globs if rules else [],
        } if rules else None,
    }
    body = (json.dumps(payload, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
    log.info("status from %s", peer_ip)
    await _send_simple(writer, "200 OK", body,
                       extra_headers={"Content-Type": "application/json; charset=utf-8"})


async def _serve_pac(writer: asyncio.StreamWriter, cfg: Config) -> None:
    path = cfg.pac_file_path
    if not os.path.isabs(path):
        path = os.path.join(os.path.dirname(os.path.abspath(__file__)), path)
    try:
        with open(path, "rb") as f:
            body = f.read()
    except OSError as exc:
        log.warning("PAC file unavailable: %s", exc)
        await _send_simple(writer, "404 Not Found", b"PAC not found\n")
        return
    proxy_host = (cfg.pac_advertised_host or cfg.bind or "127.0.0.1").encode("ascii")
    proxy_port = str(cfg.http_port).encode("ascii")
    body = body.replace(b"__PROXY_HOST__", proxy_host).replace(b"__PROXY_PORT__", proxy_port)
    headers = {"Content-Type": "application/x-ns-proxy-autoconfig",
               "Cache-Control": "no-cache"}
    await _send_simple(writer, "200 OK", body, extra_headers=headers)


async def handle_http(reader: asyncio.StreamReader, writer: asyncio.StreamWriter,
                      cfg: Config, rules: PacRules | None = None,
                      registry: ConnectionRegistry | None = None) -> None:
    peer = writer.get_extra_info("peername") or ("?", 0)
    peer_ip = peer[0] if peer else "?"

    if not cfg.is_client_allowed(peer_ip):
        log.warning("HTTP reject %s (not in allowed_cidrs)", peer_ip)
        await _send_simple(writer, "403 Forbidden", b"client IP not allowed\n")
        await safe_close(writer)
        return

    target_writer: asyncio.StreamWriter | None = None
    try:
        try:
            line = await asyncio.wait_for(
                reader.readuntil(b"\n"), timeout=cfg.handshake_timeout_s
            )
        except (asyncio.IncompleteReadError, asyncio.TimeoutError, asyncio.LimitOverrunError):
            return
        if len(line) > MAX_REQUEST_LINE:
            await _send_simple(writer, "414 URI Too Long")
            return

        method, target, version = _parse_request_line(line)

        path_only = target.split("?", 1)[0]
        if method == "GET" and path_only in cfg.pac_endpoints:
            await _serve_pac(writer, cfg)
            log.info("PAC served to %s", peer_ip)
            return
        if method == "GET" and path_only == "/check":
            await _serve_check(reader, writer, target, rules, peer_ip)
            return
        if method == "GET" and path_only == "/status":
            await _serve_status(reader, writer, rules, cfg, peer_ip)
            return

        if method == "CONNECT":
            await _handle_connect(reader, writer, cfg, rules, target, peer_ip, registry)
            return

        await _handle_absolute(reader, writer, cfg, rules, method, target, version,
                               line, peer_ip, registry)
    except Exception as exc:
        log.warning("HTTP %s error: %s", peer_ip, exc)
        try:
            await _send_simple(writer, "502 Bad Gateway")
        except Exception:
            pass
    finally:
        await safe_close(writer)
        await safe_close(target_writer)


async def _handle_connect(reader: asyncio.StreamReader, writer: asyncio.StreamWriter,
                          cfg: Config, rules: PacRules | None,
                          target: str, peer_ip: str,
                          registry: ConnectionRegistry | None = None) -> None:
    try:
        host, port = _parse_authority(target)
    except ValueError:
        await _send_simple(writer, "400 Bad Request")
        return
    if not cfg.is_connect_port_allowed(port):
        log.warning("CONNECT %s:%d from %s rejected (port not allowed)",
                    host, port, peer_ip)
        await _send_simple(writer, "403 Forbidden", b"port not allowed\n")
        return

    while True:
        try:
            extra = await asyncio.wait_for(reader.readline(), timeout=cfg.handshake_timeout_s)
        except (asyncio.TimeoutError, asyncio.IncompleteReadError):
            await _send_simple(writer, "408 Request Timeout")
            return
        if extra in (b"\r\n", b"\n", b""):
            break

    policy = POLICY_AUTO
    if rules is not None:
        policy = policy_from_pac_section(find_proxy(host, rules).matched_section)

    log.info("CONNECT %s:%d from %s policy=%s", host, port, peer_ip, policy)
    try:
        t_reader, t_writer, route = await open_with_fallback(host, port, cfg, policy)
    except asyncio.TimeoutError:
        log.warning("CONNECT %s:%d from %s TIMEOUT (both routes)",
                    host, port, peer_ip)
        await _send_simple(writer, "504 Gateway Timeout",
                           f"connect to {host}:{port} timed out\n".encode("utf-8"))
        return
    except OSError as exc:
        log.warning("CONNECT %s:%d from %s FAILED: %s (%s)",
                    host, port, peer_ip, exc.__class__.__name__, exc)
        await _send_simple(writer, "502 Bad Gateway",
                           f"connect to {host}:{port} failed: {exc}\n".encode("utf-8"))
        return

    try:
        writer.write(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        await writer.drain()
    except (ConnectionResetError, BrokenPipeError, OSError):
        await safe_close(t_writer)
        return

    sid: str | None = None
    on_progress = None
    if registry is not None:
        sid = await registry.add(peer_ip, "http", f"{host}:{port}")
        on_progress = lambda s, r: registry.update_bytes(sid, s, r)  # noqa: E731

    loop = asyncio.get_running_loop()
    t0 = loop.time()
    try:
        sent, recv = await bidirectional_relay(
            reader, writer, t_reader, t_writer, on_progress=on_progress,
        )
    finally:
        await safe_close(t_writer)
        if sid is not None and registry is not None:
            await registry.remove(sid)
    elapsed = loop.time() - t0
    log.info("CONNECT %s:%d from %s via %s closed: sent=%dB recv=%dB elapsed=%.2fs",
             host, port, peer_ip, route, sent, recv, elapsed)


async def _handle_absolute(reader: asyncio.StreamReader, writer: asyncio.StreamWriter,
                           cfg: Config, rules: PacRules | None,
                           method: str, target: str, version: str,
                           request_line: bytes, peer_ip: str,
                           registry: ConnectionRegistry | None = None) -> None:
    parts = urlsplit(target)
    if parts.scheme.lower() != "http" or not parts.hostname:
        await _send_simple(writer, "400 Bad Request",
                           b"only http:// absolute URIs are supported\n")
        return
    host = parts.hostname
    port = parts.port or 80
    if not cfg.is_connect_port_allowed(port):
        log.warning("HTTP %s %s:%d rejected (port not allowed)", method, host, port)
        await _send_simple(writer, "403 Forbidden", b"port not allowed\n")
        return

    safe_target = _redact(target) if cfg.redact_query else target
    log.info("%s %s from %s", method, safe_target, peer_ip)

    rest = await asyncio.wait_for(
        _read_until_double_crlf(reader, MAX_HEADERS),
        timeout=cfg.handshake_timeout_s,
    )
    if b"\r\n\r\n" in rest:
        header_block, _, body_so_far = rest.partition(b"\r\n\r\n")
    else:
        header_block, body_so_far = rest, b""

    new_headers, has_host, has_content_length, has_te = [], False, False, False
    for raw in header_block.split(b"\r\n"):
        if not raw:
            continue
        name, sep, _ = raw.partition(b":")
        if not sep:
            continue
        lname = name.strip().lower().decode("ascii", errors="replace")
        if lname in HOP_BY_HOP:
            continue
        if lname == "host":
            has_host = True
        if lname == "content-length":
            has_content_length = True
        if lname == "transfer-encoding":
            has_te = True
        new_headers.append(raw)

    relative = parts.path or "/"
    if parts.query:
        relative += "?" + parts.query

    rebuilt: list[bytes] = []
    rebuilt.append(f"{method} {relative} {version}".encode("iso-8859-1"))
    if not has_host:
        host_header = host if port in (80,) else f"{host}:{port}"
        rebuilt.append(f"Host: {host_header}".encode("iso-8859-1"))
    rebuilt.extend(new_headers)
    rebuilt.append(b"Connection: close")
    rebuilt.append(b"")
    rebuilt.append(b"")
    out_head = b"\r\n".join(rebuilt)

    policy = POLICY_AUTO
    if rules is not None:
        policy = policy_from_pac_section(find_proxy(host, rules).matched_section)
    try:
        t_reader, t_writer, route = await open_with_fallback(host, port, cfg, policy)
    except (OSError, asyncio.TimeoutError) as exc:
        log.warning("connect %s:%d failed: %s", host, port, exc)
        await _send_simple(writer, "502 Bad Gateway")
        return
    log.info("%s %s from %s policy=%s via %s",
             method, safe_target, peer_ip, policy, route)

    try:
        t_writer.write(out_head)
        if body_so_far:
            t_writer.write(body_so_far)
        await t_writer.drain()
    except (ConnectionResetError, BrokenPipeError, OSError):
        await safe_close(t_writer)
        return

    sid: str | None = None
    on_progress = None
    if registry is not None:
        sid = await registry.add(peer_ip, "http", f"{host}:{port}")
        on_progress = lambda s, r: registry.update_bytes(sid, s, r)  # noqa: E731

    has_body_to_forward = has_content_length or has_te
    try:
        if has_body_to_forward:
            await bidirectional_relay(
                reader, writer, t_reader, t_writer, on_progress=on_progress,
            )
        else:
            await _half_response_only(t_reader, writer)
    finally:
        await safe_close(t_writer)
        if sid is not None and registry is not None:
            await registry.remove(sid)


async def _half_response_only(t_reader: asyncio.StreamReader,
                              client_writer: asyncio.StreamWriter) -> None:
    try:
        while True:
            chunk = await t_reader.read(65536)
            if not chunk:
                break
            client_writer.write(chunk)
            await client_writer.drain()
    except (ConnectionResetError, BrokenPipeError, OSError, asyncio.CancelledError):
        pass
