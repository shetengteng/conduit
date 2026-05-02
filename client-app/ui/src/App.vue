<script setup lang="ts">
/**
 * Conduit Client 应用根组件 —— 顶层 layout + 启动相位调度。
 *
 * 与 server-app 一致的 boot phase 路由：
 *   - bootPhase Booting  → BootScreen
 *   - bootPhase Failed   → BootFailedScreen
 *   - bootPhase Ready    → 主界面（Sidebar + TopStatusBar + ViewSlot）
 *
 * 数据生命周期（M-α 阶段）：
 *   - mounted: 拉一次 healthz
 *   - 暂未引入 SSE 订阅，M-β 接入 useEvents 后开启
 */
import { onMounted, onUnmounted, computed, watch } from 'vue'
import Sidebar from '@/components/layout/Sidebar.vue'
import TopStatusBar from '@/components/layout/TopStatusBar.vue'
import BootScreen from '@/components/layout/BootScreen.vue'
import BootFailedScreen from '@/components/layout/BootFailedScreen.vue'
import ToastHost from '@/components/feedback/ToastHost.vue'
import DiscoveryView from '@/views/DiscoveryView.vue'
import ConnectedView from '@/views/ConnectedView.vue'
import DiagnoseView from '@/views/DiagnoseView.vue'
import SettingsView from '@/views/SettingsView.vue'
import { uiStore } from '@/stores/ui'
import { clientStore } from '@/stores/clientStore'
import { connectionStore } from '@/stores/connectionStore'
import { trafficStore } from '@/stores/trafficStore'
import { cacheStore } from '@/stores/cacheStore'
import { useBootPhase } from '@/composables/useBootPhase'
import { useEvents } from '@/composables/useEvents'
import { useToast } from '@/composables/useToast'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

useBootPhase()

const toast = useToast()

const isReady = computed(() => uiStore.state.bootPhase === 'Ready')
const isFailed = computed(() => uiStore.state.bootPhase === 'Failed')

// 全局 SSE 订阅:连接相关事件全部交给 connectionStore
let stopEvents: (() => void) | null = null
let unlistenTrayNav: UnlistenFn | null = null

onMounted(async () => {
  await clientStore.refresh()
  await connectionStore.refresh()

  const evt = useEvents(
    {
      connect_progress: connectionStore.onProgress,
      connect_done: connectionStore.onConnectDone,
      connection_state_changed: connectionStore.onStateChange,
      heartbeat_changed: connectionStore.onHeartbeat,
      traffic_tick: trafficStore.onTick,
      route_decision: cacheStore.onRouteDecision,
    },
    { autoStart: true },
  )
  stopEvents = evt.stop

  // 托盘菜单 → 主窗口路由切换
  try {
    unlistenTrayNav = await listen<string>('tray:navigate', (e) => {
      const key = e.payload
      if (key === 'discovery' || key === 'connected' || key === 'diagnose' || key === 'settings') {
        uiStore.setActive(key)
      }
    })
  } catch {
    // 非 Tauri 环境(浏览器调试) 直接忽略
  }
})

onUnmounted(() => {
  stopEvents?.()
  unlistenTrayNav?.()
})

// 进入 connecting / connected 时,自动跳到「已连接」视图;
// 用户主动断开后停留在「已连接」(展示空态),不强制跳转。
watch(
  () => connectionStore.connectionState.value,
  (next, prev) => {
    if (next === 'connecting' && uiStore.state.active !== 'connected') {
      uiStore.setActive('connected')
    } else if (next === 'connected' && prev === 'connecting') {
      // 连接成功提示
      const srv = connectionStore.connectedServer.value
      if (srv) toast.success(`已连接到 ${srv.name}`, { detail: `${srv.host}:${srv.port}` })
    } else if (next === 'failed') {
      const err = connectionStore.lastError.value ?? '未知错误'
      toast.error('连接失败', { detail: err })
    }
  },
)

async function handleQuit() {
  try {
    await invoke('quit_app')
  } catch (_) {
    window.close()
  }
}

function handleRetry() {
  uiStore.setBootPhase('Booting')
  clientStore.refresh()
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
          <DiscoveryView v-if="uiStore.state.active === 'discovery'" />
          <ConnectedView v-else-if="uiStore.state.active === 'connected'" />
          <DiagnoseView v-else-if="uiStore.state.active === 'diagnose'" />
          <SettingsView v-else-if="uiStore.state.active === 'settings'" />
        </main>
      </div>
    </div>

    <ToastHost />
  </template>

  <BootScreen v-else />
</template>
