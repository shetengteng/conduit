/**
 * SSE 订阅 composable，连接 client-app 的 /api/events。
 *
 * 与 server-app 的 useEvents 同构,只是事件类型不同。
 *
 * 设计要点:
 *   - apiBase() 异步：首次需要从 Tauri runtime 拿到 sidecar API port
 *   - EventSource 浏览器自动重连(指数退避)
 *   - 各事件类型分发到 callback map,未配置的事件被静默忽略
 *   - 组件卸载时自动 stop
 */
import { onUnmounted, ref } from "vue";

import { apiBase } from "../api/runtime";
import type {
  ClientEventType,
  ConnectDonePayload,
  ConnectProgressPayload,
  ConnectionStateChangedPayload,
  DiscoveredServer,
  HeartbeatChangedPayload,
  ReadyPayload,
  RouteDecisionPayload,
  ServerLostPayload,
  TrafficTickPayload,
} from "../types/client";

export interface EventHandlers {
  ready?: (payload: ReadyPayload) => void;
  server_discovered?: (payload: DiscoveredServer) => void;
  server_lost?: (payload: ServerLostPayload) => void;
  // M-β.2:
  connect_progress?: (payload: ConnectProgressPayload) => void;
  connect_done?: (payload: ConnectDonePayload) => void;
  connection_state_changed?: (payload: ConnectionStateChangedPayload) => void;
  heartbeat_changed?: (payload: HeartbeatChangedPayload) => void;
  // M-γ:
  traffic_tick?: (payload: TrafficTickPayload) => void;
  route_decision?: (payload: RouteDecisionPayload) => void;
}

export interface UseEventsOptions {
  autoStart?: boolean;
  onError?: (e: Event) => void;
}

const ALL_EVENT_TYPES: ClientEventType[] = [
  "ready",
  "server_discovered",
  "server_lost",
  "connect_progress",
  "connect_done",
  "connection_state_changed",
  "heartbeat_changed",
  "traffic_tick",
  "route_decision",
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
