/**
 * 路由缓存表 store —— M-γ。
 *
 * 数据流:
 *   1. ConnectedView mount 时 refresh() 拉一次 baseline
 *   2. SSE route_decision 实时增量:已存在 host 的 hit_count + 1 / direction 变更;
 *      不在 store 但 source != "private_ip"|"global_override" 的新增进表
 *   3. 用户点"清空" → DELETE /api/cache + 本地 reset()
 *
 * UI 侧排序:last_used desc(后端已排好);UI 默认显示 200 条上限。
 */
import { computed, reactive } from "vue";

import type {
  RouteCacheEntry,
  RouteCacheResponse,
  RouteCacheStats,
  RouteDecisionPayload,
} from "../types/client";

import { ClientApi } from "../api/client-api";

const MAX_LOCAL = 500;

interface CacheState {
  entries: RouteCacheEntry[];
  stats: RouteCacheStats | null;
  loading: boolean;
  error: string | null;
}

const state = reactive<CacheState>({
  entries: [],
  stats: null,
  loading: false,
  error: null,
});

async function refresh(): Promise<void> {
  state.loading = true;
  state.error = null;
  try {
    const data: RouteCacheResponse = await ClientApi.cache({ limit: 200 });
    state.entries = data.entries;
    state.stats = data.stats;
  } catch (e) {
    state.error = e instanceof Error ? e.message : String(e);
  } finally {
    state.loading = false;
  }
}

async function flush(): Promise<void> {
  await ClientApi.flushCache();
  state.entries = [];
  if (state.stats) state.stats = { ...state.stats, total: 0, direct_count: 0, proxy_count: 0 };
}

function onRouteDecision(payload: RouteDecisionPayload): void {
  // private_ip / global_override 不进缓存表(后端 RouteCache 也不写)
  if (payload.source === "private_ip" || payload.source === "global_override") return;

  const idx = state.entries.findIndex((e) => e.host === payload.host);
  const nowIso = new Date().toISOString();
  if (idx >= 0) {
    const e = state.entries[idx];
    state.entries.splice(idx, 1);
    state.entries.unshift({
      ...e,
      direction: payload.direction,
      source: payload.source,
      hit_count: payload.hit_count || e.hit_count + 1,
      last_used: nowIso,
    });
  } else {
    state.entries.unshift({
      host: payload.host,
      direction: payload.direction,
      source: payload.source,
      hit_count: payload.hit_count || 1,
      expires_at: nowIso,
      last_used: nowIso,
      ttl_remaining_sec: 300,
    });
    if (state.entries.length > MAX_LOCAL) state.entries.length = MAX_LOCAL;
  }
}

function reset(): void {
  state.entries = [];
  state.stats = null;
  state.error = null;
}

export const cacheStore = {
  state,
  refresh,
  flush,
  onRouteDecision,
  reset,
  entries: computed(() => state.entries),
  stats: computed(() => state.stats),
  loading: computed(() => state.loading),
  error: computed(() => state.error),
};
