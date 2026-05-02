/**
 * 发现到的 Server 列表 store。
 *
 * 数据流（M-β.1）：
 *   1. 进入 DiscoveryView → useDiscovery 调 refresh() → GET /api/servers
 *   2. 同时订阅 SSE：server_discovered / server_lost 增量更新 servers
 *   3. 离开 view → useDiscovery 自动 stop SSE
 *
 * 排序契约（与后端 snapshot 对齐）：
 *   - mdns 在线优先
 *   - 同源内按 last_seen_at desc
 *
 * 后续：M-β.2 加 selectedServerId / connectingServerId / connectedServer。
 */
import { computed, reactive } from "vue";

import type { DiscoveredServer, ServerListResponse } from "../types/client";

import { ApiError } from "../api/client";
import { ClientApi } from "../api/client-api";

interface DiscoveryState {
  servers: DiscoveredServer[];
  available: boolean;          // 后端 mDNS 是否可用(zeroconf 装了)
  loading: boolean;
  error: string | null;
  lastFetchedAt: number | null;
}

const state = reactive<DiscoveryState>({
  servers: [],
  available: true,
  loading: false,
  error: null,
  lastFetchedAt: null,
});

function _sortServers(items: DiscoveredServer[]): DiscoveredServer[] {
  return [...items].sort((a, b) => {
    const orderA = a.source === "mdns" ? 0 : 1;
    const orderB = b.source === "mdns" ? 0 : 1;
    if (orderA !== orderB) return orderA - orderB;
    return b.last_seen_at - a.last_seen_at;
  });
}

async function refresh(): Promise<void> {
  state.loading = true;
  state.error = null;
  try {
    const resp: ServerListResponse = await ClientApi.servers();
    state.servers = _sortServers(resp.servers);
    state.available = resp.available;
    state.lastFetchedAt = Date.now();
  } catch (e) {
    state.error = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e);
  } finally {
    state.loading = false;
  }
}

/** SSE 收到 server_discovered 时调,做 upsert + 重排。 */
function upsertServer(srv: DiscoveredServer): void {
  const idx = state.servers.findIndex((s) => s.server_id === srv.server_id);
  if (idx >= 0) {
    state.servers[idx] = srv;
  } else {
    state.servers.push(srv);
  }
  state.servers = _sortServers(state.servers);
}

/** SSE 收到 server_lost 时调。处理策略：
 *  - 简化版：直接从 list 移除（M-β.1 不区分历史/在线）
 *  - 完整版（M-β.2 再实现）：把 source 改为 "history" + healthy=false,保留卡片但灰显
 */
function removeServer(serverId: string): void {
  state.servers = state.servers.filter((s) => s.server_id !== serverId);
}

export const discoveryStore = {
  state,
  refresh,
  upsertServer,
  removeServer,

  servers: computed(() => state.servers),
  available: computed(() => state.available),
  loading: computed(() => state.loading),
  error: computed(() => state.error),
  isEmpty: computed(() => state.servers.length === 0),
  onlineCount: computed(() => state.servers.filter((s) => s.source === "mdns").length),
  historyCount: computed(() => state.servers.filter((s) => s.source !== "mdns").length),
};
