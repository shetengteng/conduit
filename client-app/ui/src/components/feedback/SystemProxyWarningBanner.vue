<script setup lang="ts">
/**
 * 全局警示横幅 —— 已连接 server 但 system_proxy 未自动切换。
 *
 * 触发条件:
 *   connectionState === 'connected' && system_proxy_active === false
 *
 * 这种场景在 macOS 13+ 没给应用 admin 权限时会发生(networksetup 调用失败)。
 * 连接本身可用,只是浏览器要手动配 SOCKS5 才能走代理。
 *
 * 行为:
 *   - 显示琥珀色横幅,告诉用户具体的 SOCKS5 主机:端口
 *   - "查看详情" 按钮跳到诊断页(那里有完整的修复建议)
 *   - "我已知道" 按钮仅本会话隐藏,下次启动还会出现
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

const visible = computed(() => {
  if (dismissed.value) return false
  if (connectionStore.connectionState.value !== 'connected') return false
  if (connectionStore.systemProxyActive.value) return false
  return true
})

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
      <span class="font-medium">{{ t('proxyBanner.title') }}</span>
      <i18n-t v-if="socksPort" keypath="proxyBanner.bodyWithPort" tag="span">
        <template #code>
          <code class="rounded bg-amber-200/60 px-1.5 py-0.5 font-mono text-xs dark:bg-amber-900/50">
            SOCKS5 127.0.0.1:{{ socksPort }}
          </code>
        </template>
      </i18n-t>
      <span v-else>{{ t('proxyBanner.bodyNoPort') }}</span>
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
