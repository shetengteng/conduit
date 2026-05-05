<script setup lang="ts">
/**
 * ConnectingProgress —— 5 步竖向 stepper，显示连接进度。
 *
 * 数据源:connectionStore.state.progress（key → {status, detail}）。
 * 视觉:每步是个圆点 + 标签 + detail，左侧用一根细线连接,
 *      已完成的线段用 foreground 色,未到达的用 border 色。
 *
 * 三态色彩:
 *   - 未开始:灰色边框圆 + muted 文字
 *   - running:深色实心圆 + spinner 动效 + foreground 文字
 *   - ok:emerald 边框 + 勾,foreground 文字
 *   - failed:destructive 边框 + 叉,destructive 文字
 *
 * 取消按钮 v0 只是 disconnect()。M-β.3 再做"取消未完成"。
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Button } from '@/components/ui/button'
import { Card, CardHeader, CardContent, CardTitle, CardDescription } from '@/components/ui/card'
import { RiCheckLine, RiCloseLine, RiLoader4Line, RiAlertLine, RiCloseCircleLine } from '@remixicon/vue'
import { connectionStore } from '@/stores/connectionStore'
import { discoveryStore } from '@/stores/discoveryStore'

const { t } = useI18n()
const STEP_KEYS = connectionStore.STEP_ORDER
const STEP_LABEL_KEYS = connectionStore.STEP_LABEL_KEYS

const pendingServer = computed(() => {
  const id = connectionStore.pendingServerId.value
  if (!id) return null
  return discoveryStore.servers.value.find((s) => s.server_id === id) ?? null
})

const stepDescriptors = computed(() =>
  STEP_KEYS.map((key, idx) => {
    const p = connectionStore.state.progress[key]
    return {
      idx: idx + 1,
      key,
      label: t(STEP_LABEL_KEYS[key]),
      status: p?.status ?? 'pending',
      detail: p?.detail ?? '',
    }
  }),
)

async function handleCancel() {
  try {
    await connectionStore.disconnect()
  } catch (_) {
    // 错误已被 store 落到 lastError,这里静默
  }
}
</script>

<template>
  <div class="flex flex-col gap-6 p-6">
    <div class="flex items-start justify-between gap-4">
      <div class="flex flex-col gap-1">
        <h1 class="text-2xl font-extralight tracking-tight text-foreground">{{ t('connecting.title') }}</h1>
        <p class="text-sm text-muted-foreground">
          <template v-if="pendingServer">
            {{ t('connecting.targetWith', {
              name: pendingServer.name,
              host: pendingServer.host,
              port: pendingServer.port,
            }) }}
          </template>
          <template v-else>{{ t('connecting.targetWithout') }}</template>
        </p>
      </div>
      <Button variant="outline" size="sm" class="gap-1.5" @click="handleCancel">
        <RiCloseLine class="size-3.5" />{{ t('connecting.cancel') }}
      </Button>
    </div>

    <Card v-if="connectionStore.lastError.value" size="sm" class="border-destructive/30 bg-destructive/5">
      <CardContent class="flex items-start gap-2.5 py-3 text-xs text-destructive">
        <RiAlertLine class="size-4 shrink-0 mt-px" />
        <div class="flex flex-col gap-0.5">
          <span class="font-medium">{{ t('connecting.failedTitle') }}</span>
          <span class="text-destructive/80">{{ connectionStore.lastError.value }}</span>
        </div>
      </CardContent>
    </Card>

    <Card size="sm">
      <CardHeader class="pb-2">
        <CardTitle class="text-[13px] font-semibold">{{ t('connecting.panelTitle') }}</CardTitle>
        <CardDescription class="text-xs">{{ t('connecting.panelDesc') }}</CardDescription>
      </CardHeader>
      <CardContent class="pt-2">
        <ol class="relative flex flex-col">
          <li
            v-for="(step, i) in stepDescriptors"
            :key="step.key"
            class="relative flex gap-3 pb-5 last:pb-0"
          >
            <span
              v-if="i < stepDescriptors.length - 1"
              :class="[
                'absolute left-[11px] top-7 -bottom-1 w-px',
                stepDescriptors[i].status === 'ok' ? 'bg-foreground/40' : 'bg-border',
              ]"
              aria-hidden
            />

            <span
              :class="[
                'relative z-10 mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full border text-[10px] font-semibold transition-colors',
                step.status === 'ok'
                  ? 'border-emerald-500 bg-emerald-500 text-white'
                  : step.status === 'running'
                    ? 'border-foreground bg-foreground text-background'
                    : step.status === 'failed'
                      ? 'border-destructive bg-destructive text-white'
                      : 'border-border bg-background text-muted-foreground',
              ]"
            >
              <RiCheckLine v-if="step.status === 'ok'" class="size-3.5" />
              <RiLoader4Line v-else-if="step.status === 'running'" class="size-3.5 animate-spin" />
              <RiCloseCircleLine v-else-if="step.status === 'failed'" class="size-3.5" />
              <span v-else>{{ step.idx }}</span>
            </span>

            <div class="flex flex-1 flex-col gap-0.5 pt-px">
              <span
                :class="[
                  'text-[13px] font-medium leading-snug',
                  step.status === 'failed' ? 'text-destructive' : 'text-foreground',
                ]"
              >{{ step.label }}</span>
              <span
                v-if="step.detail"
                :class="[
                  'text-[11px] leading-snug',
                  step.status === 'failed' ? 'text-destructive/80' : 'text-muted-foreground',
                ]"
              >{{ step.detail }}</span>
              <span v-else class="text-[11px] leading-snug text-muted-foreground">
                {{ step.status === 'pending' ? t('connecting.stepWaiting') : t('connecting.stepRunning') }}
              </span>
            </div>
          </li>
        </ol>
      </CardContent>
    </Card>
  </div>
</template>
