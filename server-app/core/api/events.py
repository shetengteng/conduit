"""GET /api/events — Server-Sent Events stream.

Pushes one event per ProxyCore.bus message. Format::

    event: <type>
    data:  <json payload>
    id:    <ms-since-epoch>

Sends ``: keep-alive\\n\\n`` every 15 s so middleboxes don't time out.
"""
from __future__ import annotations

import asyncio
import json
import logging
from typing import Any

from aiohttp import web

log = logging.getLogger("core.api.events")

routes = web.RouteTableDef()

KEEPALIVE_SEC = 15


@routes.get("/api/events")
async def stream_events(request: web.Request) -> web.StreamResponse:
    core = request.app["core"]
    resp = web.StreamResponse(
        status=200,
        headers={
            "Content-Type": "text/event-stream",
            "Cache-Control": "no-cache, no-transform",
            "Connection": "keep-alive",
            "X-Accel-Buffering": "no",
            # SSE 必须 prepare 前自带 CORS 头：cors_middleware 在 handler return
            # 后才加 header，但 StreamResponse.prepare 会立刻 flush header —— 中间件
            # 已无机会插入。EventSource 没有 CORS 就直接 NetworkError。
            "Access-Control-Allow-Origin": "*",
            "Access-Control-Expose-Headers": "Content-Type",
        },
    )
    await resp.prepare(request)

    q = await core.bus.subscribe()
    try:
        await _send_event(resp, "ready", {"version": getattr(core, "VERSION", "0.1.0")})
        last_ka = asyncio.get_running_loop().time()
        while True:
            timeout = max(1.0, KEEPALIVE_SEC - (asyncio.get_running_loop().time() - last_ka))
            try:
                evt = await asyncio.wait_for(q.get(), timeout=timeout)
            except asyncio.TimeoutError:
                await _send_keepalive(resp)
                last_ka = asyncio.get_running_loop().time()
                continue
            await _send_event(resp, evt.type, evt.payload, ts=evt.ts)
    except (asyncio.CancelledError, ConnectionResetError, ConnectionAbortedError):
        pass
    except Exception as exc:  # noqa: BLE001
        msg = str(exc) or exc.__class__.__name__
        if "closing transport" in msg.lower() or "cannot write" in msg.lower():
            pass
        else:
            log.warning("SSE stream broken: %s", msg)
    finally:
        await core.bus.unsubscribe(q)
    return resp


async def _send_event(
    resp: web.StreamResponse, type_: str, payload: dict[str, Any], ts: float | None = None
) -> None:
    body = json.dumps(payload, default=str, ensure_ascii=False)
    chunk = (
        f"event: {type_}\n"
        f"id: {int((ts or 0) * 1000)}\n"
        f"data: {body}\n\n"
    ).encode("utf-8")
    await resp.write(chunk)


async def _send_keepalive(resp: web.StreamResponse) -> None:
    await resp.write(b": keep-alive\n\n")
