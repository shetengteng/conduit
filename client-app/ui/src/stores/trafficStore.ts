/**
 * 流量曲线 store —— M-γ。
 *
 * 数据流:
 *   1. ConnectedView mount 时调 refresh() 拉一次 baseline
 *   2. SSE traffic_tick 1Hz 推 → onTick 把 (uplink, downlink) 推入滚动窗口
 *   3. 窗口固定 60 秒(60 个采样),旧的从头丢
 *   4. computed 暴露:当前速率(最新点) / 滚动均速 / 累计 / 最大值(给图表 Y 轴用)
 *
 * disconnect 时调 reset() 清空。
 */
import { computed, reactive } from "vue";

import type { TrafficTickPayload } from "../types/client";

import { ClientApi } from "../api/client-api";

const WINDOW_SIZE = 60;

interface TrafficSample {
  ts: number;
  uplink: number;
  downlink: number;
}

interface TrafficState {
  samples: TrafficSample[];
  totalUplink: number;
  totalDownlink: number;
  loading: boolean;
}

const state = reactive<TrafficState>({
  samples: [],
  totalUplink: 0,
  totalDownlink: 0,
  loading: false,
});

async function refresh(): Promise<void> {
  state.loading = true;
  try {
    const snap = await ClientApi.traffic();
    state.totalUplink = snap.total_uplink;
    state.totalDownlink = snap.total_downlink;
  } catch (_) {
    /* 静默 */
  } finally {
    state.loading = false;
  }
}

function onTick(payload: TrafficTickPayload): void {
  state.samples.push({
    ts: payload.ts,
    uplink: payload.uplink_bytes,
    downlink: payload.downlink_bytes,
  });
  if (state.samples.length > WINDOW_SIZE) {
    state.samples.splice(0, state.samples.length - WINDOW_SIZE);
  }
  state.totalUplink = payload.total_uplink;
  state.totalDownlink = payload.total_downlink;
}

function reset(): void {
  state.samples = [];
  state.totalUplink = 0;
  state.totalDownlink = 0;
}

const latestUplink = computed(() =>
  state.samples.length > 0 ? state.samples[state.samples.length - 1].uplink : 0,
);
const latestDownlink = computed(() =>
  state.samples.length > 0 ? state.samples[state.samples.length - 1].downlink : 0,
);
const peakUplink = computed(() =>
  state.samples.reduce((m, s) => (s.uplink > m ? s.uplink : m), 0),
);
const peakDownlink = computed(() =>
  state.samples.reduce((m, s) => (s.downlink > m ? s.downlink : m), 0),
);
const peakAny = computed(() => Math.max(peakUplink.value, peakDownlink.value, 1));
const samplesView = computed(() => state.samples.slice());

export const trafficStore = {
  state,
  refresh,
  onTick,
  reset,
  WINDOW_SIZE,
  samples: samplesView,
  latestUplink,
  latestDownlink,
  peakUplink,
  peakDownlink,
  peakAny,
  totalUplink: computed(() => state.totalUplink),
  totalDownlink: computed(() => state.totalDownlink),
};
