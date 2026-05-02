"""SOCKS5 proxy handler. RFC 1928, CMD=CONNECT only, NO-AUTH."""
from __future__ import annotations

import asyncio
import ipaddress
import logging
import socket
import struct

from active_connections import ConnectionRegistry
from config import Config
from outbound import POLICY_AUTO, open_with_fallback, policy_from_pac_section
from pac_engine import PacRules, find_proxy
from relay import bidirectional_relay, safe_close

log = logging.getLogger("socks5")

VER = 0x05
NO_AUTH = 0x00
NO_ACCEPTABLE = 0xFF

CMD_CONNECT = 0x01

ATYP_IPV4 = 0x01
ATYP_DOMAIN = 0x03
ATYP_IPV6 = 0x04

REP_OK = 0x00
REP_GENERAL = 0x01
REP_NOT_ALLOWED = 0x02
REP_NETWORK_UNREACH = 0x03
REP_HOST_UNREACH = 0x04
REP_CONNECT_REFUSED = 0x05
REP_TTL_EXPIRED = 0x06
REP_CMD_NOT_SUPPORTED = 0x07
REP_ATYP_NOT_SUPPORTED = 0x08


def _reply(rep: int, atyp: int = ATYP_IPV4,
           bnd_addr: bytes = b"\x00\x00\x00\x00", bnd_port: int = 0) -> bytes:
    return bytes([VER, rep, 0x00, atyp]) + bnd_addr + struct.pack(">H", bnd_port)


async def _read_exact(reader: asyncio.StreamReader, n: int, timeout: float) -> bytes:
    return await asyncio.wait_for(reader.readexactly(n), timeout=timeout)


async def handle_socks5(reader: asyncio.StreamReader, writer: asyncio.StreamWriter,
                        cfg: Config, rules: PacRules | None = None,
                        registry: ConnectionRegistry | None = None) -> None:
    peer = writer.get_extra_info("peername") or ("?", 0)
    peer_ip = peer[0] if peer else "?"

    if not cfg.is_client_allowed(peer_ip):
        log.warning("SOCKS reject %s (not in allowed_cidrs)", peer_ip)
        await safe_close(writer)
        return

    target_writer: asyncio.StreamWriter | None = None
    try:
        head = await _read_exact(reader, 2, cfg.handshake_timeout_s)
        ver, nmethods = head[0], head[1]
        if ver != VER or nmethods == 0:
            return
        methods = await _read_exact(reader, nmethods, cfg.handshake_timeout_s)
        if NO_AUTH not in methods:
            writer.write(bytes([VER, NO_ACCEPTABLE]))
            await writer.drain()
            return
        writer.write(bytes([VER, NO_AUTH]))
        await writer.drain()

        head4 = await _read_exact(reader, 4, cfg.handshake_timeout_s)
        ver, cmd, _, atyp = head4[0], head4[1], head4[2], head4[3]
        if ver != VER:
            return
        if cmd != CMD_CONNECT:
            writer.write(_reply(REP_CMD_NOT_SUPPORTED))
            await writer.drain()
            return

        if atyp == ATYP_IPV4:
            raw = await _read_exact(reader, 4, cfg.handshake_timeout_s)
            host = socket.inet_ntop(socket.AF_INET, raw)
        elif atyp == ATYP_IPV6:
            raw = await _read_exact(reader, 16, cfg.handshake_timeout_s)
            host = socket.inet_ntop(socket.AF_INET6, raw)
        elif atyp == ATYP_DOMAIN:
            ln_b = await _read_exact(reader, 1, cfg.handshake_timeout_s)
            ln = ln_b[0]
            if ln == 0:
                writer.write(_reply(REP_GENERAL))
                await writer.drain()
                return
            host = (await _read_exact(reader, ln, cfg.handshake_timeout_s)).decode(
                "ascii", errors="replace"
            )
        else:
            writer.write(_reply(REP_ATYP_NOT_SUPPORTED))
            await writer.drain()
            return

        port = struct.unpack(">H", await _read_exact(reader, 2, cfg.handshake_timeout_s))[0]

        if not cfg.is_connect_port_allowed(port):
            log.warning("SOCKS CONNECT %s:%d from %s rejected (port)", host, port, peer_ip)
            writer.write(_reply(REP_NOT_ALLOWED))
            await writer.drain()
            return

        policy = POLICY_AUTO
        if rules is not None:
            policy = policy_from_pac_section(find_proxy(host, rules).matched_section)

        log.info("SOCKS CONNECT %s:%d from %s policy=%s",
                 host, port, peer_ip, policy)
        try:
            t_reader, t_writer, route = await open_with_fallback(host, port, cfg, policy)
        except asyncio.TimeoutError:
            writer.write(_reply(REP_TTL_EXPIRED))
            await writer.drain()
            return
        except ConnectionRefusedError:
            writer.write(_reply(REP_CONNECT_REFUSED))
            await writer.drain()
            return
        except OSError as exc:
            log.warning("SOCKS connect %s:%d failed: %s", host, port, exc)
            writer.write(_reply(REP_NETWORK_UNREACH))
            await writer.drain()
            return

        target_writer = t_writer
        bnd_atyp, bnd_addr, bnd_port = _bnd_from_target(t_writer)
        writer.write(_reply(REP_OK, bnd_atyp, bnd_addr, bnd_port))
        await writer.drain()
        log.info("SOCKS CONNECT %s:%d from %s via %s established",
                 host, port, peer_ip, route)

        sid: str | None = None
        on_progress = None
        if registry is not None:
            sid = await registry.add(peer_ip, "socks5", f"{host}:{port}")
            on_progress = lambda s, r: registry.update_bytes(sid, s, r)  # noqa: E731

        try:
            await bidirectional_relay(
                reader, writer, t_reader, t_writer, on_progress=on_progress,
            )
        finally:
            if sid is not None and registry is not None:
                await registry.remove(sid)
    except (asyncio.IncompleteReadError, asyncio.TimeoutError):
        return
    except Exception as exc:
        log.warning("SOCKS %s error: %s", peer_ip, exc)
    finally:
        await safe_close(writer)
        await safe_close(target_writer)


def _bnd_from_target(t_writer: asyncio.StreamWriter) -> tuple[int, bytes, int]:
    sock = t_writer.get_extra_info("sockname")
    if not sock:
        return ATYP_IPV4, b"\x00\x00\x00\x00", 0
    addr, port = sock[0], sock[1]
    try:
        ip = ipaddress.ip_address(addr)
    except ValueError:
        return ATYP_IPV4, b"\x00\x00\x00\x00", port
    if ip.version == 4:
        return ATYP_IPV4, ip.packed, port
    return ATYP_IPV6, ip.packed, port
