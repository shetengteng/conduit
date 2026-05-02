<script setup lang="ts">
/**
 * Conduit Server 应用根组件 —— 顶层 layout + 启动相位调度。
 *
 * 路由策略：
 *   - bootPhase Booting  → BootScreen
 *   - bootPhase Failed   → BootFailedScreen
 *   - bootPhase Ready    → 主界面（Sidebar + TopStatusBar + ViewSlot）
 *
 * 数据生命周期：
 *   - mounted: 拉一次 status / clients / healthz；订阅 SSE
 *   - unmounted: SSE 由各 composable 自行清理
 */
import { onMounted, computed } from 'vue'
import Sidebar from '@/components/layout/Sidebar.vue'
import TopStatusBar from '@/components/layout/TopStatusBar.vue'
import BootScreen from '@/components/layout/BootScreen.vue'
import BootFailedScreen from '@/components/layout/BootFailedScreen.vue'
import FirstLaunchModal from '@/components/feedback/FirstLaunchModal.vue'
import ToastHost from '@/components/feedback/ToastHost.vue'
import DashboardView from '@/views/DashboardView.vue'
import LogsView from '@/views/LogsView.vue'
import SettingsView from '@/views/SettingsView.vue'
import { uiStore } from '@/stores/ui'
import { proxyStore } from '@/stores/proxy'
import { trafficStore } from '@/stores/traffic'
import { useEvents } from '@/composables/useEvents'
import { useBootPhase } from '@/composables/useBootPhase'
import { invoke } from '@tauri-apps/api/core'

useBootPhase()

const isReady = computed(() => uiStore.state.bootPhase === 'Ready')
const isFailed = computed(() => uiStore.state.bootPhase === 'Failed')

onMounted(async () => {
  await proxyStore.refresh()
  trafficStore.loadInitial(60).catch((e) => console.warn('[traffic] init', e))
})

useEvents({
  ready: () => proxyStore.refresh(),
  client_connected: (p) =>
    proxyStore.applyClientConnected({
      session_id: p.session_id,
      peer_ip: p.peer_ip,
      proto: p.proto,
      target: p.target,
      since: p.since,
      last_seen: p.since,
      sent_bytes: 0,
      recv_bytes: 0,
    }),
  client_disconnected: (p) => proxyStore.applyClientDisconnected(p.session_id),
  traffic_tick: (p) => trafficStore.applyTick(p),
  vpn_state_changed: (p) =>
    proxyStore.applyVpnState({
      available: p.available,
      iface: p.iface,
      default_route_via_vpn: false,
    }),
})

async function handleQuit() {
  try {
    await invoke('quit_app')
  } catch (_) {
    window.close()
  }
}

function handleRetry() {
  uiStore.setBootPhase('Booting')
  proxyStore.refresh()
}
</script>

<template>
  <BootFailedScreen
    v-if="isFailed"
    :reason="uiStore.state.bootError"
    @retry="handleRetry"
    @quit="handleQuit"
  />

  <template v-else-if="isReady">
    <div class="flex h-screen w-full overflow-hidden bg-background">
      <Sidebar />
      <div class="flex flex-1 flex-col overflow-hidden">
        <TopStatusBar />
        <main class="flex-1 overflow-y-auto">
          <DashboardView v-if="uiStore.state.active === 'dashboard'" />
          <LogsView v-else-if="uiStore.state.active === 'logs'" />
          <SettingsView v-else-if="uiStore.state.active === 'settings'" />
        </main>
      </div>
    </div>

    <FirstLaunchModal />
    <ToastHost />
  </template>

  <BootScreen v-else />
</template>
