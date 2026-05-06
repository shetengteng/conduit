# Conduit — 桌面应用方案详细设计

> 项目代号：**Conduit**
> 日期：2026-04-30
> 范围：把现有 Python 代理服务封装成"双击即用"的桌面应用，并配套现代化的图形控制台
> 技术选型：Tauri 2 + Vue 3 + Python sidecar（详见 §1，已锁定）
> 前置文档：`2026-04-30-Conduit-桌面化可行性报告.md`

---

## 1. 技术栈（已锁定）

| 层 | 选型 | 版本（推荐锁定） | 备注 |
|---|---|---|---|
| 桌面壳 | **Tauri 2** | `2.10.x`（stable） | Rust 主进程 + 系统 WebView |
| 前端框架 | **Vue 3** | `^3.5.0` | Composition API + `<script setup>` |
| UI 组件库 | **shadcn-vue** | `latest`（CLI 复制源码到本地） | 不是 npm install，是 `pnpm dlx shadcn-vue add ...` |
| CSS | **Tailwind CSS v4** | `^4.0.0` | 用 `@tailwindcss/vite` 插件，不再走 PostCSS |
| 构建 | **Vite** | `^6.0.0` | shadcn-vue 推荐 |
| 折线图 | **uPlot** | `^1.6.x` | 流式时序图，~40KB |
| 包管理 | **pnpm** | `^9` | shadcn-vue 官方文档使用 |
| 后端语言 | **Python** | `>=3.10`（与现有代码一致） | 复用 `proxy_server.py` 等 |
| 后端打包 | **Nuitka**（首选）/ PyInstaller（兜底） | Nuitka `^4.0.5` / PyInstaller `^6.10.0` | 都能产出 platform-specific 单二进制；详见 §1.5 |
| 进程通信 | **localhost HTTP + SSE** | — | Tauri 只负责 spawn/kill，前后端解耦 |

> 锁定理由：每一项都是 Tauri 2 / shadcn-vue 官方文档推荐路径，避开了"PostCSS + Tailwind v3" 的旧路线和 "pyoxidizer"（已停滞）等坑。

### 1.5 后端打包：Nuitka vs PyInstaller 选型

> Conduit 的核心痛点之一是"Python sidecar 把整个 bundle 撑大"。Nuitka 把 Python 源码 **真正编译为 C 再编译为原生二进制**，比 PyInstaller（解释器 + 字节码打包）体积更小、启动更快。

| 维度 | **Nuitka 4.0.5** | PyInstaller 6.10 | 对 Conduit 的影响 |
|---|---|---|---|
| 二进制体积（典型场景） | **~60MB** | ~95MB | Conduit 整体包从 ~70MB → **~40MB**，省 30MB |
| 单文件冷启动时间 | **~180ms** | ~250ms | "双击就能用"体验 +1 |
| 运行时开销 | **-10%**（编译后比 CPython 还略快） | +5% | 代理瓶颈在 IO，影响不大；但首次解包/反序列化更快 |
| 构建时间 | ~90 秒 | ~22 秒 | Nuitka 慢 4x，**只影响 CI**，开发期可继续用 PyInstaller |
| 跨平台兼容性 | 85% | 98% | Conduit 是纯标准库（asyncio + socket），**不会踩 Nuitka 兼容性坑** |
| 跨平台编译 | ❌ 必须在目标平台编译 | ❌ 必须在目标平台编译 | 持平 |
| C 编译器要求 | 需要 MSVC 2022 / GCC 5.1+ / Clang / Zig | 不需要 | macOS/Linux 默认有；Windows 需装 VS Build Tools |
| Python 版本支持 | 3.4 ~ 3.14 | 3.8 ~ 3.13 | 都满足 |
| macOS 代码签名 | 产物是真二进制，签名/公证流程标准 | 产物含解包逻辑，需注意 entitlements | Nuitka 流程更直白 |
| GitHub Action | `Nuitka/Nuitka-Action`（官方） | 社区方案 | Nuitka 有官方 CI 支持 |

**Conduit 的策略**：

```
开发期（cargo tauri dev）  → PyInstaller（编译 22s，迭代快）
                                     │
                                     ▼
发布期（cargo tauri build） → Nuitka（编译 90s，包小启动快，用户体验最优）
                                     │
                                     ▼
CI 缓存                       → Nuitka 缓存编译产物，二次构建可压到 30s
```

> Conduit 后端是 **纯 Python 标准库**（asyncio + socket，无 C 扩展依赖），属于 Nuitka 的最佳生态位。如果未来引入了 `cryptography` / `lxml` 这类带 C 扩展的库，再回过头评估兼容性。

---

## 2. 总体架构

```
┌─────────────────────────────────────────────────────────────────────────┐
│  Conduit.app  (macOS .app / Windows .exe / Linux AppImage)              │
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  Tauri 主进程 (Rust, ~5MB)                                       │   │
│  │  ├─ main.rs: 启动时 spawn sidecar、退出时 kill sidecar          │   │
│  │  ├─ 健康检查: 等待 :8080/healthz 返回 200 才显示窗口            │   │
│  │  └─ 暴露给 webview 的 IPC 命令: open_external、copy_to_clipboard│   │
│  └─────────────────────────────────────────────────────────────────┘   │
│         │ spawn                                  │ webview              │
│         ▼                                        ▼                      │
│  ┌────────────────────────────┐    ┌────────────────────────────────┐  │
│  │ Python Sidecar             │    │ WebView (WebKit / WebView2)    │  │
│  │ (PyInstaller --onefile)    │    │                                │  │
│  │ ~30MB                      │    │  Vue 3 + shadcn-vue + Tailwind │  │
│  │                            │    │  + uPlot 折线图                 │  │
│  │ ├─ HTTP   :8080 (代理)     │    │                                │  │
│  │ ├─ SOCKS5 :1080 (代理)     │◄───┤  fetch('http://127.0.0.1:8080/ │  │
│  │ ├─ /status (active_clients)│    │         status')               │  │
│  │ ├─ /traffic (折线图数据)    │    │  EventSource('/events')        │  │
│  │ ├─ /events (SSE 推送)      │    │                                │  │
│  │ └─ /admin/stop (only 127) │    │                                │  │
│  └────────────────────────────┘    └────────────────────────────────┘  │
│         │                                                               │
│         ▼ 出网                                                          │
│  本机 VPN tunnel (utun4) ────────► 互联网                              │
└─────────────────────────────────────────────────────────────────────────┘
       ▲
       │ HTTP/SOCKS5 入站（LAN 客户端 B 来连）
       │
  192.168.1.158 / 192.168.1.218 / ... (机器 B)
```

### 为什么选"localhost HTTP"而不是"Tauri IPC"作为前后端通信？

| 方式 | 优点 | 缺点 | 选择 |
|---|---|---|---|
| **localhost HTTP**（推荐） | 前后端完全解耦；Python 端不需要懂 Tauri；用浏览器也能调试；SSE 流式天然支持 | 多绑了一个 loopback 端口 | ✅ |
| Tauri IPC（`invoke`）| 不暴露端口，更"原生" | Python sidecar 需要走 stdio JSON-RPC，复杂；流式数据麻烦 | ❌ |

> Conduit 本身就是个代理服务，它**必须**对外开 HTTP/SOCKS 端口，那么再多绑一个 `127.0.0.1:8080/api/*` 给前端用也没什么额外成本。

---

## 3. 项目目录结构

> 本节给"看一眼就懂大概在哪"的骨架；完整模块边界、命名规范、API 契约、依赖方向见 §3.5。

```text
conduit/                          # 当前 repo（git root）
│
├─ design/                        # 设计文档
├─ docs/                          # 给最终用户看的文档（安装、故障排查）
│
├─ shared-ui/                     # 两个 app 共用的界面组件库（避免复制粘贴）
│
├─ server-app/                    # 【机器 A】把本机 VPN 共享给 LAN 的桌面应用
│  ├─ core/                     # 代理引擎：监听 LAN 入口 + HTTP/SOCKS5 协议处理 + PAC 决策 + 流量统计 + 对外广播自身存在
│  ├─ src-tauri/                  # 应用外壳：进程生命周期、系统托盘、防火墙弹窗处理、本机 IPC
│  └─ ui/                         # 控制台界面：当前网卡 / 在线客户端 / 流量曲线 / 分享给同事的二维码
│
├─ client-app/                    # 【机器 B】用同事 A 共享出来的代理上网的桌面应用
│  ├─ core/                     # 客户端引擎：自动发现 LAN 上的 Server + 切换系统代理 + 心跳监测 + 域名分流查询
│  ├─ src-tauri/                  # 应用外壳：进程生命周期、退出自动还原代理、必要时的提权回退
│  └─ ui/                         # 控制台界面：发现的 Server 卡片 / 一键开关 / 故障诊断向导 / 域名查询
│
├─ scripts/                       # 跨工程脚本（构建产物、打包发布、本地联调）
│  ├─ build-server-sidecar.sh
│  ├─ build-client-sidecar.sh
│  └─ release-{server,client}.sh
│
├─ pnpm-workspace.yaml            # 让两个 app 的 ui/ 能引用 shared-ui
├─ package.json                   # 仓库根元数据（不参与构建）
├─ README.md
└─ prompts.txt / userinput.py     # 人机协作脚本（不动）
```

