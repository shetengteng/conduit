"""Bidirectional byte-stream relay shared by the SOCKS5 frontend.

This is the same primitive used by ``server-app/core/relay.py`` — copied
here so the client-app sidecar can be packaged independently (no
cross-app Python imports).  Keep the two in sync if you change the
chunk size or back-pressure semantics.
"""
from __future__ import annotations

import asyncio
from typing import Awaitable, Callable, Optional

CHUNK = 65536

OnProgress = Callable[[int, int], Awaitable[None]]


async def _half_pipe(
    reader: asyncio.StreamReader,
    writer: asyncio.StreamWriter,
    counter: list[int],
    on_progress: Optional[OnProgress],
    is_upstream: bool,
) -> None:
    try:
        while True:
            data = await reader.read(CHUNK)
            if not data:
                break
            n = len(data)
            counter[0] += n
            writer.write(data)
            await writer.drain()
            if on_progress is not None:
                if is_upstream:
                    await on_progress(n, 0)
                else:
                    await on_progress(0, n)
    except (ConnectionResetError, BrokenPipeError, asyncio.CancelledError, OSError):
        pass
    finally:
        try:
            if writer.can_write_eof():
                writer.write_eof()
        except (OSError, RuntimeError):
            pass


async def bidirectional_relay(
    a_reader: asyncio.StreamReader,
    a_writer: asyncio.StreamWriter,
    b_reader: asyncio.StreamReader,
    b_writer: asyncio.StreamWriter,
    on_progress: Optional[OnProgress] = None,
) -> tuple[int, int]:
    a_to_b = [0]
    b_to_a = [0]
    await asyncio.gather(
        _half_pipe(a_reader, b_writer, a_to_b, on_progress, True),
        _half_pipe(b_reader, a_writer, b_to_a, on_progress, False),
        return_exceptions=True,
    )
    return a_to_b[0], b_to_a[0]


async def safe_close(writer: asyncio.StreamWriter | None) -> None:
    if writer is None:
        return
    try:
        if not writer.is_closing():
            writer.close()
        await writer.wait_closed()
    except (OSError, RuntimeError, ConnectionResetError):
        pass


__all__ = ["bidirectional_relay", "safe_close", "OnProgress", "CHUNK"]
