<script setup lang="ts">
/**
 * 在线客户端列表 —— 完全基于 shadcn-vue Table 组件实现。
 *
 * 列：会话ID（截断） / Peer / 协议 / 目标 / 下行 / 上行 / 累计字节 / 接入时长
 * 排序：复用 useTableSort composable
 */
import { computed } from 'vue'
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Table,
  TableBody,
  TableCell,
  TableEmpty,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import {
  RiArrowUpSLine,
  RiArrowDownSLine,
  RiExpandUpDownLine,
  RiUserUnfollowLine,
  RiUserStarLine,
  RiPulseLine,
} from '@remixicon/vue'
import { proxyStore } from '@/stores/proxy'
import { trafficStore } from '@/stores/traffic'
import { useTableSort } from '@/composables/useTableSort'
import { formatBps, formatBytes, formatIdleSec, formatUptimeShort } from '@/utils/format'
import type { ClientSession } from '@/types/proxy'

type SortKey =
  | 'peer'
  | 'proto'
  | 'target'
  | 'down'
  | 'up'
  | 'total'
  | 'since'

const clients = computed(() => proxyStore.state.clients)
const passiveClients = computed(() => proxyStore.state.passiveClients)

// 表格本身按 session 列展示(可以一个 client 占多行,展示并发会话/不同 target);
// 顶部统计按 peer_ip 去重,反映"几个独立设备",和 KPI 卡保持一致。
const activePeerCount = proxyStore.activePeerCount
const passiveOnlyCount = proxyStore.passiveOnlyPeerCount
const totalCount = proxyStore.uniquePeerCount

// 待命客户端区只列"仅心跳、当前没传输流量"的 peer。某个 peer 同时存在
// session 又在心跳时,它会出现在表格里(算 active),不再重复出现在底部。
const passiveOnlyClients = computed(() => {
  const activePeers = new Set(clients.value.map((c) => c.peer_ip))
  return passiveClients.value.filter((p) => !activePeers.has(p.peer_ip))
})

// 直接用 backend 的 idle_sec(随每次 refreshSilently 刷新)。
function formatIdle(s: number | null | undefined): string {
  return formatIdleSec(typeof s === 'number' ? s : Number.NaN)
}

function liveBps(peer: string, dir: 'in' | 'out'): number {
  const arr = trafficStore.state.series[peer]
  if (!arr || !arr.length) return 0
  const last = arr[arr.length - 1]
  return dir === 'in' ? last[2] : last[1]
}

function valueOf(row: ClientSession, key: SortKey): string | number {
  const now = Date.now() / 1000
  switch (key) {
    case 'peer':
      return row.peer_ip
    case 'proto':
      return row.proto
    case 'target':
      return row.target
    case 'down':
      return liveBps(row.peer_ip, 'in')
    case 'up':
      return liveBps(row.peer_ip, 'out')
    case 'total':
      return row.sent_bytes + row.recv_bytes
    case 'since':
      return now - row.since
  }
}

const { key, dir, sorted, set } = useTableSort<ClientSession, SortKey>(clients, {
  defaultKey: 'since',
  defaultDir: 'desc',
  ascendingKeys: ['peer', 'proto', 'target'],
  valueOf,
})

interface Column {
  key: SortKey
  label: string
  align: 'left' | 'right'
}

const columns: Column[] = [
  { key: 'peer', label: '客户端', align: 'left' },
  { key: 'proto', label: '协议', align: 'left' },
  { key: 'target', label: '目标', align: 'left' },
  { key: 'down', label: '下行', align: 'right' },
  { key: 'up', label: '上行', align: 'right' },
  { key: 'total', label: '累计', align: 'right' },
  { key: 'since', label: '接入', align: 'right' },
]
</script>