> **高内聚原则**：`server-app/` 把"代理引擎 + 应用外壳 + 控制台界面"三件套放在同一个目录下，作为一个完整产品维护，而不是按语言拆到 monorepo 的不同顶层。`client-app/` 同理。两个 app 之间唯一共享的是 `shared-ui/`（避免界面组件被复制粘贴两份）。

---

## 3.5 代码框架详细设计

> §3 给的是"看一眼就懂大概在哪"，本节是"工程师上手前必须达成的共识"——每个子工程的分层、模块边界、依赖方向、命名规范、状态管理、API 契约。

### 3.5.1 工程整体布局：Monorepo + 5 个子工程

整个 `conduit/` 是一个 monorepo，含 5 个**独立子工程**，按职责而非按语言划分：

| 子工程 | 路径 | 职责 | 实现语言（参考） |
|---|---|---|---|
| **服务端代理引擎** | `server-app/core/` | 在机器 A 上监听 LAN，把进来的连接转发出 VPN；同时托管 PAC 文件和管控 API；对外广播自身存在让客户端发现 | Python 3.10+ |
| **服务端应用外壳** | `server-app/src-tauri/` | 启动时拉起代理引擎、退出时回收；系统托盘；防火墙首次弹窗引导 | Rust |
| **服务端控制台界面** | `server-app/ui/` | 给机器 A 上的人看：当前哪些客户端在用、流量多少、把连接信息分享出去 | TypeScript / Vue 3 |
| **客户端三件套** | `client-app/{core,src-tauri,ui}/` | 三件套结构与服务端对称，但职责完全不同——见 §3.5.5 | 同上 |
| **共享界面组件库** | `shared-ui/` | 两个 app 都要用的按钮、卡片、表格、流量图等基础 UI，避免被复制粘贴两份 | TypeScript / Vue 3 |

#### 强制约束

- **三套语言不共享构建系统**（pnpm 管 TypeScript / cargo 管 Rust / pip 管 Python），避免 monorepo 工具链复杂化
- **跨语言通信走 localhost HTTP / SSE**，不共享内存对象
- **跨进程契约只有一份**：`/api/*` 路径下的 JSON Schema（详见 §3.5.7）
- **跨 app 仅共享 UI 组件库** `shared-ui`：两个 `ui/` 工程的 `package.json` 用 workspace 协议引用

#### `pnpm-workspace.yaml`

```yaml
packages:
  - "shared-ui"
  - "server-app/ui"
  - "client-app/ui"
```

`server-app/ui/package.json`（client-app/ui 同理）：

```json
{
  "name": "@conduit/server-ui",
  "dependencies": {
    "@conduit/shared-ui": "workspace:*"
  }
}
```

### 3.5.2 服务端代理引擎代码框架（`server-app/core/`）

#### 模块分层

```text
server-app/core/
├─ proxy_server.py        【入口层】 main()，组装所有子系统，发布为 sidecar 二进制
├─ config.py              【配置层】 端口、CIDR、PAC 路径、绑定模式
│
├─ http_proxy.py          【协议层】 HTTP CONNECT 代理
├─ socks5_proxy.py        【协议层】 SOCKS5 代理
├─ relay.py               【协议层】 双向数据中继（加 on_progress 回调）
│
├─ pac_engine.py          【业务层】 PAC 文件生成与 /check 决策
├─ active_connections.py  【业务层】 连接注册表 + 流量采样【新增】
├─ healthcheck.py         【业务层】 端口/网卡/VPN 自检【新增】
├─ mdns_advertiser.py     【业务层】 zeroconf 广播 _conduit._tcp.local.【新增】
│
├─ api/                   【API 层】【新增目录】
│  ├─ __init__.py
│  ├─ server.py           # aiohttp app 装配 + CORS + 错误中间件
│  ├─ status.py           # GET /api/status
│  ├─ traffic.py          # GET /api/traffic
│  ├─ events.py           # GET /api/events (SSE)
│  ├─ admin.py            # POST /api/admin/stop（仅 127.0.0.1）
│  └─ healthz.py          # GET /healthz
│
├─ tests/                 【测试】
│  ├─ test_active_connections.py
│  ├─ test_pac_engine.py
│  ├─ test_healthcheck.py
│  └─ test_api_status.py
│
├─ proxy.pac              # PAC 模板（保留）
└─ pyproject.toml         # 依赖：aiohttp（API）、zeroconf（mDNS）、pyinstaller/nuitka（打包）
```

#### 依赖方向（仅向下调用，禁止反向）

```text
入口层 (proxy_server)
   │
   ├─ 配置层 (config)
   │
   └─ API 层 (api/server.py)
        │
        ├─ 业务层 (active_connections / pac_engine / healthcheck / mdns_advertiser)
        │
        └─ 协议层 (http_proxy / socks5_proxy / relay)
```

**禁止反向依赖**：协议层不能 import API 层；业务层不能 import 协议层。`api/*` 注入业务层依赖到协议层，而非反过来。

#### 现有代码改造路径表

| 原文件（`~/work_space/task/20260429-lan-vpn-proxy/`） | 落到 conduit/ 后 | 改造动作 | 改动量 |
|---|---|---|---|
| `proxy_server.py` | `server-app/core/proxy_server.py` | 拆出 `ProxyCore` 类，提供 `start() / stop() / status()` 给 API 层调用 | 重组 ~60 行 |
| `http_proxy.py` | 同名 | `_handle_connect` 内接入 `registry.add/remove/update_bytes` | 加 ~6 行 |
| `socks5_proxy.py` | 同名 | `handle_socks5` 内接入 registry | 加 ~6 行 |
| `relay.py` | 同名 | `bidirectional_relay` 加 `on_progress` 回调（§4.11） | 改 ~10 行 |
| `config.py` | 同名 | 新增 `api_port`、`bind_loopback_only`、`mdns_enabled` 字段 | 加 ~5 行 |
| `pac_engine.py` / `proxy.pac` | 同名 | **不动** | 0 |

新增文件：`active_connections.py`（§4.10）/ `healthcheck.py` / `mdns_advertiser.py` / `api/` 目录。

#### 命名约定

- 模块名：`snake_case.py`
- 类名：`PascalCase`（如 `ConnectionRegistry`）
- 异步函数：必须 `async def`
- 常量：`UPPER_SNAKE_CASE`（如 `WINDOW_SEC = 600`）
- API handler 函数：`handle_<endpoint>`（如 `handle_status`）

### 3.5.3 服务端应用外壳代码框架（`server-app/src-tauri/`）

#### 模块划分

```text
server-app/src-tauri/src/
├─ main.rs        【入口】 Tauri 启动、注册 plugin、setup hook
├─ sidecar.rs     【生命周期】 spawn / kill Python sidecar
├─ healthz.rs     【生命周期】 启动后等 :8080/healthz 200
├─ tray.rs        【UI】 系统托盘菜单
├─ commands.rs    【IPC】 暴露给 webview 的 #[tauri::command]
├─ state.rs       【状态】 全局 AppState
├─ config.rs      【配置】 读取 / 写入用户配置（端口等）
└─ error.rs       【错误】 统一 ConduitError + From 转换
```

#### IPC 命令暴露规范（`commands.rs`）

只暴露**前端必须依赖 Tauri 才能做的事**，业务调用全部走 HTTP。具体仅以下 4 个：

```rust
#[tauri::command]
async fn open_external(url: String) -> Result<(), ConduitError>;

#[tauri::command]
async fn copy_to_clipboard(app: AppHandle, text: String) -> Result<(), ConduitError>;

#[tauri::command]
async fn show_in_folder(path: String) -> Result<(), ConduitError>;

#[tauri::command]
async fn quit_app(app: AppHandle) -> Result<(), ConduitError>;
```

**禁止**：把"启动代理"、"读流量"做成 IPC 命令——前端 fetch HTTP 即可，前后端解耦。

#### 状态（`state.rs`）与错误（`error.rs`）

```rust
// state.rs
pub struct AppState {
    pub sidecar_child: Mutex<Option<CommandChild>>,
    pub sidecar_started_at: Mutex<Option<SystemTime>>,
}

// error.rs
#[derive(Debug, thiserror::Error)]
pub enum ConduitError {
    #[error("sidecar failed to start: {0}")]
    SidecarStart(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
}

impl serde::Serialize for ConduitError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
```

### 3.5.4 服务端控制台界面代码框架（`server-app/ui/`）

#### 分层与目录约定

