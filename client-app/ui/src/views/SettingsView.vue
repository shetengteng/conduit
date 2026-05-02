<script setup lang="ts">
/**
 * 设置页 —— M-δ。
 *
 * 块:
 *   1. 运行时信息 (端口)
 *   2. 开机自启开关 (调用 Rust 命令写 ~/Library/LaunchAgents/com.conduit.client.plist)
 *   3. 手动连接 server (mDNS 不通时兜底)
 *   4. 路由缓存维护 (清空)
 *   5. 诊断入口 (跳到独立诊断页)
 *
 * 暂不实现 (留 M-ε):
 *   - 系统代理开关 toggle (cfg 启动时定型,运行时改要 sidecar 重启)
 *   - 日志文件路径选择
 *   - PAC 自定义路径
 */
import { onMounted, ref } from 'vue'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'
import { Separator } from '@/components/ui/separator'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import {
  RiSettings4Line,
  RiLinkM,
  RiDeleteBinLine,
  RiServerLine,
  RiAlertLine,
  RiStethoscopeLine,
  RiToggleLine,
} from '@remixicon/vue'
import { getRuntime } from '@/api/runtime'
import { connectionStore } from '@/stores/connectionStore'
import { cacheStore } from '@/stores/cacheStore'
import { useToast } from '@/composables/useToast'
import { uiStore } from '@/stores/ui'
import type { AppRuntime } from '@/types/client'
import { invoke } from '@tauri-apps/api/core'
import { Switch } from '@/components/ui/switch'

const toast = useToast()
const runtime = ref<AppRuntime | null>(null)

const manualName = ref('')
const manualHost = ref('')
const manualPort = ref(8080)
const manualSocks = ref(8081)
const manualApi = ref(8090)
const manualBusy = ref(false)

const autostartEnabled = ref(false)
const autostartBusy = ref(false)
const autostartError = ref<string | null>(null)

onMounted(async () => {
  runtime.value = await getRuntime()
  await refreshAutostart()
})

async function refreshAutostart() {
  try {
    autostartEnabled.value = await invoke<boolean>('autostart_status')
    autostartError.value = null
  } catch (e) {
    autostartError.value = e instanceof Error ? e.message : String(e)
  }
}

async function toggleAutostart(next: boolean) {
  autostartBusy.value = true
  try {
    await invoke(next ? 'autostart_enable' : 'autostart_disable')
    autostartEnabled.value = next
    toast.success(next ? '已启用开机自启' : '已关闭开机自启')
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    toast.error('开机自启切换失败', { detail: msg })
    await refreshAutostart()
  } finally {
    autostartBusy.value = false
  }
}

async function handleManualConnect() {
  if (!manualHost.value.trim() || !manualPort.value) {
    toast.error('请输入 host 和 port')
    return
  }
  const name = manualName.value.trim() || `manual-${manualHost.value}`
  // server_id 格式必须与后端一致:name@host:port
  const serverId = `${name}@${manualHost.value.trim()}:${manualPort.value}`
  manualBusy.value = true
  try {
    // 后端目前仅支持从 discoverer.snapshot() 找 server_id;手动添加暂时只
    // 在 mDNS 已经发现过的情况下能用。为了不撒谎,这里先尝试从已知列表
    // 找,找不到给明确错误。完整的"手动添加"端点会在 M-δ 加。
    await connectionStore.connectTo(serverId)
    toast.success(`已尝试连接 ${serverId}`)
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e)
    toast.error('手动连接失败', { detail: msg })
  } finally {
    manualBusy.value = false
  }
}

async function handleFlushCache() {
  try {
    await cacheStore.flush()
    toast.success('已清空路由缓存')
  } catch (e) {
    toast.error('清空失败', { detail: e instanceof Error ? e.message : String(e) })
  }
}
</script>

