"""GET /api/servers — 列出所有发现到的（在线 + 历史 + 手动添加）Conduit Server。

返回示例::

    {
      "count": 2,
      "servers": [
        {
          "server_id": "alpha@192.168.1.20:8080",
          "name": "alpha",
          "host": "192.168.1.20",
          "port": 8080,
          "socks": 1080,
          "api": 9090,
          "vpn": true,
          "version": "0.1.0",
          "pac": "/proxy.pac",
          "pac_url": "http://192.168.1.20:8080/proxy.pac",
          "source": "mdns",
          "last_seen_at": 1714665600.123,
          "healthy": true
        },
        ...
      ]
    }

`source` 字段：
- ``mdns``    — 当前 mDNS 在线
- ``history`` — 上次见过、本次还没出现
- ``manual``  — 用户手动添加（M-δ Settings）
"""
from __future__ import annotations

from aiohttp import web

from api.errors import err_response
from discoverer import server_to_payload

routes = web.RouteTableDef()


@routes.get("/api/servers")
async def list_servers(request: web.Request) -> web.Response:
    runtime = request.app["runtime"]
    discoverer = getattr(runtime, "discoverer", None)
    if discoverer is None:
        return web.json_response({"count": 0, "servers": [], "available": False})

    items = discoverer.snapshot()
    return web.json_response({
        "count": len(items),
        "available": discoverer.available,
        "servers": [server_to_payload(it) for it in items],
    })


@routes.post("/api/servers/forget")
async def forget_server(request: web.Request) -> web.Response:
    """从 known-servers.json 永久移除一个历史 server。

    Body: { "server_id": "name@host:port" }

    若该 server 当前 mDNS 在线,只会清掉它在历史里的"上次见过"记录,
    在线条目仍会保留(因为 zeroconf 还在广播);下次 client 重启时,
    若该 server 仍未出现,就不会再显示了。
    """
    try:
        data = await request.json()
        server_id = str(data["server_id"]).strip()
        if not server_id:
            raise ValueError("empty server_id")
    except (KeyError, ValueError, TypeError) as exc:
        return err_response("BAD_REQUEST", f"invalid body: {exc}", status=400)

    runtime = request.app["runtime"]
    discoverer = getattr(runtime, "discoverer", None)
    if discoverer is None:
        return err_response("NOT_AVAILABLE", "discoverer not initialized", status=503)

    removed = discoverer.forget(server_id)
    return web.json_response({"ok": True, "removed": removed, "server_id": server_id})


@routes.post("/api/servers/forget_all")
async def forget_all_history(request: web.Request) -> web.Response:
    """清空所有"曾见过"的历史 server。当前 mDNS 在线的不受影响。"""
    runtime = request.app["runtime"]
    discoverer = getattr(runtime, "discoverer", None)
    if discoverer is None:
        return err_response("NOT_AVAILABLE", "discoverer not initialized", status=503)
    removed = discoverer.forget_all_history()
    return web.json_response({"ok": True, "removed_count": removed})
