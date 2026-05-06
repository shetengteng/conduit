/**
 * Conduit 共享数据类型
 *
 * 与 Rust 后端的 serde 字段一一对齐（全部 snake_case），后端入口在
 * `server-app/src-tauri/src/proxy/control_api.rs`，类型定义在
 * `crates/conduit-core/src/types.rs`。
 *
 * 后端契约变化时此文件必须同步更新（控制 API 没有自动 codegen）。
 */

export type Proto = "http" | "socks5";

// ============================================================================
// /api/status, /api/clients (server-app)
// ============================================================================

export interface VpnStatus {
  available: boolean;
  iface: string | null;
  default_route_via_vpn: boolean;
}

export interface LanStatus {
  available: boolean;
  detail: string | null;
}

export interface MdnsStatus {
  enabled: boolean;
  /** 实际广播出去的 instance name（用户指定 --mdns-name 时为它，否则为系统短主机名） */
  name: string;
  service_type: string;
}

export interface ServerStatus {
  running: boolean;
  version: string;
  http_port: number;
  socks5_port: number;
  api_port: number;
  pac_url: string | null;
  mdns: MdnsStatus;
  vpn: VpnStatus;
  lan: LanStatus;
  clients_count: number;
  /** 通过 LAN HTTP /api/clients/heartbeat 上报的 client-app 数量(已链接但暂未传输流量) */
  passive_clients_count: number;
  uptime_sec: number;
  ready: boolean;
}

/** /api/clients 返回中的单个会话快照（也用于 SSE client_connected payload）。 */
export interface ClientSession {
  session_id: string;
  peer_ip: string;
  proto: Proto;
  target: string;
  since: number;
  last_seen: number;
  sent_bytes: number;
  recv_bytes: number;
}

/** /api/clients 中的 passive client（通过 LAN HTTP /api/clients/heartbeat 心跳登记）。 */
export interface PassiveClient {
  peer_ip: string;
  client_name: string;
  version: string;
  first_seen: number;
  last_seen: number;
  /** 距离最后一次心跳的秒数(已被 server 计算好) */
  idle_sec: number;
}

export interface ClientsResponse {
  count: number;
  clients: ClientSession[];
  /** v0.1 阶段新增:被动客户端列表(已链接但暂未传输流量) */
  passive_count: number;
  passive_clients: PassiveClient[];
}

// ============================================================================
// /api/traffic (server-app)
// ============================================================================

/** [ts, sent_bps, recv_bps] tuple as returned by the server. */
export type TrafficSamplePoint = [number, number, number];

export interface TrafficResponse {
  window_sec: number;
  now: number;
  series: Record<string, TrafficSamplePoint[]>;
}

// ============================================================================
// /healthz (server-app)
// ============================================================================

export interface HealthCheckEntry {
  name: string;
  ok: boolean;
  detail: string;
}

export interface HealthzResponse {
  ready: boolean;
  checks: HealthCheckEntry[];
  running: boolean;
  uptime_sec: number;
}

// ============================================================================
// /api/events SSE envelope (server-app)
// ============================================================================

export type ServerEventType =
  | "ready"
  | "client_connected"
  | "client_disconnected"
  | "passive_client_seen"
  | "passive_client_lost"
  | "traffic_tick"
  | "vpn_state_changed";

export interface PassiveClientSeenPayload {
  peer_ip: string;
  client_name: string;
  version: string;
  first_seen: number;
}

export interface PassiveClientLostPayload {
  peer_ip: string;
  client_name: string;
  duration_sec: number;
}

export interface ClientConnectedPayload {
  session_id: string;
  peer_ip: string;
  proto: Proto;
  target: string;
  since: number;
}

export interface ClientDisconnectedPayload {
  session_id: string;
  peer_ip: string;
  sent_bytes: number;
  recv_bytes: number;
  duration_sec: number;
}

export interface TrafficTickPayload {
  ts: number;
  per_peer: Record<string, { sent_bps: number; recv_bps: number }>;
}

export interface VpnStateChangedPayload {
  available: boolean;
  iface: string | null;
}

export type ServerEventPayload =
  | { type: "ready"; payload: { version: string } }
  | { type: "client_connected"; payload: ClientConnectedPayload }
  | { type: "client_disconnected"; payload: ClientDisconnectedPayload }
  | { type: "passive_client_seen"; payload: PassiveClientSeenPayload }
  | { type: "passive_client_lost"; payload: PassiveClientLostPayload }
  | { type: "traffic_tick"; payload: TrafficTickPayload }
  | { type: "vpn_state_changed"; payload: VpnStateChangedPayload };

// ============================================================================
// client-app (smart local proxy) — used by S5/S6, kept for forward-compat
// ============================================================================

export interface DiscoveredServer {
  server_id: string;
  name: string;
  host: string;
  port: number;
  pac_url: string;
  version: string;
  source: "mdns" | "history" | "manual";
  healthy: boolean;
  last_seen_at: number | null;
}

export type RouteDirection = "direct" | "proxy";

export type RouteSource =
  | "private_ip"
  | "pac_prefill"
  | "probe"
  | "self_heal"
  | "manual_override"
  | "global_fallback";

export interface RouteEntry {
  host: string;
  direction: RouteDirection;
  source: RouteSource;
  expires_at: number;
  hit_count: number;
  last_used_at: number;
}

// ============================================================================
// Tauri shell runtime (主进程注入到前端)
// ============================================================================

export type LifecyclePhase = "Booting" | "Ready" | "Failed" | "Stopped";

export interface AppRuntime {
  api_port: number;
  http_port: number;
  socks5_port: number;
  phase: LifecyclePhase;
  failure_reason: string | null;
  sidecar_pid: number | null;
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
