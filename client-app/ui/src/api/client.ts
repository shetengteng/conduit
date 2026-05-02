/**
 * fetch 封装：统一错误格式 + JSON 解析。
 *
 * 后端错误响应：{"error": {"code": "...", "message": "..."}}
 */
import type { ApiErrorBody } from "../types/client";

import { apiBase } from "./runtime";

export class ApiError extends Error {
  code: string;
  status: number;
  constructor(code: string, message: string, status: number) {
    super(message);
    this.code = code;
    this.status = status;
    this.name = "ApiError";
  }
}

interface RequestOptions {
  signal?: AbortSignal;
  body?: unknown;
}

export async function apiGet<T>(path: string, opts?: RequestOptions): Promise<T> {
  return apiRequest<T>("GET", path, opts);
}

export async function apiPost<T>(path: string, opts?: RequestOptions): Promise<T> {
  return apiRequest<T>("POST", path, opts);
}

export async function apiDelete<T>(path: string, opts?: RequestOptions): Promise<T> {
  return apiRequest<T>("DELETE", path, opts);
}

async function apiRequest<T>(
  method: "GET" | "POST" | "DELETE",
  path: string,
  opts: RequestOptions = {},
): Promise<T> {
  const base = await apiBase();
  const url = path.startsWith("http") ? path : `${base}${path}`;
  const init: RequestInit = {
    method,
    headers: { "Accept": "application/json" },
    signal: opts.signal,
  };
  if (opts.body !== undefined) {
    init.headers = { ...init.headers, "Content-Type": "application/json" };
    init.body = JSON.stringify(opts.body);
  }
  const resp = await fetch(url, init);
  const text = await resp.text();
  if (!resp.ok) {
    let code = `HTTP_${resp.status}`;
    let message = resp.statusText || text;
    if (text) {
      try {
        const body = JSON.parse(text) as { error?: ApiErrorBody };
        if (body?.error) {
          code = body.error.code;
          message = body.error.message;
        }
      } catch (_) {
        /* ignore non-JSON */
      }
    }
    throw new ApiError(code, message, resp.status);
  }
  if (!text) return undefined as T;
  return JSON.parse(text) as T;
}
