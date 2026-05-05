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
import { useI18n } from 'vue-i18n'
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

const { t } = useI18n()

const status = computed(() => proxyStore.state.status)
const healthz = computed(() => proxyStore.state.healthz)
const loading = computed(() => proxyStore.state.loading && !status.value)

const lanDetail = computed(() => status.value?.lan?.detail ?? t('network.lanUndetected'))
const vpnIface = computed(() => status.value?.vpn?.iface ?? '—')
const vpnRoute = computed(() => Boolean(status.value?.vpn?.default_route_via_vpn))

/**
 * 端口健康检查的人话标签映射(与 healthcheck.py CheckResult.name 严格对齐)。
 *
 * 设计:lan_ip / vpn_tunnel 这两项的 detail 已经在面板顶部"LAN" / "VPN" 行
 * 完整展示,在下面 listen 状态列表里再写一遍纯重复,所以这里只保留 3 个
 * 端口探活检查 —— 这是顶部 KPI 没有的信息(端口是否真的 listening)。
 */
const CHECK_ORDER = ['http_port', 'socks5_port', 'api_port'] as const
const CHECK_LABEL_KEYS: Record<(typeof CHECK_ORDER)[number], string> = {
  http_port: 'network.httpPort',
  socks5_port: 'network.socks5Port',
  api_port: 'network.apiPort',
}

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
      label: t(CHECK_LABEL_KEYS[key]),
      state: c ? (c.ok ? 'ok' : 'fail') : 'pending',
      detail: c?.detail ?? t('network.pendingDetail'),
    }
  })
})

const passCount = computed(() => rows.value.filter((r) => r.state === 'ok').length)
const totalCount = computed(() => rows.value.length)
</script>

<template>
  <Card size="sm" class="h-full">
    <CardHeader class="flex flex-row items-center justify-between">
      <CardTitle class="text-[13px] font-semibold">{{ t('network.title') }}</CardTitle>
      <span
        class="rounded-full px-2 py-0.5 font-mono text-[10px] font-semibold tabular-nums"
        :class="
          passCount === totalCount
            ? 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300'
            : 'bg-amber-50 text-amber-700 dark:bg-amber-950/40 dark:text-amber-300'
        "
      >
        {{ t('network.pass', { passed: passCount, total: totalCount }) }}
      </span>
    </CardHeader>

    <CardContent>
      <div v-if="loading" class="flex flex-col gap-2">
        <div class="skeleton h-5 w-2/3" />
        <div class="skeleton h-5 w-1/2" />
        <div class="skeleton h-32" />
      </div>

      <div v-else class="flex flex-col gap-2.5">
        <div class="flex flex-col gap-0.5 rounded-md bg-muted/40 px-2 py-1.5 text-xs">
          <div class="flex items-center gap-2">
            <RiShareForwardLine class="size-3.5 text-muted-foreground" />
            <span class="font-medium text-muted-foreground">{{ t('network.lanEgress') }}</span>
            <span
              class="ml-auto text-[10px] uppercase tracking-wide"
              :class="status?.lan?.available ? 'text-status-ok' : 'text-status-warn'"
            >
              {{ status?.lan?.available ? t('network.lanDetected') : t('network.lanUndetected') }}
            </span>
          </div>
          <p class="ml-5 break-all font-mono text-[11px] text-foreground">
            {{ lanDetail }}
          </p>
        </div>

        <div class="flex flex-col gap-0.5 rounded-md bg-muted/40 px-2 py-1.5 text-xs">
          <div class="flex items-center gap-2">
            <RiRouterLine class="size-3.5 text-muted-foreground" />
            <span class="font-medium text-muted-foreground">{{ t('network.vpnEgress') }}</span>
            <span
              class="ml-auto text-[10px] uppercase tracking-wide"
              :class="vpnRoute ? 'text-status-ok' : 'text-muted-foreground'"
            >
              {{ vpnRoute ? t('network.vpnViaVpn') : t('network.vpnNotViaVpn') }}
            </span>
          </div>
          <p class="ml-5 break-all font-mono text-[11px] text-foreground">
            {{ vpnIface }}
          </p>
        </div>

        <Separator />

        <p class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
          {{ t('network.portsListening') }}
        </p>
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
