<script setup lang="ts">
/**
 * 客户端顶部状态栏 —— 全局可见的本地服务状态摘要：
 *   左：状态徽章（基于 control API healthz）+ 运行时长
 *   中：SOCKS5 与 API 端口胶囊
 *   右：（M-α 暂留空，M-β 接入"断开连接"按钮）
 *
 * 高度 56px，与 Sidebar logo 区视觉对齐。
 *
 * 状态映射：
 *   - healthz === null    → "未启动"（深灰）
 *   - healthz.ready === true → "就绪"（绿）
 *   - healthz.ready === false → "异常"（红）
 *
 * M-β.2 接入连接状态后会再加：connecting / connected / disconnected /
 * heartbeat_warn / global_fallback 等。
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import StatusBadge from './StatusBadge.vue'
import { clientStore } from '@/stores/clientStore'
import { discoveryStore } from '@/stores/discoveryStore'
import { getRuntime } from '@/api/runtime'
import { formatUptimeShort } from '@/utils/format'
import type { AppRuntime } from '@/types/client'

const runtime = ref<AppRuntime | null>(null)

// 每秒 reactive tick:让顶栏的 "运行 Xs" 在 healthz polling 间隔之间也能持续增长。
const nowMs = ref(Date.now())
let tickTimer: ReturnType<typeof setInterval> | null = null

onMounted(async () => {
  runtime.value = await getRuntime()
  await clientStore.refresh()
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

const tone = computed<'running' | 'warning' | 'stopped' | 'error'>(() => {
  const h = clientStore.state.healthz
  if (clientStore.state.error) return 'error'
  if (!h) return 'stopped'
  if (!h.ready) return 'error'
  return 'running'
})

const label = computed(() => {
  const h = clientStore.state.healthz
  if (clientStore.state.error) return '连接失败'
  if (!h) return '未启动'
  if (!h.ready) return '异常'
  return '就绪'
})

const subLabel = computed(() => {
  const h = clientStore.state.healthz
  if (clientStore.state.error) return '与 client sidecar 的本地连接失败'
  if (!h) return '正在等待本地服务就绪'
  if (!h.ready) {
    const failed = h.checks.filter((c) => !c.ok).map((c) => c.name).join(', ')
    return `自检未通过：${failed || '未知'}`
  }
  const online = discoveryStore.onlineCount.value
  if (online === 0) return '正在扫描 LAN 上的 Conduit Server…'
  return `已发现 ${online} 个 server，请在"发现"页选择连接`
})

const ports = computed(() => {
  const rt = runtime.value
  if (!rt) return []
  return [
    { label: 'SOCKS5', value: rt.socks_port },
    { label: 'API', value: rt.api_port },
  ]
})

const uptime = computed(() => {
  const snapshot = clientStore.uptimeSec.value
  const fetchedAt = clientStore.state.healthzFetchedAtMs
  if (!fetchedAt || snapshot <= 0) return formatUptimeShort(snapshot)
  const drift = Math.max(0, Math.floor((nowMs.value - fetchedAt) / 1000))
  return formatUptimeShort(snapshot + drift)
})
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
        端口由 Tauri 主进程分配
      </span>
    </div>

    <div class="ml-auto flex items-center gap-3">
      <span class="hidden font-mono text-xs text-muted-foreground tabular-nums md:inline">
        运行 {{ uptime }}
      </span>
    </div>
  </header>
</template>
