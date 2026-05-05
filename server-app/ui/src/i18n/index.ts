/**
 * vue-i18n v11 入口 — Composition API 模式 (legacy: false)。
 *
 * 设计要点:
 *   1. 默认从 localStorage 读取上次选择的 locale,无值则按浏览器语言猜测
 *      (zh*-* → zh-CN, 其它一律 en-US)。
 *   2. fallback 链 zh-CN ⇄ en-US,避免任意一边 key 缺失就显示 raw key。
 *   3. 暴露 setLocale() 一处管理 — 同步内存、localStorage、html lang 属性。
 *   4. 消息文件采用 default-export 的 plain object,Vite 静态打包,
 *      无需 @intlify/unplugin-vue-i18n 等额外构建插件。
 */
import { createI18n } from "vue-i18n";

import zhCN from "./locales/zh-CN";
import enUS from "./locales/en-US";

export type Locale = "zh-CN" | "en-US";

const STORAGE_KEY = "conduit-server-locale";
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

// 让 utils/format.ts 这种"非组件"工具函数也能在不引入 useI18n 的前提下取到翻译
// (避免 utils → i18n → utils 的循环 import,且不强求每个调用方注入 t)。
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
