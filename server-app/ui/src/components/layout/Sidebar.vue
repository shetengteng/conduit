<script setup lang="ts">
/**
 * 左侧导航栏 —— 仪表盘 / 日志 / 设置 三入口。
 *
 * 设计要点：
 * 1. 220px 固定宽度，< 1000px 折叠为 64px icon-only
 * 2. 选中态：左侧 3px 蓝条 + 蓝色文字 + 背景 hover token
 * 3. 折叠态用 shadcn Tooltip 显示完整 label
 * 4. 顶部品牌区固定 56px
 *
 * 语言切换在 SettingsView -> 通用设置 区域,而不是侧栏底部 ——
 * 减少侧栏视觉负担,且语言切换是「极少操作」(可能整个生命周期切一次),
 * 不值得占据全局可见的 sidebar 卡。
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
  RiDashboardLine,
  RiFileList3Line,
  RiSettings4Line,
  RiShieldKeyholeLine,
} from '@remixicon/vue'
import { uiStore, type NavKey } from '@/stores/ui'

const collapsed = useMediaQuery('(max-width: 999px)')
const { t } = useI18n()

const navItems = computed(() => [
  { key: 'dashboard' as NavKey, icon: RiDashboardLine, label: t('nav.dashboard') },
  { key: 'logs' as NavKey, icon: RiFileList3Line, label: t('nav.logs') },
  { key: 'settings' as NavKey, icon: RiSettings4Line, label: t('nav.settings') },
])

function go(key: NavKey) {
  uiStore.setActive(key)
}
</script>

<template>
  <aside
    class="flex h-full flex-col border-r border-border bg-sidebar text-sidebar-foreground transition-[width] duration-200 ease-out"
    :class="collapsed ? 'w-14' : 'w-[200px]'"
  >
    <!-- 品牌区 -->
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
        <span class="text-[10px] text-muted-foreground">Server</span>
      </div>
    </div>

    <!-- 导航列表 -->
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
                <component :is="item.icon" class="size-4 shrink-0" />
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

    <!-- 版本号 -->
    <Separator />
    <div
      class="flex h-10 shrink-0 items-center px-3 text-[11px] text-muted-foreground"
      :class="collapsed && 'justify-center px-0'"
    >
      <span v-if="!collapsed">v0.1.0</span>
      <span v-else class="font-mono text-[10px]">0.1</span>
    </div>
  </aside>
</template>
