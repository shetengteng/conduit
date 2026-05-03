"""GET /api/diagnose —— M-δ 完整 5 步自检。

返回结构:
  ```
  {
    "ok": true,                   # 全部 5 步是否都 ok
    "checks": [
      {
        "key": "sidecar",
        "label": "Sidecar 进程",
        "ok": true,
        "detail": "PID 12345 · 已运行 1234 秒",
        "remediation": null
      },
      {"key": "mdns", "label": "mDNS 服务发现", ...},
      {"key": "server_reach", ...},
      {"key": "pac", ...},
      {"key": "system_proxy", ...}
    ],
    "checked_at": 1777689000
  }
  ```

每步有专属 remediation 文案,失败时 UI 直接展示给用户。
"""
from __future__ import annotations

import asyncio
import logging
import os
import time
from typing import Any, Optional

from aiohttp import web

from connectivity import probe

log = logging.getLogger("client.api.diagnose")
routes = web.RouteTableDef()

PROBE_TIMEOUT = 2.0
PAC_TIMEOUT = 3.0


@routes.get("/api/diagnose")
async def diagnose(request: web.Request) -> web.Response:
    runtime = request.app["runtime"]
    checks: list[dict[str, Any]] = []
    checks.append(_check_sidecar(runtime))
    checks.append(_check_mdns(runtime))
    checks.append(await _check_server_reach(runtime))
    checks.append(await _check_pac(runtime))
    checks.append(_check_system_proxy(runtime))

    overall_ok = all(c["ok"] for c in checks)
    return web.json_response({
        "ok": overall_ok,
        "checks": checks,
        "checked_at": int(time.time()),
    })


# ---------------------------------------------------------------------------
# 单步实现
# ---------------------------------------------------------------------------


def _check_sidecar(runtime) -> dict[str, Any]:
    pid = os.getpid()
    uptime = int(time.time() - getattr(runtime, "_started_at", time.time()))
    return {
        "key": "sidecar",
        "label": "Sidecar 进程",
        "ok": True,
        "detail": f"PID {pid} · SOCKS5 :{runtime.proxy.actual_port} · 控制 API :{runtime.cfg.api_port} · 已运行 {uptime} 秒",
        "remediation": None,
    }


def _check_mdns(runtime) -> dict[str, Any]:
    discoverer = getattr(runtime, "discoverer", None)
    if discoverer is None or not discoverer.available:
        return {
            "key": "mdns",
            "label": "mDNS 服务发现",
            "ok": False,
            "detail": "zeroconf 模块未加载,无法监听 _conduit._tcp.local.",
            "remediation": "在 sidecar 环境装 zeroconf:`pip install zeroconf`,然后重启 client。",
        }
    snap = discoverer.snapshot()
    online = sum(1 for s in snap if s.source == "mdns")
    history = sum(1 for s in snap if s.source != "mdns")
    if online == 0 and history == 0:
        return {
            "key": "mdns",
            "label": "mDNS 服务发现",
            "ok": False,
            "detail": "已监听,但暂未发现任何 server",
            "remediation": (
                "1. 确认 server-app 已启动且未设 CONDUIT_NO_MDNS=1\n"
                "2. 在 macOS 系统设置 → 隐私与安全性 → 本地网络 中允许本应用\n"
                "3. 公司 / 公共 WLAN 可能屏蔽 multicast,改用『设置』页手动连接"
            ),
        }
    return {
        "key": "mdns",
        "label": "mDNS 服务发现",
        "ok": True,
        "detail": f"在线 {online} 个,历史 {history} 个",
        "remediation": None,
    }


async def _check_server_reach(runtime) -> dict[str, Any]:
    server = runtime.connected_server
    if server is None:
        return {
            "key": "server_reach",
            "label": "上游 Server 可达",
            "ok": True,
            "detail": "未连接任何 server,跳过本项检查",
            "remediation": None,
        }
    try:
        result = await probe(
            host=server.host,
            http_port=server.port,
            socks_port=server.socks,
            timeout=PROBE_TIMEOUT,
        )
    except Exception as exc:  # noqa: BLE001
        return {
            "key": "server_reach",
            "label": "上游 Server 可达",
            "ok": False,
            "detail": f"探测出错: {exc}",
            "remediation": "检查 server 是否在线,网络是否变化(切换 WiFi / VPN)",
        }
    if not result.ok:
        return {
            "key": "server_reach",
            "label": "上游 Server 可达",
            "ok": False,
            "detail": f"双端口都不可达: {result.error}",
            "remediation": (
                "1. 确认 server-app 仍在运行(server 端 healthz 仍 200)\n"
                "2. 确认未切换网段\n"
                "3. 在『已连接』页点『断开连接』,然后到『发现』页重新连接"
            ),
        }
    return {
        "key": "server_reach",
        "label": "上游 Server 可达",
        "ok": True,
        "detail": (
            f"{server.host}:{server.port} (HTTP) "
            f"+ {server.host}:{server.socks} (SOCKS5) · 延迟 {result.latency_ms:.0f} ms"
        ),
        "remediation": None,
    }


