/**
 * useDiscovery —— 在视图 mount 时刷新 + 订阅 SSE，unmount 时自动断开。
 *
 * 设计要点：
 * - 进入 DiscoveryView 立刻 refresh 一次（拉历史 + 当前在线）
 * - 同时订阅 SSE 接收实时增量（server_discovered / server_lost）
 * - 提供 manualRefresh 给页面右上角"重新扫描"按钮
 * - 离开页面时自动 stop SSE（onUnmounted 由 useEvents 内部接管）
 */
import { onMounted, onUnmounted } from "vue";

import { useEvents } from "./useEvents";
import { discoveryStore } from "../stores/discoveryStore";

export function useDiscovery() {
  let stopFn: (() => void) | null = null;

  async function manualRefresh(): Promise<void> {
    await discoveryStore.refresh();
  }

  onMounted(async () => {
    await discoveryStore.refresh();
    const evt = useEvents(
      {
        server_discovered: (srv) => discoveryStore.upsertServer(srv),
        server_lost: (payload) => discoveryStore.removeServer(payload.server_id),
      },
      { autoStart: true },
    );
    stopFn = evt.stop;
  });

  onUnmounted(() => {
    stopFn?.();
  });

  return {
    servers: discoveryStore.servers,
    available: discoveryStore.available,
    loading: discoveryStore.loading,
    error: discoveryStore.error,
    isEmpty: discoveryStore.isEmpty,
    onlineCount: discoveryStore.onlineCount,
    historyCount: discoveryStore.historyCount,
    manualRefresh,
  };
}
