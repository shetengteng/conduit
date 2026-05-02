"""GET /api/cache + DELETE /api/cache + GET /api/traffic —— M-γ。

GET /api/cache:
  返回 RouteCache 当前所有 entry,按 `last_used` desc 排序。
  支持 query: ?direction=direct|proxy / ?source=pac|probe|manual / ?limit=N

DELETE /api/cache:
  清空全部条目。返回 {"removed": N}。

GET /api/traffic:
  返回当前 TrafficMeter snapshot。disconnect 状态返回 zero。
"""
from __future__ import annotations

from aiohttp import web

routes = web.RouteTableDef()


@routes.get("/api/cache")
async def list_cache(request: web.Request) -> web.Response:
    runtime = request.app["runtime"]
    cache = runtime.route_cache
    direction = request.query.get("direction")
    source = request.query.get("source")
    limit_s = request.query.get("limit")
    limit = int(limit_s) if limit_s and limit_s.isdigit() else 200

    items = []
    for _, entry in cache.items():
        if direction and entry.direction != direction:
            continue
        if source and entry.source != source:
            continue
        items.append(entry.to_dict())

    items.sort(key=lambda d: d.get("last_used", ""), reverse=True)
    items = items[:limit]

    stats = cache.stats()
    return web.json_response({
        "count": len(items),
        "total": stats.total,
        "stats": {
            "total": stats.total,
            "direct_count": stats.direct_count,
            "proxy_count": stats.proxy_count,
            "expired_count": stats.expired_count,
            "by_source": stats.by_source,
            "hits": stats.hits,
            "misses": stats.misses,
            "evictions": stats.evictions,
        },
        "entries": items,
    })


@routes.delete("/api/cache")
async def flush_cache(request: web.Request) -> web.Response:
    runtime = request.app["runtime"]
    removed = runtime.route_cache.flush_all()
    return web.json_response({"ok": True, "removed": removed})


@routes.get("/api/traffic")
async def get_traffic(request: web.Request) -> web.Response:
    runtime = request.app["runtime"]
    meter = getattr(runtime, "traffic_meter", None)
    if meter is None:
        return web.json_response({
            "ts": 0,
            "uplink_bytes": 0,
            "downlink_bytes": 0,
            "total_uplink": 0,
            "total_downlink": 0,
        })
    return web.json_response(meter.snapshot())
