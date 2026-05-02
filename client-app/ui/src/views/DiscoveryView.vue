<script setup lang="ts">
/**
 * 发现页 —— 列出 LAN 上自动发现的 Conduit Server。
 *
 * M-β.1：
 *   - 进入 mount 时拉一次 GET /api/servers + 订阅 SSE 增量
 *   - 在线 server 卡片高亮（绿点 + "在线"）
 *   - 历史 server 卡片灰显（"上次见过 X 分钟前"）
 *   - 空态分两种：mDNS 不可用（zeroconf 没装）vs 暂无 server
 *   - 右上角"重新扫描"按钮触发手动 refresh
 *
 * 卡片信息密度：
 *   - 主标题：name（粗体 14px）+ 状态点（绿/灰）
 *   - 副标题：host:port + 版本号（mono 12px,muted）
 *   - 元信息行：SOCKS / API 端口 + VPN 状态 chip
 *   - 操作区：暂禁用"连接"按钮（M-β.2 才启用）
 */
import { computed } from 'vue'
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  CardDescription,
} from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import {
  RiCompass3Line,
  RiRefreshLine,
  RiServerLine,
  RiShieldFlashLine,
  RiAlertLine,
  RiSignalWifi1Line,
  RiTimeLine,
} from '@remixicon/vue'

import { useDiscovery } from '@/composables/useDiscovery'
import { connectionStore } from '@/stores/connectionStore'
import { uiStore } from '@/stores/ui'
import { useToast } from '@/composables/useToast'
import type { DiscoveredServer } from '@/types/client'

const toast = useToast()

const {
  servers,
  available,
  loading,
  error,
  isEmpty,
  onlineCount,
  historyCount,
  manualRefresh,
} = useDiscovery()

async function handleConnect(srv: DiscoveredServer) {
  if (connectionStore.isConnecting.value) return
  // 跳到「已连接」标签页（connecting / connected 时该 view 会展示 ConnectingProgress / ConnectedView）
  uiStore.setActive('connected')
  try {
    await connectionStore.connectTo(srv.server_id)
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    toast.error('连接失败', { detail: msg })
  }
}

function isConnectedTo(srv: DiscoveredServer): boolean {
  return (
    connectionStore.isConnected.value &&
    connectionStore.connectedServer.value?.server_id === srv.server_id
  )
}

function isConnectingTo(srv: DiscoveredServer): boolean {
  return (
    connectionStore.isConnecting.value &&
    connectionStore.pendingServerId.value === srv.server_id
  )
}

function formatRelativeTime(epochSec: number): string {
  if (!epochSec) return '从未'
  const diffSec = Math.max(0, Date.now() / 1000 - epochSec)
  if (diffSec < 60) return `${Math.round(diffSec)} 秒前`
  if (diffSec < 3600) return `${Math.round(diffSec / 60)} 分钟前`
  if (diffSec < 86400) return `${Math.round(diffSec / 3600)} 小时前`
  return `${Math.round(diffSec / 86400)} 天前`
}

function isOnline(srv: DiscoveredServer): boolean {
  return srv.source === 'mdns'
}

const headerSubtitle = computed(() => {
  if (loading.value && servers.value.length === 0) return '正在扫描…'
  if (!available.value) return 'mDNS 服务未启用'
  if (isEmpty.value) return '暂未发现任何 Conduit Server'
  const parts: string[] = []
  if (onlineCount.value > 0) parts.push(`${onlineCount.value} 个在线`)
  if (historyCount.value > 0) parts.push(`${historyCount.value} 个曾见过`)
  return parts.join(' · ')
})
</script>