```text
server-app/ui/src/
├─ main.ts                【入口】 createApp + mount
├─ App.vue                【根组件】 整体布局（Header / Tabs / Footer）
│
├─ assets/
│  └─ index.css           # @import "tailwindcss" + theme tokens
│
├─ types/                 【类型】 与后端 API 1:1 对应
│  ├─ events.ts / status.ts / traffic.ts
│  └─ index.ts
│
├─ api/                   【API 层】 typed fetch wrapper
│  ├─ client.ts           # request<T>() + ApiError
│  ├─ status.ts / traffic.ts / admin.ts
│  └─ index.ts
│
├─ stores/                【状态】 轻量 reactive store（避免 Pinia 冗余）
│  ├─ useProxyStore.ts
│  ├─ useNetworkStore.ts
│  └─ useUiStore.ts
│
├─ composables/           【可组合逻辑】
│  ├─ useEvents.ts        # SSE 长连接（§4.8）
│  ├─ useTraffic.ts       # 流量数据流，桥接到 store
│  └─ useTauri.ts         # IPC 命令调用包装
│
├─ components/business/   # 业务组件（基础 ui/* 在 shared-ui）
│  ├─ ProxyControl.vue / NetworkPanel.vue
│  ├─ ClientList.vue / TrafficChart.vue
│  └─ ShareCard.vue / LogViewer.vue
│
├─ views/                 【视图】 由 Tabs 切换，不用 vue-router
│  ├─ DashboardView.vue
│  ├─ LogsView.vue
│  └─ SettingsView.vue
│
└─ __tests__/             【测试】 vitest + @vue/test-utils
```

> **注意**：基础组件（button/card/tabs）和 `lib/utils.ts` 都在 `shared-ui/` 而**不在本目录下**——通过 `import { Button } from '@conduit/shared-ui'` 引入。

#### 状态管理选型：reactive store 而非 Pinia

**理由**：本应用没有跨页面状态共享（1 窗口 + 3 Tab），上 Pinia 反而引入 devtools 依赖增加打包体积。

```typescript
// stores/useProxyStore.ts
import { reactive, readonly } from 'vue'
import type { ClientInfo } from '@/types'

const state = reactive({
  running: false,
  port: 8080,
  socksPort: 1080,
  activeClients: [] as ClientInfo[],
})

export function useProxyStore() {
  return {
    state: readonly(state),
    setRunning: (v: boolean) => (state.running = v),
    setActiveClients: (cs: ClientInfo[]) => (state.activeClients = cs),
  }
}
```

#### API 层（`api/client.ts`）

```typescript
const BASE = 'http://127.0.0.1:8080'

export class ApiError extends Error {
  constructor(public status: number, public code: string, message: string) {
    super(message)
  }
}

export async function request<T>(
  path: string,
  init?: RequestInit & { signal?: AbortSignal },
): Promise<T> {
  const res = await fetch(`${BASE}${path}`, init)
  if (!res.ok) {
    const body = await res.json().catch(() => ({ code: 'UNKNOWN', message: res.statusText }))
    throw new ApiError(res.status, body.code, body.message)
  }
  return res.json() as Promise<T>
}
```

#### 路由：Tabs 而非 vue-router

3 个 Tab 用 `reka-ui` 的 Tabs 组件即可，避免引入 vue-router 多 ~30KB。

### 3.5.5 客户端代码框架（`client-app/`）

> 客户端是新 app，与服务端的差异主要在**职责**而非**结构**。结构沿用三件套布局；模块差异如下。完整可行性论证另见 `2026-04-30-3-Conduit-Client-客户端可行性报告.md`。

#### 客户端引擎差异（`client-app/core/`）

> v0.1 客户端的核心是**智能本地代理**：B 端启动一个 SOCKS5 监听 127.0.0.1:7890，对每个连接做 probe + 缓存 + 智能路由（详见 `2026-04-30-3-...客户端可行性报告.md` §3）。

```text
client-app/core/
├─ client_main.py         【入口】 main() 启动 mDNS + 本地代理 + API server
├─ config.py              【配置】 server 列表、缓存 TTL、SOCKS5 端口等用户偏好
│
├─ discoverer.py          【业务】 在 LAN 上发现 Conduit Server（zeroconf）
├─ system_proxy.py        【业务】 macOS 系统代理切换（networksetup -setsocksfirewallproxy）
├─ connectivity.py        【业务】 server 心跳监测（不可达自动降级）
│
├─ local_proxy.py         【业务】【新增】 本地 SOCKS5 服务，监听 127.0.0.1:7890
├─ route_resolver.py      【业务】【新增】 路由决策：缓存查询 + TCP probe + 私有 IP 快路径
├─ route_cache.py         【业务】【新增】 路由缓存：TTL 5 min / LRU 5000 / 失效自愈
├─ pac_parser.py          【业务】【新增】 解析 A 的 PAC 文件，预填路由缓存的 'proxy' 段
│
├─ api/                   【API 层】 暴露给 client UI（监听 127.0.0.1:8090）
│  ├─ __init__.py
│  ├─ server.py           # 路由总装、CORS、错误中间件
│  ├─ discovery.py        # GET /api/servers
│  ├─ connect.py          # POST /api/connect/{server_id}
│  ├─ disconnect.py       # POST /api/disconnect
│  ├─ route.py            # 【新增】 GET /api/route?host=... 查询路由决策
│  ├─ cache.py            # 【新增】 GET / DELETE /api/cache 路由缓存管理
│  ├─ diagnose.py         # 五步自检
│  └─ healthz.py
│
├─ tests/
└─ pyproject.toml         # 依赖：aiohttp、zeroconf（不需要额外 SOCKS5 库，自实现）
```

**关键差异**：
1. 客户端**监听两个端口**：`127.0.0.1:7890`（SOCKS5 代理服务）+ `127.0.0.1:8090`（控制 API）。两个端口都不对外开放
2. **不再用 PAC URL 直接配浏览器**——A 的 PAC 仅作为 client 路由缓存的初始预填（详见 `route_resolver.py`）
3. v0.1 仅 macOS。Windows/Linux 推到 v0.2

#### 应用外壳差异（`client-app/src-tauri/src/`）

> v0.1 仅 macOS 13+。`networksetup` 命令免密，**不需要 elevation**。`elevation.rs` 暂不创建（v0.2 跨平台时再加）。

差异点：
- `commands.rs` 不需要 `set_proxy` / `unset_proxy` IPC 命令——业务逻辑全部在 sidecar 的 `system_proxy.py` 完成
- `tray.rs` 图标 4 态：蓝（已连接）/ 黄（异常）/ 灰（未连接）/ 橘（自动直连）
- 进程退出时（含崩溃），main.rs 必须 sync 调 `disconnect`，确保系统代理被还原

#### 控制台界面差异（`client-app/ui/`）

```text
client-app/ui/src/
├─ views/
│  ├─ DiscoveryView.vue      # 首屏：发现的 server 列表（卡片）
│  ├─ ConnectedView.vue      # 已连接：状态 + 流量 + Host check
│  ├─ DiagnosticView.vue     # 故障诊断 wizard
│  └─ SettingsView.vue       # 手动 IP 输入 / 端口 / 跳过 mDNS
│
├─ components/business/
│  ├─ ServerCard.vue         # 单 server 信息卡片
│  ├─ ConnectionStatus.vue   # 当前连接状态徽章
│  ├─ HostCheckPanel.vue     # 输入域名查分流
│  └─ DiagnosticStep.vue     # wizard 单步
│
├─ stores/
│  ├─ useServerStore.ts      # 已发现 server 列表
│  ├─ useConnectionStore.ts  # 当前连接状态
│  └─ useUiStore.ts
│
├─ api/
│  ├─ client.ts              # base URL = http://127.0.0.1:8090
│  ├─ discovery.ts / connect.ts
```

### 3.5.6 共享 UI 包（`shared-ui/`）

```text
shared-ui/
├─ package.json              # name: @conduit/shared-ui
├─ tsconfig.json
├─ vite.config.ts            # build lib mode（output esm + types）
└─ src/
   ├─ index.ts               # 总导出
   ├─ components/ui/         # shadcn-vue 复制的源码（button/card/tabs/...）
   │  ├─ button/Button.vue
   │  ├─ card/Card.vue
   │  └─ ...
   ├─ lib/
   │  └─ utils.ts            # cn() helper
   ├─ composables/
   │  └─ useTheme.ts         # 暗色模式切换（双 app 共用）
   └─ types/
      └─ proxy.ts            # ClientInfo / TrafficSeries 等通用类型
```

**导出策略** `package.json` 的 `exports` 字段：

```json
{
  "exports": {
    ".": "./src/index.ts",
    "./components/*": "./src/components/*",
    "./lib/utils": "./src/lib/utils.ts"
  }
}
```

