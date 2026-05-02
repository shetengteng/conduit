/**
 * 流量样本环形缓冲（共享）。
 *
 * 数据来源：1) 启动时 GET /api/traffic 拉历史窗口；2) SSE traffic_tick 实时推。
 */
import { computed, reactive } from "vue";

import type { TrafficSamplePoint, TrafficTickPayload } from "../types/proxy";

import { ServerApi } from "../api/server";

const MAX_POINTS = 600;

interface TrafficState {
  series: Record<string, TrafficSamplePoint[]>;
  windowSec: number;
  lastTickTs: number;
}

const state = reactive<TrafficState>({
  series: {},
  windowSec: 60,
  lastTickTs: 0,
});

async function loadInitial(window = 60): Promise<void> {
  state.windowSec = window;
  const resp = await ServerApi.traffic(window);
  state.series = resp.series;
  state.lastTickTs = resp.now;
}

function applyTick(payload: TrafficTickPayload): void {
  state.lastTickTs = payload.ts;
  for (const [peer, sample] of Object.entries(payload.per_peer)) {
    const arr = state.series[peer] ?? [];
    arr.push([payload.ts, sample.sent_bps, sample.recv_bps]);
    if (arr.length > MAX_POINTS) arr.shift();
    state.series[peer] = arr;
  }
}

function reset(): void {
  state.series = {};
  state.lastTickTs = 0;
}

export const trafficStore = {
  state,
  loadInitial,
  applyTick,
  reset,
  totalBps: computed(() => {
    let inBps = 0;
    let outBps = 0;
    for (const arr of Object.values(state.series)) {
      const last = arr[arr.length - 1];
      if (last) {
        outBps += last[1];
        inBps += last[2];
      }
    }
    return { in_bps: inBps, out_bps: outBps };
  }),
};
