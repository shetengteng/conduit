"""GET /healthz — Tauri 主进程 boot_sequence 用来探活 sidecar 的端点。

M-α 阶段返回最小信息：
    {
      "ready": true,
      "checks": [{"name": "control_api", "ok": true, "detail": "..."}],
      "uptime_sec": <float>
    }

完整自检（端口冲突 / WiFi / 系统代理生效）等推到 M-δ 的 /api/diagnose 实现。
"""
from __future__ import annotations

import time
from typing import TYPE_CHECKING

from aiohttp import web

if TYPE_CHECKING:  # pragma: no cover
    from client_main import ClientRuntime

routes = web.RouteTableDef()

_BOOT_TS = time.monotonic()


@routes.get("/healthz")
async def healthz(request: web.Request) -> web.Response:
    runtime: "ClientRuntime" = request.app["runtime"]
    socks_port = getattr(runtime.proxy, "actual_port", None) or runtime.cfg.bind_port
    api_port = runtime.cfg.api_port

    body = {
        "ready": True,
        "checks": [
            {
                "name": "control_api",
                "ok": True,
                "detail": f"listening on 127.0.0.1:{api_port}",
            },
            {
                "name": "local_proxy",
                "ok": socks_port is not None,
                "detail": f"socks5 on 127.0.0.1:{socks_port}",
            },
        ],
        "uptime_sec": round(time.monotonic() - _BOOT_TS, 3),
    }
    return web.json_response(body)
