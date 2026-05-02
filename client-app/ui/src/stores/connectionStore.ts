/**
 * 连接状态机 store —— M-β.2。
 *
 * 真相来源:后端 `GET /api/connection`。我们额外维护:
 *   - progress: ConnectProgressPayload[]  （5 步过程,UI 渲染 stepper 用）
 *   - heartbeat: HeartbeatChangedPayload  （连接后实时更新的心跳）
 *
 * 同步策略:
 *   - mount 时 refresh() 拉一次校准状态
 *   - SSE 推 connection_state_changed → 调 onStateChange
 *   - SSE 推 connect_progress → 调 onProgress
 *   - SSE 推 connect_done → 调 onConnectDone
 *   - SSE 推 heartbeat_changed → 调 onHeartbeat
 *
 * 用户操作:
 *   - connectTo(serverId) → POST /api/connect/{id};乐观地把 state 改为 connecting + 清 progress
 *   - disconnect() → POST /api/disconnect
 */
import { computed, reactive } from "vue";

import type {
  ConnectDonePayload,
  ConnectionSnapshot,
  ConnectionState,
  ConnectionStateChangedPayload,
  ConnectProgressPayload,
  ConnectStepKey,
  HeartbeatChangedPayload,
} from "../types/client";

import { ApiError } from "../api/client";
import { ClientApi } from "../api/client-api";

const STEP_ORDER: ConnectStepKey[] = [
  "probe",
  "fetch_pac",
  "prefill_cache",
  "switch_endpoint",
  "start_heartbeat",
];

const STEP_LABELS: Record<ConnectStepKey, string> = {
  probe: "可达性检查",
  fetch_pac: "拉取 PAC",
  prefill_cache: "解析 PAC 预填路由",
  switch_endpoint: "切换上游 server",
  start_heartbeat: "启动心跳与系统代理",
};

interface ConnectionStateRefs {
  state: ConnectionState;
  snapshot: ConnectionSnapshot | null;
  // 5 步状态:undefined=未开始,'running'/'ok'/'failed'
  progress: Record<ConnectStepKey, {
    status: "running" | "ok" | "failed";
    detail: string;
  } | undefined>;
  pendingServerId: string | null;   // connecting 期间显示用
  lastError: string | null;
  loading: boolean;
}

const _emptyProgress = (): ConnectionStateRefs["progress"] => ({
  probe: undefined,
  fetch_pac: undefined,
  prefill_cache: undefined,
  switch_endpoint: undefined,
  start_heartbeat: undefined,
});

const state = reactive<ConnectionStateRefs>({
  state: "idle",
  snapshot: null,
  progress: _emptyProgress(),
  pendingServerId: null,
  lastError: null,
  loading: false,
});

async function refresh(): Promise<void> {
  state.loading = true;
  try {
    const snap = await ClientApi.connection();
    state.snapshot = snap;
    state.state = snap.state;
    state.lastError = snap.last_error;
  } catch (e) {
    state.lastError = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
  } finally {
    state.loading = false;
  }
}

async function connectTo(serverId: string): Promise<void> {
  state.lastError = null;
  state.pendingServerId = serverId;
  state.progress = _emptyProgress();
  state.state = "connecting";   // 乐观置位,SSE 会确认
  try {
    const snap = await ClientApi.connect(serverId);
    state.snapshot = snap;
    state.state = snap.state;
    state.lastError = snap.last_error;
  } catch (e) {
    state.lastError = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
    state.state = "failed";
    throw e;
  }
}

async function disconnect(): Promise<void> {
  state.lastError = null;
  state.state = "disconnecting";
  try {
    await ClientApi.disconnect();
    state.state = "idle";
    state.snapshot = null;
    state.pendingServerId = null;
    state.progress = _emptyProgress();
  } catch (e) {
    state.lastError = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
    await refresh();   // 回到真相
    throw e;
  }
}

// ----- SSE 入口 -----

function onProgress(payload: ConnectProgressPayload): void {
  state.progress[payload.key] = {
    status: payload.status,
    detail: payload.detail,
  };
  if (payload.status === "failed") {
    state.lastError = payload.detail || `步骤 ${payload.step} 失败`;
  }
}

function onConnectDone(payload: ConnectDonePayload): void {
  state.snapshot = payload;
  state.state = payload.state;
  state.lastError = payload.last_error;
}

function onStateChange(payload: ConnectionStateChangedPayload): void {
  state.state = payload.state;
  if (payload.error) state.lastError = payload.error;
  if (payload.state === "idle") {
    state.snapshot = null;
    state.progress = _emptyProgress();
    state.pendingServerId = null;
  }
}

function onHeartbeat(payload: HeartbeatChangedPayload): void {
  if (state.snapshot) {
    state.snapshot.heartbeat = {
      tone: payload.tone,
      consecutive_failures: payload.consecutive_failures,
      last_check_at: Date.now() / 1000,
      last_error: payload.last_error,
    };
  }
}

export const connectionStore = {
  state,
  refresh,
  connectTo,
  disconnect,

  // SSE 回调
  onProgress,
  onConnectDone,
  onStateChange,
  onHeartbeat,

  // 元数据
  STEP_ORDER,
  STEP_LABELS,

  // computed accessors
  connectionState: computed(() => state.state),
  isConnecting: computed(() => state.state === "connecting"),
  isConnected: computed(() => state.state === "connected"),
  isFailed: computed(() => state.state === "failed"),
  connectedServer: computed(() => state.snapshot?.server ?? null),
  connectedSince: computed(() => state.snapshot?.connected_since ?? null),
  systemProxyActive: computed(() => Boolean(state.snapshot?.system_proxy_active)),
  heartbeatTone: computed(() => state.snapshot?.heartbeat?.tone ?? null),
  pendingServerId: computed(() => state.pendingServerId),
  lastError: computed(() => state.lastError),
};
