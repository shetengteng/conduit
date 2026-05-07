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
import { onMounted, onBeforeUnmount, computed } from 'vue'
import Sidebar from '@/components/layout/Sidebar.vue'
import TopStatusBar from '@/components/layout/TopStatusBar.vue'
import BootScreen from '@/components/layout/BootScreen.vue'
import BootFailedScreen from '@/components/layout/BootFailedScreen.vue'
import FirstLaunchModal from '@/components/feedback/FirstLaunchModal.vue'
import { Toaster } from 'vue-sonner'
import 'vue-sonner/style.css'
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

// 后端在被动客户端心跳到来后,touch existing 不会广播 SSE event,前端 last_seen 会停滞,
// 用一个 8s 间隔的轻量轮询拉一次 status+clients(只动 store 不弹 toast)。同时也作为
// status/uptime 的兜底刷新源 —— 否则只有 ready/disconnect 等少数事件会更新 status。
let pollTimer: ReturnType<typeof setInterval> | null = null

onMounted(async () => {
  await proxyStore.refresh()
  trafficStore.loadInitial(60).catch((e) => console.warn('[traffic] init', e))
  pollTimer = setInterval(() => {
    proxyStore.refreshSilently()
  }, 8000)
})

onBeforeUnmount(() => {
  if (pollTimer) {
    clearInterval(pollTimer)
    pollTimer = null
  }
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
  passive_client_seen: (p) => proxyStore.applyPassiveClientSeen(p),
  passive_client_lost: (p) => proxyStore.applyPassiveClientLost(p),
  traffic_tick: (p) => trafficStore.applyTick(p),
  vpn_state_changed: (p) =>
    proxyStore.applyVpnState({
      available: p.available,
      iface: p.iface,
      default_route_via_vpn: p.default_route_via_vpn ?? false,
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
  </template>

  <BootScreen v-else />

  <!-- shadcn-vue 推荐的 toast 实现，独立于 boot phase 始终挂载，
       这样 BootScreen / BootFailedScreen 阶段也能弹 toast。 -->
  <Toaster
    position="bottom-right"
    :rich-colors="true"
    :close-button="true"
    theme="system"
  />
</template>
