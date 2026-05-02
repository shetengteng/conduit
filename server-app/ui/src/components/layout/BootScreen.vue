<script setup lang="ts">
/**
 * 启动加载页 —— Tauri 主进程 spawn → healthz 期间显示。
 *
 * 4 阶段进度链：分配端口 → 启动 sidecar → 健康检查 → 就绪
 */
import { Card, CardContent } from '@/components/ui/card'
import { RiLoader4Line, RiShieldKeyholeLine } from '@remixicon/vue'

const phases = [
  { label: '分配端口' },
  { label: '启动代理引擎' },
  { label: '健康检查' },
  { label: '就绪' },
]
</script>

<template>
  <div
    class="flex h-screen w-full items-center justify-center bg-gradient-to-br from-background to-muted/40"
  >
    <Card size="sm" class="w-[420px] !ring-foreground/15">
      <CardContent class="flex flex-col items-center gap-5 px-8 py-10">
        <div
          class="relative flex size-16 items-center justify-center rounded-2xl bg-primary/10"
        >
          <RiShieldKeyholeLine class="size-7 text-primary" />
          <RiLoader4Line
            class="absolute -right-2 -bottom-2 size-6 animate-spin text-primary"
          />
        </div>

        <div class="text-center">
          <h1 class="text-base font-semibold tracking-tight">
            Conduit Server
          </h1>
          <p class="mt-1 text-xs text-muted-foreground">
            正在启动代理引擎，请稍候…
          </p>
        </div>

        <div class="flex w-full items-center gap-2">
          <template v-for="(p, i) in phases" :key="p.label">
            <div class="flex flex-1 flex-col items-center gap-1">
              <div
                class="size-2 rounded-full bg-primary/40 animate-pulse-dot"
                :style="{ animationDelay: `${i * 0.2}s` }"
              />
              <span class="text-[10px] text-muted-foreground">
                {{ p.label }}
              </span>
            </div>
            <div
              v-if="i < phases.length - 1"
              class="-mt-3 h-px flex-1 bg-border"
            />
          </template>
        </div>
      </CardContent>
    </Card>
  </div>
</template>
