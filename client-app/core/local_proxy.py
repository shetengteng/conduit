"""Local SOCKS5 proxy for the Conduit client-app.

What it does
------------

* Listens on ``127.0.0.1:7890`` (configurable) for SOCKS5 v5 / NO-AUTH
  / CMD=CONNECT (IPv4 + Domain) requests.  This subset is enough for
  every browser, ``curl --socks5``, IDE proxy settings, and the system
  SOCKS firewall hook macOS sets via ``networksetup``.

* For each ``CONNECT host:port``:

  1. Asks ``RouteResolver`` for ``direct`` vs ``proxy``.
  2. If ``direct`` → opens a TCP connection from this machine.
     If that fails (refused / timeout) we fall back to ``proxy`` and
     update the cache (self-heal).
  3. If ``proxy`` → opens a TCP connection to the configured Conduit
     Server endpoint and tunnels the target through it using HTTP
     ``CONNECT`` (the protocol that ``server-app/core/http_proxy.py``
     speaks on its 8080 port).

* Once the upstream tunnel is up, we relay bytes both ways via
  ``relay.bidirectional_relay``, the same primitive the server uses.

Cross-references:
* design/2026-04-30-3-Conduit-Client-客户端可行性报告.md §3.3
  (SOCKS5 subset rationale)
* design/2026-04-30-3-Conduit-Client-客户端可行性报告.md §3.5
  (self-heal flow inside the SOCKS5 main loop)
"""
from __future__ import annotations

import asyncio
import ipaddress
import logging
import socket
import struct
from dataclasses import dataclass

from relay import bidirectional_relay, safe_close
from route_resolver import RouteResolver

logger = logging.getLogger("conduit.client.local_proxy")

VER = 0x05
NO_AUTH = 0x00
NO_ACCEPTABLE = 0xFF

CMD_CONNECT = 0x01

ATYP_IPV4 = 0x01
ATYP_DOMAIN = 0x03
ATYP_IPV6 = 0x04

REP_OK = 0x00
REP_GENERAL = 0x01
REP_NETWORK_UNREACH = 0x03
REP_HOST_UNREACH = 0x04
REP_CONNECT_REFUSED = 0x05
REP_TTL_EXPIRED = 0x06
REP_CMD_NOT_SUPPORTED = 0x07
REP_ATYP_NOT_SUPPORTED = 0x08

DEFAULT_HANDSHAKE_TIMEOUT = 5.0
DEFAULT_DIRECT_CONNECT_TIMEOUT = 5.0
DEFAULT_PROXY_CONNECT_TIMEOUT = 8.0


# ---------------------------------------------------------------------------
# Server endpoint config
# ---------------------------------------------------------------------------


@dataclass
class ServerEndpoint:
    """Where the Conduit Server's HTTP-CONNECT proxy lives."""
    host: str
    port: int = 8080

    def label(self) -> str:
        return f"{self.host}:{self.port}"


# ---------------------------------------------------------------------------
# SOCKS5 reply helpers
# ---------------------------------------------------------------------------


def _reply(rep: int, atyp: int = ATYP_IPV4,
           bnd_addr: bytes = b"\x00\x00\x00\x00", bnd_port: int = 0) -> bytes:
    return bytes([VER, rep, 0x00, atyp]) + bnd_addr + struct.pack(">H", bnd_port)


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


async def _read_exact(reader: asyncio.StreamReader, n: int, timeout: float) -> bytes:
    return await asyncio.wait_for(reader.readexactly(n), timeout=timeout)


# ---------------------------------------------------------------------------
# CONNECT path: direct vs proxy
# ---------------------------------------------------------------------------


async def _connect_direct(
    host: str,
    port: int,
    *,
    timeout: float,
) -> tuple[asyncio.StreamReader, asyncio.StreamWriter]:
    return await asyncio.wait_for(
        asyncio.open_connection(host, port),
        timeout=timeout,
    )


async def _connect_via_server(
    host: str,
    port: int,
    server: ServerEndpoint,
    *,
    timeout: float,
) -> tuple[asyncio.StreamReader, asyncio.StreamWriter]:
    """Open a tunnel through Conduit Server's HTTP CONNECT endpoint."""
    reader, writer = await asyncio.wait_for(
        asyncio.open_connection(server.host, server.port),
        timeout=timeout,
    )
    request = (
        f"CONNECT {host}:{port} HTTP/1.1\r\n"
        f"Host: {host}:{port}\r\n"
        f"User-Agent: conduit-client/0.1\r\n"
        f"Proxy-Connection: keep-alive\r\n"
        f"\r\n"
    ).encode("ascii", errors="replace")
    writer.write(request)
    await writer.drain()

    status_line = await asyncio.wait_for(reader.readline(), timeout=timeout)
    parts = status_line.decode("ascii", errors="replace").split(" ", 2)
    if len(parts) < 2 or not parts[1].startswith("2"):
        await safe_close(writer)
        raise OSError(f"server CONNECT rejected: {status_line!r}")

    while True:
        line = await asyncio.wait_for(reader.readline(), timeout=timeout)
        if line in (b"\r\n", b"\n", b""):
            break

    return reader, writer


