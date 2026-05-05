<script setup lang="ts">
/**
 * 启动失败页 —— Tauri 主进程 emit `boot:phase=failed` 时显示。
 *
 * 提供 3 个可执行操作：重试 / 查看日志 / 退出
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
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

const { t } = useI18n()

const hints = computed(() => [
  t('boot.failedHint1'),
  t('boot.failedHint2'),
  t('boot.failedHint3'),
])
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
            <CardTitle class="text-base">{{ t('boot.failedTitle') }}</CardTitle>
            <p class="mt-1 text-xs text-muted-foreground">
              {{ t('boot.failedSub') }}
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
          <p class="text-xs font-medium text-foreground">{{ t('boot.failedHints') }}</p>
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
          {{ t('boot.quit') }}
        </Button>
        <Button @click="emit('retry')">
          <RiRefreshLine />
          {{ t('boot.retry') }}
        </Button>
      </CardFooter>
    </Card>
  </div>
</template>
