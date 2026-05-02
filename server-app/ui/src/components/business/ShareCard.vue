<script setup lang="ts">
/**
 * 接入信息卡 —— 让同事一键复制即可接入。
 *
 * 三种接入方式（PAC 推荐 / HTTP / SOCKS5）：
 *   - 用 Tabs 切换，每个面板顶部一句话说明"什么场景用"
 *   - PAC 直接显示完整 URL（mono 大字号 + 全选友好）
 *   - HTTP / SOCKS5 合并为 host:port 一行，一键复制完整 endpoint
 *
 * 空态：
 *   - 代理未启动 → 显示「请先启动代理」占位，复制按钮全部 disabled
 *   - LAN IP 解析失败 → 显示警告，提醒同事必须在同一局域网
 */
import { computed, ref } from 'vue'
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Button } from '@/components/ui/button'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { RiFileCopy2Line, RiInformationLine, RiAlertLine, RiLinkM } from '@remixicon/vue'
import { proxyStore } from '@/stores/proxy'
import { useToast } from '@/composables/useToast'

const toast = useToast()
const tab = ref<'pac' | 'http' | 'socks5'>('pac')

const status = computed(() => proxyStore.state.status)
const running = computed(() => Boolean(status.value?.running))

/**
 * 单一 host 来源：从 pac_url 解析出后端 advertised host（与 HTTP/SOCKS5 监听 host 一致）。
 *
 * 不再依赖 `lan.detail` 字符串正则 —— 后端 `_pac_url()` 已经在 `pac_advertised_host` /
 * `bind` / `0.0.0.0` 三种情况下做好兜底（0.0.0.0 时返回 null）。
 */
const advertisedHost = computed<string | null>(() => {
  const url = status.value?.pac_url
  if (!url) return null
  try {
    return new URL(url).hostname || null
  } catch {
    return null
  }
})

const pacUrl = computed(() => status.value?.pac_url ?? '')

const httpEndpoint = computed(() => {
  const h = advertisedHost.value
  const p = status.value?.http_port
  return h && p ? `${h}:${p}` : ''
})

const socksEndpoint = computed(() => {
  const h = advertisedHost.value
  const p = status.value?.socks5_port
  return h && p ? `${h}:${p}` : ''
})

/**
 * 警告条件：代理已运行但拿不到对外可达 host。
 *
 * 此时 `pac_url` 多半是 null（后端在 host == "0.0.0.0" 时直接返回 null），
 * 同事即便手动填 host:port 也连不上 —— 只能等用户重启并指定 --pac-host 或确保
 * 物理网卡有私有 IP。
 */
const noReachableHost = computed(
  () => running.value && !advertisedHost.value,
)
const lanAvailable = computed(() => Boolean(status.value?.lan?.available))

async function copy(text: string, label: string) {
  if (!text) return
  try {
    await navigator.clipboard.writeText(text)
    toast.success(`${label} 已复制`, { detail: text })
  } catch (e) {
    toast.error('复制失败', { detail: String(e) })
  }
}
</script>

