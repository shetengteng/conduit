<script setup lang="ts">
/**
 * 仪表盘核心 KPI 卡片 —— 已链接客户端数 / 下行 / 上行 / 运行时长。
 *
 * 数据全部读自 proxyStore + trafficStore,本组件只做展示。
 * 布局: 2x2 KPI 网格(紧凑模式)。
 *
 * VPN 状态不再在此卡展示 —— 与右侧 NetworkPanel"VPN 出口"完全重复,
 * 引擎卡只承载流量/客户端 KPI。
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import StatusBadge from '../layout/StatusBadge.vue'
import { proxyStore } from '@/stores/proxy'
import { trafficStore } from '@/stores/traffic'
import { formatBpsValue, formatBpsUnit, formatUptimeShort } from '@/utils/format'

const status = computed(() => proxyStore.state.status)
const loading = computed(() => proxyStore.state.loading && !status.value)

const totalBps = trafficStore.totalBps

// 每秒 reactive tick,用于让"运行时长"在 polling 间隔(8s)之间也能平滑增长。
// backend 的 uptime_sec 是快照;UI 用 (snapshot + (now - statusFetchedAtMs)/1000) 外推。
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

const liveUptimeSec = computed(() => {
  const s = status.value
  if (!s || !s.running) return 0
  const snapshot = s.uptime_sec ?? 0
  const fetchedAt = proxyStore.state.statusFetchedAtMs
  if (!fetchedAt) return snapshot
  const drift = Math.max(0, Math.floor((nowMs.value - fetchedAt) / 1000))
  return snapshot + drift
})

/**
 * KPI 配置 —— B 风格采用左侧 2px 彩色细线 + 极细字重大数字作为视觉签名。
 * accent 字段对应 border-l-2 的颜色（用 Tailwind 直接量类）。
 */
const kpis = computed(() => {
  const s = status.value
  const t = totalBps.value
  const passiveCount = s?.passive_clients_count ?? 0
  const activeCount = s?.clients_count ?? 0
  const totalCount = activeCount + passiveCount
  let clientsSub: string
  if (totalCount === 0) {
    clientsSub = '等待客户端接入'
  } else if (passiveCount === 0) {
    clientsSub = `${activeCount} 个正在传输流量`
  } else if (activeCount === 0) {
    clientsSub = `${passiveCount} 个待命中(暂无流量)`
  } else {
    clientsSub = `${activeCount} 传输中 · ${passiveCount} 待命`
  }
  return [
    {
      key: 'clients',
      label: '已链接客户端',
      value: String(totalCount),
      unit: '个',
      sub: clientsSub,
      accent: 'border-l-foreground',
    },
    {
      key: 'down',
      label: '下行',
      value: formatBpsValue(t.in_bps),
      unit: formatBpsUnit(t.in_bps),
      sub: t.in_bps > 0 ? '同事 → 服务端' : '当前空闲',
      accent: 'border-l-emerald-500',
    },
    {
      key: 'up',
      label: '上行',
      value: formatBpsValue(t.out_bps),
      unit: formatBpsUnit(t.out_bps),
      sub: t.out_bps > 0 ? '服务端 → 同事' : '当前空闲',
      accent: 'border-l-amber-500',
    },
    {
      key: 'uptime',
      label: '运行时长',
      value: formatUptimeShort(liveUptimeSec.value),
      unit: '',
      sub: s?.running ? '稳定运行中' : '尚未启动',
      accent: 'border-l-border',
    },
  ]
})

</script>

<template>
  <Card size="sm" class="h-full">
    <CardHeader class="flex flex-row items-center justify-between">
      <CardTitle class="text-[13px] font-semibold">代理引擎</CardTitle>
      <StatusBadge
        :tone="status?.running ? 'running' : 'stopped'"
        :label="status?.running ? '运行中' : '未启动'"
        :pulse="status?.running"
      />
    </CardHeader>

    <CardContent>
      <div v-if="loading" class="grid grid-cols-2 gap-2">
        <div v-for="i in 4" :key="i" class="skeleton h-[68px]" />
      </div>

      <div v-else class="grid grid-cols-2 gap-3">
        <div
          v-for="kpi in kpis"
          :key="kpi.key"
          :class="[
            'group flex flex-col gap-1 border-l-2 pl-3 transition-colors',
            kpi.accent,
          ]"
        >
          <div class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            {{ kpi.label }}
          </div>
          <div class="flex items-baseline gap-1.5">
            <span class="font-sans text-[34px] font-extralight leading-none tracking-tight text-foreground tabular-nums">
              {{ kpi.value }}
            </span>
            <span v-if="kpi.unit" class="text-xs font-medium leading-none text-muted-foreground">
              {{ kpi.unit }}
            </span>
          </div>
          <div class="text-[11px] leading-tight text-muted-foreground">
            {{ kpi.sub }}
          </div>
        </div>
      </div>

    </CardContent>
  </Card>
</template>
