<script setup lang="ts">
/**
 * 设置视图 —— v0.1 阶段除「通用 / 语言切换」外其它配置都是只读占位。
 *
 * 当前展示项：
 *   - 通用：界面语言切换 (中 / EN)，写入 localStorage
 *   - 端口：HTTP / SOCKS5 / API,来自 status,禁用编辑
 *   - 安全：CIDR + CONNECT 端口白名单,禁用编辑
 *   - mDNS 广播：开关、广播名、服务类型,禁用编辑
 *   - 关于：版本 + 后端 / 前端 元信息
 */
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Separator } from '@/components/ui/separator'
import { Button } from '@/components/ui/button'
import { Alert, AlertDescription } from '@/components/ui/alert'
import {
  RiSettings4Line,
  RiShieldKeyholeLine,
  RiBroadcastLine,
  RiInformationLine,
  RiExternalLinkLine,
  RiTranslate2,
  RiLoader4Line,
} from '@remixicon/vue'
import { proxyStore } from '@/stores/proxy'
import { setLocale, type Locale, SUPPORTED_LOCALES } from '@/i18n'
import { checkForUpdate, openExternal } from '@/composables/useUpdateCheck'
import { useToast } from '@/composables/useToast'
import { APP_VERSION } from '@/lib/appVersion'

const { t, locale } = useI18n()
const toast = useToast()
const status = computed(() => proxyStore.state.status)
const allowedCidrs = ['192.168.0.0/16', '10.0.0.0/8', '172.16.0.0/12']
const allowedPorts = [80, 443, 22, 8080, 8443]

const checkingUpdate = ref(false)

function switchLocale(next: Locale) {
  if (locale.value === next) return
  setLocale(next)
}

async function onCheckUpdate() {
  if (checkingUpdate.value) return
  checkingUpdate.value = true
  try {
    const local = status.value?.version ?? APP_VERSION
    const result = await checkForUpdate(local)
    switch (result.outcome) {
      case 'up-to-date':
        toast.success(t('settings.about.upToDate'), {
          detail: t('settings.about.upToDateDetail', {
            local: result.local,
            latest: result.latest ?? '?',
          }),
        })
        break
      case 'update-available':
        toast.info(
          t('settings.about.updateAvailable', { latest: result.latest ?? '?' }),
          {
            detail: t('settings.about.updateAvailableDetail', {
              local: result.local,
            }),
            duration: 8000,
          },
        )
        // 用户点击 button = 明确表态想看,直接打开 release 页
        await openExternal(result.releaseUrl)
        break
      case 'no-release':
        toast.warn(t('settings.about.noRelease'), {
          detail: t('settings.about.noReleaseDetail'),
        })
        break
      case 'rate-limited':
        toast.warn(t('settings.about.rateLimited'), {
          detail: t('settings.about.rateLimitedDetail'),
        })
        break
      case 'network-error':
        toast.error(t('settings.about.networkError'), {
          detail: result.detail ?? t('settings.about.networkErrorDetail'),
        })
        break
    }
  } finally {
    checkingUpdate.value = false
  }
}
</script>

