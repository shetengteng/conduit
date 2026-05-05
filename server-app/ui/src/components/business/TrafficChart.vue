<script setup lang="ts">
/**
 * 实时流量曲线 —— SVG 折线 + 渐变填充。
 *
 * 复用 useTrafficSeries composable 做布局计算（纯函数 → 单测友好）。
 * 上下行通过 shadcn-vue Tabs 切换。
 *
 * 高度策略：固定 160 像素 —— 给一屏内剩下的客户端表 + ShareCard 留位。
 */
import { ref, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Badge } from '@/components/ui/badge'
import { useTrafficSeries, type TrafficDirection } from '@/composables/useTrafficSeries'
import { trafficStore } from '@/stores/traffic'
import { formatBps } from '@/utils/format'

const { t } = useI18n()

const direction = ref<TrafficDirection>('in')

const W = 720
const H = 160
const PAD_X = 12
const PAD_Y = 12

const { seriesPaths, sampleCount, peakBps } = useTrafficSeries({
  width: W,
  height: H,
  padX: PAD_X,
  padY: PAD_Y,
  direction,
})

const isEmpty = computed(() => sampleCount.value === 0)
const lastTickIso = computed(() => {
  const ts = trafficStore.state.lastTickTs
  if (!ts) return ''
  return new Date(ts * 1000).toLocaleTimeString()
})
</script>

<template>
  <Card size="sm">
    <CardHeader class="flex flex-row items-center justify-between">
      <div class="flex items-center gap-3">
        <CardTitle class="text-[13px] font-semibold">{{ t('traffic.title') }}</CardTitle>
        <span class="font-mono text-[11px] text-muted-foreground">
          {{ t('traffic.window', { sec: trafficStore.state.windowSec, n: sampleCount }) }}
        </span>
        <Badge variant="outline" class="font-mono text-[10px]">
          {{ t('traffic.peak', { value: formatBps(peakBps) }) }}
        </Badge>
      </div>
      <Tabs v-model="direction">
        <TabsList class="h-7 bg-muted p-0.5">
          <TabsTrigger
            value="in"
            class="px-2.5 py-1 text-[11px] font-medium data-[state=active]:bg-card data-[state=active]:text-foreground data-[state=active]:shadow-sm"
          >
            {{ t('traffic.in') }}
          </TabsTrigger>
          <TabsTrigger
            value="out"
            class="px-2.5 py-1 text-[11px] font-medium data-[state=active]:bg-card data-[state=active]:text-foreground data-[state=active]:shadow-sm"
          >
            {{ t('traffic.out') }}
          </TabsTrigger>
        </TabsList>
      </Tabs>
    </CardHeader>

    <CardContent>
      <div class="relative w-full">
        <svg
          :viewBox="`0 0 ${W} ${H}`"
          preserveAspectRatio="none"
          class="h-[160px] w-full"
          aria-hidden="true"
        >
          <g stroke="var(--border)" stroke-dasharray="2 4" stroke-width="0.8">
            <line :x1="PAD_X" :x2="W - PAD_X" :y1="H * 0.25" :y2="H * 0.25" />
            <line :x1="PAD_X" :x2="W - PAD_X" :y1="H * 0.5" :y2="H * 0.5" />
            <line :x1="PAD_X" :x2="W - PAD_X" :y1="H * 0.75" :y2="H * 0.75" />
          </g>

          <g v-for="(s, i) in seriesPaths" :key="s.peer">
            <defs>
              <linearGradient :id="`grad-${i}`" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" :stop-color="s.color" stop-opacity="0.35" />
                <stop offset="100%" :stop-color="s.color" stop-opacity="0" />
              </linearGradient>
            </defs>
            <path :d="s.area" :fill="`url(#grad-${i})`" />
            <path
              :d="s.line"
              fill="none"
              :stroke="s.color"
              stroke-width="1.75"
              stroke-linejoin="round"
              stroke-linecap="round"
            />
          </g>
        </svg>

        <div
          v-if="isEmpty"
          class="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-1"
        >
          <p class="text-[13px] font-medium text-foreground">{{ t('traffic.waitingTitle') }}</p>
          <p class="text-[11px] text-muted-foreground">
            {{ t('traffic.waitingDesc') }}
          </p>
        </div>
      </div>

      <div
        v-if="seriesPaths.length"
        class="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px]"
      >
        <div
          v-for="s in seriesPaths"
          :key="s.peer"
          class="inline-flex items-center gap-1.5"
        >
          <span class="size-2 rounded-full" :style="{ background: s.color }" />
          <span class="font-mono">{{ s.peer }}</span>
          <span class="text-muted-foreground">{{ formatBps(s.peakBps) }}</span>
        </div>
        <span class="ml-auto font-mono text-muted-foreground">
          {{ lastTickIso }}
        </span>
      </div>
    </CardContent>
  </Card>
</template>
