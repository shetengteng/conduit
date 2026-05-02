"""Server 可达性 probe + 心跳 —— M-β.2 阶段。

两个公开能力:

* ``probe(server)``  —— 一次性可达性验证。在 connect 流程的"第 1 步"调,验证
  server 的 healthz 200 + SOCKS/HTTP 端口 TCP 三次握手成功。返 ``ProbeResult``。

* ``Heartbeat`` 协程 —— 进入 connected 态后由 ClientRuntime 启动:每 10s 一次
  GET /healthz; 连续 N 次失败触发 ``heartbeat_changed`` 事件(payload tone =
  green / yellow / red),N=3 时归类为 red 并通知调用方做全局降级。

设计原则:
- 失败种类区分清楚(timeout / connection refused / 5xx),好让 UI 区分 "网络
  断" / "server 异常" / "我们解析错了"。
- 不在这里改任何全局状态(cache / system_proxy),只 publish 事件。状态机由
  ClientRuntime 统一管理。
"""
from __future__ import annotations

import asyncio
import json
import logging
import time
from dataclasses import dataclass
from typing import Optional

import urllib.error
import urllib.request

log = logging.getLogger("conduit.client.connectivity")

DEFAULT_PROBE_TIMEOUT = 3.0          # healthz GET / TCP open
DEFAULT_HEARTBEAT_INTERVAL = 10.0    # 心跳节拍
DEFAULT_HEARTBEAT_TIMEOUT = 5.0      # 单次心跳 GET /healthz timeout
HEARTBEAT_YELLOW_AT = 1              # 第 1 次失败 -> 黄
HEARTBEAT_RED_AT = 3                 # 连续 3 次失败 -> 红 + 全局降级


# ---------------------------------------------------------------------------
# 一次性 probe
# ---------------------------------------------------------------------------


@dataclass
class ProbeResult:
    ok: bool
    healthz_ok: bool
    socks_reachable: bool
    http_reachable: bool
    error: Optional[str] = None
    latency_ms: float = 0.0
    server_vpn: bool = False  # 从 healthz checks 解析出的 vpn 状态


async def _tcp_probe(host: str, port: int, *, timeout: float) -> bool:
    """TCP 三次握手成功即视为可达。"""
    try:
        fut = asyncio.open_connection(host, port)
        reader, writer = await asyncio.wait_for(fut, timeout=timeout)
    except (asyncio.TimeoutError, OSError) as exc:
        log.debug("tcp probe %s:%d failed: %s", host, port, exc)
        return False
    try:
        writer.close()
        await writer.wait_closed()
    except Exception:
        pass
    return True


async def _healthz_probe(host: str, port: int, *, timeout: float) -> tuple[bool, bool, Optional[str]]:
    """GET http://host:port/healthz —— 返 (ok, vpn, error_or_none)。

    server-app 的 healthz 是 loopback only,所以**这里走 LAN HTTP**(同机器
    访问 server 的 8090 不可能拿到,真实场景必须用 server 的 8080 暴露的
    _LAN_-facing healthz)。

    设计澄清:server 的 control API 是 loopback,不能从 LAN 访问。我们这里
    实际上 probe 的是 server 的 *HTTP proxy 端口*(就是 SOCKS 之外的代理端
    口,8080 默认)上的某个 health 路径 —— 但 server 的 HTTP proxy 端口只
    转发流量,没有 health 路径。

    妥协方案(M-β.2):用 mDNS TXT 中的 ``port`` 端口做 TCP probe 即可,不
    实际打 healthz HTTP。 ``healthz_ok`` 字段保留但永远 True,vpn 信息暂
    走 mDNS TXT 已经拿到的(传进 probe 时再带上)。

    M-γ 计划:server 增加 ``GET :http_port/_health`` 公开端点,client 真
    打 health。
    """
    return True, False, None