<template>
  <div class="flex flex-col gap-6 p-6">
    <!-- 页头 -->
    <div class="flex items-start justify-between gap-4">
      <div class="flex flex-col gap-1">
        <h1 class="text-2xl font-extralight tracking-tight text-foreground">
          发现 Conduit 服务
        </h1>
        <p class="text-sm text-muted-foreground">
          {{ headerSubtitle }}
        </p>
      </div>
      <Button
        variant="outline"
        size="sm"
        :disabled="loading"
        @click="manualRefresh"
        class="gap-1.5"
      >
        <RiRefreshLine class="size-3.5" :class="{ 'animate-spin': loading }" />
        重新扫描
      </Button>
    </div>

    <!-- 错误条 -->
    <div
      v-if="error"
      class="flex items-start gap-2.5 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive"
    >
      <RiAlertLine class="size-4 shrink-0 mt-px" />
      <div class="flex flex-col gap-0.5">
        <span class="font-medium">无法获取服务列表</span>
        <span class="text-destructive/80">{{ error }}</span>
      </div>
    </div>

    <!-- mDNS 不可用提示（zeroconf 没装；正式包不会触发,但开发环境可能） -->
    <Card v-if="!available && !loading" size="sm" class="border-amber-300/50 bg-amber-50/40 dark:bg-amber-950/10">
      <CardContent class="flex items-start gap-3 py-3 text-xs">
        <RiSignalWifi1Line class="size-4 mt-0.5 text-amber-600" />
        <div class="flex flex-col gap-0.5">
          <p class="font-medium text-amber-900 dark:text-amber-200">
            自动发现未启用
          </p>
          <p class="text-amber-800/80 dark:text-amber-200/70">
            Sidecar 未加载 zeroconf 模块，无法广播 / 监听 mDNS。请检查打包配置或手动添加 server（即将推出）。
          </p>
        </div>
      </CardContent>
    </Card>

    <!-- 空态：完全没有任何 server -->
    <Card v-if="isEmpty && available && !loading && !error" size="sm">
      <CardContent class="flex flex-col items-center justify-center gap-3 py-12 text-center">
        <div class="flex size-10 items-center justify-center rounded-full bg-muted text-muted-foreground">
          <RiCompass3Line class="size-5 animate-pulse" />
        </div>
        <div class="flex flex-col gap-1">
          <p class="text-sm font-medium text-foreground">正在搜索 LAN 上的 Conduit 服务…</p>
          <p class="text-xs text-muted-foreground max-w-md">
            如果首次启动 macOS 会弹出"本地网络访问"权限请求，请允许。<br />
            通常 5–10 秒内能看到同网段开着的 server。
          </p>
        </div>
      </CardContent>
    </Card>

    <!-- 服务卡片网格 -->
    <div v-if="servers.length > 0" class="grid grid-cols-1 gap-3 lg:grid-cols-2">
      <Card
        v-for="srv in servers"
        :key="srv.server_id"
        size="sm"
        :class="[
          'transition-colors',
          isOnline(srv)
            ? 'border-foreground/15 hover:border-foreground/30'
            : 'border-border/60 bg-muted/20 opacity-80',
        ]"
      >
        <CardHeader class="flex flex-row items-start justify-between gap-3 space-y-0 pb-3">
          <div class="flex items-start gap-2.5 min-w-0">
            <div
              :class="[
                'flex size-8 shrink-0 items-center justify-center rounded-md',
                isOnline(srv)
                  ? 'bg-foreground/5 text-foreground'
                  : 'bg-muted text-muted-foreground',
              ]"
            >
              <RiServerLine class="size-4" />
            </div>
            <div class="flex flex-col gap-0.5 min-w-0">
              <CardTitle class="flex items-center gap-2 text-[13px] font-semibold truncate">
                <span class="truncate">{{ srv.name }}</span>
                <span
                  v-if="isOnline(srv)"
                  class="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-400"
                >
                  <span class="size-1.5 rounded-full bg-emerald-500"></span>
                  在线
                </span>
                <span
                  v-else
                  class="inline-flex items-center gap-1 rounded-full bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground"
                >
                  <RiTimeLine class="size-2.5" />
                  曾见过
                </span>
              </CardTitle>
              <CardDescription class="text-[11px] font-mono text-muted-foreground truncate">
                {{ srv.host }}:{{ srv.port }} · v{{ srv.version || '?' }}
              </CardDescription>
            </div>
          </div>
        </CardHeader>

        <CardContent class="flex flex-col gap-3">
          <!-- 元信息行 -->
          <div class="grid grid-cols-3 gap-2 text-[11px]">
            <div class="flex flex-col gap-0.5">
              <span class="text-muted-foreground">SOCKS</span>
              <span class="font-mono font-medium text-foreground">{{ srv.socks }}</span>
            </div>
            <div class="flex flex-col gap-0.5">
              <span class="text-muted-foreground">控制 API</span>
              <span class="font-mono font-medium text-foreground">{{ srv.api }}</span>
            </div>
            <div class="flex flex-col gap-0.5">
              <span class="text-muted-foreground">VPN</span>
              <span class="flex items-center gap-1 font-medium">
                <RiShieldFlashLine
                  v-if="srv.vpn"
                  class="size-3 text-emerald-600"
                />
                <span :class="srv.vpn ? 'text-foreground' : 'text-muted-foreground'">
                  {{ srv.vpn ? '已开' : '未开' }}
                </span>
              </span>
            </div>
          </div>

          <!-- 时间 + 操作 -->
          <div class="flex items-center justify-between gap-2 pt-1 border-t border-border/40">
            <span class="text-[11px] text-muted-foreground">
              {{ isOnline(srv) ? '广播于' : '上次见到' }} {{ formatRelativeTime(srv.last_seen_at) }}
            </span>
            <Button
              v-if="isConnectedTo(srv)"
              variant="outline"
              size="sm"
              disabled
              class="h-7 text-xs gap-1"
            >
              <span class="size-1.5 rounded-full bg-emerald-500" />已连接
            </Button>
            <Button
              v-else-if="isConnectingTo(srv)"
              variant="default"
              size="sm"
              disabled
              class="h-7 text-xs"
            >
              连接中…
            </Button>
            <Button
              v-else
              variant="default"
              size="sm"
              :disabled="!isOnline(srv) || connectionStore.isConnecting.value || connectionStore.isConnected.value"
              class="h-7 text-xs"
              :title="!isOnline(srv) ? '历史 server,等其重新广播后再连接' : connectionStore.isConnected.value ? '请先在已连接页断开当前 server' : ''"
              @click="handleConnect(srv)"
            >
              连接
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  </div>
</template>
