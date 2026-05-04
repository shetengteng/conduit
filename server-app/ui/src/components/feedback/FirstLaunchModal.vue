<script setup lang="ts">
/**
 * 首次启动风险确认弹窗 —— 基于 shadcn-vue Dialog。
 *
 * 同意后写入 localStorage["conduit:first-launch-acknowledged"]，下次启动跳过。
 */
import { ref, onMounted } from 'vue'
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

const STORAGE_KEY = 'conduit:first-launch-acknowledged'

const open = ref(false)
const acknowledged = ref(false)

onMounted(() => {
  const acked = localStorage.getItem(STORAGE_KEY) === '1'
  if (!acked) open.value = true
})

const risks = [
  '你的电脑由公司 IT 管理且有合规审计',
  '你在公司公共 WiFi、客户场地等不受控网络',
  '你不确定 LAN 上还有谁能访问到本机',
]

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
          <DialogTitle>首次启动确认</DialogTitle>
        </div>
        <DialogDescription>
          你即将启动一个把本机 VPN 共享给局域网的代理服务，请先确认使用场景
        </DialogDescription>
      </DialogHeader>

      <Alert variant="destructive">
        <RiShieldKeyholeLine />
        <AlertDescription>
          以下场景下不要启用：
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
        推荐场景：家庭 WiFi、私人办公室、自己掌控的 LAN
      </p>

      <div
        class="flex items-center gap-2 rounded-lg border border-border bg-muted/40 p-2.5"
      >
        <Switch v-model="acknowledged" id="ack-switch" />
        <Label for="ack-switch" class="text-xs cursor-pointer">
          我已了解上述风险并自行承担
        </Label>
      </div>

      <DialogFooter>
        <Button variant="outline" @click="cancel">取消</Button>
        <Button :disabled="!acknowledged" @click="confirm">启动代理</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
