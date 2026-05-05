<script setup lang="ts">
/**
 * 接入信息卡 —— 让同事一键复制即可接入。
 *
 * 三种接入方式(PAC 推荐 / HTTP / SOCKS5)平铺展示,无 tab 切换:
 *   - 用户反馈"只有 3 行,放在一起"
 *   - 每行: 模式 badge + 端点 + 复制按钮 + 一句使用场景
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { RiFileCopy2Line, RiInformationLine, RiAlertLine } from '@remixicon/vue'
import { proxyStore } from '@/stores/proxy'
import { useToast } from '@/composables/useToast'

const { t } = useI18n()
const toast = useToast()

const status = computed(() => proxyStore.state.status)
const running = computed(() => Boolean(status.value?.running))

/**
 * 单一 host 来源：从 pac_url 解析出后端 advertised host（与 HTTP/SOCKS5 监听 host 一致）。
 *
 * 不再依赖 `lan.detail` 字符串正则 —— 后端 `_pac_url()` 已经在 `pac_advertised_host` /
 * `bind` / `0.0.0.0` 三种情况下做好兜底（0.0.0.0 时返回 null）。
 */
const advertisedHost = computed<string | null>(() => {
  const url = status.value?.pac_url
  if (!url) return null
  try {
    return new URL(url).hostname || null
  } catch {
    return null
  }
})

const pacUrl = computed(() => status.value?.pac_url ?? '')

const httpEndpoint = computed(() => {
  const h = advertisedHost.value
  const p = status.value?.http_port
  return h && p ? `${h}:${p}` : ''
})

const socksEndpoint = computed(() => {
  const h = advertisedHost.value
  const p = status.value?.socks5_port
  return h && p ? `${h}:${p}` : ''
})

/**
 * 警告条件：代理已运行但拿不到对外可达 host。
 *
 * 此时 `pac_url` 多半是 null（后端在 host == "0.0.0.0" 时直接返回 null），
 * 同事即便手动填 host:port 也连不上 —— 只能等用户重启并指定 --pac-host 或确保
 * 物理网卡有私有 IP。
 */
const noReachableHost = computed(
  () => running.value && !advertisedHost.value,
)
const lanAvailable = computed(() => Boolean(status.value?.lan?.available))

async function copy(text: string, label: string) {
  if (!text) return
  try {
    await navigator.clipboard.writeText(text)
    toast.success(t('share.toastCopied', { label }), { detail: text })
  } catch (e) {
    toast.error(t('share.toastCopyFail'), { detail: String(e) })
  }
}
</script>

