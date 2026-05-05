<script setup lang="ts">
/**
 * 设置页 —— M-δ。
 *
 * 块:
 *   1. 运行时信息 (端口)
 *   2. 开机自启开关 (调用 Rust 命令写 ~/Library/LaunchAgents/com.conduit.client.plist)
 *   3. 手动连接 server (mDNS 不通时兜底)
 *   4. 路由缓存维护 (清空)
 *   5. 诊断入口 (跳到独立诊断页)
 *
 * 暂不实现 (留 M-ε):
 *   - 系统代理开关 toggle (cfg 启动时定型,运行时改要 sidecar 重启)
 *   - 日志文件路径选择
 *   - PAC 自定义路径
 */
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'
import { Separator } from '@/components/ui/separator'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import {
  RiSettings4Line,
  RiLinkM,
  RiDeleteBinLine,
  RiServerLine,
  RiAlertLine,
  RiStethoscopeLine,
  RiToggleLine,
  RiTranslate2,
  RiExternalLinkLine,
  RiLoader4Line,
  RiShieldKeyholeLine,
} from '@remixicon/vue'
import { getRuntime } from '@/api/runtime'
import { connectionStore } from '@/stores/connectionStore'
import { cacheStore } from '@/stores/cacheStore'
import { useToast } from '@/composables/useToast'
import { uiStore } from '@/stores/ui'
import type { AppRuntime } from '@/types/client'
import { invoke } from '@tauri-apps/api/core'
import { Switch } from '@/components/ui/switch'
import { setLocale, type Locale, SUPPORTED_LOCALES } from '@/i18n'
import { checkForUpdate, openExternal } from '@/composables/useUpdateCheck'

// 与 client-app/core/pyproject.toml 的 version 保持一致;打包时会被同步,
// 短期不会自动从 sidecar 拿(healthz 不暴露 version)。下次升级走 release tag。
const CLIENT_VERSION = '0.1.0'

const { t, locale } = useI18n()
const toast = useToast()
const runtime = ref<AppRuntime | null>(null)

const manualName = ref('')
const manualHost = ref('')
const manualPort = ref(8080)
const manualSocks = ref(8081)
const manualApi = ref(8090)
const manualBusy = ref(false)

const autostartEnabled = ref(false)
const autostartBusy = ref(false)
const autostartError = ref<string | null>(null)

const checkingUpdate = ref(false)

function switchLocale(next: Locale) {
  if (locale.value === next) return
  setLocale(next)
}

async function onCheckUpdate() {
  if (checkingUpdate.value) return
  checkingUpdate.value = true
  try {
    const result = await checkForUpdate(CLIENT_VERSION)
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
            detail: t('settings.about.updateAvailableDetail', { local: result.local }),
            duration: 8000,
          },
        )
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

onMounted(async () => {
  runtime.value = await getRuntime()
  await refreshAutostart()
})

async function refreshAutostart() {
  try {
    autostartEnabled.value = await invoke<boolean>('autostart_status')
    autostartError.value = null
  } catch (e) {
    autostartError.value = e instanceof Error ? e.message : String(e)
  }
}

async function toggleAutostart(next: boolean) {
  autostartBusy.value = true
  try {
    await invoke(next ? 'autostart_enable' : 'autostart_disable')
    autostartEnabled.value = next
    toast.success(
      next
        ? t('settings.autostart.toastEnabled')
        : t('settings.autostart.toastDisabled'),
    )
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    toast.error(t('settings.autostart.toastFail'), { detail: msg })
    await refreshAutostart()
  } finally {
    autostartBusy.value = false
  }
}

async function handleManualConnect() {
  if (!manualHost.value.trim() || !manualPort.value) {
    toast.error(t('settings.manual.toastNeedHostPort'))
    return
  }
  const name = manualName.value.trim() || `manual-${manualHost.value}`
  // server_id 格式必须与后端一致:name@host:port
  const serverId = `${name}@${manualHost.value.trim()}:${manualPort.value}`
  manualBusy.value = true
  try {
    // 后端目前仅支持从 discoverer.snapshot() 找 server_id;手动添加暂时只
    // 在 mDNS 已经发现过的情况下能用。为了不撒谎,这里先尝试从已知列表
    // 找,找不到给明确错误。完整的"手动添加"端点会在 M-δ 加。
    await connectionStore.connectTo(serverId)
    toast.success(t('settings.manual.toastTried', { id: serverId }))
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    toast.error(t('settings.manual.toastFail'), { detail: msg })
  } finally {
    manualBusy.value = false
  }
}

