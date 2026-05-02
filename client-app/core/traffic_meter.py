"""TrafficMeter —— M-γ 流量计量器。

设计:
  - LocalProxyServer 的 relay 每读到一段字节,调 ``on_chunk(uplink, downlink)``
  - TrafficMeter 累积进 1 秒 bucket,每秒 flush 一次 -> EventBus.publish("traffic_tick", ...)
  - 同时 keep cumulative `total_uplink` / `total_downlink`,UI 可计算总流量
  - bucket 为空也照样 publish(让 UI 曲线连续,不出现"空白")

事件 payload(`traffic_tick`):
  ```
  {
    "ts": 1777686900,             # epoch seconds(int)
    "uplink_bytes": 12345,        # 这一秒的上行
    "downlink_bytes": 67890,      # 这一秒的下行
    "total_uplink": 1234567,      # 自连接以来累计上行
    "total_downlink": 9876543,    # 自连接以来累计下行
  }
  ```

线程模型:
  - on_chunk 在 asyncio loop 里异步调用,直接 write bucket(无需锁,单 loop)
  - flush_loop 是 asyncio.Task,每秒 publish 一次

生命周期:
  - connect 成功后 ClientRuntime 创建 + start
  - disconnect 时 stop,cumulative 清零
"""
from __future__ import annotations

import asyncio
import logging
import time
from typing import Optional

log = logging.getLogger("conduit.client.traffic")

DEFAULT_TICK_INTERVAL = 1.0


class TrafficMeter:
    def __init__(self, bus, *, tick_interval: float = DEFAULT_TICK_INTERVAL) -> None:
        self.bus = bus
        self.tick_interval = tick_interval
        self.total_uplink = 0
        self.total_downlink = 0
        self._bucket_uplink = 0
        self._bucket_downlink = 0
        self._task: Optional[asyncio.Task] = None
        self._stop = asyncio.Event()

    async def on_chunk(self, uplink: int, downlink: int) -> None:
        """relay 回调入口。直接累加,无需 await(签名必须是 async 因 relay 期望 awaitable)。"""
        if uplink:
            self._bucket_uplink += uplink
            self.total_uplink += uplink
        if downlink:
            self._bucket_downlink += downlink
            self.total_downlink += downlink

    async def start(self) -> None:
        if self._task is not None:
            return
        self._stop.clear()
        self._task = asyncio.create_task(self._loop(), name="traffic_meter")
        log.info("traffic_meter started (tick=%.1fs)", self.tick_interval)

    async def stop(self) -> None:
        if self._task is None:
            return
        self._stop.set()
        try:
            await asyncio.wait_for(self._task, timeout=2.0)
        except asyncio.TimeoutError:
            self._task.cancel()
            try:
                await self._task
            except asyncio.CancelledError:
                pass
        self._task = None
        log.info(
            "traffic_meter stopped (cumulative up=%d B, down=%d B)",
            self.total_uplink, self.total_downlink,
        )

    def snapshot(self) -> dict:
        """`GET /api/traffic` 用,返回当前累计 + 最近一秒桶。"""
        return {
            "ts": int(time.time()),
            "uplink_bytes": self._bucket_uplink,
            "downlink_bytes": self._bucket_downlink,
            "total_uplink": self.total_uplink,
            "total_downlink": self.total_downlink,
        }

    async def _loop(self) -> None:
        while not self._stop.is_set():
            try:
                await asyncio.wait_for(self._stop.wait(), timeout=self.tick_interval)
            except asyncio.TimeoutError:
                pass
            self._flush()

    def _flush(self) -> None:
        up = self._bucket_uplink
        down = self._bucket_downlink
        self._bucket_uplink = 0
        self._bucket_downlink = 0
        self.bus.publish("traffic_tick", {
            "ts": int(time.time()),
            "uplink_bytes": up,
            "downlink_bytes": down,
            "total_uplink": self.total_uplink,
            "total_downlink": self.total_downlink,
        })


__all__ = ["TrafficMeter"]
