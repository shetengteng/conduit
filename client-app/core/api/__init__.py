"""Conduit Client 控制 HTTP API。

模块布局（M-α 阶段只完成最小子集，剩余在 M-β/γ/δ 增补）：

* ``server.py``      — aiohttp app 构建器、CORS / loopback-only 中间件、生命周期包装。
* ``healthz.py``     — `GET /healthz`，Tauri 主进程 boot_sequence 的就绪信号。
* (M-β) discovery.py / connect.py / disconnect.py / events.py
* (M-γ) route.py / cache.py
* (M-δ) diagnose.py
"""

from .server import ApiServer, build_app

__all__ = ["ApiServer", "build_app"]
