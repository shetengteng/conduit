<script setup lang="ts">
/**
 * 全局警示横幅 —— 已连接 server 但浏览器流量没真的走 conduit。
 *
 * 触发条件(任一满足即弹):
 *   1. 连接成功但 system_proxy_active === false
 *      (macOS 13+ 没授权 / SC commit 失败 / 平台不支持)
 *   2. 连接成功 & system_proxy_active === true 但 system_proxy_overridden
 *      (SC commit 成功了,回查发现 SOCKSEnable 被外部 daemon 改回 0;
 *       常见于公司装了 Zoom workplace dev / Okta Verify / MDM 之类的
 *       代理管控工具,它们持续监听 SCPreferences 变更并立刻覆写)
 *
 * 两种情况文案不同:
 *   case 1 → "系统代理切换失败, 请手动配 SOCKS5"
 *   case 2 → "代理设置被企业工具覆盖, 请手动配 SOCKS5"
 *
 * 共享:都给当前 SOCKS5 host:port + 复制按钮 + 诊断页跳转。
 */
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Button } from '@/components/ui/button'
import { RiAlertLine, RiCloseLine, RiStethoscopeLine } from '@remixicon/vue'
import { connectionStore } from '@/stores/connectionStore'
import { clientStore } from '@/stores/clientStore'
import { uiStore } from '@/stores/ui'
import { useToast } from '@/composables/useToast'

const { t } = useI18n()
const dismissed = ref(false)
const toast = useToast()

const isOverridden = computed(
  () =>
    connectionStore.systemProxyActive.value &&
    connectionStore.systemProxyOverridden.value,
)

const visible = computed(() => {
  if (dismissed.value) return false
  if (connectionStore.connectionState.value !== 'connected') return false
  // case 1: system_proxy 没切换成功
  if (!connectionStore.systemProxyActive.value) return true
  // case 2: 切换"成功"但被外部覆盖
  if (isOverridden.value) return true
  return false
})

const titleKey = computed(() =>
  isOverridden.value ? 'proxyBanner.titleOverridden' : 'proxyBanner.title',
)

const bodyKeyWithPort = computed(() =>
  isOverridden.value
    ? 'proxyBanner.bodyOverriddenWithPort'
    : 'proxyBanner.bodyWithPort',
)

const bodyKeyNoPort = computed(() =>
  isOverridden.value
    ? 'proxyBanner.bodyOverriddenNoPort'
    : 'proxyBanner.bodyNoPort',
)

const socksPort = computed(() => {
  // healthz 里 local_proxy detail 形如 "socks5 on 127.0.0.1:18498"
  const h = clientStore.state.healthz
  if (!h) return null
  const lp = h.checks.find((c) => c.name === 'local_proxy')
  if (!lp || !lp.detail) return null
  const m = lp.detail.match(/127\.0\.0\.1:(\d+)/)
  return m ? Number(m[1]) : null
})

function gotoDiagnose() {
  uiStore.setActive('diagnose')
}

async function copyConfig() {
  if (!socksPort.value) return
  const text = `SOCKS5 ${socksPort.value === null ? '<port>' : `127.0.0.1:${socksPort.value}`}`
  try {
    await navigator.clipboard.writeText(text)
    toast.success(t('proxyBanner.toastCopied'), { detail: text })
  } catch {
    toast.error(t('proxyBanner.toastCopyFail'), {
      detail: t('proxyBanner.toastCopyFailHint'),
    })
  }
}
</script>

<template>
  <div
    v-if="visible"
    class="flex items-start gap-3 border-b border-amber-200 bg-amber-50 px-6 py-2.5 text-sm text-amber-900 dark:border-amber-900/40 dark:bg-amber-950/40 dark:text-amber-200"
  >
    <RiAlertLine class="mt-0.5 size-4 shrink-0" />
    <div class="flex-1 leading-relaxed">
      <span class="font-medium">{{ t(titleKey) }}</span>
      <i18n-t v-if="socksPort" :keypath="bodyKeyWithPort" tag="span">
        <template #code>
          <code class="rounded bg-amber-200/60 px-1.5 py-0.5 font-mono text-xs dark:bg-amber-900/50">
            SOCKS5 127.0.0.1:{{ socksPort }}
          </code>
        </template>
      </i18n-t>
      <span v-else>{{ t(bodyKeyNoPort) }}</span>
    </div>
    <div class="flex shrink-0 items-center gap-1.5">
      <Button
        v-if="socksPort"
        variant="ghost"
        size="sm"
        class="h-7 px-2 text-xs text-amber-900 hover:bg-amber-200/40 dark:text-amber-200 dark:hover:bg-amber-900/40"
        @click="copyConfig"
      >
        {{ t('proxyBanner.copy') }}
      </Button>
      <Button
        variant="ghost"
        size="sm"
        class="h-7 px-2 text-xs text-amber-900 hover:bg-amber-200/40 dark:text-amber-200 dark:hover:bg-amber-900/40"
        @click="gotoDiagnose"
      >
        <RiStethoscopeLine class="size-3.5" />
        {{ t('proxyBanner.detail') }}
      </Button>
      <Button
        variant="ghost"
        size="sm"
        class="h-7 w-7 p-0 text-amber-900 hover:bg-amber-200/40 dark:text-amber-200 dark:hover:bg-amber-900/40"
        :title="t('proxyBanner.dismissTitle')"
        @click="dismissed = true"
      >
        <RiCloseLine class="size-4" />
      </Button>
    </div>
  </div>
</template>
