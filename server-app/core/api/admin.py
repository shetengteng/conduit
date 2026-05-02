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

    # 关键:必须延迟触发 stop,否则 core.stop() 会在 response 写出之前
    # 关掉 control API server,前端拿到 connection reset 误以为停止失败。
    # 50ms 足够 aiohttp 把 200 响应 flush 给客户端。
    async def _delayed_stop() -> None:
        await asyncio.sleep(0.05)
        try:
            await core.stop()
        except Exception:  # noqa: BLE001
            log.exception("delayed core.stop() failed")

    asyncio.create_task(_delayed_stop())
    return web.json_response({"ok": True})


def _is_loopback(remote: str) -> bool:
    return remote in ("127.0.0.1", "::1") or remote.startswith("127.")
