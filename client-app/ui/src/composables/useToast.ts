/**
 * 全局 Toast 通知系统。
 *
 * 用法：
 *   import { useToast } from "@/composables/useToast";
 *   const toast = useToast();
 *   toast.success("已复制到剪贴板");
 *   toast.error("启动失败", { detail: "address already in use" });
 *
 * - reactive 数组挂在模块单例上，所有调用方共享
 * - 默认 duration 3000ms，error 默认 5000ms
 * - 由 <ToastHost /> 单一组件渲染（挂在 App.vue 根）
 */
import { reactive } from "vue";

export type ToastTone = "success" | "error" | "warn" | "info";

export interface ToastOptions {
  duration?: number;
  detail?: string;
}

export interface ToastItem {
  id: number;
  tone: ToastTone;
  title: string;
  detail?: string;
  duration: number;
  createdAt: number;
}

const items = reactive<ToastItem[]>([]);
let nextId = 1;

function push(tone: ToastTone, title: string, opts?: ToastOptions): number {
  const defaultDur = tone === "error" ? 5000 : 3000;
  const item: ToastItem = {
    id: nextId++,
    tone,
    title,
    detail: opts?.detail,
    duration: opts?.duration ?? defaultDur,
    createdAt: Date.now(),
  };
  items.push(item);
  if (item.duration > 0) {
    window.setTimeout(() => dismiss(item.id), item.duration);
  }
  return item.id;
}

function dismiss(id: number): void {
  const idx = items.findIndex((x) => x.id === id);
  if (idx >= 0) items.splice(idx, 1);
}

function clear(): void {
  items.splice(0, items.length);
}

export function useToast() {
  return {
    items,
    success: (title: string, opts?: ToastOptions) => push("success", title, opts),
    error: (title: string, opts?: ToastOptions) => push("error", title, opts),
    warn: (title: string, opts?: ToastOptions) => push("warn", title, opts),
    info: (title: string, opts?: ToastOptions) => push("info", title, opts),
    dismiss,
    clear,
  };
}
