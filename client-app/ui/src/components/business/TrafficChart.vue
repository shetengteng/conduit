<script setup lang="ts">
/**
 * TrafficChart —— 60 秒滚动双线 SVG。
 *
 * 设计:
 *   - viewBox 固定 600 × 120,响应式 width=100%
 *   - 上行(emerald) / 下行(blue) 两条折线 + 半透明面积填充
 *   - 左侧 0,右侧最新;peak 自动作为 Y 轴上限
 *   - 不引入 chart 库,不依赖动画(每秒重渲染就是最自然的"动画")
 *   - 暗色模式适配 stroke-current + opacity
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { trafficStore } from '@/stores/trafficStore'

const { t } = useI18n()

const VW = 600
const VH = 120
const PAD_X = 4
const PAD_Y = 6

function fmtBytes(n: number | null | undefined): string {
  // 兜底:SSE payload 字段名漂移 / refresh() 前 store 字段尚未填充时不让 UI 显示 NaN
  const v = Number.isFinite(n as number) ? (n as number) : 0
  if (v < 1024) return `${v} B`
  if (v < 1024 * 1024) return `${(v / 1024).toFixed(1)} KB`
  if (v < 1024 * 1024 * 1024) return `${(v / 1024 / 1024).toFixed(2)} MB`
  return `${(v / 1024 / 1024 / 1024).toFixed(2)} GB`
}

function fmtRate(bytesPerSec: number | null | undefined): string {
  return `${fmtBytes(bytesPerSec)}/s`
}

function _path(values: number[], peak: number): string {
  if (values.length === 0 || peak <= 0) return ''
  const innerW = VW - PAD_X * 2
  const innerH = VH - PAD_Y * 2
  const stepX = values.length > 1 ? innerW / (trafficStore.WINDOW_SIZE - 1) : 0
  const offsetX = innerW - stepX * (values.length - 1)
  return values
    .map((v, i) => {
      const x = PAD_X + offsetX + stepX * i
      const y = VH - PAD_Y - (v / peak) * innerH
      return `${i === 0 ? 'M' : 'L'} ${x.toFixed(2)} ${y.toFixed(2)}`
    })
    .join(' ')
}

function _areaPath(values: number[], peak: number): string {
  const line = _path(values, peak)
  if (!line || values.length === 0) return ''
  const innerW = VW - PAD_X * 2
  const stepX = values.length > 1 ? innerW / (trafficStore.WINDOW_SIZE - 1) : 0
  const offsetX = innerW - stepX * (values.length - 1)
  const startX = PAD_X + offsetX
  const endX = PAD_X + offsetX + stepX * (values.length - 1)
  return `${line} L ${endX.toFixed(2)} ${VH - PAD_Y} L ${startX.toFixed(2)} ${VH - PAD_Y} Z`
}

const peak = computed(() => trafficStore.peakAny.value)
const uplinkValues = computed(() => trafficStore.samples.value.map((s) => s.uplink))
const downlinkValues = computed(() => trafficStore.samples.value.map((s) => s.downlink))

const upLine = computed(() => _path(uplinkValues.value, peak.value))
const upArea = computed(() => _areaPath(uplinkValues.value, peak.value))
const downLine = computed(() => _path(downlinkValues.value, peak.value))
const downArea = computed(() => _areaPath(downlinkValues.value, peak.value))

const isEmpty = computed(() => trafficStore.samples.value.length === 0)
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="grid grid-cols-2 gap-4 sm:grid-cols-4">
      <div class="flex flex-col gap-0.5">
        <span class="flex items-center gap-1.5 text-[11px] uppercase tracking-wide text-muted-foreground">
          <span class="size-1.5 rounded-full bg-emerald-500" />{{ t('traffic.upRate') }}
        </span>
        <span class="text-base font-mono font-medium text-foreground">{{ fmtRate(trafficStore.latestUplink.value) }}</span>
      </div>
      <div class="flex flex-col gap-0.5">
        <span class="flex items-center gap-1.5 text-[11px] uppercase tracking-wide text-muted-foreground">
          <span class="size-1.5 rounded-full bg-blue-500" />{{ t('traffic.downRate') }}
        </span>
        <span class="text-base font-mono font-medium text-foreground">{{ fmtRate(trafficStore.latestDownlink.value) }}</span>
      </div>
      <div class="flex flex-col gap-0.5">
        <span class="text-[11px] uppercase tracking-wide text-muted-foreground">{{ t('traffic.upTotal') }}</span>
        <span class="text-base font-mono font-medium text-foreground">{{ fmtBytes(trafficStore.totalUplink.value) }}</span>
      </div>
      <div class="flex flex-col gap-0.5">
        <span class="text-[11px] uppercase tracking-wide text-muted-foreground">{{ t('traffic.downTotal') }}</span>
        <span class="text-base font-mono font-medium text-foreground">{{ fmtBytes(trafficStore.totalDownlink.value) }}</span>
      </div>
    </div>

    <div class="relative w-full rounded-md border border-border bg-muted/20 p-1">
      <svg :viewBox="`0 0 ${VW} ${VH}`" preserveAspectRatio="none" class="block w-full" :style="{ height: `${VH}px` }">
        <!-- 网格 -->
        <line x1="0" :y1="VH * 0.5" :x2="VW" :y2="VH * 0.5"
              stroke="currentColor" stroke-opacity="0.08" stroke-dasharray="2 4" />
        <line x1="0" :y1="VH * 0.25" :x2="VW" :y2="VH * 0.25"
              stroke="currentColor" stroke-opacity="0.05" stroke-dasharray="2 4" />
        <line x1="0" :y1="VH * 0.75" :x2="VW" :y2="VH * 0.75"
              stroke="currentColor" stroke-opacity="0.05" stroke-dasharray="2 4" />

        <template v-if="!isEmpty">
          <path :d="downArea" fill="rgb(59 130 246)" fill-opacity="0.10" />
          <path :d="downLine" fill="none" stroke="rgb(59 130 246)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          <path :d="upArea" fill="rgb(16 185 129)" fill-opacity="0.10" />
          <path :d="upLine" fill="none" stroke="rgb(16 185 129)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        </template>
      </svg>
      <div v-if="isEmpty" class="absolute inset-0 flex items-center justify-center text-xs text-muted-foreground pointer-events-none">
        {{ t('traffic.waiting') }}
      </div>
    </div>

    <div class="flex items-center justify-between text-[10px] text-muted-foreground">
      <span>{{ t('traffic.windowDesc') }}</span>
      <span>{{ t('traffic.peak', { value: fmtRate(trafficStore.peakAny.value) }) }}</span>
    </div>
  </div>
</template>
