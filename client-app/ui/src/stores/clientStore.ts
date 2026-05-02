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

interface ClientState {
  healthz: HealthzResponse | null;
  loading: boolean;
  error: string | null;
}

const state = reactive<ClientState>({
  healthz: null,
  loading: false,
  error: null,
});

async function refresh(): Promise<void> {
  state.loading = true;
  const wasError = state.error;
  state.error = null;
  try {
    state.healthz = await ClientApi.healthz();
  } catch (e) {
    const msg = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
    state.error = msg;
    if (!wasError) {
      try {
        const { useToast } = await import("../composables/useToast");
        useToast().error("无法连接到客户端服务", { detail: msg });
      } catch (_) {
        /* toast 不可用时静默 */
      }
    }
  } finally {
    state.loading = false;
  }
}

export const clientStore = {
  state,
  refresh,
  isReady: computed(() => Boolean(state.healthz?.ready)),
  uptimeSec: computed(() => state.healthz?.uptime_sec ?? 0),
  checks: computed(() => state.healthz?.checks ?? []),
};
