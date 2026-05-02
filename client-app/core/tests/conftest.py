"""Shared pytest fixtures for the client-app core test suite."""
from __future__ import annotations

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
def free_port():
    return _free_port()
