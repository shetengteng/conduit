/**
 * 监听 Tauri 主进程 emit 的 boot 生命周期事件。
 *
 * 启动流程：
 *   1. Webview 一启动 → 主进程 emit "boot:phase" "booting"
 *   2. sidecar healthz 通过 → emit "boot:phase" "ready"
 *   3. 9s 超时 / spawn 失败 → emit "boot:phase" "failed" + "boot:error" <reason>
 *
 * 竞态修正（2026-05-01）：
 *   主进程的 boot_sequence 经常在 webview 还没 mount 完之前就 emit "ready"，
 *   导致 listener 错过事件、UI 永远卡在 BootScreen。
 *   解决方案：mount 时先主动 invoke('get_runtime') 拉一次当前 phase 兜底，
 *   再注册 listener 接管后续事件。这样无论事件早到还是晚到都不丢。
 *
 * 浏览器降级（无 Tauri 上下文）：
 *   直接把 phase 置为 "Ready"，让前端可以独立 mock 调试。
 */
import { onMounted, onUnmounted } from "vue";

import { uiStore } from "../stores/ui";
import { getRuntime } from "../api/runtime";
import type { LifecyclePhase } from "../types/client";

interface TauriEventApi {
  listen: (
    event: string,
    handler: (e: { payload: unknown }) => void,
  ) => Promise<() => void>;
}

async function getTauriEvent(): Promise<TauriEventApi | null> {
  if (typeof window === "undefined") return null;
  if (!("__TAURI_INTERNALS__" in window)) return null;
  try {
    return (await import("@tauri-apps/api/event")) as unknown as TauriEventApi;
  } catch (_) {
    return null;
  }
}

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function normalisePhase(raw: unknown): LifecyclePhase | null {
  const s = String(raw ?? "").toLowerCase();
  if (s === "ready") return "Ready";
  if (s === "failed") return "Failed";
  if (s === "booting") return "Booting";
  if (s === "stopped") return "Stopped";
  return null;
}

/**
 * 处理 phase 状态更新 —— 仅允许"前进"，不接受倒退。
 *
 * 时间顺序：Booting (初始) → Ready (sidecar healthz 通过) → Stopped (用户停止)
 *                          ↘ Failed (启动失败)
 *
 * 之前的 bug：fallback 兜底 `getRuntime()` 在 listener 已经收到 Ready 后
 * 又把 state 倒退回 Booting (因为 invoke 完成时 boot_sequence 的 set_phase 还没跑)，
 * 导致 UI 永远卡在 BootScreen。这里加 ordering 校验防止倒退。
 */
function applyPhase(next: LifecyclePhase, reason: string | null = null): void {
  const current = uiStore.state.bootPhase;
  if (next === "Failed") {
    uiStore.setBootPhase("Failed", reason ?? uiStore.state.bootError);
    return;
  }
  if (next === "Booting" && current !== "Booting") return;
  uiStore.setBootPhase(next, reason);
}

export function useBootPhase() {
  const unlisteners: Array<() => void> = [];

  onMounted(async () => {
    if (!isTauri()) {
      applyPhase("Ready");
      return;
    }

    const ev = await getTauriEvent();
    if (!ev) {
      applyPhase("Ready");
      return;
    }

    unlisteners.push(
      await ev.listen("boot:phase", (e) => {
        const next = normalisePhase(e.payload);
        if (!next) return;
        applyPhase(next);
      }),
      await ev.listen("boot:error", (e) => {
        applyPhase("Failed", String(e.payload ?? "unknown"));
      }),
    );

    try {
      const rt = await getRuntime();
      if (!rt) return;
      applyPhase(rt.phase, rt.failure_reason ?? null);
    } catch (err) {
      console.warn("[useBootPhase] get_runtime failed", err);
    }
  });

  onUnmounted(() => {
    for (const off of unlisteners) off();
    unlisteners.length = 0;
  });
}