async def _check_pac(runtime) -> dict[str, Any]:
    server = runtime.connected_server
    if server is None:
        return {
            "key": "pac",
            "label": "PAC 文件",
            "ok": True,
            "detail": "未连接 server,跳过本项检查",
            "remediation": None,
        }
    pac_url = f"http://{server.host}:{server.port}{server.pac or '/proxy.pac'}"
    try:
        from urllib.request import urlopen  # local import,与 connectivity 一致
        loop = asyncio.get_running_loop()
        text = await asyncio.wait_for(
            loop.run_in_executor(None, lambda: urlopen(pac_url, timeout=PAC_TIMEOUT).read().decode("utf-8")),
            timeout=PAC_TIMEOUT + 0.5,
        )
    except asyncio.TimeoutError:
        return {
            "key": "pac",
            "label": "PAC 文件",
            "ok": False,
            "detail": f"拉取 {pac_url} 超时(>{PAC_TIMEOUT}s)",
            "remediation": "Server 可能负载过高或防火墙拦截,尝试断开后重新连接",
        }
    except Exception as exc:  # noqa: BLE001
        return {
            "key": "pac",
            "label": "PAC 文件",
            "ok": False,
            "detail": f"拉取失败: {exc}",
            "remediation": "检查 server 端的 proxy.pac 文件是否被误删",
        }
    return {
        "key": "pac",
        "label": "PAC 文件",
        "ok": True,
        "detail": f"拉取 {len(text)} 字节,缓存中已预填 {len(runtime.cache)} 条规则",
        "remediation": None,
    }


def _check_system_proxy(runtime) -> dict[str, Any]:
    sp = runtime.system_proxy
    if sp is None:
        return {
            "key": "system_proxy",
            "label": "系统代理",
            "ok": True,
            "detail": "本平台不支持系统代理切换(非 macOS),需用户手动配 SOCKS5",
            "remediation": None,
        }
    if not runtime.cfg.enable_system_proxy:
        return {
            "key": "system_proxy",
            "label": "系统代理",
            "ok": True,
            "detail": "已禁用自动切换(--no-system-proxy 或 CONDUIT_NO_SYSTEM_PROXY=1),需用户手动配 SOCKS5",
            "remediation": None,
        }
    if runtime.connected_server is None:
        return {
            "key": "system_proxy",
            "label": "系统代理",
            "ok": True,
            "detail": "未连接,系统代理保持原状",
            "remediation": None,
        }
    if not runtime._system_proxy_active:
        # 连接本身是好的,只是系统代理没切换成功 ——
        # 用户用 SOCKS5 :{port} 手动配置浏览器/系统也能正常走代理,
        # 因此返回 ok=True(警示性 detail),而不是直接 FAIL 误导用户。
        last_err = getattr(runtime, "_system_proxy_last_error", None) or ""
        port = runtime.proxy.actual_port
        # 提取 networksetup 报错首行,避免 detail 过长
        err_brief = last_err.splitlines()[0][:120] if last_err else ""
        detail = f"未自动切换 · 请手动配 SOCKS5 127.0.0.1:{port}"
        if err_brief:
            detail += f" · {err_brief}"
        return {
            "key": "system_proxy",
            "label": "系统代理",
            "ok": True,
            "detail": detail,
            "remediation": (
                f"macOS 13+ 修改系统代理需管理员权限,Conduit 默认不要求 sudo,因此采用手动配置:\n"
                f"  • 浏览器/App 内直接填 SOCKS5 主机=127.0.0.1 端口={port}\n"
                f"  • 或『系统设置 → 网络 → 详细信息 → 代理』里手动添加同样配置\n"
                f"如希望自动切换:在『设置』页关闭后再开,会重新尝试;仍失败则需以管理员身份启动 Conduit。"
            ),
        }
    return {
        "key": "system_proxy",
        "label": "系统代理",
        "ok": True,
        "detail": f"SOCKS5 :{runtime.proxy.actual_port} 已设置到当前主网卡",
        "remediation": None,
    }
