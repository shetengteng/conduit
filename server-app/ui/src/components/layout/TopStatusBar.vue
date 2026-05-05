<script setup lang="ts">
/**
 * 顶部状态栏 —— 全局可见的运行状态摘要：
 *   左：状态徽章 + 状态描述文案
 *   中：HTTP / SOCKS5 / API 三端口胶囊（未启动时合并为占位提示）
 *   右：运行时长 + Stop 按钮
 *
 * 高度 56px，与 Sidebar logo 区视觉对齐。
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { RiCloseCircleLine, RiAlertLine, RiRestartLine } from '@remixicon/vue'
import StatusBadge from './StatusBadge.vue'
import { proxyStore } from '@/stores/proxy'
import { useToast } from '@/composables/useToast'
import { ServerApi } from '@/api/server'
import { ApiError } from '@/api/client'
import { formatUptimeShort } from '@/utils/format'

const { t } = useI18n()
const toast = useToast()
const confirmOpen = ref(false)
const stopping = ref(false)
const restarting = ref(false)

// Tauri invoke 在浏览器/dev 模式下未必可用,做个软探测
async function tauriInvoke(cmd: string): Promise<unknown> {
  const w = window as any
  const fn = w.__TAURI__?.core?.invoke ?? w.__TAURI_INTERNALS__?.invoke
  if (typeof fn !== 'function') {
    throw new Error(t('topbar.tauriUnavailable'))
  }
  return fn(cmd)
}

const status = computed(() => proxyStore.state.status)

const tone = computed<'running' | 'warning' | 'stopped' | 'error'>(() => {
  const s = status.value
  if (!s) return 'stopped'
  if (!s.running) return 'stopped'
  if (s.vpn && !s.vpn.available) return 'warning'
  if (!s.ready) return 'error'
  return 'running'
})

const label = computed(() => {
  const s = status.value
  if (!s) return t('status.notStarted')
  if (!s.running) return t('status.stopped')
  if (s.vpn && !s.vpn.available) return t('status.vpnError')
  if (!s.ready) return t('status.notReady')
  return t('status.running')
})

const subLabel = computed(() => {
  const s = status.value
  if (!s) return t('topbar.sub.notStarted')
  if (!s.running) return t('topbar.sub.stopped')
  if (s.vpn && !s.vpn.available) return t('topbar.sub.vpnError')
  if (!s.ready) return t('topbar.sub.notReady')
  // 按 peer_ip 去重(同一客户端开多个 tab 不重复计数)
  const active = proxyStore.activePeerCount.value
  const passive = proxyStore.passiveOnlyPeerCount.value
  const total = proxyStore.uniquePeerCount.value
  if (total === 0) return t('topbar.sub.waiting')
  if (active === 0) return t('topbar.sub.passiveOnly', { count: passive })
  if (passive === 0) return t('topbar.sub.activeOnly', { count: active })
  return t('topbar.sub.mixed', { total, active, passive })
})

const ports = computed(() => {
  const s = status.value
  if (!s) return []
  return [
    { label: 'HTTP', value: s.http_port },
    { label: 'SOCKS5', value: s.socks5_port },
    { label: 'API', value: s.api_port },
  ]
})

// 每秒 tick 让顶栏的 "运行 Xs" 在 polling 间隔之间持续增长。
const nowMs = ref(Date.now())
let tickTimer: ReturnType<typeof setInterval> | null = null
onMounted(() => {
  tickTimer = setInterval(() => {
    nowMs.value = Date.now()
  }, 1000)
})
onBeforeUnmount(() => {
  if (tickTimer) {
    clearInterval(tickTimer)
    tickTimer = null
  }
})

const uptime = computed(() => {
  const s = status.value
  if (!s || !s.running) return formatUptimeShort(0)
  const snapshot = s.uptime_sec ?? 0
  const fetchedAt = proxyStore.state.statusFetchedAtMs
  const drift = fetchedAt ? Math.max(0, Math.floor((nowMs.value - fetchedAt) / 1000)) : 0
  return formatUptimeShort(snapshot + drift)
})

function openConfirm() {
  confirmOpen.value = true
}

async function handleConfirmStop() {
  stopping.value = true
  try {
    await ServerApi.adminStop()
    toast.success(t('topbar.toastStopOk'), { detail: t('topbar.toastStopOkDetail') })
    confirmOpen.value = false
    // 不主动 quit Tauri 主进程 —— sidecar 退出后窗口仍然存在(显示"已停止"),
    // 让用户自行关闭或者最小化。如果未来想严格 lockstep,这里可以加
    // window.__TAURI_INTERNALS__.invoke('plugin:app|exit', 0)。
  } catch (e) {
    const msg = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e)
    toast.error(t('topbar.toastStopFail'), { detail: msg })
  } finally {
    stopping.value = false
  }
}

async function handleRestart() {
  if (restarting.value) return
  restarting.value = true
  try {
    toast.info(t('topbar.toastRestartTip'), { detail: t('topbar.toastRestartTipDetail') })
    await tauriInvoke('restart_app')
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    toast.error(t('topbar.toastRestartFail'), {
      detail: `${msg}\n${t('topbar.toastRestartFailHint')}`,
    })
    restarting.value = false
  }
}
</script>

<template>
  <header
    class="flex h-14 shrink-0 items-center gap-3 border-b border-border bg-card px-6"
  >
    <div class="flex items-center gap-2.5">
      <StatusBadge :tone="tone" :label="label" :pulse="tone === 'running'" />
      <span class="hidden text-xs text-muted-foreground md:inline">
        {{ subLabel }}
      </span>
    </div>

    <span class="mx-2 hidden text-muted-foreground/40 md:inline" aria-hidden="true">•</span>

    <div class="flex items-center gap-1.5 text-xs">
      <template v-if="ports.length">
        <div
          v-for="p in ports"
          :key="p.label"
          class="flex items-center gap-1.5 rounded-md bg-muted px-2 py-1 transition-colors hover:bg-accent"
        >
          <span class="text-[10px] uppercase tracking-wider text-muted-foreground font-medium">
            {{ p.label }}
          </span>
          <span class="font-mono text-xs font-medium text-foreground tabular-nums">{{ p.value }}</span>
        </div>
      </template>
      <span v-else class="text-[11px] italic text-muted-foreground">
        {{ t('status.portsPending') }}
      </span>
    </div>

    <div class="ml-auto flex items-center gap-3">
      <span class="hidden font-mono text-xs text-muted-foreground tabular-nums md:inline">
        {{ t('topbar.uptime', { value: uptime }) }}
      </span>
      <Button
        v-if="status?.running"
        variant="destructive"
        size="sm"
        :disabled="stopping"
        @click="openConfirm"
      >
        <RiCloseCircleLine />
        {{ t('topbar.stopAndQuit') }}
      </Button>
      <Button
        v-else
        variant="default"
        size="sm"
        :disabled="restarting"
        @click="handleRestart"
        :title="t('topbar.restartTitle')"
      >
        <RiRestartLine :class="{ 'animate-spin': restarting }" />
        {{ restarting ? t('topbar.restarting') : t('topbar.restart') }}
      </Button>
    </div>
  </header>

  <Dialog v-model:open="confirmOpen">
    <DialogContent class="sm:max-w-[440px]">
      <DialogHeader>
        <DialogTitle class="flex items-center gap-2 text-base">
          <RiAlertLine class="size-4 text-destructive" />
          {{ t('topbar.confirmStopTitle') }}
        </DialogTitle>
        <DialogDescription class="pt-2 text-sm leading-relaxed text-muted-foreground">
          {{ t('topbar.confirmStopBody') }}
          <br>
          <i18n-t keypath="topbar.confirmStopHint" tag="span">
            <template #cmd>
              <code class="font-mono text-foreground">pnpm dev:server</code>
            </template>
          </i18n-t>
        </DialogDescription>
      </DialogHeader>
      <DialogFooter class="gap-2 sm:gap-2">
        <Button variant="outline" size="sm" :disabled="stopping" @click="confirmOpen = false">
          {{ t('topbar.cancel') }}
        </Button>
        <Button variant="destructive" size="sm" :disabled="stopping" @click="handleConfirmStop">
          {{ stopping ? t('topbar.confirmStopOkLoading') : t('topbar.confirmStopOk') }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
