/**
 * Tauri 运行时桥：从主进程拿动态分配的 api_port，并拼接成基础 URL。
 *
 * 浏览器纯 dev（pnpm --filter @conduit/client-ui dev 单跑）三级 fallback：
 *   1. URL query: ?api_port=NNNN  (e.g. http://localhost:1421/?api_port=19121)
 *   2. localStorage: "conduit-api-base"  (e.g. http://127.0.0.1:19121)
 *   3. VITE_API_BASE 环境变量
 *   4. 硬编码 8091
 *
 * 与 server-app 的 runtime.ts 唯一差异：fallback port 8091（避免与 server 8090 冲突）。
 */
import { invoke } from "@tauri-apps/api/core";
import type { AppRuntime } from "../types/client";

function _resolveDevFallback(): string {
  if (typeof window !== "undefined") {
    const params = new URLSearchParams(window.location.search);
    const p = params.get("api_port");
    if (p) return `http://127.0.0.1:${p}`;
    try {
      const ls = window.localStorage?.getItem("conduit-api-base");
      if (ls) return ls;
    } catch (_) {
      // localStorage 不可用，忽略
    }
  }
  return (import.meta.env.VITE_API_BASE as string | undefined) ??
    "http://127.0.0.1:8091";
}

const FALLBACK_BASE = _resolveDevFallback();

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
