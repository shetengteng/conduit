/**
 * vue-i18n v11 入口 — Composition API 模式 (legacy: false)。
 *
 * 与 server-app/ui 入口结构一致:
 *   - 默认从 localStorage 读取上次选择的 locale,无值则按浏览器语言猜测
 *   - fallback 链 zh-CN ⇄ en-US
 *   - 暴露 setLocale() 一处管理 — 同步内存、localStorage、html lang 属性
 *   - globalThis 暴露给 utils 等"非组件"代码 (避免 utils → i18n 循环 import)
 */
import { createI18n } from "vue-i18n";

import zhCN from "./locales/zh-CN";
import enUS from "./locales/en-US";

export type Locale = "zh-CN" | "en-US";

const STORAGE_KEY = "conduit-client-locale";
const DEFAULT_LOCALE: Locale = "zh-CN";

export const SUPPORTED_LOCALES: Array<{ code: Locale; label: string }> = [
  { code: "zh-CN", label: "简体中文" },
  { code: "en-US", label: "English" },
];

function detectInitialLocale(): Locale {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "zh-CN" || saved === "en-US") return saved;
  } catch {
    // localStorage 在某些 sandbox 里可能不可用,忽略
  }
  const nav =
    typeof navigator !== "undefined" ? navigator.language || "" : "";
  if (nav.toLowerCase().startsWith("zh")) return "zh-CN";
  return "en-US";
}

const initial = detectInitialLocale();

export const i18n = createI18n({
  legacy: false,
  globalInjection: true,
  locale: initial,
  fallbackLocale: DEFAULT_LOCALE,
  messages: {
    "zh-CN": zhCN,
    "en-US": enUS,
  },
  missingWarn: false,
  fallbackWarn: false,
});

if (typeof document !== "undefined") {
  document.documentElement.lang = initial;
}

(globalThis as unknown as { __conduit_i18n__?: typeof i18n }).__conduit_i18n__ = i18n;

export function setLocale(next: Locale): void {
  const i18nGlobal = i18n.global as unknown as { locale: { value: Locale } };
  i18nGlobal.locale.value = next;
  try {
    localStorage.setItem(STORAGE_KEY, next);
  } catch {
    // ignore quota / privacy errors
  }
  if (typeof document !== "undefined") {
    document.documentElement.lang = next;
  }
}

export function getLocale(): Locale {
  const i18nGlobal = i18n.global as unknown as { locale: { value: Locale } };
  return i18nGlobal.locale.value;
}
