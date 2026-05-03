/**
 * Tauri 运行时桥:从主进程拿动态分配的 api_port,并拼接成基础 URL。
 *
 * Fallback 优先级(浏览器纯 dev,pnpm --filter @conduit/server-ui dev 单跑):
 *   1. URL query 参数 ?api=19883      —— 调试时最方便,无需改 env
 *   2. VITE_API_BASE 环境变量          —— 编译期固定
 *   3. 硬编码 http://127.0.0.1:8090    —— mock 期最早占位
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
  // ?api=xxx 永远优先,不缓存 —— 方便浏览器 dev 切端口立即生效
  const fromQuery = readApiFromQuery();
  if (fromQuery) return fromQuery;
  if (cachedBase) return cachedBase;
  const rt = await getRuntime();
  cachedBase = rt ? `http://127.0.0.1:${rt.api_port}` : ENV_FALLBACK_BASE;
  return cachedBase;
}

export function clearRuntimeCache(): void {
  cached = null;
  cachedBase = null;
}