async function handleFlushCache() {
  try {
    await cacheStore.flush()
    toast.success(t('cache.toastFlushed'))
  } catch (e) {
    toast.error(t('cache.toastFlushFail'), {
      detail: e instanceof Error ? e.message : String(e),
    })
  }
}
</script>

<template>
  <div class="flex flex-col gap-6 p-6">
    <div class="flex flex-col gap-1">
      <h1 class="text-2xl font-extralight tracking-tight text-foreground">{{ t('settings.title') }}</h1>
      <p class="text-sm text-muted-foreground">
        {{ t('settings.sub') }}
      </p>
    </div>

    <!-- 通用 / General — 语言切换 -->
    <Card size="sm">
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiTranslate2 class="size-4 text-muted-foreground" />
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
          <div class="inline-flex h-7 items-center rounded-md border border-border bg-background p-0.5 text-[11px]">
            <button
              v-for="opt in SUPPORTED_LOCALES"
              :key="opt.code"
              type="button"
              :class="[
                'inline-flex h-6 cursor-pointer items-center rounded-[5px] px-2.5 transition-colors',
                locale === opt.code
                  ? 'bg-primary text-primary-foreground'
                  : 'text-muted-foreground hover:bg-accent hover:text-foreground',
              ]"
              @click="switchLocale(opt.code as Locale)"
            >
              {{ opt.label }}
            </button>
          </div>
        </div>
        <p class="text-[10px] text-muted-foreground">
          {{ t('settings.general.languageHint') }}
        </p>
      </CardContent>
    </Card>

    <!-- 运行时端口 -->
    <Card size="sm">
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiSettings4Line class="size-4 text-muted-foreground" />
          {{ t('settings.runtime.title') }}
        </CardTitle>
        <CardDescription class="text-xs">
          {{ t('settings.runtime.desc') }}
        </CardDescription>
      </CardHeader>
      <CardContent class="grid grid-cols-2 gap-3">
        <div class="flex flex-col gap-1.5">
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">{{ t('settings.runtime.socksPort') }}</Label>
          <span class="rounded-md bg-muted px-2 py-1 font-mono text-sm tabular-nums">{{ runtime?.socks_port ?? '—' }}</span>
        </div>
        <div class="flex flex-col gap-1.5">
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">{{ t('settings.runtime.apiPort') }}</Label>
          <span class="rounded-md bg-muted px-2 py-1 font-mono text-sm tabular-nums">{{ runtime?.api_port ?? '—' }}</span>
        </div>
      </CardContent>
    </Card>

    <!-- 开机自启 -->
    <Card size="sm">
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiToggleLine class="size-4 text-muted-foreground" />
          {{ t('settings.autostart.title') }}
        </CardTitle>
        <CardDescription class="text-xs">
          {{ t('settings.autostart.desc') }}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex items-center justify-between gap-3">
        <div class="flex flex-col gap-0.5 text-xs text-muted-foreground">
          <span>{{ t('settings.autostart.currentLabel') }}
            <span :class="autostartEnabled ? 'text-emerald-700 dark:text-emerald-400 font-medium' : 'text-muted-foreground'">
              {{ autostartEnabled ? t('settings.autostart.enabled') : t('settings.autostart.disabled') }}
            </span>
          </span>
          <span v-if="autostartError" class="text-destructive">{{ autostartError }}</span>
        </div>
        <Switch
          :model-value="autostartEnabled"
          :disabled="autostartBusy"
          @update:model-value="toggleAutostart"
        />
      </CardContent>
    </Card>

    <!-- 手动添加 server -->
    <Card size="sm">
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiServerLine class="size-4 text-muted-foreground" />
          {{ t('settings.manual.title') }}
        </CardTitle>
        <CardDescription class="text-xs">
          {{ t('settings.manual.desc') }}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
          <div class="flex flex-col gap-1">
            <Label class="text-[11px]">{{ t('settings.manual.name') }}</Label>
            <Input v-model="manualName" :placeholder="t('settings.manual.namePlaceholder')" class="h-8 text-xs" />
          </div>
          <div class="flex flex-col gap-1">
            <Label class="text-[11px]">{{ t('settings.manual.host') }}</Label>
            <Input v-model="manualHost" :placeholder="t('settings.manual.hostPlaceholder')" class="h-8 text-xs font-mono" />
          </div>
          <div class="flex flex-col gap-1">
            <Label class="text-[11px]">{{ t('settings.manual.httpPort') }}</Label>
            <Input v-model.number="manualPort" type="number" placeholder="8080" class="h-8 text-xs font-mono" />
          </div>
          <div class="flex flex-col gap-1">
            <Label class="text-[11px]">{{ t('settings.manual.socksPort') }}</Label>
            <Input v-model.number="manualSocks" type="number" placeholder="8081" class="h-8 text-xs font-mono" />
          </div>
          <div class="flex flex-col gap-1">
            <Label class="text-[11px]">{{ t('settings.manual.apiPort') }}</Label>
            <Input v-model.number="manualApi" type="number" placeholder="8090" class="h-8 text-xs font-mono" />
          </div>
        </div>
        <Button :disabled="manualBusy" class="w-fit gap-1.5" @click="handleManualConnect">
          <RiLinkM class="size-3.5" />{{ manualBusy ? t('settings.manual.btnBusy') : t('settings.manual.btn') }}
        </Button>
        <div class="flex items-start gap-2 rounded-md border border-amber-300/50 bg-amber-50/40 px-3 py-2 text-[11px] text-amber-900 dark:bg-amber-950/10 dark:text-amber-200">
          <RiAlertLine class="size-3.5 mt-0.5 shrink-0" />
          <i18n-t keypath="settings.manual.tip" tag="span">
            <template #code>
              <code class="font-mono">name@host:port</code>
            </template>
          </i18n-t>
        </div>
      </CardContent>
    </Card>

    <!-- 缓存维护 -->
    <Card size="sm">
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiDeleteBinLine class="size-4 text-muted-foreground" />
          {{ t('settings.cache.title') }}
        </CardTitle>
        <CardDescription class="text-xs">
          {{ t('settings.cache.desc') }}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex items-center justify-between gap-3">
        <div class="flex flex-col gap-0.5 text-xs text-muted-foreground">
          <span>{{ t('settings.cache.currentEntries') }}<span class="font-mono font-medium text-foreground">{{ cacheStore.entries.value.length }}</span></span>
          <span v-if="cacheStore.stats.value">{{ t('settings.cache.hitMiss') }}<span class="font-mono">{{ cacheStore.stats.value.hits }} / {{ cacheStore.stats.value.misses }}</span></span>
        </div>
        <Button variant="outline" :disabled="cacheStore.entries.value.length === 0" class="gap-1.5" @click="handleFlushCache">
          <RiDeleteBinLine class="size-3.5" />{{ t('settings.cache.btn') }}
        </Button>
      </CardContent>
    </Card>

    <!-- 诊断入口 (完整 5 步在独立诊断页) -->
    <Card size="sm">
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiStethoscopeLine class="size-4 text-muted-foreground" />
          {{ t('settings.diag.title') }}
        </CardTitle>
        <CardDescription class="text-xs">
          {{ t('settings.diag.desc') }}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex items-center justify-between gap-3">
        <p class="text-xs text-muted-foreground">
          {{ t('settings.diag.hint') }}
        </p>
        <Button variant="outline" size="sm" class="gap-1.5" @click="uiStore.setActive('diagnose')">
          <RiStethoscopeLine class="size-3.5" />
          {{ t('settings.diag.btn') }}
        </Button>
      </CardContent>
    </Card>

    <Separator />

    <Card size="sm">
      <CardHeader>
        <CardTitle class="text-[13px] font-semibold">{{ t('settings.about.title') }}</CardTitle>
      </CardHeader>
      <CardContent class="flex items-center justify-between gap-3">
        <div class="flex items-center gap-3">
          <div class="flex size-9 items-center justify-center rounded-md bg-primary text-primary-foreground">
            <RiShieldKeyholeLine class="size-4" />
          </div>
          <div class="flex flex-col gap-0.5 text-xs text-muted-foreground">
            <p class="text-sm font-semibold text-foreground">{{ t('settings.about.version') }}</p>
            <p>{{ t('settings.about.tagline') }}</p>
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
