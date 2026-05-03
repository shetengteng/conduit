<script setup lang="ts">
/**
 * 已连接视图 —— M-β.2。
 *
 * 三态:
 *   - connecting: 渲染 ConnectingProgress 五步进度
 *   - connected: 渲染当前 server 卡 + 心跳 + 断开按钮
 *   - 其他(idle/failed): 引导回发现页
 *
 * M-γ 还会在这里加流量曲线 + 路由命中表。
 */
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import {
  RiPlugLine,
  RiPlugFill,
  RiHeartPulseLine,
  RiShieldFlashLine,
  RiCloseLine,
  RiAlertLine,
  RiCompass3Line,
} from '@remixicon/vue'
import ConnectingProgress from '@/components/business/ConnectingProgress.vue'
import TrafficChart from '@/components/business/TrafficChart.vue'
import CacheTable from '@/components/business/CacheTable.vue'
import { connectionStore } from '@/stores/connectionStore'
import { trafficStore } from '@/stores/trafficStore'
import { cacheStore } from '@/stores/cacheStore'
import { uiStore } from '@/stores/ui'
import { useToast } from '@/composables/useToast'

const toast = useToast()
const isDisconnecting = ref(false)
const now = ref(Date.now() / 1000)
let tick: number | null = null

onMounted(() => {
  tick = window.setInterval(() => {
    now.value = Date.now() / 1000
  }, 1000)
  // 总是 refresh 一次,不依赖 isConnected:
  //   - 浏览器刷新后 ConnectedView 可能在 App.vue 的 connectionStore.refresh 之前就 mount 了,
  //     此时 isConnected=false 会让 traffic/cache 永远是 0,直到用户切换标签
  //   - backend /api/traffic 返回的累计值在未连接也安全(只是返回 baseline 0)
  trafficStore.refresh()
  cacheStore.refresh()
})

onUnmounted(() => {
  if (tick !== null) window.clearInterval(tick)
})

watch(
  () => connectionStore.connectionState.value,
  (next) => {
    if (next === 'connected') {
      trafficStore.refresh()
      cacheStore.refresh()
    } else if (next === 'idle') {
      trafficStore.reset()
      cacheStore.reset()
    }
  },
)

const connectedSince = computed(() => connectionStore.connectedSince.value)
const elapsedSeconds = computed(() => {
  const since = connectedSince.value
  if (!since) return 0
  return Math.max(0, Math.floor(now.value - since))
})

