<script setup lang="ts">
/**
 * 顶部状态栏 —— 全局可见的运行状态摘要：
 *   左：状态徽章 + 状态描述文案
 *   中：HTTP / SOCKS5 / API 三端口胶囊（未启动时合并为占位提示）
 *   右：运行时长 + Stop 按钮
 *
 * 高度 56px，与 Sidebar logo 区视觉对齐。
 */
import { computed } from 'vue'
import { Button } from '@/components/ui/button'
import { RiStopCircleLine } from '@remixicon/vue'
import StatusBadge from './StatusBadge.vue'
import { proxyStore } from '@/stores/proxy'
import { useToast } from '@/composables/useToast'
import { ServerApi } from '@/api/server'
import { ApiError } from '@/api/client'
import { formatUptimeShort } from '@/utils/format'

const toast = useToast()

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
  if (!s) return '未启动'
  if (!s.running) return '已停止'
  if (s.vpn && !s.vpn.available) return 'VPN 异常'
  if (!s.ready) return '未就绪'
  return '运行中'
})

const subLabel = computed(() => {
  const s = status.value
  if (!s) return '点击侧边栏的「设置」开始配置代理'
  if (!s.running) return '代理服务已停止，等待重新启动'
  if (s.vpn && !s.vpn.available) return 'VPN 接口未就绪，部分流量可能无法走代理'
  if (!s.ready) return '代理正在启动中，端口尚未就绪'
  return `${s.clients_count ?? 0} 个客户端正在使用代理`
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

const uptime = computed(() => {
  const sec = status.value?.uptime_sec ?? 0
  return formatUptimeShort(sec)
})

async function handleStop() {
  try {
    await ServerApi.adminStop()
    toast.success('已发送停止指令', { detail: '代理引擎正在退出…' })
  } catch (e) {
    const msg = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e)
    toast.error('停止失败', { detail: msg })
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
        端口待分配
      </span>
    </div>

    <div class="ml-auto flex items-center gap-3">
      <span class="hidden font-mono text-xs text-muted-foreground tabular-nums md:inline">
        运行 {{ uptime }}
      </span>
      <Button
        variant="default"
        size="sm"
        :disabled="!status?.running"
        @click="handleStop"
      >
        <RiStopCircleLine />
        停止代理
      </Button>
    </div>
  </header>
</template>
