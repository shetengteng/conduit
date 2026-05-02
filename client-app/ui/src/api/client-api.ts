/**
 * client-app 控制 API 的强类型封装。
 *
 * M-α:    healthz
 * M-β.1:  servers (mDNS 发现结果)
 * M-β.2:  connect / disconnect / connection
 * 后续 M-γ/δ 增量补 route / cache / diagnose。
 *
 * 路径与 client-app/core/api/*.py 严格对齐。
 */
import type {
  ConnectionSnapshot,
  HealthzResponse,
  RouteCacheResponse,
  ServerListResponse,
  TrafficSnapshot,
} from "../types/client";

import { apiDelete, apiGet, apiPost } from "./client";

export const ClientApi = {
  healthz: () => apiGet<HealthzResponse>("/healthz"),
  servers: () => apiGet<ServerListResponse>("/api/servers"),
  connection: () => apiGet<ConnectionSnapshot>("/api/connection"),
  connect: (serverId: string) =>
    apiPost<ConnectionSnapshot>(`/api/connect/${encodeURIComponent(serverId)}`),
  disconnect: () => apiPost<{ ok: boolean; state: string }>("/api/disconnect"),
  // M-γ
  cache: (params?: { direction?: "direct" | "proxy"; source?: string; limit?: number }) => {
    const qs = new URLSearchParams();
    if (params?.direction) qs.set("direction", params.direction);
    if (params?.source) qs.set("source", params.source);
    if (params?.limit) qs.set("limit", String(params.limit));
    const suffix = qs.toString() ? `?${qs}` : "";
    return apiGet<RouteCacheResponse>(`/api/cache${suffix}`);
  },
  flushCache: () => apiDelete<{ ok: boolean; removed: number }>("/api/cache"),
  traffic: () => apiGet<TrafficSnapshot>("/api/traffic"),
};
