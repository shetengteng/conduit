"""Shared pytest fixtures for the server-app core test suite."""
from __future__ import annotations

import asyncio
import os
import socket
import sys
from pathlib import Path

import pytest

CORE_DIR = Path(__file__).resolve().parent.parent
if str(CORE_DIR) not in sys.path:
    sys.path.insert(0, str(CORE_DIR))


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@pytest.fixture
def free_ports():
    return [_free_port() for _ in range(4)]


@pytest.fixture
def core_cfg(free_ports):
    from config import Config

    cfg = Config()
    cfg.http_port, cfg.socks_port, cfg.api_port, _ = free_ports
    cfg.bind = "127.0.0.1"
    cfg.pac_advertised_host = "127.0.0.1"
    cfg.mdns_enabled = False
    cfg.physical_iface_ip = "127.0.0.1"
    cfg.allowed_cidrs = ["127.0.0.0/8"]
    cfg.allowed_connect_ports = {19999}
    cfg.direct_first = False
    return cfg


@pytest.fixture
async def core(core_cfg):
    from proxy_core import ProxyCore

    c = ProxyCore(core_cfg)
    await c.start()
    try:
        yield c
    finally:
        if c.running:
            await c.stop()


@pytest.fixture
async def echo_target():
    """Spin up a TCP echo on 127.0.0.1:19999 (the only allowed CONNECT port in tests)."""
    async def echo(reader, writer):
        try:
            while True:
                data = await reader.read(4096)
                if not data:
                    break
                writer.write(data)
                await writer.drain()
        finally:
            writer.close()
            try:
                await writer.wait_closed()
            except Exception:
                pass

    srv = await asyncio.start_server(echo, "127.0.0.1", 19999)
    try:
        yield srv
    finally:
        srv.close()
        await srv.wait_closed()
