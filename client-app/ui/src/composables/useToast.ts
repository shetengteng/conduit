/**
 * 全局 Toast 通知系统 —— vue-sonner thin wrapper（shadcn-vue 官方推荐 toast 实现）。
 *
 * 用法（与之前自实现的版本完全兼容，调用点零改动）：
 *   import { useToast } from "@/composables/useToast";
 *   const toast = useToast();
 *   toast.success("已复制到剪贴板");
 *   toast.error("启动失败", { detail: "address already in use" });
 *
 * 渲染由挂在 App.vue 根的 <Toaster /> from "vue-sonner" 接管，
 * 我们的 detail 字段映射到 sonner 的 description。
 */
import { toast as sonner } from "vue-sonner";

export type ToastTone = "success" | "error" | "warn" | "info";

export interface ToastOptions {
  duration?: number;
  detail?: string;
}

type SonnerId = string | number;

function defaultDuration(tone: ToastTone): number {
  return tone === "error" ? 5000 : 3000;
}

function show(tone: ToastTone, title: string, opts?: ToastOptions): SonnerId {
  const duration = opts?.duration ?? defaultDuration(tone);
  const sonnerOpts = {
    description: opts?.detail,
    duration,
  };
  switch (tone) {
    case "success":
      return sonner.success(title, sonnerOpts);
    case "error":
      return sonner.error(title, sonnerOpts);
    case "warn":
      return sonner.warning(title, sonnerOpts);
    case "info":
    default:
      return sonner.info(title, sonnerOpts);
  }
}

function dismiss(id?: SonnerId): void {
  if (id === undefined) sonner.dismiss();
  else sonner.dismiss(id);
}

function clear(): void {
  sonner.dismiss();
}

export function useToast() {
  return {
    success: (title: string, opts?: ToastOptions) => show("success", title, opts),
    error: (title: string, opts?: ToastOptions) => show("error", title, opts),
    warn: (title: string, opts?: ToastOptions) => show("warn", title, opts),
    info: (title: string, opts?: ToastOptions) => show("info", title, opts),
    dismiss,
    clear,
  };
}
