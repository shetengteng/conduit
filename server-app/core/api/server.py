"""aiohttp app builder & runner for the local control API.

Single shared application owned by ProxyCore. Bound to ``127.0.0.1`` by
default (set ``cfg.api_bind_loopback_only=False`` to expose, e.g. for
remote debugging — not recommended).
"""
from __future__ import annotations

import logging
from typing import Optional, TYPE_CHECKING

from aiohttp import web

from .admin import routes as admin_routes
from .errors import error_middleware, err_response
from .events import routes as events_routes
from .healthz import routes as healthz_routes
from .status import routes as status_routes
from .traffic import routes as traffic_routes

if TYPE_CHECKING:  # pragma: no cover
    from proxy_core import ProxyCore

log = logging.getLogger("core.api")


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


def build_app(core: "ProxyCore", *, loopback_only: bool = True) -> web.Application:
    app = web.Application(
        middlewares=[cors_middleware, loopback_only_middleware, error_middleware]
    )
    app["core"] = core
    app["loopback_only"] = loopback_only
    for tbl in (
        healthz_routes,
        status_routes,
        traffic_routes,
        events_routes,
        admin_routes,
    ):
        app.add_routes(tbl)
    return app


class ApiServer:
    """Lifecycle wrapper around the aiohttp ``AppRunner`` / ``TCPSite``."""

    def __init__(self, core: "ProxyCore") -> None:
        self.core = core
        self.cfg = core.cfg
        self._runner: Optional[web.AppRunner] = None
        self._site: Optional[web.TCPSite] = None

    @property
    def running(self) -> bool:
        return self._site is not None

    async def start(self) -> None:
        if self.running:
            return
        bind = "127.0.0.1" if self.cfg.api_bind_loopback_only else self.cfg.bind
        app = build_app(self.core, loopback_only=self.cfg.api_bind_loopback_only)
        self._runner = web.AppRunner(app, access_log=None)
        await self._runner.setup()
        self._site = web.TCPSite(self._runner, bind, self.cfg.api_port)
        await self._site.start()
        log.info("control API listening on %s:%d", bind, self.cfg.api_port)

    async def stop(self) -> None:
        if self._runner is not None:
            await self._runner.cleanup()
            self._runner = None
            self._site = None
            log.info("control API stopped")
