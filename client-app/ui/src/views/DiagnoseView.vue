<script setup lang="ts">
/**
 * 诊断视图 —— M-δ。
 *
 * 5 步自检（与 client-app/core/api/diagnose.py 严格 1:1）:
 *   1. Sidecar 进程
 *   2. mDNS 服务发现
 *   3. 上游 Server 可达
 *   4. PAC 文件
 *   5. 系统代理
 *
 * 交互:
 *   - 进入页面自动跑一次
 *   - 顶部「重新检测」按钮一键重跑
 *   - 整体可一键复制为多行文本（贴到 issue / 群里方便排错）
 *   - 任何 ok=false 的检查项都会展开 remediation 文案
 *
 * 设计与 server-app 一致 B 风格:净白底 + 细边框 + extralight 标题。
 */
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import {
  RiStethoscopeLine,
  RiRefreshLine,
  RiClipboardLine,
  RiCheckboxCircleFill,
  RiErrorWarningFill,
  RiInformationLine,
  RiServerLine,
  RiBroadcastLine,
  RiGlobeLine,
  RiFileTextLine,
  RiSettings4Line,
  RiAlertLine,
} from '@remixicon/vue'
import DiagnoseRow from '@/components/business/DiagnoseRow.vue'
import { ClientApi } from '@/api/client-api'
import { useToast } from '@/composables/useToast'
import type { Component } from 'vue'
import type { DiagnoseResponse } from '@/types/client'

const { t, locale } = useI18n()
const toast = useToast()

const loading = ref(false)
const lastError = ref<string | null>(null)
const result = ref<DiagnoseResponse | null>(null)

const ICON_MAP: Record<string, Component> = {
  sidecar: RiServerLine,
  mdns: RiBroadcastLine,
  server_reach: RiGlobeLine,
  pac: RiFileTextLine,
  system_proxy: RiSettings4Line,
}

function iconFor(key: string): Component {
  return ICON_MAP[key] ?? RiInformationLine
}

const overallOk = computed(() => result.value?.ok ?? false)
const failedCount = computed(() =>
  result.value ? result.value.checks.filter((c) => !c.ok).length : 0,
)
const checkedAtLabel = computed(() => {
  if (!result.value) return t('diagnose.lastRunNever')
  const localeArg = locale.value === 'zh-CN' ? 'zh-CN' : 'en-US'
  const time = new Date(result.value.checked_at * 1000).toLocaleTimeString(localeArg, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  })
  return t('diagnose.lastRun', { time })
})

async function run() {
  loading.value = true
  lastError.value = null
  try {
    result.value = await ClientApi.diagnose()
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    lastError.value = msg
    toast.error(t('diagnose.toastRunFail'), { detail: msg })
  } finally {
    loading.value = false
  }
}

async function copyReport() {
  if (!result.value) return
  const lines: string[] = []
  lines.push(t('diagnose.reportHeader', {
    time: new Date(result.value.checked_at * 1000).toISOString(),
  }))
  lines.push(t('diagnose.reportOverall', {
    status: result.value.ok ? 'OK' : 'FAILED',
  }))
  lines.push('')
  for (const c of result.value.checks) {
    lines.push(`[${c.ok ? 'OK' : 'FAIL'}] ${c.label} (${c.key})`)
    lines.push(`  ${c.detail}`)
    if (!c.ok && c.remediation) {
      const remediation = c.remediation
        .split('\n')
        .map((l) => `    ${l}`)
        .join('\n')
      lines.push(`${t('diagnose.reportRemediation')}\n${remediation}`)
    }
    lines.push('')
  }
  const text = lines.join('\n').trim() + '\n'
  try {
    await navigator.clipboard.writeText(text)
    toast.success(t('diagnose.toastCopied'))
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    toast.error(t('diagnose.toastCopyFail'), { detail: msg })
  }
}

onMounted(() => {
  run()
})
</script>

<template>
  <div class="flex flex-col gap-6 p-6">
    <div class="flex items-start justify-between gap-4">
      <div class="flex flex-col gap-1">
        <h1 class="text-2xl font-extralight tracking-tight text-foreground flex items-center gap-3">
          <RiStethoscopeLine class="size-6 text-muted-foreground" />
          {{ t('diagnose.title') }}
        </h1>
        <p class="text-sm text-muted-foreground">
          {{ t('diagnose.sub') }}
        </p>
      </div>
      <div class="flex items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          :disabled="loading || !result"
          class="gap-1.5"
          @click="copyReport"
        >
          <RiClipboardLine class="size-3.5" />
          {{ t('diagnose.copy') }}
        </Button>
        <Button size="sm" :disabled="loading" class="gap-1.5" @click="run">
          <RiRefreshLine :class="['size-3.5', loading && 'animate-spin']" />
          {{ loading ? t('diagnose.running') : t('diagnose.rerun') }}
        </Button>
      </div>
    </div>

    <!-- 总状态卡 -->
    <Card>
      <CardHeader class="flex flex-row items-center justify-between gap-3 space-y-0 pb-3">
        <div class="flex items-center gap-3">
          <div
            :class="[
              'flex size-10 items-center justify-center rounded-md',
              result === null
                ? 'bg-muted text-muted-foreground'
                : overallOk
                  ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
                  : 'bg-destructive/10 text-destructive',
            ]"
          >
            <RiCheckboxCircleFill v-if="overallOk && result" class="size-5" />
            <RiErrorWarningFill v-else-if="result && !overallOk" class="size-5" />
            <RiStethoscopeLine v-else class="size-5" />
          </div>
          <div class="flex flex-col gap-0.5">
            <CardTitle class="text-base font-semibold tracking-tight">
              {{ result === null ? t('diagnose.statusReady') : overallOk ? t('diagnose.statusOk') : t('diagnose.statusFail', { n: failedCount }) }}
            </CardTitle>
            <CardDescription class="text-xs">
              {{ checkedAtLabel }}
            </CardDescription>
          </div>
        </div>
        <span
          v-if="result"
          :class="[
            'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-medium',
            overallOk
              ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-400'
              : 'bg-destructive/10 text-destructive',
          ]"
        >
          {{ overallOk ? 'OK' : 'FAILED' }}
        </span>
      </CardHeader>
    </Card>

    <!-- 拉接口失败兜底 -->
    <Card v-if="lastError" size="sm" class="border-destructive/30 bg-destructive/5">
      <CardContent class="flex items-start gap-2.5 py-3 text-xs text-destructive">
        <RiAlertLine class="size-4 shrink-0 mt-px" />
        <div class="flex flex-col gap-0.5">
          <span class="font-medium">{{ t('diagnose.fetchFailTitle') }}</span>
          <span class="text-destructive/80">{{ lastError }}</span>
          <span class="text-destructive/70">{{ t('diagnose.fetchFailHint') }}</span>
        </div>
      </CardContent>
    </Card>

    <!-- 5 项检查 -->
    <Card v-if="result" size="sm">
      <CardHeader class="pb-2">
        <CardTitle class="text-[13px] font-semibold">{{ t('diagnose.listTitle') }}</CardTitle>
        <CardDescription class="text-xs">
          {{ t('diagnose.listSub') }}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col">
        <template v-for="(check, idx) in result.checks" :key="check.key">
          <Separator v-if="idx > 0" />
          <DiagnoseRow :check="check" :icon="iconFor(check.key)" />
        </template>
      </CardContent>
    </Card>
  </div>
</template>
