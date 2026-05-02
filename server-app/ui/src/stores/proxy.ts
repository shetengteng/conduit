/**
 * 代理状态 store（reactive，无 Pinia 依赖）。
 *
 * 单例：跨组件共享。被 Dashboard / Tray badge / 顶部状态栏消费。
 */
import { computed, reactive } from "vue";

import type {
  ClientSession,
  HealthzResponse,
  ServerStatus,
  VpnStatus,
} from "../types/proxy";

import { ApiError } from "../api/client";
import { ServerApi } from "../api/server";

interface ProxyState {
  status: ServerStatus | null;
  clients: ClientSession[];
  healthz: HealthzResponse | null;
  loading: boolean;
  error: string | null;
}

const state = reactive<ProxyState>({
  status: null,
  clients: [],
  healthz: null,
  loading: false,
  error: null,
});

async function refresh(): Promise<void> {
  state.loading = true;
  const wasError = state.error;
  state.error = null;
  try {
    const [status, clients, healthz] = await Promise.all([
      ServerApi.status(),
      ServerApi.clients(),
      ServerApi.healthz(),
    ]);
    state.status = status;
    state.clients = clients.clients;
    state.healthz = healthz;
  } catch (e) {
    const msg = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
    state.error = msg;
    if (!wasError) {
      try {
        const { useToast } = await import("../composables/useToast");
        useToast().error("无法连接到代理服务", { detail: msg });
      } catch (_) {
        /* toast 不可用时静默 */
      }
    }
  } finally {
    state.loading = false;
  }
}

function applyClientConnected(s: ClientSession): void {
  if (state.clients.some((c) => c.session_id === s.session_id)) return;
  state.clients.push(s);
  if (state.status) state.status.clients_count = state.clients.length;
}

function applyClientDisconnected(session_id: string): void {
  state.clients = state.clients.filter((c) => c.session_id !== session_id);
  if (state.status) state.status.clients_count = state.clients.length;
}

function applyVpnState(v: VpnStatus): void {
  if (state.status) state.status.vpn = v;
}

export const proxyStore = {
  state,
  refresh,
  applyClientConnected,
  applyClientDisconnected,
  applyVpnState,
  isRunning: computed(() => Boolean(state.status?.running)),
  isReady: computed(() => Boolean(state.status?.ready)),
  pacUrl: computed(() => state.status?.pac_url ?? null),
};
