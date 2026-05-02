"""GET /api/status, GET /api/clients — ProxyCore introspection.

Wire format intentionally snake_case to match shared-ui/src/types/proxy.ts.
"""
from __future__ import annotations

from aiohttp import web

routes = web.RouteTableDef()


@routes.get("/api/status")
async def get_status(request: web.Request) -> web.Response:
    core = request.app["core"]
    return web.json_response(await core.status())


@routes.get("/api/clients")
async def get_clients(request: web.Request) -> web.Response:
    core = request.app["core"]
    return web.json_response({
        "count": len(core.registry),
        "clients": core.registry.snapshot(),
    })