<template>
  <div class="mx-auto flex max-w-[960px] flex-col gap-5 p-6">
    <header>
      <h1 class="text-2xl font-semibold tracking-tight text-foreground">
        {{ t('settings.title') }}
      </h1>
      <p class="mt-1 text-sm text-muted-foreground">
        {{ t('settings.subtitle') }}
      </p>
    </header>

    <Alert variant="default">
      <RiInformationLine />
      <AlertDescription>
        <i18n-t keypath="settings.readonlyAlertBody" tag="span">
          <template #mdns>
            <code class="rounded bg-muted px-1 py-0.5 font-mono text-[11px]">--mdns-name "MyServer"</code>
          </template>
          <template #port>
            <code class="rounded bg-muted px-1 py-0.5 font-mono text-[11px]">--http-port 8080</code>
          </template>
        </i18n-t>
      </AlertDescription>
    </Alert>

    <!-- 通用 / General — 第一张卡,只放语言切换(将来可放暗色模式等) -->
    <Card>
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiTranslate2 class="size-3.5 text-foreground" />
          {{ t('settings.general.title') }}
        </CardTitle>
        <CardDescription class="text-xs">
          {{ t('settings.general.desc') }}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <div class="flex items-center justify-between">
          <Label class="text-xs font-medium text-foreground">
            {{ t('settings.general.languageLabel') }}
          </Label>
          <div
            class="flex items-center gap-0.5 rounded-md border border-border bg-muted/40 p-0.5"
            role="group"
          >
            <button
              v-for="loc in SUPPORTED_LOCALES"
              :key="loc.code"
              type="button"
              class="rounded-sm px-3 py-1 text-[11px] font-medium transition-colors"
              :class="locale === loc.code
                ? 'bg-background text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground'"
              @click="switchLocale(loc.code)"
            >
              {{ loc.label }}
            </button>
          </div>
        </div>
        <p class="text-[10px] text-muted-foreground">
          {{ t('settings.general.languageHint') }}
        </p>
      </CardContent>
    </Card>

    <Card>
      <CardHeader class="flex flex-row items-start justify-between gap-3 space-y-0">
        <div class="flex flex-col gap-1">
          <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
            <RiSettings4Line class="size-3.5 text-foreground" />
            {{ t('settings.ports.title') }}
          </CardTitle>
          <CardDescription class="text-xs">
            {{ t('settings.ports.desc') }}
          </CardDescription>
        </div>
        <span class="rounded-full border border-border/60 bg-muted/40 px-2 py-0.5 font-mono text-[10px] tracking-wide text-muted-foreground">
          {{ t('settings.readonly') }}
        </span>
      </CardHeader>
      <CardContent class="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <div class="flex flex-col gap-1.5">
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            {{ t('settings.ports.http') }}
          </Label>
          <Input
            :model-value="String(status?.http_port ?? '—')"
            readonly
            disabled
            :title="t('settings.ports.httpHint')"
            class="h-8 cursor-not-allowed bg-muted/40 font-mono text-sm tabular-nums opacity-90"
          />
        </div>
        <div class="flex flex-col gap-1.5">
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            {{ t('settings.ports.socks5') }}
          </Label>
          <Input
            :model-value="String(status?.socks5_port ?? '—')"
            readonly
            disabled
            :title="t('settings.ports.socks5Hint')"
            class="h-8 cursor-not-allowed bg-muted/40 font-mono text-sm tabular-nums opacity-90"
          />
        </div>
        <div class="flex flex-col gap-1.5">
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            {{ t('settings.ports.api') }}
          </Label>
          <Input
            :model-value="String(status?.api_port ?? '—')"
            readonly
            disabled
            :title="t('settings.ports.apiHint')"
            class="h-8 cursor-not-allowed bg-muted/40 font-mono text-sm tabular-nums opacity-90"
          />
        </div>
      </CardContent>
    </Card>

    <Card>
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiShieldKeyholeLine class="size-3.5 text-foreground" />
          {{ t('settings.security.title') }}
        </CardTitle>
        <CardDescription class="text-xs">
          {{ t('settings.security.desc') }}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <div>
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            {{ t('settings.security.allowedCidrs') }}
          </Label>
          <div class="mt-2 flex flex-wrap gap-1.5">
            <span
              v-for="c in allowedCidrs"
              :key="c"
              class="rounded-md bg-muted px-2 py-1 font-mono text-[11px] font-medium text-foreground"
            >
              {{ c }}
            </span>
          </div>
        </div>
        <Separator />
        <div>
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            {{ t('settings.security.allowedPorts') }}
          </Label>
          <div class="mt-2 flex flex-wrap gap-1.5">
            <span
              v-for="p in allowedPorts"
              :key="p"
              class="rounded-md bg-muted px-2 py-1 font-mono text-[11px] font-medium tabular-nums text-foreground"
            >
              {{ p }}
            </span>
          </div>
        </div>
      </CardContent>
    </Card>

    <Card>
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiBroadcastLine class="size-3.5 text-foreground" />
          {{ t('settings.mdns.title') }}
        </CardTitle>
        <CardDescription class="text-xs">
          {{ t('settings.mdns.desc') }}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <div class="flex items-center justify-between">
          <Label for="mdns-enable" class="cursor-pointer text-xs">
            {{ t('settings.mdns.enable') }}
          </Label>
          <Switch id="mdns-enable" :model-value="status?.mdns?.enabled ?? true" disabled />
        </div>
        <Separator />
        <div class="grid grid-cols-2 gap-3">
          <div class="flex flex-col gap-1.5">
            <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
              {{ t('settings.mdns.name') }}
            </Label>
            <Input
              :model-value="status?.mdns?.name ?? '—'"
              readonly
              disabled
              :title="t('settings.mdns.nameTitle', { cmd: '--mdns-name' })"
              class="h-8 cursor-not-allowed bg-muted/40 text-sm opacity-90"
            />
            <p class="text-[10px] text-muted-foreground">
              <i18n-t keypath="settings.mdns.nameHint" tag="span">
                <template #cmd>
                  <code class="font-mono">--mdns-name "MyServer"</code>
                </template>
              </i18n-t>
            </p>
          </div>
          <div class="flex flex-col gap-1.5">
            <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
              {{ t('settings.mdns.type') }}
            </Label>
            <Input
              :model-value="status?.mdns?.service_type ?? '_conduit._tcp.local.'"
              readonly
              disabled
              class="h-8 cursor-not-allowed bg-muted/40 font-mono text-sm opacity-90"
            />
          </div>
        </div>
      </CardContent>
    </Card>

    <Card>
      <CardHeader>
        <CardTitle class="text-[13px] font-semibold">{{ t('settings.about.title') }}</CardTitle>
      </CardHeader>
      <CardContent class="flex items-center justify-between">
        <div class="flex items-center gap-3">
          <div
            class="flex size-9 items-center justify-center rounded-md bg-primary text-primary-foreground"
          >
            <RiShieldKeyholeLine class="size-4" />
          </div>
          <div>
            <p class="text-sm font-semibold text-foreground">Conduit Server</p>
            <p class="font-mono text-xs text-muted-foreground">
              {{ status?.version ? `v${status.version}` : '--' }} · Tauri 2 + Vue 3 + Rust
            </p>
          </div>
        </div>
        <Button
          variant="outline"
          size="sm"
          class="h-8"
          :disabled="checkingUpdate"
          @click="onCheckUpdate"
        >
          <RiLoader4Line v-if="checkingUpdate" class="size-3.5 animate-spin" />
          <RiExternalLinkLine v-else class="size-3.5" />
          {{ checkingUpdate ? t('settings.about.checking') : t('settings.about.checkUpdate') }}
        </Button>
      </CardContent>
    </Card>
  </div>
</template>