<template>
  <div class="flex flex-col gap-6 p-6">
    <div class="flex flex-col gap-1">
      <h1 class="text-2xl font-extralight tracking-tight text-foreground">设置</h1>
      <p class="text-sm text-muted-foreground">
        运行时信息、手动添加 server、缓存维护
      </p>
    </div>

    <!-- 运行时端口 -->
    <Card size="sm">
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiSettings4Line class="size-4 text-muted-foreground" />
          运行时
        </CardTitle>
        <CardDescription class="text-xs">
          这些端口由 Tauri 主进程在启动时通过 portpicker 动态分配
        </CardDescription>
      </CardHeader>
      <CardContent class="grid grid-cols-2 gap-3">
        <div class="flex flex-col gap-1.5">
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">SOCKS5 端口</Label>
          <span class="rounded-md bg-muted px-2 py-1 font-mono text-sm tabular-nums">{{ runtime?.socks_port ?? '—' }}</span>
        </div>
        <div class="flex flex-col gap-1.5">
          <Label class="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">Control API 端口</Label>
          <span class="rounded-md bg-muted px-2 py-1 font-mono text-sm tabular-nums">{{ runtime?.api_port ?? '—' }}</span>
        </div>
      </CardContent>
    </Card>

    <!-- 开机自启 -->
    <Card size="sm">
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiToggleLine class="size-4 text-muted-foreground" />
          开机自启
        </CardTitle>
        <CardDescription class="text-xs">
          通过 macOS launchctl 在 ~/Library/LaunchAgents/ 写入 plist，登录后自动启动 Conduit Client
        </CardDescription>
      </CardHeader>
      <CardContent class="flex items-center justify-between gap-3">
        <div class="flex flex-col gap-0.5 text-xs text-muted-foreground">
          <span>当前状态:
            <span :class="autostartEnabled ? 'text-emerald-700 dark:text-emerald-400 font-medium' : 'text-muted-foreground'">
              {{ autostartEnabled ? '已启用' : '未启用' }}
            </span>
          </span>
          <span v-if="autostartError" class="text-destructive">{{ autostartError }}</span>
        </div>
        <Switch
          :model-value="autostartEnabled"
          :disabled="autostartBusy"
          @update:model-value="toggleAutostart"
        />
      </CardContent>
    </Card>

    <!-- 手动添加 server -->
    <Card size="sm">
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiServerLine class="size-4 text-muted-foreground" />
          手动连接 server
        </CardTitle>
        <CardDescription class="text-xs">
          mDNS 不通(跨网段 / 沙箱 / 公司 WLAN 限制 multicast)时,在此手动指定 server。
          注意:目前仅支持先在「发现」页见过 server_id 后再用本表单复连;完整的手动注册端点会在 M-δ 推出。
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-3">
        <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
          <div class="flex flex-col gap-1">
            <Label class="text-[11px]">名称</Label>
            <Input v-model="manualName" placeholder="同事的 Mac" class="h-8 text-xs" />
          </div>
          <div class="flex flex-col gap-1">
            <Label class="text-[11px]">Host</Label>
            <Input v-model="manualHost" placeholder="192.168.1.14" class="h-8 text-xs font-mono" />
          </div>
          <div class="flex flex-col gap-1">
            <Label class="text-[11px]">HTTP / PAC 端口</Label>
            <Input v-model.number="manualPort" type="number" placeholder="8080" class="h-8 text-xs font-mono" />
          </div>
          <div class="flex flex-col gap-1">
            <Label class="text-[11px]">SOCKS5 端口</Label>
            <Input v-model.number="manualSocks" type="number" placeholder="8081" class="h-8 text-xs font-mono" />
          </div>
          <div class="flex flex-col gap-1">
            <Label class="text-[11px]">控制 API 端口</Label>
            <Input v-model.number="manualApi" type="number" placeholder="8090" class="h-8 text-xs font-mono" />
          </div>
        </div>
        <Button :disabled="manualBusy" class="w-fit gap-1.5" @click="handleManualConnect">
          <RiLinkM class="size-3.5" />{{ manualBusy ? '连接中…' : '尝试连接' }}
        </Button>
        <div class="flex items-start gap-2 rounded-md border border-amber-300/50 bg-amber-50/40 px-3 py-2 text-[11px] text-amber-900 dark:bg-amber-950/10 dark:text-amber-200">
          <RiAlertLine class="size-3.5 mt-0.5 shrink-0" />
          <span>
            提示:server_id 是 <code class="font-mono">name@host:port</code>。如果发现页从来没看到过这个 server,这里的连接会返回 NOT_FOUND。
          </span>
        </div>
      </CardContent>
    </Card>

    <!-- 缓存维护 -->
    <Card size="sm">
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiDeleteBinLine class="size-4 text-muted-foreground" />
          缓存维护
        </CardTitle>
        <CardDescription class="text-xs">
          清空当前路由缓存(host → direct/proxy 决策)。常用于 server 端 PAC 规则更新后强制重新探测。
        </CardDescription>
      </CardHeader>
      <CardContent class="flex items-center justify-between gap-3">
        <div class="flex flex-col gap-0.5 text-xs text-muted-foreground">
          <span>当前缓存条目:<span class="font-mono font-medium text-foreground">{{ cacheStore.entries.value.length }}</span></span>
          <span v-if="cacheStore.stats.value">命中 / 未命中:<span class="font-mono">{{ cacheStore.stats.value.hits }} / {{ cacheStore.stats.value.misses }}</span></span>
        </div>
        <Button variant="outline" :disabled="cacheStore.entries.value.length === 0" class="gap-1.5" @click="handleFlushCache">
          <RiDeleteBinLine class="size-3.5" />清空路由缓存
        </Button>
      </CardContent>
    </Card>

    <!-- 诊断入口 (完整 5 步在独立诊断页) -->
    <Card size="sm">
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-[13px] font-semibold">
          <RiStethoscopeLine class="size-4 text-muted-foreground" />
          自检详情
        </CardTitle>
        <CardDescription class="text-xs">
          完整 5 步自检 (sidecar / mDNS / server 可达 / PAC / 系统代理) 在独立的诊断页
        </CardDescription>
      </CardHeader>
      <CardContent class="flex items-center justify-between gap-3">
        <p class="text-xs text-muted-foreground">
          失败项会给出可操作的修复建议，并支持一键复制完整报告
        </p>
        <Button variant="outline" size="sm" class="gap-1.5" @click="uiStore.setActive('diagnose')">
          <RiStethoscopeLine class="size-3.5" />
          打开诊断
        </Button>
      </CardContent>
    </Card>

    <Separator />

    <Card size="sm">
      <CardHeader>
        <CardTitle class="text-[13px] font-semibold">关于</CardTitle>
      </CardHeader>
      <CardContent class="text-xs text-muted-foreground">
        <p>Conduit Client v0.1.0 · macOS only</p>
        <p class="mt-1">智能本地代理:自动按目标域走 direct 或 server VPN</p>
      </CardContent>
    </Card>
  </div>
</template>
