"""POST /api/admin/stop — graceful shutdown command.

Only allowed from 127.0.0.1 (already enforced at the app level if
``api_bind_loopback_only=True``; the explicit check here is belt-and-braces
in case the operator binds to a wider interface for debugging).
"""
from __future__ import annotations

import asyncio
import logging

from aiohttp import web

from .errors import err_response

log = logging.getLogger("core.api.admin")

routes = web.RouteTableDef()


@routes.post("/api/admin/stop")
async def stop_proxy(request: web.Request) -> web.Response:
    if not _is_loopback(request.remote or ""):
        return err_response("FORBIDDEN", "loopback only", status=403)

    core = request.app["core"]
    log.info("admin stop requested by %s", request.remote)
    asyncio.create_task(core.stop())
    return web.json_response({"ok": True})


def _is_loopback(remote: str) -> bool:
    return remote in ("127.0.0.1", "::1") or remote.startswith("127.")
