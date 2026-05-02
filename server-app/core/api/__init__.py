"""HTTP control API package — wires up aiohttp routes against ProxyCore.

Public entry point: :func:`api.server.build_app`. The Tauri shell launches
the sidecar with ``--api-port 8090``; only ``127.0.0.1`` is allowed in.

Layout (one feature per file, see design §3.5.2 / §4.6 / §4.8 / §4.10):

- ``server.py``  — app builder, error middleware, CORS for localhost
- ``errors.py``  — unified ``{"error": {...}}`` envelope
- ``status.py``  — GET /api/status / GET /api/clients
- ``traffic.py`` — GET /api/traffic
- ``events.py``  — GET /api/events  (SSE stream)
- ``admin.py``   — POST /api/admin/stop
- ``healthz.py`` — GET /healthz
"""

from .server import build_app  # noqa: F401
