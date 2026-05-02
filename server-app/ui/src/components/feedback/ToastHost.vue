<script setup lang="ts">
/**
 * Toast 渲染容器 —— 基于 shadcn-vue Alert 组件 + 自实现的位置/动画。
 *
 * 数据来源：composables/useToast.ts 单例 reactive 数组。
 * 与 sonner 不同的是，我们直接复用项目现有的 toast 数据结构，避免改业务层。
 */
import { computed } from 'vue'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  RiCheckboxCircleLine,
  RiErrorWarningLine,
  RiAlertLine,
  RiInformationLine,
  RiCloseLine,
} from '@remixicon/vue'
import { useToast, type ToastTone } from '@/composables/useToast'

const toast = useToast()

const iconMap: Record<ToastTone, any> = {
  success: RiCheckboxCircleLine,
  error: RiErrorWarningLine,
  warn: RiAlertLine,
  info: RiInformationLine,
}

const variantMap: Record<ToastTone, string> = {
  success: 'border-status-ok/30 bg-status-ok/10 text-foreground',
  error: 'border-status-error/30 bg-status-error/10 text-foreground',
  warn: 'border-status-warn/30 bg-status-warn/10 text-foreground',
  info: 'border-status-info/30 bg-status-info/10 text-foreground',
}

const toneIconColor: Record<ToastTone, string> = {
  success: 'text-status-ok',
  error: 'text-status-error',
  warn: 'text-status-warn',
  info: 'text-status-info',
}

const items = computed(() => toast.items)
</script>

<template>
  <div
    class="pointer-events-none fixed top-4 right-4 z-50 flex w-[360px] flex-col gap-2"
  >
    <TransitionGroup
      enter-active-class="transition duration-200 ease-out"
      enter-from-class="translate-x-2 opacity-0"
      enter-to-class="translate-x-0 opacity-100"
      leave-active-class="transition duration-150 ease-in"
      leave-from-class="opacity-100"
      leave-to-class="opacity-0 translate-x-2"
    >
      <Alert
        v-for="t in items"
        :key="t.id"
        :class="['pointer-events-auto shadow-lg', variantMap[t.tone]]"
      >
        <component
          :is="iconMap[t.tone]"
          :class="['size-4', toneIconColor[t.tone]]"
        />
        <AlertTitle>{{ t.title }}</AlertTitle>
        <AlertDescription v-if="t.detail">
          {{ t.detail }}
        </AlertDescription>
        <div class="absolute top-1.5 right-1.5">
          <Button
            variant="ghost"
            size="icon-xs"
            class="size-5"
            @click="toast.dismiss(t.id)"
          >
            <RiCloseLine class="size-3" />
          </Button>
        </div>
      </Alert>
    </TransitionGroup>
  </div>
</template>
