"""GET /healthz — liveness probe used by the Tauri shell.

Always returns 200 with the per-check breakdown so the shell can render
the failing component. The Tauri startup polls this until ``ready: true``
or 9 s timeout.
"""
from __future__ import annotations

from aiohttp import web

routes = web.RouteTableDef()


@routes.get("/healthz")
async def healthz(request: web.Request) -> web.Response:
    core = request.app["core"]
    detail = await core.health.to_dict()
    detail["running"] = core.running
    detail["uptime_sec"] = core.uptime_sec
    return web.json_response(detail)
