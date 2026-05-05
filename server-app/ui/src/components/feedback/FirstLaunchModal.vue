<script setup lang="ts">
/**
 * 首次启动风险确认弹窗 —— 基于 shadcn-vue Dialog。
 *
 * 同意后写入 localStorage["conduit:first-launch-acknowledged"]，下次启动跳过。
 */
import { ref, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { Label } from '@/components/ui/label'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { RiAlertLine, RiShieldKeyholeLine } from '@remixicon/vue'

const { t } = useI18n()

const STORAGE_KEY = 'conduit:first-launch-acknowledged'

const open = ref(false)
const acknowledged = ref(false)

onMounted(() => {
  const acked = localStorage.getItem(STORAGE_KEY) === '1'
  if (!acked) open.value = true
})

const risks = computed(() => [
  t('firstLaunch.risk1'),
  t('firstLaunch.risk2'),
  t('firstLaunch.risk3'),
])

function confirm() {
  if (!acknowledged.value) return
  localStorage.setItem(STORAGE_KEY, '1')
  open.value = false
}

function cancel() {
  open.value = false
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="!max-w-[480px]">
      <DialogHeader>
        <div class="mb-1 flex items-center gap-2">
          <div
            class="flex size-8 items-center justify-center rounded-lg bg-status-warn/15 text-status-warn"
          >
            <RiAlertLine class="size-4" />
          </div>
          <DialogTitle>{{ t('firstLaunch.title') }}</DialogTitle>
        </div>
        <DialogDescription>
          {{ t('firstLaunch.desc') }}
        </DialogDescription>
      </DialogHeader>

      <Alert variant="destructive">
        <RiShieldKeyholeLine />
        <AlertDescription>
          {{ t('firstLaunch.avoidTitle') }}
        </AlertDescription>
      </Alert>

      <ul class="flex flex-col gap-1.5 pl-1">
        <li
          v-for="(r, i) in risks"
          :key="i"
          class="flex items-start gap-2 text-xs text-muted-foreground"
        >
          <span class="mt-0.5 inline-block size-1 shrink-0 rounded-full bg-status-error" />
          {{ r }}
        </li>
      </ul>

      <p class="text-xs text-muted-foreground">
        {{ t('firstLaunch.recommend') }}
      </p>

      <div
        class="flex items-center gap-2 rounded-lg border border-border bg-muted/40 p-2.5"
      >
        <Switch v-model="acknowledged" id="ack-switch" />
        <Label for="ack-switch" class="text-xs cursor-pointer">
          {{ t('firstLaunch.ack') }}
        </Label>
      </div>

      <DialogFooter>
        <Button variant="outline" @click="cancel">{{ t('firstLaunch.cancel') }}</Button>
        <Button :disabled="!acknowledged" @click="confirm">{{ t('firstLaunch.start') }}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
