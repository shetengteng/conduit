<script setup lang="ts">
/**
 * 设置视图 —— v0.1 阶段为只读占位（实际表单 S4 与打包同步落地）。
 *
 * 当前展示项：
 *   - 端口（HTTP / SOCKS5 / API），来自 status，禁用编辑
 *   - mDNS 广播（开关只读），CIDR 白名单（占位 chip）
 *   - 关于（版本 + 后端 / 前端 元信息）
 */
import { computed } from 'vue'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Separator } from '@/components/ui/separator'
import { Button } from '@/components/ui/button'
import { Alert, AlertDescription } from '@/components/ui/alert'
import {
  RiSettings4Line,
  RiShieldKeyholeLine,
  RiBroadcastLine,
  RiInformationLine,
  RiExternalLinkLine,
} from '@remixicon/vue'
import { proxyStore } from '@/stores/proxy'

const status = computed(() => proxyStore.state.status)
const allowedCidrs = ['192.168.0.0/16', '10.0.0.0/8', '172.16.0.0/12']
const allowedPorts = [80, 443, 22, 8080, 8443]
</script>

<template>
  <div class="mx-auto flex max-w-[960px] flex-col gap-5 p-6">
    <header>
      <h1 class="text-2xl font-semibold tracking-tight text-foreground">设置</h1>
      <p class="mt-1 text-sm text-muted-foreground">
        端口、安全策略、mDNS 广播 —— v0.1 阶段为只读占位
      </p>
    </header>

    <Alert variant="default">
      <RiInformationLine />
      <AlertDescription>
        v0.1 阶段所有配置项为<strong class="font-medium text-foreground">只读展示</strong>,与代理运行状态无关。
        如需自定义,请通过启动参数(如 <code class="rounded bg-muted px-1 py-0.5 font-mono text-[11px]">--mdns-name "MyServer"</code> /
        <code class="rounded bg-muted px-1 py-0.5 font-mono text-[11px]">--http-port 8080</code>)指定。
        在窗口里编辑并热重启的能力将在 S4 与打包同步发布。
      </AlertDescription>
    </Alert>

    <Card>
      <CardHeader class="flex flex-row items-start justify-between gap-3 space-y-0">
        <div class="flex flex-col gap-1">
          <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
            <RiSettings4Line class="size-3.5 text-foreground" />
            端口
          </CardTitle>
          <CardDescription class="text-xs">
            这些端口由 Tauri 主进程在启动时通过 portpicker 动态分配,无法在窗口里修改
          </CardDescription>
        </div>
        <span class="rounded-full border border-border/60 bg-muted/40 px-2 py-0.5 font-mono text-[10px] tracking-wide text-muted-foreground">
          只读
        </span>
      </CardHeader>
      <CardContent class="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <div class="flex flex-col gap-1.5">
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            HTTP 代理
          </Label>
          <Input
            :model-value="String(status?.http_port ?? '—')"
            readonly
            disabled
            title="v0.1 阶段不支持窗口内编辑;启动 server 时用 --http-port 指定"
            class="h-8 cursor-not-allowed bg-muted/40 font-mono text-sm tabular-nums opacity-90"
          />
        </div>
        <div class="flex flex-col gap-1.5">
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            SOCKS5 代理
          </Label>
          <Input
            :model-value="String(status?.socks5_port ?? '—')"
            readonly
            disabled
            title="v0.1 阶段不支持窗口内编辑;启动 server 时用 --socks5-port 指定"
            class="h-8 cursor-not-allowed bg-muted/40 font-mono text-sm tabular-nums opacity-90"
          />
        </div>
        <div class="flex flex-col gap-1.5">
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            控制 API
          </Label>
          <Input
            :model-value="String(status?.api_port ?? '—')"
            readonly
            disabled
            title="v0.1 阶段不支持窗口内编辑;启动 server 时用 --api-port 指定(loopback only)"
            class="h-8 cursor-not-allowed bg-muted/40 font-mono text-sm tabular-nums opacity-90"
          />
        </div>
      </CardContent>
    </Card>

    <Card>
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiShieldKeyholeLine class="size-3.5 text-foreground" />
          安全
        </CardTitle>
        <CardDescription class="text-xs">
          仅允许下列 LAN 段 / 目标端口的连接通过代理
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <div>
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            允许接入的 LAN 段（CIDR）
          </Label>
          <div class="mt-2 flex flex-wrap gap-1.5">
            <span
              v-for="c in allowedCidrs"
              :key="c"
              class="rounded-md bg-muted px-2 py-1 font-mono text-[11px] font-medium text-foreground"
            >
              {{ c }}
            </span>
          </div>
        </div>
        <Separator />
        <div>
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            CONNECT 允许的目标端口
          </Label>
          <div class="mt-2 flex flex-wrap gap-1.5">
            <span
              v-for="p in allowedPorts"
              :key="p"
              class="rounded-md bg-muted px-2 py-1 font-mono text-[11px] font-medium tabular-nums text-foreground"
            >
              {{ p }}
            </span>
          </div>
        </div>
      </CardContent>
    </Card>

    <Card>
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiBroadcastLine class="size-3.5 text-foreground" />
          mDNS 广播
        </CardTitle>
        <CardDescription class="text-xs">
          让 LAN 上的 Conduit Client 自动发现本机
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <div class="flex items-center justify-between">
          <Label for="mdns-enable" class="cursor-pointer text-xs">
            启用 mDNS 广播
          </Label>
          <Switch id="mdns-enable" :model-value="status?.mdns?.enabled ?? true" disabled />
        </div>
        <Separator />
        <div class="grid grid-cols-2 gap-3">
          <div class="flex flex-col gap-1.5">
            <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
              广播名称
            </Label>
            <Input
              :model-value="status?.mdns?.name ?? '—'"
              readonly
              disabled
              title="v0.1 不支持窗口内编辑;启动时用 --mdns-name '你的服务名' 指定"
              class="h-8 cursor-not-allowed bg-muted/40 text-sm opacity-90"
            />
            <p class="text-[10px] text-muted-foreground">
              默认取系统短主机名;如需自定义请用 <code class="font-mono">--mdns-name "你的服务名"</code> 启动 server
            </p>
          </div>
          <div class="flex flex-col gap-1.5">
            <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
              服务类型
            </Label>
            <Input
              :model-value="status?.mdns?.service_type ?? '_conduit._tcp.local.'"
              readonly
              disabled
              class="h-8 cursor-not-allowed bg-muted/40 font-mono text-sm opacity-90"
            />
          </div>
        </div>
      </CardContent>
    </Card>

    <Card>
      <CardHeader>
        <CardTitle class="text-[13px] font-semibold">关于</CardTitle>
      </CardHeader>
      <CardContent class="flex items-center justify-between">
        <div class="flex items-center gap-3">
          <div
            class="flex size-9 items-center justify-center rounded-md bg-primary text-primary-foreground"
          >
            <RiShieldKeyholeLine class="size-4" />
          </div>
          <div>
            <p class="text-sm font-semibold text-foreground">Conduit Server</p>
            <p class="font-mono text-xs text-muted-foreground">
              v{{ status?.version ?? '0.1.0' }} · Tauri 2 + Vue 3 + Python
            </p>
          </div>
        </div>
        <Button variant="outline" size="sm" class="h-8">
          <RiExternalLinkLine class="size-3.5" />
          检查更新
        </Button>
      </CardContent>
    </Card>
  </div>
</template>
