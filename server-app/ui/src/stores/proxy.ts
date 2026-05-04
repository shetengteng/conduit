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
  // 上一次成功拉到 status 的本地时间戳(ms),用于 UI 端外推 uptime/run-time。
  // backend 给的 uptime_sec 是个快照,UI 想要每秒平滑增长就必须本地外推。
  statusFetchedAtMs: number;
}

const state = reactive<ProxyState>({
  status: null,
  clients: [],
  passiveClients: [],
  healthz: null,
  loading: false,
  error: null,
  statusFetchedAtMs: 0,
});

async function refresh(): Promise<void> {
  state.loading = true;
  const wasError = state.error;
  state.error = null;

  // 第一次启动时,UI 切换到 Ready 后主界面立即挂载并发请求,
  // 但 sidecar control API socket 刚 LISTEN 可能还没准备好接受连接,
  // 导致 fetch 抛 "Load failed"。给 3 次重试,每次间隔 300ms,失败才 toast。
  const MAX_ATTEMPTS = 3;
  let lastErr: unknown = null;
  for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
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
      state.statusFetchedAtMs = Date.now();
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
      const { useToast } = await import("../composables/useToast");
      useToast().error("无法连接到代理服务", { detail: msg });
    } catch (_) {
      /* toast 不可用时静默 */
    }
  }
  state.loading = false;
}

// 仅刷新 clients(含 idle_sec)+ status:用于轻量 polling,不动 healthz/不报错。
// 主要服务于"待命客户端最后心跳秒数"场景 —— 客户端心跳 backend 不会广播
// SSE event(touch existing 不发事件),所以必须靠前端轮询拉新值。
async function refreshSilently(): Promise<void> {
  try {
    const [status, clients] = await Promise.all([
      ServerApi.status(),
      ServerApi.clients(),
    ]);
    state.status = status;
    state.clients = clients.clients;
    state.passiveClients = clients.passive_clients ?? [];
    state.statusFetchedAtMs = Date.now();
  } catch (_) {
    /* 静默:正式 refresh 已负责报错 */
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
  refreshSilently,
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