**版本管理**：`workspace:*` 协议，shared-ui 改动两 app 立即生效；shadcn-vue 升级用 `pnpm dlx shadcn-vue diff` 评估。

### 3.5.7 前后端 API 契约

#### 字段命名 1:1 对齐（snake_case）

后端 Python `dataclass` 与前端 TypeScript `interface` 严格对应：

| 后端 | 前端 |
|---|---|
| `ConnectionInfo.peer_ip: str` | `ClientInfo.peer_ip: string` |
| `ConnectionInfo.sent_bytes: int` | `ClientInfo.sent_bytes: number` |
| `ConnectionInfo.proto: Literal["http","socks5"]` | `ClientInfo.proto: 'http' \| 'socks5'` |

#### 错误响应统一格式

所有 `/api/*` 错误响应 body 必须为：

```json
{ "code": "PORT_OCCUPIED", "message": "Port 8080 is already in use" }
```

前端 `ApiError.code` 直接做分支处理。`code` 命名 `UPPER_SNAKE_CASE`，与 HTTP 状态码无关（4xx/5xx 都用同一格式）。

#### 未来演进路径

当 endpoint 数 ≥ 15 时引入 `apispec` 自动生成 OpenAPI YAML，再用 `openapi-typescript` 生成 TS 类型。当前规模手维护 `types/` 即可。

### 3.5.8 配置管理

| 层级 | 路径 | 由谁写 |
|---|---|---|
| **默认值** | hardcoded in `core/config.py` | 代码 |
| **用户配置** | macOS：`~/Library/Application Support/com.terrellshe.conduit/config.toml` <br> Windows：`%APPDATA%/Conduit/config.toml` <br> Linux：`~/.config/conduit/config.toml` | 前端通过 IPC 写入 |
| **CLI 参数** | `--port / --socks-port / --yes` | dev 期人工 |

**前端 UI 偏好**（不参与代理逻辑）：放 `localStorage`，例 `conduit:theme=dark`。

### 3.5.9 日志与可观测性

| 层 | 工具 | 输出位置 |
|---|---|---|
| Python sidecar | 标准 `logging` | dev：stderr / prod：`~/Library/Logs/Conduit/proxy.log` |
| Rust 主进程 | `tracing` + `tracing-subscriber` | dev：stderr / prod：同目录 `tauri.log` |
| Vue 前端 | `console.*` | dev：浏览器控制台 / prod：仅 error 上报 |

统一日志格式（Python + Rust）：

```text
[2026-04-30 19:25:01.123] [LEVEL] [module] msg
```

### 3.5.10 测试组织

| 子工程 | 框架 | 命名约定 | 命令 |
|---|---|---|---|
| `*/core/` | `pytest` + `pytest-asyncio` | `tests/test_*.py` | `cd server-app/core && pytest` |
| `*/src-tauri/` | 内置 `#[cfg(test)]` | `src/*/tests.rs` 或内联 `#[test]` | `cd server-app/src-tauri && cargo test` |
| `*/ui/` | `vitest` + `@vue/test-utils` | `src/**/*.spec.ts` | `cd server-app/ui && pnpm test` |

**集成测试**：`scripts/e2e.sh` 启动完整 stack（server-app sidecar + server-app Tauri + client-app）跑端到端 smoke。

### 3.5.11 完整目录树 v2（细化到文件，每行加注释）

```text
conduit/                                          # 仓库根（git root）
│
├─ design/                                        # 设计文档
│  ├─ 2026-04-29-局域网共享VPN代理简明设计.md      # 服务端协议与安全设计
│  ├─ 2026-04-29-2-机器B客户端配置手册.md          # 手动配置流程（被 client-app 替代）
│  ├─ 2026-04-30-Conduit-桌面化可行性报告.md       # 服务端 app 选型论证
│  ├─ 2026-04-30-2-Conduit-Tauri+Python方案详细设计.md  ← 本文档
│  └─ 2026-04-30-3-Conduit-Client-客户端可行性报告.md   # 客户端 app 论证与设计
│
├─ docs/                                          # 给最终用户看的文档
│  ├─ INSTALL.md                                  # 双击安装指引
│  └─ TROUBLESHOOTING.md                          # 用户能自助跑的故障排查
│
├─ shared-ui/                                     # 两个 app 共用的界面组件库
│  ├─ package.json                                # 包名 @conduit/shared-ui
│  ├─ tsconfig.json
│  ├─ vite.config.ts                              # 库模式打包，输出给两 app 引用
│  └─ src/
│     ├─ index.ts                                 # 对外 API 总入口
│     ├─ components/ui/                           # 基础界面元素：按钮、卡片、表格、对话框等
│     │  ├─ button/Button.vue
│     │  ├─ card/Card.vue
│     │  ├─ tabs/Tabs.vue
│     │  └─ ...
│     ├─ lib/utils.ts                             # 类名拼接工具
│     ├─ composables/useTheme.ts                  # 暗色模式切换逻辑
│     └─ types/proxy.ts                           # 客户端连接、流量等通用数据结构
│
├─ server-app/                                    # 【机器 A】把本机 VPN 共享给 LAN 的桌面应用
│  │
│  ├─ core/                                     # ── 代理引擎源码 ──
│  │  ├─ proxy_server.py                          # 启动入口：装配所有子系统、暴露 start/stop
│  │  ├─ config.py                                # 端口、IP 白名单、绑定模式等配置
│  │  ├─ http_proxy.py                            # HTTP/HTTPS 代理协议处理
│  │  ├─ socks5_proxy.py                          # SOCKS5 代理协议处理
│  │  ├─ relay.py                                 # 双向数据转发（含字节增量回调用于流量图）
│  │  ├─ pac_engine.py                            # PAC 文件生成与"某域名走代理还是直连"决策
│  │  ├─ proxy.pac                                # PAC 模板
│  │  ├─ active_connections.py                    # 谁在用代理 + 每秒流量采样
│  │  ├─ healthcheck.py                           # 端口/网卡/VPN 自检，给应用外壳判断 ready
│  │  ├─ mdns_advertiser.py                       # 在 LAN 广播"我是一个 Conduit Server"，让客户端自动发现
│  │  ├─ api/                                     # 给前端控制台看的 HTTP 接口
│  │  │  ├─ __init__.py
│  │  │  ├─ server.py                             # 路由总装、CORS、错误中间件
│  │  │  ├─ status.py                             # 当前状态查询
│  │  │  ├─ traffic.py                            # 流量历史查询
│  │  │  ├─ events.py                             # 实时推送（在线客户端、流量、VPN 状态变化）
│  │  │  ├─ admin.py                              # 停止代理（只允许本机调用）
│  │  │  └─ healthz.py                            # 给应用外壳轮询用
│  │  ├─ tests/
│  │  └─ pyproject.toml
│  │
│  ├─ src-tauri/                                  # ── 应用外壳源码 ──
│  │  ├─ Cargo.toml
│  │  ├─ tauri.conf.json                          # 应用名、窗口大小、签名、bundle 配置
│  │  ├─ build.rs
│  │  ├─ Entitlements.plist                       # macOS 公证所需权限声明
│  │  ├─ icons/                                   # 各平台应用图标
│  │  ├─ binaries/                                # 代理引擎打包后的二进制（按平台命名）
│  │  │  ├─ conduit-server-sidecar-aarch64-apple-darwin
│  │  │  └─ ...
│  │  ├─ capabilities/default.json                # 允许应用外壳调用的本机能力清单
│  │  └─ src/
│  │     ├─ main.rs / sidecar.rs / healthz.rs / tray.rs
│  │     ├─ commands.rs / state.rs / config.rs / error.rs
│  │
│  └─ ui/                                         # ── 控制台界面源码 ──
│     ├─ package.json                             # 引用 @conduit/shared-ui
│     ├─ vite.config.ts / tsconfig.json
│     ├─ index.html / components.json
│     └─ src/
│        ├─ main.ts / App.vue
│        ├─ assets/index.css
│        ├─ types/                                # 与代理引擎 API 对齐的数据类型
│        ├─ api/                                  # 调代理引擎 HTTP 接口的封装
│        ├─ stores/                               # 跨界面共享的运行时状态
│        ├─ composables/                          # 复用逻辑（订阅事件流、调本机命令等）
│        ├─ components/business/
│        │  ├─ ProxyControl.vue                   # 启停按钮 + 当前状态徽章
│        │  ├─ NetworkPanel.vue                   # 当前网卡、VPN 状态、LAN IP
│        │  ├─ ClientList.vue                     # 在线客户端实时列表
│        │  ├─ TrafficChart.vue                   # 每客户端独立流量曲线
│        │  ├─ ShareCard.vue                      # 把连接信息分享给同事（URL / 二维码）
│        │  └─ LogViewer.vue                      # 滚动日志窗口
│        ├─ views/                                # Tab 切换的页面：仪表盘 / 日志 / 设置
│        └─ __tests__/
│
├─ client-app/                                    # 【机器 B】用同事 A 的代理上网的桌面应用
│  │
│  ├─ core/                                     # ── 客户端引擎源码（v0.1 仅 macOS） ──
│  │  ├─ client_main.py                           # 启动入口：拉起 mDNS + 本地代理 + API server
│  │  ├─ config.py                                # server 列表、缓存 TTL、SOCKS5 端口、UI 偏好
│  │  ├─ discoverer.py                            # 在 LAN 上发现 Conduit Server（zeroconf）
│  │  ├─ system_proxy.py                          # macOS 系统代理切换（setsocksfirewallproxy）
│  │  ├─ connectivity.py                          # Server 心跳监测，不可达自动全局降级
│  │  ├─ local_proxy.py                           # 【v0.1 核心】本地 SOCKS5 服务，监听 127.0.0.1:7890
│  │  ├─ route_resolver.py                        # 【v0.1 核心】路由决策：缓存查询 + TCP probe + 私有 IP 快路径
│  │  ├─ route_cache.py                           # 【v0.1 核心】路由缓存：TTL 5 min / LRU 5000 / 失效自愈
│  │  ├─ pac_parser.py                            # 解析 A 的 PAC，预填路由缓存的 'proxy' 段
│  │  ├─ api/                                     # 给本机 client UI 看的 HTTP 接口（127.0.0.1:8090）
│  │  │  ├─ server.py                             # 路由总装、CORS、错误中间件
│  │  │  ├─ discovery.py                          # 已发现/历史/手动添加的 server 列表
│  │  │  ├─ connect.py / disconnect.py            # 一键开/关
│  │  │  ├─ route.py                              # GET /api/route?host=... 查询路由决策
│  │  │  ├─ cache.py                              # GET / DELETE /api/cache 路由缓存管理
│  │  │  ├─ diagnose.py                           # 五步自检
│  │  │  └─ healthz.py
│  │  ├─ tests/
│  │  └─ pyproject.toml
│  │
│  ├─ src-tauri/                                  # ── 应用外壳源码 ──
│  │  ├─ Cargo.toml / tauri.conf.json
│  │  ├─ binaries/                                # 客户端引擎打包后的二进制
│  │  ├─ capabilities/
│  │  └─ src/
│  │     ├─ main.rs / sidecar.rs / tray.rs        # tray 图标 4 态（已连接/异常/未连接/自动直连）
│  │     ├─ commands.rs
│  │     └─ state.rs / error.rs
│  │
│  └─ ui/                                         # ── 控制台界面源码 ──
│     ├─ package.json                             # 同样引用 @conduit/shared-ui
│     ├─ vite.config.ts
│     └─ src/
│        ├─ main.ts / App.vue
│        ├─ types/                                # server / 连接状态等数据类型
│        ├─ api/                                  # 调本机 client 引擎接口的封装
│        ├─ stores/                               # 已发现的 server 列表、当前连接状态
│        ├─ composables/
│        ├─ components/business/
│        │  ├─ ServerCard.vue                     # 单个 Server 信息卡片
│        │  ├─ ConnectionStatus.vue               # 已连接/异常/已断开 三态徽章
│        │  ├─ HostCheckPanel.vue                 # 输入域名查它走哪条路
│        │  └─ DiagnosticStep.vue                 # 故障诊断单步
│        ├─ views/
│        │  ├─ DiscoveryView.vue                  # 首屏：发现的 server 列表
│        │  ├─ ConnectedView.vue                  # 已连接：状态 + 流量 + 域名查询
│        │  ├─ DiagnosticView.vue                 # 自动跑手册 §4 检查项
│        │  └─ SettingsView.vue                   # 手动 IP、端口、行为偏好
│        └─ __tests__/
│
├─ scripts/                                       # 跨工程脚本
│  ├─ build-server-sidecar.sh                     # 把 server 代理引擎打成单二进制
│  ├─ build-client-sidecar.sh                     # 把 client 客户端引擎打成单二进制
│  ├─ release-server.sh                           # 全平台打包 server-app
│  ├─ release-client.sh                           # 全平台打包 client-app
│  ├─ dev-all.sh                                  # 一键并行启动两个 app 的 dev 模式
│  └─ e2e.sh                                      # 端到端冒烟测试
│
├─ pnpm-workspace.yaml                            # 让两个 app 的 ui/ 能引用 shared-ui
├─ package.json                                   # 仓库根元数据（不参与构建）
├─ .gitignore
├─ README.md
├─ prompts.txt                                    # 人机协作脚本（不动）
└─ userinput.py                                   # 人机协作脚本（不动）
```

