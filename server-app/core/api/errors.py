"""Unified error response envelope.

Shape (matches the wire-format chapter in design §3.5.7)::

    {"error": {"code": "PORT_IN_USE", "message": "8080 already bound"}}
"""
from __future__ import annotations

import logging
from typing import Any

from aiohttp import web

log = logging.getLogger("core.api")


def err_response(code: str, message: str, status: int = 400, **extra: Any) -> web.Response:
    body: dict[str, Any] = {"error": {"code": code, "message": message}}
    if extra:
        body["error"].update(extra)
    return web.json_response(body, status=status)


@web.middleware
async def error_middleware(request: web.Request, handler):  # type: ignore[no-untyped-def]
    try:
        return await handler(request)
    except web.HTTPException as exc:
        if exc.status >= 400:
            return err_response(
                code=f"HTTP_{exc.status}",
                message=exc.reason or exc.text or "http_error",
                status=exc.status,
            )
        raise
    except Exception as exc:  # noqa: BLE001 — top-level guard
        log.exception("unhandled error in %s %s", request.method, request.path)
        return err_response(
            code="INTERNAL_ERROR",
            message=str(exc) or exc.__class__.__name__,
            status=500,
        )