const elapsedHuman = computed(() => {
  const s = elapsedSeconds.value
  if (s < 60) return `${s} 秒`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m} 分 ${s % 60} 秒`
  const h = Math.floor(m / 60)
  return `${h} 小时 ${m % 60} 分`
})

const heartbeatTone = computed(() => connectionStore.heartbeatTone.value ?? 'green')
const heartbeatLabel = computed(() => {
  switch (heartbeatTone.value) {
    case 'green': return '健康'
    case 'yellow': return '波动'
    case 'red': return '丢失'
    default: return '未知'
  }
})
const heartbeatToneClass = computed(() => {
  switch (heartbeatTone.value) {
    case 'green': return 'text-emerald-700 dark:text-emerald-400 bg-emerald-500/10'
    case 'yellow': return 'text-amber-700 dark:text-amber-400 bg-amber-500/10'
    case 'red': return 'text-destructive bg-destructive/10'
    default: return 'text-muted-foreground bg-muted'
  }
})

async function handleDisconnect() {
  isDisconnecting.value = true
  try {
    await connectionStore.disconnect()
    toast.info('已断开连接')
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    toast.error('断开失败', { detail: msg })
  } finally {
    isDisconnecting.value = false
  }
}

function backToDiscovery() {
  uiStore.setActive('discovery')
}
</script>

<template>
  <!-- connecting:进度 stepper -->
  <ConnectingProgress v-if="connectionStore.isConnecting.value" />

  <!-- connected:当前 server 信息 -->
  <div v-else-if="connectionStore.isConnected.value && connectionStore.connectedServer.value" class="flex flex-col gap-6 p-6">
    <div class="flex items-start justify-between gap-4">
      <div class="flex flex-col gap-1">
        <h1 class="text-2xl font-extralight tracking-tight text-foreground flex items-center gap-3">
          已连接到
          <span class="font-medium">{{ connectionStore.connectedServer.value.name }}</span>
        </h1>
        <p class="text-sm text-muted-foreground">
          {{ connectionStore.systemProxyActive.value ? '系统代理已开启,流量自动走 Conduit' : '系统代理未开启,需手动配置 SOCKS5' }}
        </p>
      </div>
      <Button
        variant="outline"
        size="sm"
        :disabled="isDisconnecting"
        class="gap-1.5 hover:border-destructive hover:text-destructive"
        @click="handleDisconnect"
      >
        <RiCloseLine class="size-3.5" />
        {{ isDisconnecting ? '断开中…' : '断开连接' }}
      </Button>
    </div>

    <!-- 主信息卡 -->
    <Card>
      <CardHeader class="flex flex-row items-center justify-between gap-3 space-y-0 pb-3">
        <div class="flex items-center gap-3">
          <div class="flex size-10 items-center justify-center rounded-md bg-foreground/5 text-foreground">
            <RiPlugFill class="size-5" />
          </div>
          <div class="flex flex-col gap-0.5">
            <CardTitle class="text-base font-semibold tracking-tight flex items-center gap-2">
              {{ connectionStore.connectedServer.value.name }}
              <span class="text-xs font-mono text-muted-foreground">v{{ connectionStore.connectedServer.value.version || '?' }}</span>
            </CardTitle>
            <CardDescription class="text-xs font-mono text-muted-foreground">
              {{ connectionStore.connectedServer.value.host }}:{{ connectionStore.connectedServer.value.port }}
            </CardDescription>
          </div>
        </div>
        <span
          :class="[
            'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-medium',
            heartbeatToneClass,
          ]"
        >
          <RiHeartPulseLine class="size-3" />
          心跳 · {{ heartbeatLabel }}
        </span>
      </CardHeader>

      <Separator />

      <CardContent class="grid grid-cols-1 gap-4 pt-4 sm:grid-cols-2 lg:grid-cols-4">
        <!-- 连接时长 -->
        <div class="flex flex-col gap-1">
          <span class="text-[11px] uppercase tracking-wide text-muted-foreground">已连接</span>
          <span class="text-lg font-extralight tracking-tight text-foreground">{{ elapsedHuman }}</span>
        </div>
        <!-- SOCKS 端口 -->
        <div class="flex flex-col gap-1">
          <span class="text-[11px] uppercase tracking-wide text-muted-foreground">远端 SOCKS</span>
          <span class="text-lg font-mono font-medium text-foreground">{{ connectionStore.connectedServer.value.socks }}</span>
        </div>
        <!-- 控制 API -->
        <div class="flex flex-col gap-1">
          <span class="text-[11px] uppercase tracking-wide text-muted-foreground">远端控制 API</span>
          <span class="text-lg font-mono font-medium text-foreground">{{ connectionStore.connectedServer.value.api }}</span>
        </div>
        <!-- VPN -->
        <div class="flex flex-col gap-1">
          <span class="text-[11px] uppercase tracking-wide text-muted-foreground">远端 VPN</span>
          <span class="flex items-center gap-1.5">
            <RiShieldFlashLine
              v-if="connectionStore.connectedServer.value.vpn"
              class="size-4 text-emerald-600"
            />
            <span :class="connectionStore.connectedServer.value.vpn ? 'text-foreground font-medium' : 'text-muted-foreground'">
              {{ connectionStore.connectedServer.value.vpn ? '已启用' : '未启用' }}
            </span>
          </span>
        </div>
      </CardContent>
    </Card>

    <!-- 心跳故障警告 -->
    <Card
      v-if="heartbeatTone === 'red'"
      size="sm"
      class="border-destructive/30 bg-destructive/5"
    >
      <CardContent class="flex items-start gap-2.5 py-3 text-xs text-destructive">
        <RiAlertLine class="size-4 shrink-0 mt-px" />
        <div class="flex flex-col gap-0.5">
          <span class="font-medium">与 server 失联</span>
          <span class="text-destructive/80">
            连续多次心跳失败,流量可能受影响。Server 会在 mDNS 重新出现时自动恢复;
            如果长期红色,请尝试断开后重新连接。
          </span>
        </div>
      </CardContent>
    </Card>

    <!-- 流量曲线 (M-γ) -->
    <Card size="sm">
      <CardHeader class="pb-2">
        <CardTitle class="text-[13px] font-semibold">流量曲线</CardTitle>
        <CardDescription class="text-xs">最近 60 秒的本地代理上下行速率(只统计已成功建立的连接)</CardDescription>
      </CardHeader>
      <CardContent>
        <TrafficChart />
      </CardContent>
    </Card>

    <!-- 路由缓存表 (M-γ) -->
    <Card size="sm">
      <CardHeader class="pb-2">
        <CardTitle class="text-[13px] font-semibold">路由命中</CardTitle>
        <CardDescription class="text-xs">每个 host 的方向决策与命中次数,direct 走本机,proxy 走 server</CardDescription>
      </CardHeader>
      <CardContent>
        <CacheTable />
      </CardContent>
    </Card>
  </div>

  <!-- 其他状态(idle / failed):引导 -->
  <div v-else class="flex flex-col gap-6 p-6">
    <div class="flex flex-col gap-1">
      <h1 class="text-2xl font-extralight tracking-tight text-foreground">尚未连接</h1>
      <p class="text-sm text-muted-foreground">
        请到「发现」页选择一个 Conduit Server 进行连接
      </p>
    </div>

    <Card v-if="connectionStore.lastError.value" size="sm" class="border-destructive/30 bg-destructive/5">
      <CardContent class="flex items-start gap-2.5 py-3 text-xs text-destructive">
        <RiAlertLine class="size-4 shrink-0 mt-px" />
        <div class="flex flex-col gap-0.5">
          <span class="font-medium">上次连接失败</span>
          <span class="text-destructive/80">{{ connectionStore.lastError.value }}</span>
        </div>
      </CardContent>
    </Card>

    <Card size="sm">
      <CardContent class="flex flex-col items-center justify-center gap-3 py-12 text-center">
        <div class="flex size-10 items-center justify-center rounded-full bg-muted text-muted-foreground">
          <RiPlugLine class="size-5" />
        </div>
        <div class="flex flex-col gap-1">
          <p class="text-sm font-medium text-foreground">没有活跃的连接</p>
          <p class="text-xs text-muted-foreground max-w-md">
            点击下方按钮去「发现」页选择一个 server
          </p>
        </div>
        <Button variant="default" size="sm" class="mt-2 gap-1.5" @click="backToDiscovery">
          <RiCompass3Line class="size-3.5" />
          去发现页
        </Button>
      </CardContent>
    </Card>
  </div>
</template>
