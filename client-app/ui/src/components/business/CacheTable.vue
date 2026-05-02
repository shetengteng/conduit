<script setup lang="ts">
/**
 * CacheTable —— 路由缓存可视化。
 *
 * - shadcn-vue Table 渲染
 * - 顶部:搜索框(host substring) + direction 过滤(全部/direct/proxy) + 清空按钮
 * - 高亮:direction 用绿/橙胶囊;source 用 ghost 胶囊;hit_count 数字加粗
 * - 时间列展示"X 秒前"相对时间(每 5s 自动重渲染)
 */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Table, TableBody, TableCell, TableHead, TableHeader, TableRow,
} from '@/components/ui/table'
import {
  RiDeleteBinLine,
  RiSearchLine,
  RiArrowGoBackLine,
  RiShareForwardLine,
} from '@remixicon/vue'
import { cacheStore } from '@/stores/cacheStore'
import { useToast } from '@/composables/useToast'
import type { RouteDirection } from '@/types/client'

const toast = useToast()

const search = ref('')
const directionFilter = ref<'all' | RouteDirection>('all')
const flushBusy = ref(false)
const now = ref(Date.now() / 1000)
let tick: number | null = null

onMounted(() => {
  tick = window.setInterval(() => { now.value = Date.now() / 1000 }, 5000)
})
onUnmounted(() => { if (tick !== null) window.clearInterval(tick) })

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  return cacheStore.entries.value.filter((e) => {
    if (directionFilter.value !== 'all' && e.direction !== directionFilter.value) return false
    if (q && !e.host.toLowerCase().includes(q)) return false
    return true
  })
})

function relativeTime(iso: string): string {
  if (!iso) return '—'
  const t = Date.parse(iso) / 1000
  if (!t) return '—'
  const diff = Math.max(0, now.value - t)
  if (diff < 60) return `${Math.round(diff)} 秒前`
  if (diff < 3600) return `${Math.round(diff / 60)} 分钟前`
  if (diff < 86400) return `${Math.round(diff / 3600)} 小时前`
  return `${Math.round(diff / 86400)} 天前`
}

const SOURCE_LABELS: Record<string, string> = {
  pac: 'PAC 预填',
  probe: 'TCP 探测',
  manual: '手动',
  cache: '缓存命中',
  pattern: '通配符',
  private_ip: '内网',
  global_override: '全局降级',
  self_heal: '自愈',
}

async function handleFlush() {
  flushBusy.value = true
  try {
    await cacheStore.flush()
    toast.success('已清空路由缓存')
  } catch (e) {
    toast.error('清空失败', { detail: e instanceof Error ? e.message : String(e) })
  } finally {
    flushBusy.value = false
  }
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <!-- 顶栏:搜索 + 过滤 + 清空 -->
    <div class="flex flex-wrap items-center gap-2">
      <div class="relative flex-1 min-w-[200px]">
        <RiSearchLine class="absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input v-model="search" placeholder="按 host 搜索…" class="h-8 pl-8 text-xs" />
      </div>
      <div class="flex items-center gap-1 rounded-md border border-border bg-background p-0.5 text-xs">
        <Button :variant="directionFilter === 'all' ? 'default' : 'ghost'" size="sm" class="h-6 px-2.5 text-xs" @click="directionFilter = 'all'">全部</Button>
        <Button :variant="directionFilter === 'direct' ? 'default' : 'ghost'" size="sm" class="h-6 gap-1 px-2.5 text-xs" @click="directionFilter = 'direct'">
          <RiArrowGoBackLine class="size-3" />直连
        </Button>
        <Button :variant="directionFilter === 'proxy' ? 'default' : 'ghost'" size="sm" class="h-6 gap-1 px-2.5 text-xs" @click="directionFilter = 'proxy'">
          <RiShareForwardLine class="size-3" />走 server
        </Button>
      </div>
      <span class="text-[11px] text-muted-foreground">{{ filtered.length }} / {{ cacheStore.entries.value.length }} 条</span>
      <Button variant="outline" size="sm" :disabled="flushBusy || cacheStore.entries.value.length === 0" class="h-8 gap-1.5 text-xs" @click="handleFlush">
        <RiDeleteBinLine class="size-3.5" />清空
      </Button>
    </div>

    <!-- 表 -->
    <div class="rounded-md border border-border bg-background">
      <Table>
        <TableHeader>
          <TableRow class="hover:bg-transparent">
            <TableHead class="h-9 text-[11px] uppercase tracking-wide">Host</TableHead>
            <TableHead class="h-9 w-24 text-[11px] uppercase tracking-wide">方向</TableHead>
            <TableHead class="h-9 w-28 text-[11px] uppercase tracking-wide">来源</TableHead>
            <TableHead class="h-9 w-16 text-right text-[11px] uppercase tracking-wide">命中</TableHead>
            <TableHead class="h-9 w-32 text-right text-[11px] uppercase tracking-wide">最近使用</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableRow v-if="filtered.length === 0" class="hover:bg-transparent">
            <TableCell colspan="5" class="h-20 text-center text-xs text-muted-foreground">
              {{ cacheStore.entries.value.length === 0 ? '还没有任何路由决策。浏览任意网站后会自动出现。' : '没有匹配的条目' }}
            </TableCell>
          </TableRow>
          <TableRow v-for="entry in filtered" :key="entry.host" class="text-xs">
            <TableCell class="font-mono">{{ entry.host }}</TableCell>
            <TableCell>
              <span
                v-if="entry.direction === 'direct'"
                class="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-400"
              >
                <RiArrowGoBackLine class="size-2.5" />直连
              </span>
              <span
                v-else
                class="inline-flex items-center gap-1 rounded-full bg-orange-500/10 px-1.5 py-0.5 text-[10px] font-medium text-orange-700 dark:text-orange-400"
              >
                <RiShareForwardLine class="size-2.5" />走 server
              </span>
            </TableCell>
            <TableCell>
              <span class="inline-flex rounded border border-border bg-background px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                {{ SOURCE_LABELS[entry.source] ?? entry.source }}
              </span>
            </TableCell>
            <TableCell class="text-right font-mono font-medium">{{ entry.hit_count }}</TableCell>
            <TableCell class="text-right text-[11px] text-muted-foreground">{{ relativeTime(entry.last_used) }}</TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </div>
  </div>
</template>