# ---------------------------------------------------------------------------
# Server
# ---------------------------------------------------------------------------


class LocalProxyServer:
    """Owns the asyncio listener + per-connection handler dispatch."""

    def __init__(
        self,
        resolver: RouteResolver,
        *,
        bind_host: str = "127.0.0.1",
        bind_port: int = 7890,
        server_endpoint: ServerEndpoint | None = None,
        handshake_timeout: float = DEFAULT_HANDSHAKE_TIMEOUT,
        direct_connect_timeout: float = DEFAULT_DIRECT_CONNECT_TIMEOUT,
        proxy_connect_timeout: float = DEFAULT_PROXY_CONNECT_TIMEOUT,
        progress_callback=None,
    ) -> None:
        self.resolver = resolver
        self.bind_host = bind_host
        self.bind_port = bind_port
        self._server_endpoint = server_endpoint
        self.handshake_timeout = handshake_timeout
        self.direct_connect_timeout = direct_connect_timeout
        self.proxy_connect_timeout = proxy_connect_timeout
        # M-γ:连接成功后由 ClientRuntime 注入 traffic_meter.on_chunk
        self._progress_callback = progress_callback
        self._server: asyncio.base_events.Server | None = None
        self._stats = {
            "connections": 0,
            "direct": 0,
            "proxy": 0,
            "self_healed": 0,
            "errors": 0,
        }

    def set_progress_callback(self, callback) -> None:
        """M-γ:连接 / 断开时由 ClientRuntime 切换。None = 不计量。"""
        self._progress_callback = callback

    # ------------------------------------------------------------------
    # lifecycle
    # ------------------------------------------------------------------

    @property
    def is_running(self) -> bool:
        return self._server is not None and self._server.is_serving()

    @property
    def actual_port(self) -> int:
        if self._server is None or not self._server.sockets:
            return self.bind_port
        return self._server.sockets[0].getsockname()[1]

    @property
    def server_endpoint(self) -> ServerEndpoint | None:
        return self._server_endpoint

    def set_server_endpoint(self, endpoint: ServerEndpoint | None) -> None:
        self._server_endpoint = endpoint

    @property
    def stats(self) -> dict[str, int]:
        return dict(self._stats)

    async def start(self) -> None:
        if self._server is not None:
            return
        self._server = await asyncio.start_server(
            self._handle_client, self.bind_host, self.bind_port,
        )
        port = self.actual_port
        logger.info(
            "local_proxy listening on %s:%d (server endpoint=%s)",
            self.bind_host, port,
            self._server_endpoint.label() if self._server_endpoint else "<none>",
        )

    async def stop(self) -> None:
        if self._server is None:
            return
        self._server.close()
        try:
            await self._server.wait_closed()
        except Exception:
            pass
        self._server = None
        logger.info("local_proxy stopped")

    # ------------------------------------------------------------------
    # connection handler
    # ------------------------------------------------------------------

    async def _handle_client(
        self,
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
    ) -> None:
        peer = writer.get_extra_info("peername") or ("?", 0)
        peer_ip = peer[0] if peer else "?"
        target_writer: asyncio.StreamWriter | None = None
        self._stats["connections"] += 1

        try:
            if not await self._socks5_handshake(reader, writer):
                return
            host, port = await self._read_connect_request(reader, writer)
            if host is None:
                return

            decision = await self.resolver.resolve(host, port)
            logger.debug(
                "CONNECT %s:%d via %s (cache %s)",
                host, port, decision.direction, decision.source,
            )

            t_reader, t_writer, used_direction, used_source = await self._open_target(
                host, port, decision.direction, decision.source,
            )
            if t_reader is None or t_writer is None:
                writer.write(_reply(REP_NETWORK_UNREACH))
                await writer.drain()
                self._stats["errors"] += 1
                return
            target_writer = t_writer

            if used_direction == "direct":
                self._stats["direct"] += 1
            else:
                self._stats["proxy"] += 1
            if used_source == "self_heal":
                self._stats["self_healed"] += 1

            bnd_atyp, bnd_addr, bnd_port = _bnd_from_target(t_writer)
            writer.write(_reply(REP_OK, bnd_atyp, bnd_addr, bnd_port))
            await writer.drain()
            logger.info(
                "CONNECT %s:%d from %s -> %s/%s established",
                host, port, peer_ip, used_direction, used_source,
            )

            await bidirectional_relay(reader, writer, t_reader, t_writer, on_progress=self._progress_callback)
        except (asyncio.IncompleteReadError, asyncio.TimeoutError):
            return
        except Exception as exc:  # pragma: no cover - defensive
            logger.warning("local_proxy %s error: %s", peer_ip, exc)
            self._stats["errors"] += 1
        finally:
            await safe_close(writer)
            await safe_close(target_writer)

    # ------------------------------------------------------------------
    # SOCKS5 protocol fragments
    # ------------------------------------------------------------------

    async def _socks5_handshake(
        self,
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
    ) -> bool:
        head = await _read_exact(reader, 2, self.handshake_timeout)
        ver, nmethods = head[0], head[1]
        if ver != VER or nmethods == 0:
            return False
        methods = await _read_exact(reader, nmethods, self.handshake_timeout)
        if NO_AUTH not in methods:
            writer.write(bytes([VER, NO_ACCEPTABLE]))
            await writer.drain()
            return False
        writer.write(bytes([VER, NO_AUTH]))
        await writer.drain()
        return True

    async def _read_connect_request(
        self,
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
    ) -> tuple[str | None, int]:
        head4 = await _read_exact(reader, 4, self.handshake_timeout)
        ver, cmd, _rsv, atyp = head4[0], head4[1], head4[2], head4[3]
        if ver != VER:
            return None, 0
        if cmd != CMD_CONNECT:
            writer.write(_reply(REP_CMD_NOT_SUPPORTED))
            await writer.drain()
            return None, 0

        if atyp == ATYP_IPV4:
            raw = await _read_exact(reader, 4, self.handshake_timeout)
            host = socket.inet_ntop(socket.AF_INET, raw)
        elif atyp == ATYP_IPV6:
            raw = await _read_exact(reader, 16, self.handshake_timeout)
            host = socket.inet_ntop(socket.AF_INET6, raw)
        elif atyp == ATYP_DOMAIN:
            ln_b = await _read_exact(reader, 1, self.handshake_timeout)
            ln = ln_b[0]
            if ln == 0:
                writer.write(_reply(REP_GENERAL))
                await writer.drain()
                return None, 0
            host = (await _read_exact(reader, ln, self.handshake_timeout)).decode(
                "ascii", errors="replace",
            )
        else:
            writer.write(_reply(REP_ATYP_NOT_SUPPORTED))
            await writer.drain()
            return None, 0

        port = struct.unpack(
            ">H", await _read_exact(reader, 2, self.handshake_timeout),
        )[0]
        return host, port

    # ------------------------------------------------------------------
    # outbound dispatch with self-heal
    # ------------------------------------------------------------------

    async def _open_target(
        self,
        host: str,
        port: int,
        direction: str,
        source: str,
    ) -> tuple[
        asyncio.StreamReader | None,
        asyncio.StreamWriter | None,
        str,
        str,
    ]:
        if direction == "direct":
            try:
                r, w = await _connect_direct(
                    host, port, timeout=self.direct_connect_timeout,
                )
                return r, w, "direct", source
            except (OSError, asyncio.TimeoutError) as exc:
                logger.info(
                    "direct->%s:%d failed (%s); self-healing to proxy",
                    host, port, exc,
                )
                self.resolver.mark_direct_failed(host, port)
                direction = "proxy"
                source = "self_heal"

        if direction == "proxy":
            ep = self._server_endpoint
            if ep is None:
                logger.warning(
                    "no server endpoint configured; cannot forward %s:%d",
                    host, port,
                )
                return None, None, "proxy", source
            try:
                r, w = await _connect_via_server(
                    host, port, ep, timeout=self.proxy_connect_timeout,
                )
                return r, w, "proxy", source
            except (OSError, asyncio.TimeoutError) as exc:
                logger.warning(
                    "proxy %s -> %s:%d failed: %s",
                    ep.label(), host, port, exc,
                )
                self.resolver.mark_proxy_failed(host, port)
                return None, None, "proxy", source

        return None, None, direction, source


__all__ = [
    "DEFAULT_HANDSHAKE_TIMEOUT",
    "DEFAULT_DIRECT_CONNECT_TIMEOUT",
    "DEFAULT_PROXY_CONNECT_TIMEOUT",
    "LocalProxyServer",
    "ServerEndpoint",
]
