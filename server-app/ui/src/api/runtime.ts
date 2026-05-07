/**
 * Tauri 运行时桥:从主进程拿动态分配的 api_port,并拼接成基础 URL。
 *
 * Fallback 优先级(浏览器纯 dev,pnpm --filter @conduit/server-ui dev 单跑):
 *   1. URL query 参数 ?api=19883      —— 调试时最方便,无需改 env
 *   2. VITE_API_BASE 环境变量          —— 编译期固定
 *   3. 硬编码 http://127.0.0.1:8090    —— mock 期最早占位
 *
 * 缓存策略（2026-05-06 调整）：
 *   不再缓存 cachedBase。dev 模式下 cargo watcher 热重启 binary 后 api_port 会变，
 *   旧缓存会让所有 fetch 指向已不存在的端口（症状：白屏 + Unable to reach proxy service）。
 *   每次 apiBase() 都 invoke get_runtime —— 是同进程 IPC，开销可忽略。
 *   release 模式下 binary 不会重启，invoke 也只是一次 mutex read，仍然便宜。
 */
import { invoke } from "@tauri-apps/api/core";
import type { AppRuntime } from "../types/proxy";

function readApiFromQuery(): string | null {
  if (typeof window === "undefined") return null;
  try {
    const sp = new URLSearchParams(window.location.search);
    const port = sp.get("api");
    if (!port) return null;
    if (!/^\d{2,5}$/.test(port)) return null;
    return `http://127.0.0.1:${port}`;
  } catch {
    return null;
  }
}

const ENV_FALLBACK_BASE = (import.meta.env.VITE_API_BASE as string | undefined) ??
  "http://127.0.0.1:8090";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function getRuntime(): Promise<AppRuntime | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<AppRuntime>("get_runtime");
  } catch (e) {
    console.warn("[runtime] get_runtime failed, fallback to mock", e);
    return null;
  }
}

export async function apiBase(): Promise<string> {
  const fromQuery = readApiFromQuery();
  if (fromQuery) return fromQuery;
  const rt = await getRuntime();
  return rt ? `http://127.0.0.1:${rt.api_port}` : ENV_FALLBACK_BASE;
}

export function clearRuntimeCache(): void {
  // 兼容旧调用方（已无内部缓存，留空作为 no-op，下个 apiBase() 自然就重新 invoke 了）。
}