<template>
  <Card size="sm" class="h-full">
    <CardHeader class="flex flex-row items-baseline justify-between">
      <CardTitle class="text-[13px] font-semibold">{{ t('share.title') }}</CardTitle>
      <span class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        {{ t('share.subtitle') }}
      </span>
    </CardHeader>

    <CardContent class="flex flex-col gap-2.5">
      <Alert v-if="!running" variant="default" class="py-2">
        <RiInformationLine />
        <AlertDescription class="text-[11px]">
          {{ t('share.notRunning') }}
        </AlertDescription>
      </Alert>

      <Alert v-else-if="noReachableHost" variant="destructive" class="py-2">
        <RiAlertLine />
        <AlertDescription class="text-[11px]">
          <i18n-t keypath="share.noReachable" tag="span">
            <template #flag>
              <code class="font-mono">--pac-host</code>
            </template>
          </i18n-t>
        </AlertDescription>
      </Alert>

      <Alert
        v-else-if="running && !lanAvailable"
        variant="default"
        class="border-status-warn/30 bg-status-warn/10 py-2"
      >
        <RiAlertLine class="text-status-warn" />
        <AlertDescription class="text-[11px]">
          <i18n-t keypath="share.notLan" tag="span">
            <template #host>
              <code class="font-mono">{{ advertisedHost }}</code>
            </template>
          </i18n-t>
        </AlertDescription>
      </Alert>

      <!-- 平铺三行接入方式: PAC / HTTP / SOCKS5。无 tab,所有信息一眼看完。 -->
      <div class="flex flex-col gap-2">
        <!-- PAC: 强烈推荐,显著标识 -->
        <div class="flex flex-col gap-1">
          <div class="flex items-baseline justify-between gap-2">
            <div class="flex items-center gap-1.5">
              <span class="rounded-sm bg-primary px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-primary-foreground">
                {{ t('share.pacBadge') }}
              </span>
              <span class="text-[11px] font-medium text-foreground">{{ t('share.pacTitle') }}</span>
            </div>
            <span class="text-[10px] text-muted-foreground">{{ t('share.pacHint') }}</span>
          </div>
          <div class="flex items-stretch overflow-hidden rounded-md border border-border bg-muted/30 transition-colors hover:border-border hover:bg-muted/50">
            <code
              v-if="pacUrl"
              class="flex-1 select-all overflow-x-auto whitespace-nowrap py-1.5 px-2.5 font-mono text-[11px] leading-tight"
              :title="pacUrl"
            >
              {{ pacUrl }}
            </code>
            <span
              v-else
              class="flex-1 py-1.5 px-2.5 text-[11px] italic text-muted-foreground"
            >
              {{ t('share.placeholder') }}
            </span>
            <Button
              variant="ghost"
              size="sm"
              class="h-auto rounded-none border-l border-border px-2.5"
              :disabled="!pacUrl"
              @click="copy(pacUrl, t('share.pacLabel'))"
            >
              <RiFileCopy2Line class="size-3.5" />
            </Button>
          </div>
        </div>

        <!-- HTTP -->
        <div class="flex flex-col gap-1">
          <div class="flex items-baseline justify-between gap-2">
            <span class="text-[11px] font-medium text-foreground">{{ t('share.httpTitle') }}</span>
            <span class="text-[10px] text-muted-foreground">{{ t('share.httpHint') }}</span>
          </div>
          <div class="flex items-stretch overflow-hidden rounded-md border border-border bg-muted/30 transition-colors hover:border-border hover:bg-muted/50">
            <code
              v-if="httpEndpoint"
              class="flex-1 select-all overflow-x-auto whitespace-nowrap py-1.5 px-2.5 font-mono text-[11px] font-medium leading-tight"
            >
              {{ httpEndpoint }}
            </code>
            <span
              v-else
              class="flex-1 py-1.5 px-2.5 text-[11px] italic text-muted-foreground"
            >
              {{ t('share.placeholder') }}
            </span>
            <Button
              variant="ghost"
              size="sm"
              class="h-auto rounded-none border-l border-border px-2.5"
              :disabled="!httpEndpoint"
              @click="copy(httpEndpoint, t('share.httpLabel'))"
            >
              <RiFileCopy2Line class="size-3.5" />
            </Button>
          </div>
        </div>

        <!-- SOCKS5 -->
        <div class="flex flex-col gap-1">
          <div class="flex items-baseline justify-between gap-2">
            <span class="text-[11px] font-medium text-foreground">{{ t('share.socksTitle') }}</span>
            <span class="text-[10px] text-muted-foreground">{{ t('share.socksHint') }}</span>
          </div>
          <div class="flex items-stretch overflow-hidden rounded-md border border-border bg-muted/30 transition-colors hover:border-border hover:bg-muted/50">
            <code
              v-if="socksEndpoint"
              class="flex-1 select-all overflow-x-auto whitespace-nowrap py-1.5 px-2.5 font-mono text-[11px] font-medium leading-tight"
            >
              {{ socksEndpoint }}
            </code>
            <span
              v-else
              class="flex-1 py-1.5 px-2.5 text-[11px] italic text-muted-foreground"
            >
              {{ t('share.placeholder') }}
            </span>
            <Button
              variant="ghost"
              size="sm"
              class="h-auto rounded-none border-l border-border px-2.5"
              :disabled="!socksEndpoint"
              @click="copy(socksEndpoint, t('share.socksLabel'))"
            >
              <RiFileCopy2Line class="size-3.5" />
            </Button>
          </div>
        </div>
      </div>
    </CardContent>
  </Card>
</template>
