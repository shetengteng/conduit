<script setup lang="ts">
/**
 * 顶部状态栏 —— 全局可见的运行状态摘要：
 *   左：状态徽章 + 状态描述文案
 *   中：HTTP / SOCKS5 / API 三端口胶囊（未启动时合并为占位提示）
 *   右：运行时长 + Stop 按钮
 *
 * 高度 56px，与 Sidebar logo 区视觉对齐。
 */
import { computed, ref } from 'vue'
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

const toast = useToast()
const confirmOpen = ref(false)
const stopping = ref(false)
const restarting = ref(false)

// Tauri invoke 在浏览器/dev 模式下未必可用,做个软探测
async function tauriInvoke(cmd: string): Promise<unknown> {
  const w = window as any
  const fn = w.__TAURI__?.core?.invoke ?? w.__TAURI_INTERNALS__?.invoke
  if (typeof fn !== 'function') {
    throw new Error('Tauri invoke 不可用 (可能在浏览器中预览)')
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
  const active = s.clients_count ?? 0
  const passive = s.passive_clients_count ?? 0
  const total = active + passive
  if (total === 0) return '等待客户端接入'
  if (active === 0) return `${passive} 个客户端已链接(待命中)`
  if (passive === 0) return `${active} 个客户端正在传输流量`
  return `共 ${total} 个客户端 · ${active} 传输 + ${passive} 待命`
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

function openConfirm() {
  confirmOpen.value = true
}

async function handleConfirmStop() {
  stopping.value = true
  try {
    await ServerApi.adminStop()
    toast.success('应用即将退出', { detail: '代理引擎已停止,Conduit Server 进程正在清理资源…' })
    confirmOpen.value = false
    // 不主动 quit Tauri 主进程 —— sidecar 退出后窗口仍然存在(显示"已停止"),
    // 让用户自行关闭或者最小化。如果未来想严格 lockstep,这里可以加
    // window.__TAURI_INTERNALS__.invoke('plugin:app|exit', 0)。
  } catch (e) {
    const msg = e instanceof ApiError ? `${e.code}: ${e.message}` : String(e)
    toast.error('停止失败', { detail: msg })
  } finally {
    stopping.value = false
  }
}

async function handleRestart() {
  if (restarting.value) return
  restarting.value = true
  try {
    toast.info('正在重启应用…', { detail: '主窗口将立即关闭并重新启动 sidecar' })
    await tauriInvoke('restart_app')
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    toast.error('重启失败', { detail: `${msg}\n请手动退出并重新打开 Conduit Server` })
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
        端口待分配
      </span>
    </div>

    <div class="ml-auto flex items-center gap-3">
      <span class="hidden font-mono text-xs text-muted-foreground tabular-nums md:inline">
        运行 {{ uptime }}
      </span>
      <Button
        v-if="status?.running"
        variant="destructive"
        size="sm"
        :disabled="stopping"
        @click="openConfirm"
      >
        <RiCloseCircleLine />
        停止代理并退出
      </Button>
      <Button
        v-else
        variant="default"
        size="sm"
        :disabled="restarting"
        @click="handleRestart"
        title="重新启动 sidecar 让代理回到运行中"
      >
        <RiRestartLine :class="{ 'animate-spin': restarting }" />
        {{ restarting ? '重启中…' : '重启代理' }}
      </Button>
    </div>
  </header>

  <Dialog v-model:open="confirmOpen">
    <DialogContent class="sm:max-w-[440px]">
      <DialogHeader>
        <DialogTitle class="flex items-center gap-2 text-base">
          <RiAlertLine class="size-4 text-destructive" />
          确认停止 Conduit Server?
        </DialogTitle>
        <DialogDescription class="pt-2 text-sm leading-relaxed text-muted-foreground">
          停止后代理引擎(HTTP / SOCKS5 / mDNS 广播)将全部下线,正在使用本机 VPN 的客户端会立刻断开。
          <br>
          v0.1 阶段不支持在窗口里"重启代理",需要重新打开 Conduit Server 应用(或在终端跑
          <code class="font-mono text-foreground">pnpm dev:server</code>)才能再次启动。
        </DialogDescription>
      </DialogHeader>
      <DialogFooter class="gap-2 sm:gap-2">
        <Button variant="outline" size="sm" :disabled="stopping" @click="confirmOpen = false">
          取消
        </Button>
        <Button variant="destructive" size="sm" :disabled="stopping" @click="handleConfirmStop">
          {{ stopping ? '停止中…' : '确认停止' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