<template>
  <Card size="sm" class="h-full">
    <CardHeader class="flex flex-row items-center justify-between">
      <CardTitle class="text-[13px] font-semibold">在线客户端</CardTitle>
      <span class="font-mono text-[11px] text-muted-foreground tabular-nums">
        共 {{ totalCount }} 个
        <template v-if="totalCount > 0">
          ·
          <span class="text-emerald-600 dark:text-emerald-400">{{ activePeerCount }} 传输中</span>
          ·
          <span class="text-blue-600 dark:text-blue-400">{{ passiveOnlyCount }} 待命</span>
        </template>
      </span>
    </CardHeader>

    <CardContent class="!px-0">
      <Table>
        <TableHeader class="sticky top-0 z-10 bg-card">
          <TableRow>
            <TableHead
              v-for="col in columns"
              :key="col.key"
              :class="[
                'cursor-pointer select-none whitespace-nowrap text-[11px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground',
                col.align === 'right' && 'text-right',
              ]"
              @click="set(col.key)"
            >
              <span
                :class="[
                  'inline-flex items-center gap-1',
                  col.align === 'right' && 'justify-end',
                ]"
              >
                {{ col.label }}
                <component
                  :is="
                    key === col.key
                      ? dir === 'asc'
                        ? RiArrowUpSLine
                        : RiArrowDownSLine
                      : RiExpandUpDownLine
                  "
                  :class="[
                    'size-3',
                    key === col.key ? 'text-foreground' : 'text-muted-foreground/40',
                  ]"
                />
              </span>
            </TableHead>
          </TableRow>
        </TableHeader>

        <TableBody>
          <TableRow v-for="row in sorted" :key="row.session_id">
            <TableCell class="font-mono font-medium text-foreground">
              {{ row.peer_ip }}
            </TableCell>
            <TableCell>
              <span class="rounded-md bg-muted px-1.5 py-0.5 font-mono text-[10px] font-semibold uppercase tracking-wide text-foreground">
                {{ row.proto }}
              </span>
            </TableCell>
            <TableCell class="max-w-[280px] truncate font-mono text-xs">
              {{ row.target }}
            </TableCell>
            <TableCell class="text-right font-mono tabular-nums">
              {{ formatBps(liveBps(row.peer_ip, 'in')) }}
            </TableCell>
            <TableCell class="text-right font-mono tabular-nums">
              {{ formatBps(liveBps(row.peer_ip, 'out')) }}
            </TableCell>
            <TableCell class="text-right font-mono tabular-nums text-muted-foreground">
              {{ formatBytes(row.sent_bytes + row.recv_bytes) }}
            </TableCell>
            <TableCell class="text-right font-mono tabular-nums text-muted-foreground">
              {{ formatUptimeShort(Date.now() / 1000 - row.since) }}
            </TableCell>
          </TableRow>

          <TableEmpty v-if="!sorted.length" :colspan="columns.length">
            <div class="flex items-center justify-center gap-3 py-4">
              <div
                class="flex size-7 items-center justify-center rounded-full bg-muted text-muted-foreground"
              >
                <RiUserUnfollowLine class="size-3.5" />
              </div>
              <div class="flex items-baseline gap-2">
                <p class="text-xs font-medium">
                  {{ passiveOnlyClients.length > 0 ? '暂无客户端在传输流量' : '还没有客户端连进来' }}
                </p>
                <p class="text-[11px] text-muted-foreground">
                  {{ passiveOnlyClients.length > 0
                    ? '下方"待命"客户端正等着发起请求'
                    : '把右侧 PAC URL 分享给同事即可接入' }}
                </p>
              </div>
            </div>
          </TableEmpty>
        </TableBody>
      </Table>

      <!-- 被动客户端区(已链接但暂未传输流量)。仅列出顶部"待命"数对应的 peer。 -->
      <div
        v-if="passiveOnlyClients.length > 0"
        class="border-t border-border/40 bg-muted/30 px-4 py-3"
      >
        <div class="mb-2 flex items-center gap-2">
          <RiUserStarLine class="size-3.5 text-blue-600 dark:text-blue-400" />
          <span class="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
            待命客户端 · 已链接但暂无流量
          </span>
        </div>
        <div class="flex flex-col gap-1">
          <div
            v-for="pc in passiveOnlyClients"
            :key="pc.peer_ip"
            class="flex items-center justify-between gap-3 rounded-md bg-background px-2.5 py-1.5"
          >
            <div class="flex min-w-0 items-center gap-2.5">
              <span class="size-1.5 shrink-0 rounded-full bg-blue-500" />
              <span class="truncate text-xs font-semibold text-foreground">
                {{ pc.client_name }}
              </span>
              <span class="font-mono text-[11px] text-muted-foreground tabular-nums">
                {{ pc.peer_ip }}
              </span>
              <span class="rounded bg-muted px-1.5 py-px font-mono text-[10px] text-muted-foreground">
                v{{ pc.version }}
              </span>
            </div>
            <span class="flex items-center gap-1 font-mono text-[11px] text-muted-foreground tabular-nums">
              <RiPulseLine class="size-3 text-blue-500/80" />
              {{ formatIdle(pc.idle_sec) }}
            </span>
          </div>
        </div>
      </div>
    </CardContent>
  </Card>
</template>
