import { onMounted, onUnmounted, ref, watch } from "vue";

export type Theme = "light" | "dark" | "system";

const STORAGE_KEY = "conduit:theme";

function readStored(): Theme {
  if (typeof localStorage === "undefined") return "system";
  const v = localStorage.getItem(STORAGE_KEY);
  return v === "light" || v === "dark" || v === "system" ? v : "system";
}

function systemPrefersDark(): boolean {
  if (typeof window === "undefined" || !window.matchMedia) return false;
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function applyClass(isDark: boolean) {
  if (typeof document === "undefined") return;
  document.documentElement.classList.toggle("dark", isDark);
}

/**
 * Theme composable shared by both server-app and client-app.
 *
 * - persists user choice to localStorage under `conduit:theme`
 * - falls back to system preference
 * - reactively follows system changes when in "system" mode
 */
export function useTheme() {
  const theme = ref<Theme>(readStored());
  const isDark = ref<boolean>(false);

  function recompute() {
    isDark.value = theme.value === "dark" || (theme.value === "system" && systemPrefersDark());
    applyClass(isDark.value);
  }

  let mql: MediaQueryList | null = null;
  const onSystemChange = () => {
    if (theme.value === "system") recompute();
  };

  onMounted(() => {
    if (typeof window !== "undefined" && window.matchMedia) {
      mql = window.matchMedia("(prefers-color-scheme: dark)");
      mql.addEventListener("change", onSystemChange);
    }
    recompute();
  });

  onUnmounted(() => {
    mql?.removeEventListener("change", onSystemChange);
  });

  watch(theme, (v) => {
    if (typeof localStorage !== "undefined") localStorage.setItem(STORAGE_KEY, v);
    recompute();
  });

  function setTheme(v: Theme) {
    theme.value = v;
  }

  return { theme, isDark, setTheme };
}
