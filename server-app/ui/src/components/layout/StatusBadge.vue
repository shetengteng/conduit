<script setup lang="ts">
/**
 * 状态徽章 —— 基于 shadcn-vue Badge 包装，统一映射 5 种业务状态：
 *   running  / stopped / warning / error / connecting
 *
 * 通过 CSS 变量绑定 `--status-*` token，与 light / dark 主题联动。
 */
import { computed } from 'vue'
import { Badge } from '@/components/ui/badge'

type Tone = 'running' | 'stopped' | 'warning' | 'error' | 'connecting'

const props = withDefaults(
  defineProps<{
    tone: Tone
    label: string
    pulse?: boolean
  }>(),
  { pulse: false },
)

const styleMap: Record<Tone, { bg: string; fg: string; dotBg: string }> = {
  running: {
    bg: 'bg-status-ok/15',
    fg: 'text-status-ok',
    dotBg: 'bg-status-ok',
  },
  stopped: {
    bg: 'bg-muted',
    fg: 'text-muted-foreground',
    dotBg: 'bg-muted-foreground',
  },
  warning: {
    bg: 'bg-status-warn/15',
    fg: 'text-status-warn',
    dotBg: 'bg-status-warn',
  },
  error: {
    bg: 'bg-status-error/15',
    fg: 'text-status-error',
    dotBg: 'bg-status-error',
  },
  connecting: {
    bg: 'bg-status-info/15',
    fg: 'text-status-info',
    dotBg: 'bg-status-info',
  },
}

const cls = computed(() => styleMap[props.tone])
</script>

<template>
  <Badge variant="outline" :class="['gap-1.5 border-transparent', cls.bg, cls.fg]">
    <span
      class="inline-block size-1.5 rounded-full"
      :class="[cls.dotBg, pulse && 'animate-pulse-dot']"
    />
    <span class="font-medium">{{ label }}</span>
  </Badge>
</template>
