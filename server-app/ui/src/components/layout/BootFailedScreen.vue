<script setup lang="ts">
/**
 * 启动失败页 —— Tauri 主进程 emit `boot:phase=failed` 时显示。
 *
 * 提供 3 个可执行操作：重试 / 查看日志 / 退出
 */
import { Card, CardContent, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Separator } from '@/components/ui/separator'
import {
  RiAlertLine,
  RiRefreshLine,
  RiCloseCircleLine,
  RiInformationLine,
} from '@remixicon/vue'

defineProps<{
  reason?: string | null
}>()

const emit = defineEmits<{
  (e: 'retry'): void
  (e: 'quit'): void
}>()

const hints = [
  '点「重试」再启动一次，sidecar 冷启动有时需要更久',
  '如反复失败，查看日志：~/.conduit/logs/proxy.log',
  '若怀疑环境问题，可重启电脑或重新安装 Conduit Server',
]
</script>

<template>
  <div
    class="flex h-screen w-full items-center justify-center bg-gradient-to-br from-background to-muted/40"
  >
    <Card class="w-[480px] !ring-status-error/30">
      <CardHeader>
        <div class="flex items-start gap-3">
          <div
            class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-status-error/15 text-status-error"
          >
            <RiAlertLine class="size-5" />
          </div>
          <div>
            <CardTitle class="text-base">代理引擎启动失败</CardTitle>
            <p class="mt-1 text-xs text-muted-foreground">
              Tauri 主进程未能在超时时间内完成健康检查
            </p>
          </div>
        </div>
      </CardHeader>

      <CardContent class="flex flex-col gap-3">
        <Alert v-if="reason" variant="destructive" class="!gap-1">
          <RiInformationLine />
          <AlertDescription class="font-mono text-[12px] break-all">
            {{ reason }}
          </AlertDescription>
        </Alert>

        <Separator />

        <div class="flex flex-col gap-2">
          <p class="text-xs font-medium text-foreground">可以这样做：</p>
          <ul class="flex flex-col gap-1.5">
            <li
              v-for="(h, idx) in hints"
              :key="idx"
              class="flex items-start gap-2 text-xs text-muted-foreground"
            >
              <span
                class="mt-0.5 inline-flex size-4 shrink-0 items-center justify-center rounded-full bg-muted font-mono text-[10px]"
              >
                {{ idx + 1 }}
              </span>
              <span>{{ h }}</span>
            </li>
          </ul>
        </div>
      </CardContent>

      <CardFooter class="flex justify-end gap-2 pb-4">
        <Button variant="outline" @click="emit('quit')">
          <RiCloseCircleLine />
          退出
        </Button>
        <Button @click="emit('retry')">
          <RiRefreshLine />
          重试
        </Button>
      </CardFooter>
    </Card>
  </div>
</template>
