/**
 * Conduit Client 共享数据类型
 *
 * 与后端 Rust `client-app/src-tauri/src/proxy/control_api.rs` 的 JSON 响应严格 1:1 对齐（snake_case）。
 * 一旦后端契约变化，此文件必须同步更新。
 *
 * 设计原则：M-α 阶段只声明已有 API（healthz + Tauri runtime），
 * 后续 M-β/γ/δ 增量补 discovery / route / cache / diagnose 类型。
 */

// ============================================================================
// /healthz (client-app)
// ============================================================================

export interface HealthCheckEntry {
  name: string;
  ok: boolean;
  detail: string;
}

export interface HealthzResponse {
  ready: boolean;
  checks: HealthCheckEntry[];
  uptime_sec: number;
}

// ============================================================================
// Tauri shell runtime (主进程通过 invoke('get_runtime') 暴露)
// ============================================================================

export type LifecyclePhase = "Booting" | "Ready" | "Failed" | "Stopped";

export interface AppRuntime {
  api_port: number;
  socks_port: number;
  phase: LifecyclePhase;
  failure_reason: string | null;
  sidecar_pid: number | null;
}

// ============================================================================
// /api/servers (M-β.1)
// ============================================================================

/** 单个被发现到（或历史上见过）的 Conduit Server。
 *
 * 字段语义见 client-app/src-tauri/src/proxy/discoverer.rs · DiscoveredServer。
 */
export interface DiscoveredServer {
  server_id: string;          // name@host:port,跨 session 稳定
  name: string;
  host: string;
  port: number;               // HTTP proxy / PAC 端口
  socks: number;
  api: number;
  vpn: boolean;
  version: string;
  pac: string;                // PAC URL 相对路径,如 "/proxy.pac"
  pac_url: string;            // 完整 URL,UI 可直接展示
  source: "mdns" | "history" | "manual";
  last_seen_at: number;       // epoch seconds
  healthy: boolean;
}

export interface ServerListResponse {
  count: number;
  available: boolean;         // mDNS 是否可用(zeroconf 装了)
  servers: DiscoveredServer[];
}

// ============================================================================
// /api/events SSE
// ============================================================================

export type ClientEventType =
  | "ready"
  | "server_discovered"
  | "server_lost"
  // M-β.2:
  | "connect_progress"
  | "connect_done"
  | "connection_state_changed"
  | "heartbeat_changed"
  // M-γ:
  | "traffic_tick"
  | "route_decision";

export interface ReadyPayload {
  version: string;
}

export interface ServerLostPayload {
  server_id: string;
  name: string;
}

// ============================================================================
// /api/connect/{id} + /api/disconnect + /api/connection (M-β.2)
// ============================================================================

export type ConnectionState =
  | "idle"
  | "connecting"
  | "connected"
  | "disconnecting"
  | "failed";

export type ConnectStepKey =
  | "probe"
  | "fetch_pac"
  | "prefill_cache"
  | "switch_endpoint"
  | "start_heartbeat";

export type ConnectStepStatus = "running" | "ok" | "failed";

export interface ConnectProgressPayload {
  step: number;        // 1..5
  total: number;       // 5
  key: ConnectStepKey;
  label: string;       // 中文标签 (后端给好,UI 直接 render)
  status: ConnectStepStatus;
  detail: string;
  server_id: string;
}

export type HeartbeatTone = "green" | "yellow" | "red";

export interface HeartbeatChangedPayload {
  tone: HeartbeatTone;
  consecutive_failures: number;
  recovered: boolean;
  last_error: string | null;
  host: string;
}

export interface ConnectionStateChangedPayload {
  state: ConnectionState;
  server_id?: string;
  error?: string;
}

export interface ConnectedServerSummary {
  server_id: string;
  name: string;
  host: string;
  port: number;
  socks: number;
  api: number;
  vpn: boolean;
  version: string;
}

export interface ConnectionSnapshot {
  ok: boolean;
  state: ConnectionState;
  server: ConnectedServerSummary | null;
  connected_since: number | null;
  system_proxy_active: boolean;
  heartbeat: {
    tone: HeartbeatTone;
    consecutive_failures: number;
    last_check_at: number;
    last_error: string | null;
  } | null;
  last_error: string | null;
}

/** connect_done 事件 payload —— 与 ConnectionSnapshot 同结构 + server_id 顶层。 */
export interface ConnectDonePayload extends ConnectionSnapshot {
  server_id: string;
}

// ============================================================================
// M-γ:路由缓存 / 流量
// ============================================================================

export type RouteDirection = "direct" | "proxy";
export type RouteSource =
  | "pac"
  | "probe"
  | "manual"
  | "cache"
  | "pattern"
  | "private_ip"
  | "global_override"
  | "self_heal";

export interface RouteCacheEntry {
  host: string;
  direction: RouteDirection;
  source: RouteSource;
  hit_count: number;
  expires_at: string;        // ISO 8601
  last_used: string;         // ISO 8601
  ttl_remaining_sec: number;
}

export interface RouteCacheStats {
  total: number;
  direct_count: number;
  proxy_count: number;
  expired_count: number;
  by_source: Record<string, number>;
  hits: number;
  misses: number;
  evictions: number;
}

export interface RouteCacheResponse {
  count: number;
  total: number;
  stats: RouteCacheStats;
  entries: RouteCacheEntry[];
}

export interface TrafficTickPayload {
  ts: number;
  uplink_bytes: number;        // 这一秒的上行
  downlink_bytes: number;      // 这一秒的下行
  total_uplink: number;
  total_downlink: number;
}

export interface TrafficSnapshot extends TrafficTickPayload {}

export interface RouteDecisionPayload {
  host: string;
  port: number;
  direction: RouteDirection;
  source: RouteSource;
  hit_count: number;
}

// ============================================================================
// /api/diagnose (M-δ)
// ============================================================================

export type DiagnoseCheckKey =
  | "sidecar"
  | "mdns"
  | "server_reach"
  | "pac"
  | "system_proxy";

export interface DiagnoseCheck {
  key: DiagnoseCheckKey;
  label: string;
  ok: boolean;
  detail: string;
  remediation: string | null;
}

export interface DiagnoseResponse {
  ok: boolean;
  checks: DiagnoseCheck[];
  checked_at: number;
}

// ============================================================================
// 统一错误响应（所有 4xx/5xx）
// ============================================================================

export interface ApiErrorBody {
  code: string;
  message: string;
}

export interface ApiErrorEnvelope {
  error: ApiErrorBody;
}
