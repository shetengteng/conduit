<script setup lang="ts">
/**
 * 仪表盘核心 KPI 卡片 —— 在线客户端数 / 总流量 / 运行时长 / VPN 状态。
 *
 * 数据全部读自 proxyStore + trafficStore，本组件只做展示。
 * 布局：2x2 KPI 网格（紧凑模式）+ 底部 VPN 状态行。
 *   - tile 内 icon 移到右上角 + 大数字主体 + 副信息槽，避免一行只有一个数字的"广告位"感
 *   - 数字尺寸提到 22px / mono / tabular-nums
 */
import { computed } from 'vue'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Separator } from '@/components/ui/separator'
import { RiShieldCheckLine, RiShieldCrossLine } from '@remixicon/vue'
import StatusBadge from '../layout/StatusBadge.vue'
import { proxyStore } from '@/stores/proxy'
import { trafficStore } from '@/stores/traffic'
import { formatBpsValue, formatBpsUnit, formatUptimeShort } from '@/utils/format'

const status = computed(() => proxyStore.state.status)
const loading = computed(() => proxyStore.state.loading && !status.value)

const totalBps = trafficStore.totalBps

/**
 * KPI 配置 —— B 风格采用左侧 2px 彩色细线 + 极细字重大数字作为视觉签名。
 * accent 字段对应 border-l-2 的颜色（用 Tailwind 直接量类）。
 */
const kpis = computed(() => {
  const s = status.value
  const t = totalBps.value
  return [
    {
      key: 'clients',
      label: '在线客户端',
      value: String(s?.clients_count ?? 0),
      unit: '个',
      sub: s?.clients_count ? '正在使用代理' : '等待客户端接入',
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
      value: formatUptimeShort(s?.uptime_sec ?? 0),
      unit: '',
      sub: s?.running ? '稳定运行中' : '尚未启动',
      accent: 'border-l-border',
    },
  ]
})

const vpnAvailable = computed(() => Boolean(status.value?.vpn?.available))
const vpnIface = computed(() => status.value?.vpn?.iface ?? '—')
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

      <Separator class="my-2.5" />

      <div class="flex items-center gap-2 text-xs">
        <component
          :is="vpnAvailable ? RiShieldCheckLine : RiShieldCrossLine"
          class="size-3.5"
          :class="vpnAvailable ? 'text-status-ok' : 'text-status-warn'"
        />
        <span class="text-muted-foreground">VPN 出口</span>
        <span class="font-mono font-medium">{{ vpnIface }}</span>
        <span
          class="ml-auto text-[10px] uppercase tracking-wide"
          :class="vpnAvailable ? 'text-status-ok' : 'text-status-warn'"
        >
          {{ vpnAvailable ? '可用' : '未检测到' }}
        </span>
      </div>
    </CardContent>
  </Card>
</template>
