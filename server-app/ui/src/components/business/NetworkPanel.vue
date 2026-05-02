<script setup lang="ts">
/**
 * 本机网络 + 健康检查面板。
 *
 * 数据：
 *   - status.vpn / lan：来自 /api/status
 *   - healthz.checks：来自 /healthz（5 项 named check：http_port / socks5_port / api_port / lan_ip / vpn_tunnel）
 *
 * 设计：
 *   - h-full 跟随同行 ProxyControl 的高度，让两卡底部对齐
 *   - 健康检查 5 项始终显示完整列表（未启动时显示占位状态），保证信息密度恒定
 *   - 项中文标签 + icon，避免裸技术名 "http_port" 直接示人
 */
import { computed } from 'vue'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Separator } from '@/components/ui/separator'
import {
  RiCheckLine,
  RiCloseLine,
  RiSubtractLine,
  RiRouterLine,
  RiShareForwardLine,
} from '@remixicon/vue'
import { proxyStore } from '@/stores/proxy'

const status = computed(() => proxyStore.state.status)
const healthz = computed(() => proxyStore.state.healthz)
const loading = computed(() => proxyStore.state.loading && !status.value)

const lanDetail = computed(() => status.value?.lan?.detail ?? '未检测')
const vpnIface = computed(() => status.value?.vpn?.iface ?? '—')
const vpnRoute = computed(() => Boolean(status.value?.vpn?.default_route_via_vpn))

/**
 * 5 项健康检查的人话标签映射（与 healthcheck.py CheckResult.name 严格对齐）。
 */
const CHECK_LABELS: Record<string, string> = {
  http_port: 'HTTP 代理端口',
  socks5_port: 'SOCKS5 代理端口',
  api_port: '管控 API 端口',
  lan_ip: '局域网 IP 检测',
  vpn_tunnel: 'VPN 隧道',
}
const CHECK_ORDER = ['http_port', 'socks5_port', 'api_port', 'lan_ip', 'vpn_tunnel'] as const

interface CheckRow {
  key: string
  label: string
  state: 'ok' | 'fail' | 'pending'
  detail: string
}

const rows = computed<CheckRow[]>(() => {
  const real = new Map(healthz.value?.checks?.map((c) => [c.name, c]) ?? [])
  return CHECK_ORDER.map((key) => {
    const c = real.get(key)
    return {
      key,
      label: CHECK_LABELS[key] ?? key,
      state: c ? (c.ok ? 'ok' : 'fail') : 'pending',
      detail: c?.detail ?? '尚未检测',
    }
  })
})

const passCount = computed(() => rows.value.filter((r) => r.state === 'ok').length)
const totalCount = computed(() => rows.value.length)
</script>

<template>
  <Card size="sm" class="h-full">
    <CardHeader class="flex flex-row items-center justify-between">
      <CardTitle class="text-[13px] font-semibold">本机网络</CardTitle>
      <span
        class="rounded-full px-2 py-0.5 font-mono text-[10px] font-semibold tabular-nums"
        :class="
          passCount === totalCount
            ? 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300'
            : 'bg-amber-50 text-amber-700 dark:bg-amber-950/40 dark:text-amber-300'
        "
      >
        {{ passCount }} / {{ totalCount }} 通过
      </span>
    </CardHeader>

    <CardContent>
      <div v-if="loading" class="flex flex-col gap-2">
        <div class="skeleton h-5 w-2/3" />
        <div class="skeleton h-5 w-1/2" />
        <div class="skeleton h-32" />
      </div>

      <div v-else class="flex flex-col gap-2">
        <div class="flex items-center gap-2 text-xs">
          <RiShareForwardLine class="size-3.5 text-muted-foreground" />
          <span class="text-muted-foreground">LAN</span>
          <span class="ml-auto truncate font-mono font-medium">{{ lanDetail }}</span>
        </div>
        <div class="flex items-center gap-2 text-xs">
          <RiRouterLine class="size-3.5 text-muted-foreground" />
          <span class="text-muted-foreground">VPN</span>
          <span class="font-mono font-medium">{{ vpnIface }}</span>
          <span
            class="ml-auto text-[10px]"
            :class="vpnRoute ? 'text-status-ok' : 'text-muted-foreground'"
          >
            {{ vpnRoute ? '默认路由 → VPN' : '未走 VPN' }}
          </span>
        </div>

        <Separator />

        <ul class="flex flex-col">
          <li
            v-for="r in rows"
            :key="r.key"
            class="flex items-center gap-2 rounded-sm px-1 py-1 transition-colors hover:bg-muted/40"
          >
            <span
              class="flex size-3.5 items-center justify-center rounded-full"
              :class="{
                'bg-status-ok/15 text-status-ok': r.state === 'ok',
                'bg-status-error/15 text-status-error': r.state === 'fail',
                'bg-muted text-muted-foreground': r.state === 'pending',
              }"
            >
              <component
                :is="r.state === 'ok' ? RiCheckLine : r.state === 'fail' ? RiCloseLine : RiSubtractLine"
                class="size-2.5"
              />
            </span>
            <span class="text-xs">{{ r.label }}</span>
            <span
              class="ml-auto truncate font-mono text-[10px] text-muted-foreground"
              :title="r.detail"
            >
              {{ r.detail }}
            </span>
          </li>
        </ul>
      </div>
    </CardContent>
  </Card>
</template>