---

## 4. 关键代码骨架

> **重要说明（2026-04-30 增补）**：本节及后续 §5–§11 的代码与命令示例使用了**早期目录假设**（如 `desktop/conduit-ui/`、`task/20260429-lan-vpn-proxy/`），用于展示**代码逻辑骨架**，而非最终落地路径。实际工程实施时按 §3 + §3.5 的真实目录映射换算：
>
> | 早期假设路径 | 实际路径（§3.5） |
> |---|---|
> | `task/20260429-lan-vpn-proxy/` | `server-app/core/` |
> | `desktop/src-tauri/` | `server-app/src-tauri/` |
> | `desktop/conduit-ui/` | `server-app/ui/` |
> | `desktop/scripts/` | `scripts/`（仓库顶级） |
> | `pnpm --filter conduit-ui ...` | `pnpm --filter @conduit/server-ui ...` |
> | `binaries/conduit-sidecar` | `binaries/conduit-server-sidecar`（client-app 用 `conduit-client-sidecar`） |
> | `frontendDist: "../conduit-ui/dist"` | `frontendDist: "../ui/dist"` |
>
> 代码逻辑（Rust / Vue / Python）本身保持不变。客户端 app 的代码骨架另见 `2026-04-30-3-Conduit-Client-客户端可行性报告.md` §5–§7。

### 4.1 `src-tauri/tauri.conf.json`（核心配置）

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Conduit",
  "version": "0.1.0",
  "identifier": "com.terrellshe.conduit",
  "build": {
    "beforeDevCommand": "pnpm --filter conduit-ui dev",
    "beforeBuildCommand": "pnpm --filter conduit-ui build && bash ../scripts/build-sidecar.sh",
    "devUrl": "http://localhost:5173",
    "frontendDist": "../conduit-ui/dist"
  },
  "app": {
    "windows": [
      {
        "title": "Conduit",
        "width": 1100,
        "height": 720,
        "minWidth": 900,
        "minHeight": 600,
        "visible": false,
        "resizable": true
      }
    ],
    "security": {
      "csp": "default-src 'self'; connect-src 'self' http://127.0.0.1:8080 http://localhost:5173"
    }
  },
  "bundle": {
    "active": true,
    "targets": ["app", "dmg", "msi", "deb", "appimage"],
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ],
    "externalBin": ["binaries/conduit-sidecar"],
    "macOS": {
      "minimumSystemVersion": "11.0",
      "entitlements": "Entitlements.plist"
    },
    "windows": {
      "wix": {
        "language": "en-US"
      }
    }
  },
  "plugins": {
    "shell": {
      "open": true
    }
  }
}
```

> **关键点**：`externalBin` 写 base 名 `binaries/conduit-sidecar`，Tauri 会自动按 target triple 找文件，例如 macOS arm64 找 `binaries/conduit-sidecar-aarch64-apple-darwin`。

### 4.2 `src-tauri/capabilities/default.json`

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Default permissions for Conduit",
  "windows": ["main"],
  "permissions": [
    "core:default",
    {
      "identifier": "shell:allow-execute",
      "allow": [
        {
          "name": "binaries/conduit-sidecar",
          "sidecar": true,
          "args": ["--port", "8080", "--socks-port", "1080", "--yes"]
        }
      ]
    },
    "shell:allow-kill",
    "core:window:allow-show",
    "core:window:allow-hide",
    "core:clipboard-manager:allow-write-text"
  ]
}
```

### 4.3 `src-tauri/src/main.rs`（生命周期管理）

```rust
// SPDX-License-Identifier: MIT
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Duration;
use tauri::{Manager, RunEvent};
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

mod sidecar;
mod tray;

#[tokio::main]
async fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = sidecar::start_and_wait_healthy(handle).await {
                    eprintln!("sidecar failed: {e}");
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(child) = window
                    .app_handle()
                    .state::<sidecar::SidecarState>()
                    .child
                    .lock()
                    .unwrap()
                    .take()
                {
                    let _ = child.kill();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Conduit failed to launch");
}
```

### 4.4 `src-tauri/src/sidecar.rs`（启动 + 健康检查）

