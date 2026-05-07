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

// i18n key — UI 渲染时 t('connecting.step.<value>')。
// 这里只存 key,不存文案,由 vue-i18n 在渲染层翻译。
const STEP_LABEL_KEYS: Record<ConnectStepKey, string> = {
  probe: "connecting.step.probe",
  fetch_pac: "connecting.step.fetchPac",
  prefill_cache: "connecting.step.prefillCache",
  switch_endpoint: "connecting.step.switchEndpoint",
  start_heartbeat: "connecting.step.startHeartbeat",
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
  /**
   * 是否有 connect/disconnect HTTP 请求正在飞。**与 `state` 不同**:
   * - `state` 反映后端连接状态机(idle/connecting/connected/...)
   * - `inFlight` 反映 UI 是否正在等本次 connect/disconnect HTTP 响应
   *
   * 用来防止用户连续多次点击 connect/disconnect 按钮(后端尽管已修了死锁
   * 路径,但乐观地避免并发请求依然是好做法)。组件应该用 `isBusy` 来禁用
   * 按钮。
   */
  inFlight: boolean;
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
  inFlight: false,
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

/**
 * 最小 inFlight 显示时间(毫秒)。后端可能很快返回(尤其是幂等命中分支
 * 或本地 cache 命中),如果不强制最小显示,UI 上的 spinner 会一闪而过
 * 用户根本看不见。500ms 足够人眼感知"我点了按钮 → 系统在响应"。
 */
const MIN_INFLIGHT_MS = 500;

async function withMinDuration<T>(p: Promise<T>): Promise<T> {
  const t0 = Date.now();
  try {
    return await p;
  } finally {
    const elapsed = Date.now() - t0;
    if (elapsed < MIN_INFLIGHT_MS) {
      await new Promise((r) => setTimeout(r, MIN_INFLIGHT_MS - elapsed));
    }
  }
}

async function connectTo(serverId: string): Promise<void> {
  // 防御:同一时刻只允许一次 connect/disconnect 飞。直接 return 比抛错友好,
  // 用户不会看到红屏,只会看到按钮"灰着不响应"。
  if (state.inFlight) {
    return;
  }
  state.inFlight = true;
  state.lastError = null;
  state.pendingServerId = serverId;
  state.progress = _emptyProgress();
  state.state = "connecting";   // 乐观置位,SSE 会确认
  try {
    const snap = await withMinDuration(ClientApi.connect(serverId));
    state.snapshot = snap;
    state.state = snap.state;
    state.lastError = snap.last_error;
  } catch (e) {
    state.lastError = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
    state.state = "failed";
    throw e;
  } finally {
    state.inFlight = false;
  }
}

async function disconnect(): Promise<void> {
  if (state.inFlight) {
    return;
  }
  state.inFlight = true;
  state.lastError = null;
  state.state = "disconnecting";
  try {
    await withMinDuration(ClientApi.disconnect());
    state.state = "idle";
    state.snapshot = null;
    state.pendingServerId = null;
    state.progress = _emptyProgress();
  } catch (e) {
    state.lastError = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
    await refresh();   // 回到真相
    throw e;
  } finally {
    state.inFlight = false;
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
  STEP_LABEL_KEYS,

  // computed accessors
  connectionState: computed(() => state.state),
  isConnecting: computed(() => state.state === "connecting"),
  isConnected: computed(() => state.state === "connected"),
  isFailed: computed(() => state.state === "failed"),
  /**
   * 综合判断 UI 是否处于"正在响应连接/断开请求"状态:
   * - inFlight=true:本次 connect/disconnect HTTP 请求未返回
   * - state=connecting/disconnecting:后端报告正在过渡
   * 任一为真都禁用按钮,防止重复点击。
   */
  isBusy: computed(
    () => state.inFlight || state.state === "connecting" || state.state === "disconnecting",
  ),
  /**
   * 是否正在执行"连接"流程(用于 ConnectedView 决定渲染 ConnectingProgress
   * 还是已连接卡):
   * - state=connecting:经典路径,后端确认 connecting
   * - inFlight && pendingServerId:用户刚点连接按钮,inFlight 已置位但
   *   后端 SSE / HTTP 还没把 state 翻成 connecting
   * 关键是 disconnecting 不算 — 断开时不该显示"5 步连接进度页"。
   */
  isConnectingOrPending: computed(
    () =>
      state.state === "connecting" ||
      (state.inFlight && state.pendingServerId !== null),
  ),
  connectedServer: computed(() => state.snapshot?.server ?? null),
  connectedSince: computed(() => state.snapshot?.connected_since ?? null),
  systemProxyActive: computed(() => Boolean(state.snapshot?.system_proxy_active)),
  heartbeatTone: computed(() => state.snapshot?.heartbeat?.tone ?? null),
  pendingServerId: computed(() => state.pendingServerId),
  lastError: computed(() => state.lastError),
};
