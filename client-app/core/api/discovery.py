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
