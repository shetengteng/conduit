<script setup lang="ts">
/**
 * 客户端左侧导航栏 —— 发现 / 已连接 / 设置 三入口。
 *
 * 设计要点（与 server-app Sidebar 同一套视觉规范，B 风格）：
 * 1. 200px 固定宽度，< 1000px 折叠为 56px icon-only
 * 2. 选中态：黑底白字（B 风格签名色）
 * 3. 折叠态用 shadcn Tooltip 显示完整 label
 * 4. 顶部品牌区固定 56px，与 TopStatusBar 视觉对齐
 * 5. 客户端品牌副标 "Client"（区别 server 的 "Server"）
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useMediaQuery } from '@vueuse/core'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import {
  RiCompass3Line,
  RiPlugLine,
  RiSettings4Line,
  RiShieldKeyholeLine,
  RiStethoscopeLine,
} from '@remixicon/vue'
import { uiStore, type NavKey } from '@/stores/ui'
import { connectionStore } from '@/stores/connectionStore'
import { APP_VERSION, APP_VERSION_LABEL } from '@/lib/appVersion'

const { t } = useI18n()
const collapsed = useMediaQuery('(max-width: 999px)')
const versionShort = computed(() => APP_VERSION.split('.').slice(0, 2).join('.'))

const navItems = computed(() => [
  { key: 'discovery' as NavKey, icon: RiCompass3Line, label: t('nav.discovery') },
  { key: 'connected' as NavKey, icon: RiPlugLine, label: t('nav.connected') },
  { key: 'diagnose' as NavKey, icon: RiStethoscopeLine, label: t('nav.diagnose') },
  { key: 'settings' as NavKey, icon: RiSettings4Line, label: t('nav.settings') },
])

// 「已连接」标签的指示色:connecting 黄,connected 绿,failed 红,idle/disconnecting 无。
const connectionIndicator = computed(() => {
  switch (connectionStore.connectionState.value) {
    case 'connecting': return 'bg-amber-500 animate-pulse'
    case 'connected': return 'bg-emerald-500'
    case 'failed':    return 'bg-destructive'
    default:          return null
  }
})

function go(key: NavKey) {
  uiStore.setActive(key)
}
</script>

<template>
  <aside
    class="flex h-full flex-col border-r border-border bg-sidebar text-sidebar-foreground transition-[width] duration-200 ease-out"
    :class="collapsed ? 'w-14' : 'w-[200px]'"
  >
    <div
      class="flex h-14 shrink-0 items-center gap-2 border-b border-border px-3"
      :class="collapsed && 'justify-center px-0'"
    >
      <div
        class="flex size-7 items-center justify-center rounded-md bg-primary text-primary-foreground"
      >
        <RiShieldKeyholeLine class="size-3.5" />
      </div>
      <div v-if="!collapsed" class="flex flex-col leading-tight">
        <span class="text-[13px] font-semibold tracking-tight">Conduit</span>
        <span class="text-[10px] text-muted-foreground">Client</span>
      </div>
    </div>

    <nav class="flex flex-1 flex-col gap-0.5 px-2 py-3">
      <TooltipProvider :delay-duration="200">
        <template v-for="item in navItems" :key="item.key">
          <Tooltip :disable-hoverable-content="!collapsed">
            <TooltipTrigger as-child>
              <Button
                variant="ghost"
                :class="[
                  'h-8 w-full justify-start rounded-md px-2.5 text-[13px] font-medium',
                  uiStore.state.active === item.key
                    ? 'bg-primary text-primary-foreground hover:bg-primary hover:text-primary-foreground'
                    : 'text-muted-foreground hover:bg-accent hover:text-foreground',
                  collapsed && 'justify-center px-0',
                ]"
                @click="go(item.key)"
              >
                <span class="relative inline-flex shrink-0">
                  <component :is="item.icon" class="size-4" />
                  <span
                    v-if="item.key === 'connected' && connectionIndicator"
                    :class="[
                      'absolute -right-0.5 -top-0.5 size-1.5 rounded-full ring-2 ring-sidebar',
                      connectionIndicator,
                    ]"
                    aria-hidden
                  />
                </span>
                <span v-if="!collapsed">{{ item.label }}</span>
              </Button>
            </TooltipTrigger>
            <TooltipContent v-if="collapsed" side="right">
              {{ item.label }}
            </TooltipContent>
          </Tooltip>
        </template>
      </TooltipProvider>
    </nav>

    <Separator />
    <div
      class="flex h-10 shrink-0 items-center px-3 text-[11px] text-muted-foreground"
      :class="collapsed && 'justify-center px-0'"
    >
      <span v-if="!collapsed">{{ APP_VERSION_LABEL }}</span>
      <span v-else class="font-mono text-[10px]">{{ versionShort }}</span>
    </div>
  </aside>
</template>
