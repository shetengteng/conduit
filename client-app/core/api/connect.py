"""POST /api/connect/{server_id} + POST /api/disconnect + GET /api/connection。

设计要点:
- ``connect`` 与 ``disconnect`` 由 ClientRuntime 串行化(同一时刻只允许一
  个进行中)。runtime 内部 _connect_lock 保证。API 不重复加锁。
- ``connect`` 同步等待全 5 步完成才返回(最多约 ``probe_timeout +
  pac_timeout = 6s``,可接受);UI 端如果想要"立刻进入 connecting 视图"
  应当订阅 SSE 看 connect_progress,而非死等 HTTP 响应。
- ``connect`` 接受可选 body ``{"override": true}``(M-γ 用)在已连接时
  先 disconnect 再 connect;M-β.2 不实现,直接返 ALREADY_CONNECTED。
"""
from __future__ import annotations

import logging

from aiohttp import web

log = logging.getLogger("client.api.connect")

routes = web.RouteTableDef()


@routes.post("/api/connect/{server_id}")
async def connect_to_server(request: web.Request) -> web.Response:
    runtime = request.app["runtime"]
    discoverer = getattr(runtime, "discoverer", None)
    if discoverer is None:
        return web.json_response(
            {"error": {"code": "NO_DISCOVERER", "message": "discoverer 未启动"}},
            status=503,
        )

    server_id = request.match_info["server_id"]
    server = next((s for s in discoverer.snapshot() if s.server_id == server_id), None)
    if server is None:
        return web.json_response(
            {"error": {"code": "NOT_FOUND",
                       "message": f"未找到 server_id={server_id};请先在'发现'页等待广播或重新扫描"}},
            status=404,
        )

    result = await runtime.connect_to(server)
    if result.get("ok") is False:
        # 区分 "用户错误"(409) vs "客户端尝试失败"(502 上游问题)
        code = result.get("error", "")
        status = 409 if code in ("BUSY", "ALREADY_CONNECTED") else 502
        return web.json_response(
            {"error": {"code": code, "message": result.get("message", "")},
             "step": result.get("step"), "step_key": result.get("step_key")},
            status=status,
        )
    return web.json_response(result)


@routes.post("/api/disconnect")
async def disconnect_from_server(request: web.Request) -> web.Response:
    runtime = request.app["runtime"]
    result = await runtime.disconnect()
    if result.get("ok") is False:
        return web.json_response(
            {"error": {"code": result.get("error", "UNKNOWN"),
                       "message": result.get("message", "")}},
            status=409,
        )
    return web.json_response(result)


@routes.get("/api/connection")
async def get_connection(request: web.Request) -> web.Response:
    runtime = request.app["runtime"]
    return web.json_response(runtime.connection_snapshot())
