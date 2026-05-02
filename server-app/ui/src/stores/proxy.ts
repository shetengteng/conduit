/**
 * 代理状态 store（reactive，无 Pinia 依赖）。
 *
 * 单例：跨组件共享。被 Dashboard / Tray badge / 顶部状态栏消费。
 */
import { computed, reactive } from "vue";

import type {
  ClientSession,
  HealthzResponse,
  PassiveClient,
  PassiveClientLostPayload,
  PassiveClientSeenPayload,
  ServerStatus,
  VpnStatus,
} from "../types/proxy";

import { ApiError } from "../api/client";
import { ServerApi } from "../api/server";

interface ProxyState {
  status: ServerStatus | null;
  clients: ClientSession[];
  passiveClients: PassiveClient[];
  healthz: HealthzResponse | null;
  loading: boolean;
  error: string | null;
}

const state = reactive<ProxyState>({
  status: null,
  clients: [],
  passiveClients: [],
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
    state.passiveClients = clients.passive_clients ?? [];
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

function applyPassiveClientSeen(p: PassiveClientSeenPayload): void {
  // 同 IP 已存在则只刷新 last_seen,否则插入。
  const idx = state.passiveClients.findIndex((c) => c.peer_ip === p.peer_ip);
  const now = p.first_seen ?? Date.now() / 1000;
  if (idx >= 0) {
    state.passiveClients[idx].last_seen = now;
    state.passiveClients[idx].client_name = p.client_name;
    state.passiveClients[idx].version = p.version;
  } else {
    state.passiveClients.unshift({
      peer_ip: p.peer_ip,
      client_name: p.client_name,
      version: p.version,
      first_seen: p.first_seen,
      last_seen: now,
      idle_sec: 0,
    });
  }
  if (state.status) state.status.passive_clients_count = state.passiveClients.length;
}

function applyPassiveClientLost(p: PassiveClientLostPayload): void {
  state.passiveClients = state.passiveClients.filter((c) => c.peer_ip !== p.peer_ip);
  if (state.status) state.status.passive_clients_count = state.passiveClients.length;
}

export const proxyStore = {
  state,
  refresh,
  applyClientConnected,
  applyClientDisconnected,
  applyVpnState,
  applyPassiveClientSeen,
  applyPassiveClientLost,
  isRunning: computed(() => Boolean(state.status?.running)),
  isReady: computed(() => Boolean(state.status?.ready)),
  pacUrl: computed(() => state.status?.pac_url ?? null),
  /** 总客户端数:活跃会话 + 被动登记 */
  totalClientsCount: computed(
    () => state.clients.length + state.passiveClients.length,
  ),
};
