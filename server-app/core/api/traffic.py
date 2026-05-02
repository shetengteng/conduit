"""GET /api/traffic — per-peer time-series for the Dashboard chart.

Query params:

- ``window`` (int, default 60, max ``cfg.traffic_sample_window_sec``)
- ``peer``   (optional, restrict to one peer IP)

Response::

    {
      "window_sec": 60,
      "now": 1234567890.123,
      "series": {
        "192.168.1.5": [[ts, sent_bps, recv_bps], ...]
      }
    }
"""
from __future__ import annotations

import time
from typing import Any

from aiohttp import web

from .errors import err_response

routes = web.RouteTableDef()


@routes.get("/api/traffic")
async def get_traffic(request: web.Request) -> web.Response:
    core = request.app["core"]
    cap = max(1, int(getattr(core.cfg, "traffic_sample_window_sec", 60)))

    try:
        window = int(request.query.get("window", "60"))
    except ValueError:
        return err_response("BAD_PARAM", "window must be int", status=400)
    if window <= 0:
        window = 60
    window = min(window, cap)

    peer = request.query.get("peer")
    if peer:
        series = {peer: [list(t) for t in core.sampler.series(peer, window)]}
    else:
        series = {
            ip: [list(t) for t in samples]
            for ip, samples in core.sampler.all_series(window).items()
        }

    body: dict[str, Any] = {
        "window_sec": window,
        "now": time.time(),
        "series": series,
    }
    return web.json_response(body)
