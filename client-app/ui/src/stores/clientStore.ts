/**
 * 客户端运行时状态 store（reactive，无 Pinia 依赖）。
 *
 * M-α 阶段只跟踪 control API 的 healthz 状态：用 healthz.ready 判断
 * sidecar 是否在跑。后续 M-β 接入连接状态（connected / disconnected /
 * connecting）、当前 server、SOCKS5 端口；M-γ 接入路由 cache 统计。
 */
import { computed, reactive } from "vue";

import type { HealthzResponse } from "../types/client";

import { ApiError } from "../api/client";
import { ClientApi } from "../api/client-api";
import { useToast } from "../composables/useToast";

interface ClientState {
  healthz: HealthzResponse | null;
  loading: boolean;
  error: string | null;
  // 上一次成功拉到 healthz 的本地时间戳(ms),用于 UI 端外推 uptime_sec。
  // backend 的 uptime_sec 是个快照,UI 想要每秒平滑增长就必须本地外推。
  healthzFetchedAtMs: number;
}

const state = reactive<ClientState>({
  healthz: null,
  loading: false,
  error: null,
  healthzFetchedAtMs: 0,
});

async function refresh(): Promise<void> {
  state.loading = true;
  const wasError = state.error;
  state.error = null;

  // 第一次启动时,UI 切换到 Ready 后立即发请求,sidecar control API socket
  // 刚 LISTEN 可能还没接受连接,fetch 会 "Load failed"。给 3 次重试,300ms 间隔。
  const MAX_ATTEMPTS = 3;
  let lastErr: unknown = null;
  for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
    try {
      state.healthz = await ClientApi.healthz();
      state.healthzFetchedAtMs = Date.now();
      state.loading = false;
      return;
    } catch (e) {
      lastErr = e;
      if (attempt < MAX_ATTEMPTS) {
        await new Promise((r) => setTimeout(r, 300));
      }
    }
  }

  const msg = lastErr instanceof ApiError
    ? `${lastErr.code}: ${lastErr.message}`
    : String(lastErr);
  state.error = msg;
  if (!wasError) {
    try {
      useToast().error("无法连接到客户端服务", { detail: msg });
    } catch (_) {
      /* toast 不可用时静默 */
    }
  }
  state.loading = false;
}

// 仅刷新 healthz,不弹错误 toast。专门服务于 polling 让 uptime/ready 持续新鲜。
async function refreshSilently(): Promise<void> {
  try {
    state.healthz = await ClientApi.healthz();
    state.healthzFetchedAtMs = Date.now();
  } catch (_) {
    /* 静默:正式 refresh 已负责报错 */
  }
}

export const clientStore = {
  state,
  refresh,
  refreshSilently,
  isReady: computed(() => Boolean(state.healthz?.ready)),
  uptimeSec: computed(() => state.healthz?.uptime_sec ?? 0),
  checks: computed(() => state.healthz?.checks ?? []),
};