<template>
  <Card size="sm" class="h-full">
    <CardHeader class="flex flex-row items-baseline justify-between">
      <CardTitle class="text-[13px] font-semibold">接入信息</CardTitle>
      <span class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        分享给同事
      </span>
    </CardHeader>

    <CardContent class="flex flex-col gap-2.5">
      <Alert v-if="!running" variant="default" class="py-2">
        <RiInformationLine />
        <AlertDescription class="text-[11px]">
          代理未启动，启动后此处会显示同事可用的接入信息
        </AlertDescription>
      </Alert>

      <Alert v-else-if="noReachableHost" variant="destructive" class="py-2">
        <RiAlertLine />
        <AlertDescription class="text-[11px]">
          代理监听在 0.0.0.0 但未检测到对外可达地址，同事将无法连接。
          请确认本机已加入有线/Wi-Fi 网络，或在启动时显式指定 <code class="font-mono">--pac-host</code>
        </AlertDescription>
      </Alert>

      <Alert
        v-else-if="running && !lanAvailable"
        variant="default"
        class="border-status-warn/30 bg-status-warn/10 py-2"
      >
        <RiAlertLine class="text-status-warn" />
        <AlertDescription class="text-[11px]">
          未检测到物理网卡的私有 IP。当前接入信息可用，但同事必须能直接访问
          <code class="font-mono">{{ advertisedHost }}</code>
        </AlertDescription>
      </Alert>

      <Tabs v-model="tab">
        <TabsList class="h-8 w-full bg-muted p-0.5">
          <TabsTrigger
            value="pac"
            class="flex-1 text-[11px] font-medium data-[state=active]:bg-card data-[state=active]:text-foreground data-[state=active]:shadow-sm"
          >
            PAC
          </TabsTrigger>
          <TabsTrigger
            value="http"
            class="flex-1 text-[11px] font-medium data-[state=active]:bg-card data-[state=active]:text-foreground data-[state=active]:shadow-sm"
          >
            HTTP
          </TabsTrigger>
          <TabsTrigger
            value="socks5"
            class="flex-1 text-[11px] font-medium data-[state=active]:bg-card data-[state=active]:text-foreground data-[state=active]:shadow-sm"
          >
            SOCKS5
          </TabsTrigger>
        </TabsList>

        <TabsContent value="pac" class="mt-2.5 flex flex-col gap-2">
          <p class="text-[11px] text-muted-foreground">
            <span class="mr-1 inline-flex items-center rounded-sm bg-primary px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-primary-foreground">
              推荐
            </span>
            填到系统代理的「自动配置脚本」即可，按规则智能分流，国内站不绕路
          </p>
          <div
            class="group flex items-stretch overflow-hidden rounded-md border border-border bg-muted/30 transition-colors hover:border-border hover:bg-muted/50"
          >
            <div class="flex items-center justify-center px-2.5 text-muted-foreground">
              <RiLinkM class="size-3.5" />
            </div>
            <code
              v-if="pacUrl"
              class="flex-1 select-all overflow-x-auto whitespace-nowrap py-2 pr-2 font-mono text-xs leading-tight"
              :title="pacUrl"
            >
              {{ pacUrl }}
            </code>
            <span
              v-else
              class="flex-1 py-2 pr-2 text-[11px] italic text-muted-foreground"
            >
              启动代理后将在此显示完整 PAC URL
            </span>
            <Button
              variant="ghost"
              size="sm"
              class="h-auto rounded-none border-l border-border px-3"
              :disabled="!pacUrl"
              @click="copy(pacUrl, 'PAC URL')"
            >
              <RiFileCopy2Line class="size-3.5" />
            </Button>
          </div>
        </TabsContent>

        <TabsContent value="http" class="mt-2.5 flex flex-col gap-2">
          <p class="text-[11px] text-muted-foreground">
            手动配「HTTP 代理」时填这一行（host:port）—— 全局走代理，国内站会变慢
          </p>
          <div
            class="group flex items-stretch overflow-hidden rounded-md border border-border bg-muted/30 transition-colors hover:border-border hover:bg-muted/50"
          >
            <div class="flex items-center px-2.5 text-[10px] uppercase tracking-wide text-muted-foreground">
              host:port
            </div>
            <code
              v-if="httpEndpoint"
              class="flex-1 select-all overflow-x-auto whitespace-nowrap py-2 pr-2 text-right font-mono text-sm font-medium"
            >
              {{ httpEndpoint }}
            </code>
            <span
              v-else
              class="flex-1 py-2 pr-2 text-right text-[11px] italic text-muted-foreground"
            >
              启动后显示
            </span>
            <Button
              variant="ghost"
              size="sm"
              class="h-auto rounded-none border-l border-border px-3"
              :disabled="!httpEndpoint"
              @click="copy(httpEndpoint, 'HTTP 代理')"
            >
              <RiFileCopy2Line class="size-3.5" />
            </Button>
          </div>
        </TabsContent>

        <TabsContent value="socks5" class="mt-2.5 flex flex-col gap-2">
          <p class="text-[11px] text-muted-foreground">
            适合 curl / git / SSH 等命令行工具的 TCP 全协议代理
          </p>
          <div
            class="group flex items-stretch overflow-hidden rounded-md border border-border bg-muted/30 transition-colors hover:border-border hover:bg-muted/50"
          >
            <div class="flex items-center px-2.5 text-[10px] uppercase tracking-wide text-muted-foreground">
              host:port
            </div>
            <code
              v-if="socksEndpoint"
              class="flex-1 select-all overflow-x-auto whitespace-nowrap py-2 pr-2 text-right font-mono text-sm font-medium"
            >
              {{ socksEndpoint }}
            </code>
            <span
              v-else
              class="flex-1 py-2 pr-2 text-right text-[11px] italic text-muted-foreground"
            >
              启动后显示
            </span>
            <Button
              variant="ghost"
              size="sm"
              class="h-auto rounded-none border-l border-border px-3"
              :disabled="!socksEndpoint"
              @click="copy(socksEndpoint, 'SOCKS5 代理')"
            >
              <RiFileCopy2Line class="size-3.5" />
            </Button>
          </div>
        </TabsContent>
      </Tabs>
    </CardContent>
  </Card>
</template>