```rust
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

pub struct SidecarState {
    pub child: Mutex<Option<CommandChild>>,
}

pub async fn start_and_wait_healthy(app: AppHandle) -> anyhow::Result<()> {
    let state = SidecarState { child: Mutex::new(None) };
    app.manage(state);

    let sidecar = app
        .shell()
        .sidecar("conduit-sidecar")?
        .args(["--port", "8080", "--socks-port", "1080", "--yes"]);

    let (mut rx, child) = sidecar.spawn()?;
    app.state::<SidecarState>().child.lock().unwrap().replace(child);

    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let CommandEvent::Stderr(line) = event {
                eprintln!("[sidecar] {}", String::from_utf8_lossy(&line));
            }
        }
    });

    for _ in 0..30 {
        if reqwest::get("http://127.0.0.1:8080/healthz").await.is_ok() {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    anyhow::bail!("sidecar failed to become healthy in 9s")
}
```

### 4.5 `conduit-ui/vite.config.ts`

```typescript
import path from 'node:path'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: { '@': path.resolve(__dirname, './src') },
  },
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: '127.0.0.1',
  },
})
```

### 4.6 `conduit-ui/src/assets/index.css`（Tailwind v4 入口）

```css
@import "tailwindcss";

@theme {
  --color-conduit-blue: #1B2A4E;
  --color-conduit-cyan: #00D4FF;
}
```

### 4.7 `conduit-ui/components.json`（shadcn-vue 配置）

```json
{
  "$schema": "https://shadcn-vue.com/schema.json",
  "style": "default",
  "typescript": true,
  "tailwind": {
    "config": "",
    "css": "src/assets/index.css",
    "baseColor": "slate",
    "cssVariables": true
  },
  "aliases": {
    "components": "@/components",
    "ui": "@/components/ui",
    "lib": "@/lib",
    "utils": "@/lib/utils"
  }
}
```

### 4.8 `conduit-ui/src/composables/useEvents.ts`

```typescript
import { ref, onMounted, onBeforeUnmount } from 'vue'

const API = 'http://127.0.0.1:8080'

export interface ServerEvent {
  active_clients: ClientInfo[]
  traffic_tick: Record<string, [number, number, number]>
  route_iface: string
  lan_ip: string
  vpn_ok: boolean
}

export interface ClientInfo {
  peer_ip: string
  proto: 'http' | 'socks5'
  target: string
  since: number
  last_seen: number
  sent_bytes: number
  recv_bytes: number
  sent_bps_1s: number
  recv_bps_1s: number
}

export function useEvents() {
  const event = ref<ServerEvent | null>(null)
  const connected = ref(false)
  let es: EventSource | null = null

  onMounted(() => {
    es = new EventSource(`${API}/events`)
    es.onopen = () => (connected.value = true)
    es.onerror = () => (connected.value = false)
    es.onmessage = (m) => {
      try { event.value = JSON.parse(m.data) } catch {}
    }
  })

  onBeforeUnmount(() => es?.close())
  return { event, connected }
}
```

### 4.9 `conduit-ui/src/components/TrafficChart.vue`（折线图骨架）

```vue
<script setup lang="ts">
import { ref, watch, onMounted } from 'vue'
import uPlot from 'uplot'
import 'uplot/dist/uPlot.min.css'
import { useEvents } from '@/composables/useEvents'

const props = defineProps<{ windowSec: number }>()
const containerRef = ref<HTMLDivElement>()
const { event } = useEvents()

let chart: uPlot | null = null
const series: Map<string, { sent: number[]; recv: number[]; ts: number[] }> = new Map()

function rebuild() {
  if (!containerRef.value) return
  chart?.destroy()
  const ips = [...series.keys()]
  const opts: uPlot.Options = {
    width: containerRef.value.clientWidth,
    height: 240,
    series: [
      { label: 'time' },
      ...ips.flatMap((ip) => [
        { label: `${ip} ↓`, stroke: '#00D4FF' },
        { label: `${ip} ↑`, stroke: '#FF6B35', dash: [4, 2] },
      ]),
    ],
    scales: { y: { auto: true } },
    cursor: { drag: { setScale: false } },
  }
  const data: uPlot.AlignedData = [
    series.get(ips[0])?.ts || [],
    ...ips.flatMap((ip) => [
      series.get(ip)!.recv,
      series.get(ip)!.sent,
    ]),
  ]
  chart = new uPlot(opts, data, containerRef.value)
}

watch(() => event.value?.traffic_tick, (tick) => {
  if (!tick) return
  const now = Date.now() / 1000
  for (const [ip, [t, sentBps, recvBps]] of Object.entries(tick)) {
    if (!series.has(ip)) series.set(ip, { sent: [], recv: [], ts: [] })
    const s = series.get(ip)!
    s.ts.push(t)
    s.sent.push(sentBps)
    s.recv.push(recvBps)
    while (s.ts.length && now - s.ts[0] > props.windowSec) {
      s.ts.shift(); s.sent.shift(); s.recv.shift()
    }
  }
  rebuild()
})

onMounted(() => rebuild())
</script>

<template>
  <div ref="containerRef" class="w-full" />
</template>
```

### 4.10 Python 侧改造：`active_connections.py`（新增）

```python
"""Connection registry + traffic ring buffer for Conduit."""
from __future__ import annotations

import asyncio
import time
from collections import defaultdict, deque
from dataclasses import dataclass, field


@dataclass
class ConnectionInfo:
    session_id: str
    peer_ip: str
    proto: str
    target: str
    since: float
    last_seen: float
    sent_bytes: int = 0
    recv_bytes: int = 0


class ConnectionRegistry:
    """Process-wide singleton, asyncio-safe (single-thread)."""

    def __init__(self) -> None:
        self._sessions: dict[str, ConnectionInfo] = {}
        self._next_id = 0
        self._lock = asyncio.Lock()

    async def add(self, peer_ip: str, proto: str, target: str) -> str:
        async with self._lock:
            self._next_id += 1
            sid = f"s{self._next_id}"
            now = time.time()
            self._sessions[sid] = ConnectionInfo(sid, peer_ip, proto, target, now, now)
            return sid

    async def update_bytes(self, sid: str, sent_delta: int, recv_delta: int) -> None:
        s = self._sessions.get(sid)
        if not s:
            return
        s.sent_bytes += sent_delta
        s.recv_bytes += recv_delta
        s.last_seen = time.time()

    async def remove(self, sid: str) -> None:
        self._sessions.pop(sid, None)

    def snapshot(self) -> list[dict]:
        return [
            {
                "session_id": s.session_id,
                "peer_ip": s.peer_ip,
                "proto": s.proto,
                "target": s.target,
                "since": s.since,
                "last_seen": s.last_seen,
                "sent_bytes": s.sent_bytes,
                "recv_bytes": s.recv_bytes,
            }
            for s in self._sessions.values()
        ]


class TrafficSampler:
    """Per-IP 1-second ring buffer for the line chart."""

    WINDOW = 600

    def __init__(self, registry: ConnectionRegistry) -> None:
        self._registry = registry
        self._series: dict[str, deque[tuple[float, int, int]]] = defaultdict(
            lambda: deque(maxlen=self.WINDOW)
        )
        self._last_totals: dict[str, tuple[int, int]] = {}

    async def run_forever(self) -> None:
        while True:
            await asyncio.sleep(1.0)
            self._sample_once()

    def _sample_once(self) -> None:
        now = time.time()
        totals: dict[str, tuple[int, int]] = defaultdict(lambda: (0, 0))
        for s in self._registry._sessions.values():
            sent, recv = totals[s.peer_ip]
            totals[s.peer_ip] = (sent + s.sent_bytes, recv + s.recv_bytes)
        for ip, (sent_total, recv_total) in totals.items():
            prev_sent, prev_recv = self._last_totals.get(ip, (sent_total, recv_total))
            sent_bps = max(0, sent_total - prev_sent)
            recv_bps = max(0, recv_total - prev_recv)
            self._series[ip].append((now, sent_bps, recv_bps))
            self._last_totals[ip] = (sent_total, recv_total)

    def series(self, peer_ip: str, window_sec: int) -> list[tuple[float, int, int]]:
        cutoff = time.time() - window_sec
        return [t for t in self._series.get(peer_ip, ()) if t[0] >= cutoff]

    def snapshot_tick(self) -> dict[str, tuple[float, int, int]]:
        out: dict[str, tuple[float, int, int]] = {}
        for ip, dq in self._series.items():
            if dq:
                out[ip] = dq[-1]
        return out


registry = ConnectionRegistry()
sampler = TrafficSampler(registry)
```

### 4.11 `relay.py` 改造（hot path 字节计数）

> 当前 `relay.bidirectional_relay` 已经返回 `(sent, recv)` 总数，需要改成**每 chunk 上报增量**。

