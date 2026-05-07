/**
 * server-app 控制 API 的强类型封装。
 *
 * 路径与 server-app/core/api/{status,traffic,admin,healthz}.py 对齐。
 */
import type {
  ClientsResponse,
  HealthzResponse,
  RecentSessionsResponse,
  ServerStatus,
  TrafficResponse,
} from "../types/proxy";

import { apiGet, apiPost } from "./client";

export const ServerApi = {
  status: () => apiGet<ServerStatus>("/api/status"),
  clients: () => apiGet<ClientsResponse>("/api/clients"),
  recentSessions: () => apiGet<RecentSessionsResponse>("/api/sessions/recent"),
  traffic: (window = 60, peer?: string) => {
    const qs = new URLSearchParams({ window: String(window) });
    if (peer) qs.set("peer", peer);
    return apiGet<TrafficResponse>(`/api/traffic?${qs.toString()}`);
  },
  healthz: () => apiGet<HealthzResponse>("/healthz"),
  adminStop: () => apiPost<{ ok: true }>("/api/admin/stop"),
};