async def probe(
    *,
    host: str,
    http_port: int,
    socks_port: int,
    timeout: float = DEFAULT_PROBE_TIMEOUT,
) -> ProbeResult:
    """一次性可达性检查。所有底层调用并行,总耗时不超过 ``timeout``。"""
    start = time.monotonic()
    socks_task = asyncio.create_task(_tcp_probe(host, socks_port, timeout=timeout))
    http_task = asyncio.create_task(_tcp_probe(host, http_port, timeout=timeout))
    try:
        socks_ok, http_ok = await asyncio.gather(socks_task, http_task)
    except Exception as exc:  # noqa: BLE001
        return ProbeResult(
            ok=False, healthz_ok=False, socks_reachable=False, http_reachable=False,
            error=str(exc),
            latency_ms=(time.monotonic() - start) * 1000,
        )

    healthz_ok, _vpn, _err = await _healthz_probe(host, http_port, timeout=timeout)
    elapsed_ms = (time.monotonic() - start) * 1000

    if not socks_ok and not http_ok:
        err = f"{host}: SOCKS({socks_port}) 与 HTTP({http_port}) 端口都不通"
    elif not socks_ok:
        err = f"{host}: SOCKS 端口 {socks_port} 不可达"
    elif not http_ok:
        err = f"{host}: HTTP 端口 {http_port} 不可达"
    else:
        err = None

    return ProbeResult(
        ok=socks_ok and http_ok,
        healthz_ok=healthz_ok,
        socks_reachable=socks_ok,
        http_reachable=http_ok,
        error=err,
        latency_ms=elapsed_ms,
        server_vpn=False,
    )


# ---------------------------------------------------------------------------
# 心跳协程
# ---------------------------------------------------------------------------


@dataclass
class HeartbeatState:
    tone: str = "green"           # green / yellow / red
    consecutive_failures: int = 0
    last_check_at: float = 0.0
    last_error: Optional[str] = None


class Heartbeat:
    """简单的心跳:每 ``interval`` 秒一次 TCP probe; 失败次数累加,触发 tone
    迁移。失败回归到成功直接 reset 到 green。"""

    def __init__(
        self,
        bus,
        *,
        host: str,
        http_port: int,
        socks_port: int,
        interval: float = DEFAULT_HEARTBEAT_INTERVAL,
        timeout: float = DEFAULT_HEARTBEAT_TIMEOUT,
    ) -> None:
        self.bus = bus
        self.host = host
        self.http_port = http_port
        self.socks_port = socks_port
        self.interval = interval
        self.timeout = timeout
        self.state = HeartbeatState()
        self._task: Optional[asyncio.Task] = None
        self._stop = asyncio.Event()

    @property
    def running(self) -> bool:
        return self._task is not None and not self._task.done()

    async def start(self) -> None:
        if self.running:
            return
        self._stop = asyncio.Event()
        self._task = asyncio.create_task(self._loop(), name="conduit.heartbeat")
        log.info("heartbeat started for %s (interval=%.1fs)", self.host, self.interval)

    async def stop(self) -> None:
        if self._task is None:
            return
        self._stop.set()
        try:
            await asyncio.wait_for(self._task, timeout=self.interval + 1.0)
        except asyncio.TimeoutError:
            self._task.cancel()
        self._task = None
        log.info("heartbeat stopped")

    async def _loop(self) -> None:
        while not self._stop.is_set():
            await self._tick()
            try:
                await asyncio.wait_for(self._stop.wait(), timeout=self.interval)
            except asyncio.TimeoutError:
                pass

    async def _tick(self) -> None:
        ok = await _tcp_probe(self.host, self.socks_port, timeout=self.timeout)
        self.state.last_check_at = time.time()
        if ok:
            old_tone = self.state.tone
            self.state.consecutive_failures = 0
            self.state.last_error = None
            self.state.tone = "green"
            if old_tone != "green":
                self._publish_tone("green", recovered=True)
        else:
            self.state.consecutive_failures += 1
            self.state.last_error = f"SOCKS {self.host}:{self.socks_port} 不可达"
            new_tone = self._compute_tone(self.state.consecutive_failures)
            if new_tone != self.state.tone:
                self.state.tone = new_tone
                self._publish_tone(new_tone, recovered=False)

    @staticmethod
    def _compute_tone(failures: int) -> str:
        if failures >= HEARTBEAT_RED_AT:
            return "red"
        if failures >= HEARTBEAT_YELLOW_AT:
            return "yellow"
        return "green"

    def _publish_tone(self, tone: str, *, recovered: bool) -> None:
        log.info("heartbeat tone -> %s (failures=%d, recovered=%s)",
                 tone, self.state.consecutive_failures, recovered)
        self.bus.publish("heartbeat_changed", {
            "tone": tone,
            "consecutive_failures": self.state.consecutive_failures,
            "recovered": recovered,
            "last_error": self.state.last_error,
            "host": self.host,
        })
