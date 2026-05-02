/**
 * UI 状态：当前 sidebar 选项、boot phase（启动加载页/失败页驱动）。
 */
import { reactive } from "vue";

import type { LifecyclePhase } from "../types/client";

export type NavKey = "discovery" | "connected" | "diagnose" | "settings";

interface UiState {
  active: NavKey;
  bootPhase: LifecyclePhase;
  bootError: string | null;
}

const state = reactive<UiState>({
  active: "discovery",
  bootPhase: "Booting",
  bootError: null,
});

export const uiStore = {
  state,
  setActive(k: NavKey) {
    state.active = k;
  },
  setBootPhase(p: LifecyclePhase, err: string | null = null) {
    state.bootPhase = p;
    state.bootError = err;
  },
};
