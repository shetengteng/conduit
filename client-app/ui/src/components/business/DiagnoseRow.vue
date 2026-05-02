<script setup lang="ts">
/**
 * 诊断单行 —— 给 DiagnoseView 复用 5 次。
 *
 * 视觉:
 *   - 左侧大方块图标 (来自 DiagnoseView 的 ICON_MAP 映射)
 *   - 中间标题 + ok/fail 徽章 + key
 *   - 描述文本 (whitespace-pre-line 保留 \n)
 *   - 失败时再额外渲染琥珀色 remediation 块
 */
import type { Component } from 'vue'
import type { DiagnoseCheck } from '@/types/client'

defineProps<{
  check: DiagnoseCheck
  icon: Component
}>()
</script>

<template>
  <div class="flex items-start gap-3 py-3">
    <div
      :class="[
        'flex size-8 shrink-0 items-center justify-center rounded-md',
        check.ok
          ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
          : 'bg-destructive/10 text-destructive',
      ]"
    >
      <component :is="icon" class="size-4" />
    </div>
    <div class="flex flex-1 flex-col gap-1 min-w-0">
      <div class="flex flex-wrap items-center gap-2">
        <span class="text-[13px] font-medium text-foreground">{{ check.label }}</span>
        <span
          :class="[
            'inline-flex items-center rounded-full px-1.5 py-0.5 text-[10px] font-medium tracking-wide uppercase',
            check.ok
              ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-400'
              : 'bg-destructive/10 text-destructive',
          ]"
        >
          {{ check.ok ? 'OK' : 'FAIL' }}
        </span>
        <span class="text-[10px] font-mono text-muted-foreground/70">{{ check.key }}</span>
      </div>
      <p class="text-xs text-muted-foreground break-words whitespace-pre-line">
        {{ check.detail }}
      </p>
      <div
        v-if="!check.ok && check.remediation"
        class="mt-1 rounded-md border border-amber-300/40 bg-amber-50/40 px-3 py-2 text-[11px] text-amber-900 dark:bg-amber-950/10 dark:text-amber-200 whitespace-pre-line"
      >
        {{ check.remediation }}
      </div>
    </div>
  </div>
</template>