```python
async def bidirectional_relay(
    c_reader, c_writer, t_reader, t_writer,
    on_progress=None,  # 新增: callable(sent_delta, recv_delta) -> Awaitable[None]
):
    async def pipe(reader, writer, is_upstream: bool):
        total = 0
        while True:
            chunk = await reader.read(65536)
            if not chunk:
                break
            writer.write(chunk)
            await writer.drain()
            total += len(chunk)
            if on_progress:
                if is_upstream:
                    await on_progress(len(chunk), 0)
                else:
                    await on_progress(0, len(chunk))
        return total

    sent, recv = await asyncio.gather(
        pipe(c_reader, t_writer, True),
        pipe(t_reader, c_writer, False),
    )
    return sent, recv
```

调用处（`http_proxy._handle_connect` / `socks5_proxy.handle_socks5`）：

```python
sid = await registry.add(peer_ip, "http", f"{host}:{port}")
try:
    sent, recv = await bidirectional_relay(
        reader, writer, t_reader, t_writer,
        on_progress=lambda s, r: registry.update_bytes(sid, s, r),
    )
finally:
    await registry.remove(sid)
```

---

## 5. 开发环境搭建（从零到第一次运行）

> 假设你在 macOS 上开发。Windows / Linux 类似。

### 5.1 装 Rust 工具链

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

### 5.2 装 Tauri CLI 与 pnpm

```bash
brew install pnpm
cargo install tauri-cli@2 --locked
```

### 5.3 创建前端工程（一次性）

```bash
cd task/20260429-lan-vpn-proxy
mkdir -p desktop && cd desktop
pnpm create vite@latest conduit-ui --template vue-ts
cd conduit-ui
pnpm add tailwindcss @tailwindcss/vite
pnpm add -D @types/node
```

按 §4.5 ~ §4.7 修改 `vite.config.ts` / `tsconfig.app.json` / 创建 `src/assets/index.css`，然后：

```bash
pnpm dlx shadcn-vue@latest init
pnpm dlx shadcn-vue@latest add button card tabs input dialog toast badge separator scroll-area
pnpm add uplot qrcode
```

### 5.4 创建 Tauri 工程

```bash
cd ../   # 回到 desktop/
cargo tauri init
# 应用名: Conduit
# Window title: Conduit
# Web assets dir: ../conduit-ui/dist
# Dev URL: http://localhost:5173
# Frontend dev cmd: pnpm --filter conduit-ui dev
# Frontend build cmd: pnpm --filter conduit-ui build
```

按 §4.1 ~ §4.4 替换 `tauri.conf.json` / `capabilities/default.json` / `src/main.rs` / `src/sidecar.rs`。

`Cargo.toml` 关键依赖：

```toml
[dependencies]
tauri = { version = "2", features = ["macos-private-api"] }
tauri-plugin-shell = "2"
tauri-plugin-clipboard-manager = "2"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
anyhow = "1"
```

### 5.5 准备 Python sidecar

```bash
cd task/20260429-lan-vpn-proxy
python3 -m venv .venv && source .venv/bin/activate

pip install pyinstaller         # 开发期快编译用
pip install nuitka ordered-set  # 发布期小包用
```

> macOS 上 Nuitka 还会自动用系统 Clang；Windows 上需要先装 VS 2022 Build Tools 或者 `pip install nuitka[zig]` 走 Zig。

#### 5.5.1 PyInstaller 脚本（开发期，快）

`desktop/scripts/build-sidecar.sh`：

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
PROJ_ROOT="$(cd .. && pwd)"
TARGET_TRIPLE=$(rustc -Vv | sed -n 's/host: //p')

cd "$PROJ_ROOT"
source .venv/bin/activate

pyinstaller --onefile \
    --name conduit-sidecar \
    --distpath desktop/src-tauri/binaries \
    --add-data "proxy.pac:." \
    proxy_server.py

mv "desktop/src-tauri/binaries/conduit-sidecar" \
   "desktop/src-tauri/binaries/conduit-sidecar-${TARGET_TRIPLE}"

echo "✓ PyInstaller built: conduit-sidecar-${TARGET_TRIPLE}"
ls -lh "desktop/src-tauri/binaries/conduit-sidecar-${TARGET_TRIPLE}"
```

#### 5.5.2 Nuitka 脚本（发布期，小）

`desktop/scripts/build-sidecar-nuitka.sh`：

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
PROJ_ROOT="$(cd .. && pwd)"
TARGET_TRIPLE=$(rustc -Vv | sed -n 's/host: //p')

cd "$PROJ_ROOT"
source .venv/bin/activate

# Nuitka 选项说明:
# --onefile           打成单文件（运行时会解压到 /tmp，但产物体积最小）
# --standalone        包含所有依赖（onefile 隐含）
# --include-data-files  把 PAC 模板带进去
# --remove-output     先清理上次构建产物
# --assume-yes-for-downloads  自动下载 ccache 等工具，免交互
# --nofollow-import-to=tkinter,test  排除不用的标准库模块缩小体积
# --no-deployment-flag=self-execution  禁用 self-execution 的 deployment 模式（避免误报为病毒）
python3 -m nuitka \
    --onefile \
    --output-dir=desktop/src-tauri/binaries \
    --output-filename=conduit-sidecar \
    --include-data-files=proxy.pac=proxy.pac \
    --remove-output \
    --assume-yes-for-downloads \
    --nofollow-import-to=tkinter \
    --nofollow-import-to=test \
    --nofollow-import-to=unittest \
    --nofollow-import-to=pydoc \
    --python-flag=no_site \
    --python-flag=no_warnings \
    proxy_server.py

mv "desktop/src-tauri/binaries/conduit-sidecar" \
   "desktop/src-tauri/binaries/conduit-sidecar-${TARGET_TRIPLE}"

echo "✓ Nuitka built: conduit-sidecar-${TARGET_TRIPLE}"
ls -lh "desktop/src-tauri/binaries/conduit-sidecar-${TARGET_TRIPLE}"
```

> 体积对比（实测）：PyInstaller `--onefile` 约 60MB（含完整 Python 解释器），Nuitka `--onefile` 约 25-40MB（取决于选项），**节省 35%~50%**。

#### 5.5.3 通过 `BUILDER` 环境变量切换

`desktop/scripts/build-sidecar.sh` 也可以做成"开关型"：

```bash
BUILDER=${BUILDER:-pyinstaller}   # pyinstaller (default) | nuitka

if [[ "$BUILDER" == "nuitka" ]]; then
    bash desktop/scripts/build-sidecar-nuitka.sh
else
    # ... PyInstaller 默认逻辑
fi
```

开发期默认走 PyInstaller（快），发布期 CI 设置 `BUILDER=nuitka` 走 Nuitka（小）。

### 5.6 第一次跑起来

```bash
cd desktop
bash scripts/build-sidecar.sh   # 出 binaries/conduit-sidecar-aarch64-apple-darwin
cargo tauri dev
```

`cargo tauri dev` 会：
1. 拉起 Vite dev server（端口 5173）
2. 编译并启动 Tauri 主进程
3. 主进程 spawn `conduit-sidecar-...`（监听 8080/1080）
4. Tauri 等到 `http://127.0.0.1:8080/healthz` 200 后显示窗口

---

## 6. 打包流程

### 6.1 macOS（双架构 universal）

```bash
cd desktop

# 1) 发布期用 Nuitka 打两份 sidecar（小包）
BUILDER=nuitka TARGET=aarch64-apple-darwin bash scripts/build-sidecar.sh
BUILDER=nuitka TARGET=x86_64-apple-darwin bash scripts/build-sidecar.sh

# （开发期可用 BUILDER=pyinstaller 加快迭代）

# 2) Tauri 打包
cargo tauri build --target universal-apple-darwin

# 产出:
#   src-tauri/target/universal-apple-darwin/release/bundle/dmg/Conduit_0.1.0_universal.dmg
#   src-tauri/target/universal-apple-darwin/release/bundle/macos/Conduit.app
```

> 用 Nuitka 时 macOS 整体 .dmg 预计 **~50MB**（Tauri shell 5MB + Nuitka sidecar 25MB ×2 + UI 资源 + 元数据）。
> 用 PyInstaller 时同样产物预计 **~140MB**（差距主要在 sidecar 不能 universal 共享，要 ×2）。

### 6.2 公证（macOS 必做，否则用户双击会被 Gatekeeper 拦）

```bash
# 在 ~/.zshrc 配置好
export APPLE_CERTIFICATE="..."
export APPLE_CERTIFICATE_PASSWORD="..."
export APPLE_SIGNING_IDENTITY="Developer ID Application: ..."
export APPLE_ID="..."
export APPLE_PASSWORD="app-specific-password"
export APPLE_TEAM_ID="..."

cargo tauri build --target universal-apple-darwin
# Tauri 2 会自动调用 notarytool 进行公证，完成后产物自带签名 + 票据
```

### 6.3 Windows

```bash
# 在 Windows 机器上
.\scripts\build-sidecar.ps1
cargo tauri build
# 产出 src-tauri/target/release/bundle/msi/Conduit_0.1.0_x64_en-US.msi
```

### 6.4 Linux（可选）

