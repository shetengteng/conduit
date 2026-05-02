/**
 * Tauri 运行时桥：从主进程拿动态分配的 api_port，并拼接成基础 URL。
 *
 * 浏览器纯 dev（pnpm --filter @conduit/server-ui dev 单跑）会 fallback 到
 * VITE_API_BASE 或硬编码 8090，方便前端 mock 期独立调试。
 */
import { invoke } from "@tauri-apps/api/core";
import type { AppRuntime } from "../types/proxy";

const FALLBACK_BASE = (import.meta.env.VITE_API_BASE as string | undefined) ??
  "http://127.0.0.1:8090";

let cached: AppRuntime | null = null;
let cachedBase: string | null = null;

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function getRuntime(): Promise<AppRuntime | null> {
  if (cached) return cached;
  if (!isTauri()) return null;
  try {
    cached = await invoke<AppRuntime>("get_runtime");
    return cached;
  } catch (e) {
    console.warn("[runtime] get_runtime failed, fallback to mock", e);
    return null;
  }
}

export async function apiBase(): Promise<string> {
  if (cachedBase) return cachedBase;
  const rt = await getRuntime();
  cachedBase = rt ? `http://127.0.0.1:${rt.api_port}` : FALLBACK_BASE;
  return cachedBase;
}

export function clearRuntimeCache(): void {
  cached = null;
  cachedBase = null;
}
