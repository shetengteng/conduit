<script setup lang="ts">
/**
 * 启动加载页 —— Tauri 主进程 spawn → healthz 期间显示。
 *
 * 4 阶段进度链：分配端口 → 启动 sidecar → 健康检查 → 就绪
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Card, CardContent } from '@/components/ui/card'
import { RiLoader4Line, RiShieldKeyholeLine } from '@remixicon/vue'

const { t } = useI18n()

const phases = computed(() => [
  { label: t('boot.phase.port') },
  { label: t('boot.phase.engine') },
  { label: t('boot.phase.health') },
  { label: t('boot.phase.ready') },
])
</script>

<template>
  <div
    class="flex h-screen w-full items-center justify-center bg-gradient-to-br from-background to-muted/40"
  >
    <Card size="sm" class="w-[520px] !ring-foreground/15">
      <CardContent class="flex flex-col items-center gap-6 px-10 py-12">
        <div class="flex size-16 items-center justify-center">
          <RiLoader4Line class="size-12 animate-spin text-primary" />
        </div>

        <div class="text-center">
          <h1
            class="flex items-center justify-center gap-2 text-base font-semibold tracking-tight"
          >
            <RiShieldKeyholeLine class="size-4 text-primary" />
            {{ t('boot.splashTitle') }}
          </h1>
          <p class="mt-1 text-xs text-muted-foreground">
            {{ t('boot.splashSub') }}
          </p>
        </div>

        <div class="flex w-full items-start gap-1.5">
          <template v-for="(p, i) in phases" :key="p.label">
            <div class="flex flex-1 flex-col items-center gap-1.5">
              <div
                class="size-2 rounded-full bg-primary/40 animate-pulse-dot"
                :style="{ animationDelay: `${i * 0.2}s` }"
              />
              <span class="whitespace-nowrap text-xs text-muted-foreground">
                {{ p.label }}
              </span>
            </div>
            <div
              v-if="i < phases.length - 1"
              class="mt-[3px] h-px flex-1 bg-border"
            />
          </template>
        </div>
      </CardContent>
    </Card>
  </div>
</template>