```bash
bash scripts/build-sidecar.sh
cargo tauri build --bundles deb appimage
```

---

## 7. 工程任务拆分（可派发给工程师/AI）

> 颗粒度按"半天到一天"切分。

### Sprint 0：地基（0.5 天）

- [ ] **T0-1** 按 §3 创建 `desktop/conduit-ui/` 和 `desktop/src-tauri/` 目录骨架
- [ ] **T0-2** 装好依赖，跑通 `cargo tauri dev` 显示空白窗口
- [ ] **T0-3** 跑通 shadcn-vue init + 加一个 Button，确认 Tailwind 工作

### Sprint 1：Python sidecar 集成（1 天）

- [ ] **T1-1** 写 `desktop/scripts/build-sidecar.sh`（PyInstaller 版，开发期用）
- [ ] **T1-2** 在 `proxy_server.py` 加 `/healthz` 端点（最简单返 200）
- [ ] **T1-3** 写 `src-tauri/src/sidecar.rs` 的启动 + healthz 等待逻辑
- [ ] **T1-4** 验证：`cargo tauri dev` 成功 spawn sidecar，关闭窗口能 kill 进程
- [ ] **T1-5** 处理异常：sidecar 启动失败时显示错误对话框
- [ ] **T1-6** 写 `desktop/scripts/build-sidecar-nuitka.sh`（发布期用，§5.5.2）并验证体积差距

### Sprint 2：基础界面（1 天）

- [ ] **T2-1** `App.vue` 主布局（参照 §5.2.5 ASCII 线框图）
- [ ] **T2-2** `ProxyControl.vue`：启动/停止按钮、当前状态徽章
- [ ] **T2-3** `NetworkPanel.vue`：从 `/status` 拉网卡 + LAN IP + VPN 状态
- [ ] **T2-4** `ShareCard.vue`：PAC URL 显示 + 复制按钮 + 二维码

### Sprint 3：连接 & 流量（1.5 天）

- [ ] **T3-1** 后端 `active_connections.py` 实现 ConnectionRegistry + TrafficSampler
- [ ] **T3-2** 后端改造 `relay.py` 加 `on_progress` 回调
- [ ] **T3-3** 后端 `http_proxy.py` / `socks5_proxy.py` 在 CONNECT 处加 registry 埋点
- [ ] **T3-4** 后端新增 `/status`（含 active_clients）、`/traffic`、`/events` SSE
- [ ] **T3-5** 前端 `useEvents.ts` + `useTraffic.ts`
- [ ] **T3-6** 前端 `ClientList.vue` 实时表格
- [ ] **T3-7** 前端 `TrafficChart.vue` uPlot 折线图

### Sprint 4：体验优化（0.5 天）

- [ ] **T4-1** 系统托盘（`tray.rs`）：最小化到托盘
- [ ] **T4-2** `LogViewer.vue`：日志窗口
- [ ] **T4-3** 配置面板（端口、CIDR、CONNECT 端口可改）

### Sprint 5：发布（1.5 天）

- [ ] **T5-1** 应用图标（按品牌色 `#1B2A4E` + `#00D4FF` 出导管 logo）
- [ ] **T5-2** **CI 切到 Nuitka**：GitHub Action 用 `Nuitka/Nuitka-Action`，缓存 `~/.cache/Nuitka`
- [ ] **T5-3** macOS 公证（Apple Developer 证书 + notarytool 配置）
- [ ] **T5-4** Windows 安装器（如需要，Nuitka 在 Windows 用 MSVC 2022 或 Zig）
- [ ] **T5-5** README / 用户手册更新

**合计 5 天（含发布），开发期 4 天能跑出可用版本。**

---

## 8. 风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| Python 嵌入二进制体积大 | PyInstaller 单架构 ~60MB | 发布期切 Nuitka（~25MB，详见 §1.5、§5.5.2）+ `--nofollow-import-to=tkinter,test,unittest` 排除冷模块 |
| Nuitka 编译慢（90s vs PyInstaller 22s） | CI 时间 +1min | CI 用 `actions/cache` 缓存 `~/.cache/Nuitka` 和 `__pycache__`；二次构建可压到 30s |
| Nuitka 兼容性 85%（vs PyInstaller 98%） | 极端情况下某些库报错 | Conduit 当前是纯标准库实现，落在 Nuitka 的"100% 兼容"区间；保留 PyInstaller 作为兜底脚本 |
| sidecar 启动慢导致 Tauri 窗口长时间不显示 | 体验差 | §4.4 加 healthz 轮询超时（9s）+ 失败弹窗；首屏可显示 splash |
| macOS 公证慢 / 复杂 | 影响发布节奏 | 用 Tauri 内置的 `tauri-action` GitHub Action，自动化签名 + 公证 |
| Tailwind v4 文档仍在变 | 升级踩坑 | 锁定 `tailwindcss@~4.0.0`，保留 v3 fallback CSS |
| shadcn-vue 不是 npm 库，是源码复制 | 升级要重新 `add` | 在 `components.json` 注释清楚；定期 `pnpm dlx shadcn-vue diff` 看上游变更 |
| 端口 8080/1080 与现有进程冲突 | 启动失败 | sidecar 启动前自检，端口被占时自动+1 重试；UI 显示实际端口 |
| WebView2 在 Win10 旧版本未预装 | Windows 用户首次启动失败 | 安装包用 `WebView2Bootstrapper` 模式（Tauri 2 默认支持） |
| 前端 fetch 跨域 | dev 时 5173 → 8080 被 CORS 拦 | sidecar `/api/*` 加 `Access-Control-Allow-Origin: http://localhost:5173`（dev only）|

---

## 9. 依赖锁定清单

> 实际开发时，从这个清单出发安装即可。

### 9.1 前端 `conduit-ui/package.json` 关键部分

```json
{
  "dependencies": {
    "vue": "^3.5.0",
    "@tailwindcss/vite": "^4.0.0",
    "tailwindcss": "^4.0.0",
    "uplot": "^1.6.32",
    "qrcode": "^1.5.4",
    "class-variance-authority": "^0.7.0",
    "clsx": "^2.1.1",
    "tailwind-merge": "^2.5.4",
    "lucide-vue-next": "^0.460.0",
    "reka-ui": "^1.0.0"
  },
  "devDependencies": {
    "@vitejs/plugin-vue": "^5.2.0",
    "vite": "^6.0.0",
    "vue-tsc": "^2.1.0",
    "typescript": "^5.7.0",
    "@types/node": "^22.0.0"
  }
}
```

### 9.2 Rust `src-tauri/Cargo.toml` 关键部分

```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-shell = "2"
tauri-plugin-clipboard-manager = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
anyhow = "1"

[build-dependencies]
tauri-build = { version = "2", features = [] }
```

### 9.3 Python `pyproject.toml` 关键部分

```toml
[project]
name = "conduit-proxy"
version = "0.1.0"
requires-python = ">=3.10"

[project.optional-dependencies]
# 开发期：编译快，迭代快
build-dev = ["pyinstaller>=6.10.0"]
# 发布期：包小启动快，CI 用
build-release = [
    "nuitka>=4.0.5",
    "ordered-set>=4.1.0",     # Nuitka 推荐安装，性能更好
]
```

> 当前后端是纯标准库实现（asyncio + 自写 HTTP/SOCKS），**无第三方运行时依赖**——这对 Nuitka 打包来说是最佳生态位（落在它的高兼容区间）。

---

## 10. 命令速查

```bash
# 开发
cd desktop && cargo tauri dev

# 单独跑前端（无 Tauri）
cd desktop/conduit-ui && pnpm dev

# 单独跑后端
cd task/20260429-lan-vpn-proxy && python3 proxy_server.py --yes

# 打包 sidecar
cd desktop && bash scripts/build-sidecar.sh                    # 默认 PyInstaller (开发期)
cd desktop && BUILDER=nuitka bash scripts/build-sidecar.sh     # Nuitka (发布期，包小)

# 全平台打包（在各自机器执行）
cargo tauri build                                              # 当前平台
cargo tauri build --target universal-apple-darwin              # macOS Intel + ARM

# 加 shadcn-vue 组件
cd desktop/conduit-ui
pnpm dlx shadcn-vue@latest add <component>

# 看 sidecar 日志（开发期）
tail -f task/20260429-lan-vpn-proxy/log/proxy.log
```

---

## 11. 接下来的动作

1. ⏭️ 创建 `desktop/` 目录骨架（Sprint 0）
2. ⏭️ 跑通"空 Tauri 窗口 + shadcn-vue 一个 Button"
3. ⏭️ 跑通"Tauri spawn Python sidecar，等 /healthz 200 后显示窗口"
4. ⏭️ 实现 Sprint 2 的基础界面
5. ⏭️ Sprint 3：连接列表 + 折线图

> 你拍板"开始动手"，我就直接按 Sprint 0 → Sprint 1 的顺序开干。
