<script setup lang="ts">
/**
 * 日志视图 —— 实时订阅 SSE 事件流，渲染为单行文本日志。
 *
 * 数据：useEvents composable 订阅所有 ServerEventType。
 * UI：shadcn-vue Card + Input + Switch + ScrollArea。
 */
import { ref, computed, nextTick, watch } from 'vue'
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { Label } from '@/components/ui/label'
import { Button } from '@/components/ui/button'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Separator } from '@/components/ui/separator'
import { RiSearchLine, RiDeleteBinLine, RiPulseLine } from '@remixicon/vue'
import { useEvents } from '@/composables/useEvents'

type LogLevel = 'INFO' | 'WARN' | 'ERROR'
interface LogLine {
  id: number
  ts: number
  level: LogLevel
  text: string
}

const lines = ref<LogLine[]>([])
const search = ref('')
const autoScroll = ref(true)
const scrollEl = ref<HTMLElement | null>(null)
let nextId = 1
const MAX = 500

function push(level: LogLevel, text: string) {
  lines.value.push({ id: nextId++, ts: Date.now(), level, text })
  if (lines.value.length > MAX) lines.value.shift()
}

const { connected } = useEvents({
  ready: (p) => push('INFO', `代理引擎就绪 (v${p.version})`),
  client_connected: (p) =>
    push(
      'INFO',
      `客户端接入 ${p.peer_ip} → ${p.target} (proto=${p.proto}, session=${p.session_id.slice(0, 8)})`,
    ),
  client_disconnected: (p) =>
    push(
      'INFO',
      `客户端离开 ${p.peer_ip} sent=${p.sent_bytes}B recv=${p.recv_bytes}B duration=${p.duration_sec.toFixed(1)}s`,
    ),
  traffic_tick: () => {},
  vpn_state_changed: (p) =>
    push(
      p.available ? 'INFO' : 'WARN',
      `VPN 状态变更 available=${p.available} iface=${p.iface ?? '(none)'}`,
    ),
})

const filtered = computed(() => {
  if (!search.value.trim()) return lines.value
  const q = search.value.toLowerCase()
  return lines.value.filter((l) => l.text.toLowerCase().includes(q))
})

const counters = computed(() => ({
  info: lines.value.filter((l) => l.level === 'INFO').length,
  warn: lines.value.filter((l) => l.level === 'WARN').length,
  error: lines.value.filter((l) => l.level === 'ERROR').length,
}))

const levelClass: Record<LogLevel, string> = {
  INFO: 'text-muted-foreground',
  WARN: 'text-status-warn',
  ERROR: 'text-status-error',
}

watch(filtered, async () => {
  if (!autoScroll.value) return
  await nextTick()
  if (scrollEl.value) {
    scrollEl.value.scrollTop = scrollEl.value.scrollHeight
  }
})

function fmt(ts: number) {
  const d = new Date(ts)
  return `${String(d.getHours()).padStart(2, '0')}:${String(
    d.getMinutes(),
  ).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`
}

function clearLogs() {
  lines.value = []
}
</script>

<template>
  <div class="mx-auto flex max-w-[1440px] flex-col gap-5 p-6">
    <header class="flex items-baseline justify-between">
      <div>
        <h1 class="text-2xl font-semibold tracking-tight text-foreground">日志</h1>
        <p class="mt-1 text-sm text-muted-foreground">
          实时订阅代理引擎的事件流，按关键词过滤，便于排查接入问题
        </p>
      </div>
      <div class="flex items-center gap-1.5 text-xs">
        <span
          class="size-1.5 rounded-full"
          :class="connected ? 'bg-emerald-500 animate-pulse-dot' : 'bg-muted-foreground'"
        />
        <span class="font-mono text-muted-foreground">
          {{ connected ? 'SSE 已订阅' : 'SSE 未连接' }}
        </span>
      </div>
    </header>

    <Card>
      <CardHeader class="flex flex-row items-center justify-between">
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiPulseLine class="size-3.5 text-foreground" />
          事件流
        </CardTitle>

        <div class="flex items-center gap-1.5">
          <span class="rounded-md bg-muted px-2 py-0.5 font-mono text-[10px] font-semibold tabular-nums text-foreground">
            INFO <span class="ml-0.5">{{ counters.info }}</span>
          </span>
          <span
            class="rounded-md px-2 py-0.5 font-mono text-[10px] font-semibold tabular-nums"
            :class="counters.warn > 0
              ? 'bg-amber-50 text-amber-700 dark:bg-amber-950/40 dark:text-amber-300'
              : 'bg-muted text-muted-foreground'"
          >
            WARN <span class="ml-0.5">{{ counters.warn }}</span>
          </span>
          <span
            class="rounded-md px-2 py-0.5 font-mono text-[10px] font-semibold tabular-nums"
            :class="counters.error > 0
              ? 'bg-red-50 text-red-700 dark:bg-red-950/40 dark:text-red-300'
              : 'bg-muted text-muted-foreground'"
          >
            ERROR <span class="ml-0.5">{{ counters.error }}</span>
          </span>
        </div>
      </CardHeader>

      <CardContent class="flex flex-col gap-3">
        <div class="flex items-center gap-2">
          <div class="relative flex-1">
            <RiSearchLine
              class="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
            />
            <Input
              v-model="search"
              placeholder="搜索 host / IP / 关键词…"
              class="h-8 pl-8 font-mono text-xs"
            />
          </div>
          <Button variant="outline" size="sm" class="h-8" @click="clearLogs">
            <RiDeleteBinLine class="size-3.5" />
            清空
          </Button>
        </div>

        <Separator />

        <ScrollArea
          ref="scrollEl"
          class="h-[440px] rounded-md border border-border bg-muted/30 font-mono text-[12px]"
        >
          <div class="flex flex-col px-3 py-2">
            <div
              v-for="line in filtered"
              :key="line.id"
              class="flex items-baseline gap-2 py-0.5 leading-relaxed"
            >
              <span class="shrink-0 text-muted-foreground tabular-nums">
                {{ fmt(line.ts) }}
              </span>
              <span :class="['shrink-0 w-12 font-semibold', levelClass[line.level]]">
                {{ line.level }}
              </span>
              <span class="break-all text-foreground">{{ line.text }}</span>
            </div>
            <div
              v-if="!filtered.length"
              class="py-10 text-center text-muted-foreground"
            >
              {{ search ? '未匹配到任何日志' : '尚无事件流入，等待客户端接入…' }}
            </div>
          </div>
        </ScrollArea>

        <div class="flex items-center justify-between text-xs">
          <div class="flex items-center gap-2">
            <Switch v-model:checked="autoScroll" id="auto-scroll" />
            <Label for="auto-scroll" class="cursor-pointer">自动滚动到最新</Label>
          </div>
          <span class="font-mono text-muted-foreground tabular-nums">
            最多保留 {{ MAX }} 条
          </span>
        </div>
      </CardContent>
    </Card>
  </div>
</template>
