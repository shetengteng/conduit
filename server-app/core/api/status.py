"""GET /api/status, GET /api/clients, POST /api/clients/heartbeat — ProxyCore introspection.

Wire format intentionally snake_case to match server-app/ui/src/types/proxy.ts.
"""
from __future__ import annotations

from aiohttp import web

from .errors import err_response

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
        "passive_count": len(core.passive_clients),
        "passive_clients": core.passive_clients.snapshot(),
    })


@routes.post("/api/clients/heartbeat")
async def post_client_heartbeat(request: web.Request) -> web.Response:
    """Client-app 心跳上报:让 server 知道哪些 client 已链接但暂未传输流量。

    Body: { "client_name": "MyMac", "version": "0.1.0" }
    peer_ip 由 aiohttp 自动从 socket 拿,不接受客户端伪造。

    返回 { "ok": true, "created": bool, "ttl_sec": <60> }
    """
    core = request.app["core"]
    try:
        data = await request.json()
        client_name = str(data.get("client_name", "")).strip() or "anonymous"
        version = str(data.get("version", "")).strip() or "unknown"
    except Exception as exc:  # noqa: BLE001
        return err_response("BAD_REQUEST", f"invalid JSON: {exc}", status=400)

    peer_ip = request.remote or "0.0.0.0"
    created = core.passive_clients.touch(peer_ip, client_name, version)
    return web.json_response({
        "ok": True,
        "created": created,
        "ttl_sec": int(core.passive_clients._ttl),
    })
