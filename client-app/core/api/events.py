"""GET /api/events — Server-Sent Events 流。

每个 EventBus 事件推一条 SSE。格式::

    event: <type>
    id:    <ms-since-epoch>
    data:  <json payload>

每 15 秒发一次 ``: keep-alive\\n\\n``，避免 webview / 中间盒超时断流。

事件类型契约（M-β.1 范围）：
- ``ready``              首条事件，payload ``{"version": "..."}``
- ``server_discovered``  payload 见 discoverer.server_to_payload
- ``server_lost``        payload ``{server_id, name}``

后续 M-β.2 增 ``connect_progress`` / ``connect_done`` / ``heartbeat_changed``。
"""
from __future__ import annotations

import asyncio
import json
import logging
from typing import Any

from aiohttp import web

log = logging.getLogger("client.api.events")

routes = web.RouteTableDef()

KEEPALIVE_SEC = 15
VERSION = "0.1.1"


@routes.get("/api/events")
async def stream_events(request: web.Request) -> web.StreamResponse:
    runtime = request.app["runtime"]
    bus = getattr(runtime, "bus", None)
    if bus is None:
        return web.json_response(
            {"error": {"code": "NO_BUS", "message": "EventBus not available"}},
            status=503,
        )

    resp = web.StreamResponse(
        status=200,
        headers={
            "Content-Type": "text/event-stream",
            "Cache-Control": "no-cache, no-transform",
            "Connection": "keep-alive",
            "X-Accel-Buffering": "no",
            # SSE 必须 prepare 前自带 CORS 头：cors_middleware 在 handler return
            # 后才加 header，但 StreamResponse.prepare 会把 header 立刻 flush
            # 出去 —— 中间件已无机会插入。EventSource 没有 CORS 就直接 NetworkError。
            "Access-Control-Allow-Origin": "*",
            "Access-Control-Expose-Headers": "Content-Type",
        },
    )
    await resp.prepare(request)

    q = await bus.subscribe()
    try:
        await _send_event(resp, "ready", {"version": VERSION})
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
        await bus.unsubscribe(q)
    return resp


async def _send_event(
    resp: web.StreamResponse,
    type_: str,
    payload: dict[str, Any],
    ts: float | None = None,
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
