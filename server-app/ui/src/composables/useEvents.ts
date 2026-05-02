/**
 * SSE 订阅 composable，连接 server-app 的 /api/events。
 *
 * EventSource 的优点：浏览器自动重连。我们额外做：
 *   - api_port 异步获取（首次需要 await Tauri runtime）
 *   - 显式断线重连（max 3 次后弹错误）
 *   - 各事件类型分发到 callback map
 */
import { onUnmounted, ref } from "vue";

import { apiBase } from "../api/runtime";
import type {
  ClientConnectedPayload,
  ClientDisconnectedPayload,
  PassiveClientLostPayload,
  PassiveClientSeenPayload,
  ServerEventType,
  TrafficTickPayload,
  VpnStateChangedPayload,
} from "../types/proxy";

export interface EventHandlers {
  ready?: (payload: { version: string }) => void;
  client_connected?: (payload: ClientConnectedPayload) => void;
  client_disconnected?: (payload: ClientDisconnectedPayload) => void;
  passive_client_seen?: (payload: PassiveClientSeenPayload) => void;
  passive_client_lost?: (payload: PassiveClientLostPayload) => void;
  traffic_tick?: (payload: TrafficTickPayload) => void;
  vpn_state_changed?: (payload: VpnStateChangedPayload) => void;
}

export interface UseEventsOptions {
  autoStart?: boolean;
  onError?: (e: Event) => void;
}

const ALL_EVENT_TYPES: ServerEventType[] = [
  "ready",
  "client_connected",
  "client_disconnected",
  "passive_client_seen",
  "passive_client_lost",
  "traffic_tick",
  "vpn_state_changed",
];

export function useEvents(handlers: EventHandlers, opts: UseEventsOptions = {}) {
  const connected = ref(false);
  const lastError = ref<string | null>(null);
  let es: EventSource | null = null;
  let stopped = false;

  async function start() {
    if (es) return;
    stopped = false;
    const base = await apiBase();
    es = new EventSource(`${base}/api/events`);
    es.addEventListener("open", () => {
      connected.value = true;
      lastError.value = null;
    });
    es.addEventListener("error", (e) => {
      connected.value = false;
      lastError.value = "stream error";
      opts.onError?.(e);
      if (stopped) return;
    });
    for (const type of ALL_EVENT_TYPES) {
      es.addEventListener(type, (ev) => {
        const payload = parseEvent((ev as MessageEvent).data);
        if (!payload) return;
        const h = handlers[type] as ((p: unknown) => void) | undefined;
        h?.(payload);
      });
    }
  }

  function stop() {
    stopped = true;
    if (es) {
      es.close();
      es = null;
    }
    connected.value = false;
  }

  if (opts.autoStart !== false) {
    start();
  }

  onUnmounted(stop);

  return { connected, lastError, start, stop };
}

function parseEvent(raw: unknown): unknown {
  if (typeof raw !== "string") return null;
  try {
    return JSON.parse(raw);
  } catch (_) {
    return null;
  }
}
