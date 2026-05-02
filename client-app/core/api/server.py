"""客户端控制 HTTP API 的 aiohttp app builder & runner。

设计与 server-app/core/api/server.py 同构：

* 仅监听 127.0.0.1（loopback only），TCP 层就把外网拒之门外；
* loopback_only_middleware 兜底：远端非 127.x 一律 403；
* cors_middleware 在最外层，让 webview（tauri://localhost / http://localhost:1421）能跨源 fetch；
* OPTIONS preflight 由 cors_middleware 直接 204，无需注册路由。

为什么 client 需要单独的 control API：
  Tauri 主进程通过 healthz 探活 sidecar，UI 通过这个 API 读路由 cache、
  操控连接 / 断开、订阅 SSE 事件。
"""
from __future__ import annotations

import logging
from typing import Optional, TYPE_CHECKING

from aiohttp import web

from .cache import routes as cache_routes
from .connect import routes as connect_routes
from .diagnose import routes as diagnose_routes
from .discovery import routes as discovery_routes
from .errors import error_middleware, err_response
from .events import routes as events_routes
from .healthz import routes as healthz_routes

if TYPE_CHECKING:  # pragma: no cover
    from client_main import ClientRuntime

log = logging.getLogger("client.api")


@web.middleware
async def loopback_only_middleware(request: web.Request, handler):  # type: ignore[no-untyped-def]
    if request.method == "OPTIONS":
        return await handler(request)
    if request.app.get("loopback_only", True):
        remote = request.remote or ""
        if remote not in ("127.0.0.1", "::1") and not remote.startswith("127."):
            return err_response("FORBIDDEN", "loopback only", status=403)
    return await handler(request)


_ALLOWED_HEADERS = "Content-Type,Accept,Cache-Control,Last-Event-ID"
_EXPOSED_HEADERS = "Content-Type"


@web.middleware
async def cors_middleware(request: web.Request, handler):  # type: ignore[no-untyped-def]
    if request.method == "OPTIONS":
        resp: web.StreamResponse = web.Response(status=204)
    else:
        resp = await handler(request)
    resp.headers["Access-Control-Allow-Origin"] = "*"
    resp.headers["Access-Control-Allow-Methods"] = "GET,POST,DELETE,OPTIONS"
    resp.headers["Access-Control-Allow-Headers"] = _ALLOWED_HEADERS
    resp.headers["Access-Control-Expose-Headers"] = _EXPOSED_HEADERS
    resp.headers["Access-Control-Max-Age"] = "600"
    return resp


def build_app(runtime: "ClientRuntime", *, loopback_only: bool = True) -> web.Application:
    app = web.Application(
        middlewares=[cors_middleware, loopback_only_middleware, error_middleware]
    )
    app["runtime"] = runtime
    app["loopback_only"] = loopback_only
    for tbl in (healthz_routes, discovery_routes, events_routes, connect_routes, cache_routes, diagnose_routes):
        app.add_routes(tbl)
    return app


class ApiServer:
    """对 aiohttp ``AppRunner`` / ``TCPSite`` 的生命周期包装。"""

    def __init__(self, runtime: "ClientRuntime", *, port: int, loopback_only: bool = True) -> None:
        self.runtime = runtime
        self.port = port
        self.loopback_only = loopback_only
        self._runner: Optional[web.AppRunner] = None
        self._site: Optional[web.TCPSite] = None

    @property
    def running(self) -> bool:
        return self._site is not None

    async def start(self) -> None:
        if self.running:
            return
        bind = "127.0.0.1" if self.loopback_only else "0.0.0.0"
        app = build_app(self.runtime, loopback_only=self.loopback_only)
        self._runner = web.AppRunner(app, access_log=None)
        await self._runner.setup()
        self._site = web.TCPSite(self._runner, bind, self.port)
        await self._site.start()
        log.info("control API listening on %s:%d", bind, self.port)

    async def stop(self) -> None:
        if self._runner is not None:
            await self._runner.cleanup()
            self._runner = None
            self._site = None
            log.info("control API stopped")
