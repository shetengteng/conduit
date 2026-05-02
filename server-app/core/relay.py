"""Bidirectional byte-stream relay primitives shared by HTTP & SOCKS5 handlers.

The relay reports incremental progress to an optional ``on_progress``
callback so the upstream connection registry can compute live traffic
rates. Existing return value ``(bytes_a_to_b, bytes_b_to_a)`` is preserved
for backward compatibility with any caller that only wants the totals.
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
    """Returns (bytes_a_to_b, bytes_b_to_a) when both halves finish.

    If ``on_progress`` is set, it will be awaited once per chunk in the
    form ``await on_progress(sent_delta, recv_delta)`` where ``sent`` is
    the upstream direction (client → target).
    """
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
