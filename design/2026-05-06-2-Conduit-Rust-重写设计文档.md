# Conduit Rust 重写设计文档

> 文档版本：v1.0  
> 创建日期：2026-05-06  
> 分支：`feat/python-to-rust-feasibility`  
> 上游文档：`design/2026-05-06-1-技术栈精简可行性分析.md`  
> 状态：**Draft，待 review，待 POC 完成后定稿**

---

## 0. 摘要

把 Python sidecar 与 Tauri Rust 主进程**合并为单一 Rust 二进制**，删除 PyInstaller 打包链路与 127.0.0.1 控制 API。**不保留中间过渡态**——一次性切换，旧 Python 代码删除，dist 仅产出 Tauri .app/.dmg。

技术栈从 4 种（Python + Rust + TS + Bash）降到 3 种（Rust + TS + Bash）；进程数从 2 降到 1；双端 dmg 总体积从 ~70MB 降到 ~30MB；冷启动从 1~3s 降到 < 0.3s。

---

## 1. 设计目标 & 非目标

### 1.1 目标

| # | 目标 | 验收 |
|---|---|---|
| G1 | 完全移除 Python，server-app/core 与 client-app/core 删除 | 仓库 grep `*.py` 无业务代码 |
| G2 | 删除 sidecar 进程边界，HTTP/SOCKS5/mDNS 全部跑在 Tauri 主进程 | 任务管理器仅 1 个进程 |
| G3 | 删除 127.0.0.1:8090 控制 API，UI 改用 Tauri IPC | `lsof -i :8090` 无监听 |
| G4 | 跨架构支持：macOS arm64 + macOS x86_64 + Windows + Linux | 通过 `cargo build --target` 一行命令 |
| G5 | 行为完全等价：HTTP/SOCKS5 路由决策、mDNS TXT 字段、PAC 输出、networksetup 状态机一致 | e2e.sh 全套回归通过 |
| G6 | 跨层契约由编译期保证（Rust struct → TS type 自动同步） | UI 调用 `invoke('xxx')` 时类型自动推断 |

### 1.2 非目标

- **不动 UI**：server-ui、client-ui 的 Vue/TS 代码原样保留，只改 IPC 调用方式（`fetch` → `invoke`、`EventSource` → `listen`）
- **不动 PAC 文件**：`proxy.pac` 仍是浏览器跑的 JS 文件，作为静态资源 `include_str!` 嵌入二进制
- **不引入新协议**：mDNS service type / TXT 字段、HTTP CONNECT 行为、SOCKS5 NO-AUTH 等保持现状
- **不做 feature flag 切流**：用户量为 0，激进切换；旧 Python 代码删除时不留 fallback
- **不引入数据库**：route_cache 仍持久化到 JSON（`~/Library/Application Support/Conduit/`）
- **不做 v0.1 → v0.2 数据迁移工具**：known-servers.json / route_cache JSON 字段保持 wire-compatible，无需迁移脚本

---

## 2. 目标架构

### 2.1 总览

```
┌──────────────────────────────────────────────────────────────┐
│ Tauri Rust 主进程（单一进程）                                  │
│                                                              │
│  ┌──── tokio runtime（multi-thread） ────┐                   │
│  │                                        │                  │
│  │   server-app 端（仅 server bundle）：    │                  │
│  │     - http_proxy listener (0.0.0.0:8080)                  │
│  │     - socks5 listener   (0.0.0.0:1080)                    │
│  │     - mdns_advertiser   (_conduit._tcp.local.)            │
│  │     - connection_registry + traffic_meter                 │
│  │     - healthcheck                                         │
│  │                                                            │
│  │   client-app 端（仅 client bundle）：    │                  │
│  │     - mdns_discoverer                  │                  │
│  │     - local_proxy listener (127.0.0.1:7890)               │
│  │     - route_resolver + route_cache                        │
│  │     - system_proxy（networksetup wrapper）                │
│  │     - connectivity_diag                                   │
│  └────────────────────────────────────────┘                  │
│                                                              │
│  ┌──── Tauri IPC ────┐                                        │
│  │  #[tauri::command] fn 暴露给 UI                            │
│  │  app.emit("event", payload) 推 UI                          │
│  └───────────────────┘                                        │
│                                                              │
│  ┌──── tray / autostart ────┐                                 │
│  │  保留现有 Tauri 实现                                        │
│  └──────────────────────────┘                                 │
└──────────────────────────────────────────────────────────────┘
                  ↑↓ Tauri IPC
                Vue 3 UI（保持不变）
```

### 2.2 与现状对比

| 维度 | 现状 | 设计后 |
|---|---|---|
| 进程数 | Tauri + Python sidecar = 2 | **1** |
| 端口数 | 8080 + 1080 + 8090（控制 API）+ mDNS | **8080 + 1080 + mDNS**（去 8090）|
| UI ↔ Backend 通道 | HTTP REST + SSE | **Tauri IPC（command + emit）** |
| 业务代码语种 | Python | Rust |
| 配置入口 | argparse CLI（sidecar.rs 里 spawn 时传） | **Tauri 设置文件 + clap CLI（dev 模式）** |
| 日志 | Python logging → `~/.conduit/logs/sidecar-server.log` | tracing → `~/.conduit/logs/conduit-server.log` |
| 跨架构编译 | 不能（PyInstaller 平台绑死） | **`cargo build --target`** |
| 打包产物 | Tauri .app + 内嵌 PyInstaller onedir | **纯 Tauri .app** |

---

## 3. 仓库目录布局

### 3.1 整体（精简后）

```
conduit/
├── crates/                              ← 新增：Rust 共享 crate
│   └── conduit-core/                    ← server + client 共用：协议常量、wire types、relay、events_bus
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── mdns.rs                  ← service type / TXT 字段约定常量
│           ├── pac.rs                   ← PAC 引擎（regex 平移）
│           ├── relay.rs                 ← bidirectional copy
│           ├── events.rs                ← EventBus（broadcast channel）
│           ├── types.rs                 ← 双端共享 wire types（serde + specta）
│           └── error.rs
├── server-app/
│   ├── src-tauri/
│   │   ├── Cargo.toml                   ← 增加 hyper / mdns-sd / fast-socks5 等
│   │   ├── tauri.conf.json              ← 移除 bundle.resources 中 binaries-dir
│   │   ├── build.rs
│   │   └── src/
│   │       ├── main.rs
│   │       ├── lib.rs                   ← Tauri builder
│   │       ├── proxy/                   ← 业务模块（来自 server-app/core）
│   │       │   ├── mod.rs
│   │       │   ├── core.rs              ← ProxyCore 编排
│   │       │   ├── http.rs              ← HTTP CONNECT + absolute-URI + PAC serving
│   │       │   ├── socks5.rs            ← fast-socks5 wrapper
│   │       │   ├── outbound.rs          ← POLICY_AUTO + DIRECT-first race
│   │       │   ├── connections.rs       ← ConnectionRegistry + PassiveClientRegistry
│   │       │   ├── traffic.rs           ← TrafficSampler
│   │       │   ├── healthcheck.rs
│   │       │   ├── advertiser.rs        ← mDNS 广播
│   │       │   └── config.rs
│   │       ├── ipc/                     ← Tauri command + emit
│   │       │   ├── mod.rs
│   │       │   ├── commands.rs          ← #[tauri::command] 全集
│   │       │   └── events.rs            ← app.emit() 包装
│   │       ├── tray.rs                  ← 不动
│   │       └── error.rs
│   └── ui/                              ← 不动
├── client-app/
│   ├── src-tauri/
│   │   ├── Cargo.toml
│   │   ├── tauri.conf.json
│   │   └── src/
│   │       ├── main.rs
│   │       ├── lib.rs
│   │       ├── proxy/
│   │       │   ├── mod.rs
│   │       │   ├── core.rs              ← ClientCore 编排
│   │       │   ├── local.rs             ← 本地 SOCKS5 listener
│   │       │   ├── route.rs             ← route_resolver + route_cache
│   │       │   ├── discoverer.rs        ← mDNS browse
│   │       │   ├── system_proxy.rs      ← networksetup wrapper
│   │       │   ├── connectivity.rs      ← 连通性诊断
│   │       │   ├── traffic.rs           ← TrafficMeter（client 侧统计）
│   │       │   └── config.rs
│   │       ├── ipc/{mod,commands,events}.rs
│   │       ├── tray.rs                  ← 不动
│   │       ├── autostart.rs             ← 不动
│   │       └── error.rs
│   └── ui/                              ← 不动
├── scripts/
│   ├── release.sh                       ← 移除 build-sidecars.sh 调用
│   ├── bump-version.sh                  ← 改成只更 Cargo.toml + package.json
│   ├── e2e.sh                           ← 适配新启动方式
│   └── publish-release-notes.sh         ← 不动
├── design/
│   ├── 2026-05-06-1-技术栈精简可行性分析.md
│   └── 2026-05-06-2-Conduit-Rust-重写设计文档.md   ← 本文档
├── package.json                         ← 移除 dev:server / dev:client 中的 sidecar 启动
├── pnpm-workspace.yaml                  ← 移除 server-app/core, client-app/core 引用
└── Cargo.toml                           ← 新增工作区根，把 conduit-core 与两个 src-tauri 串起来
```

### 3.2 关键删除项

```
- server-app/core/                       ← 整个目录删除
- client-app/core/                       ← 整个目录删除
- server-app/src-tauri/binaries-dir/     ← 整个目录删除
- client-app/src-tauri/binaries-dir/     ← 整个目录删除
- server-app/src-tauri/src/sidecar.rs    ← 删除
- client-app/src-tauri/src/sidecar.rs    ← 删除
- server-app/src-tauri/src/healthz.rs    ← 删除（功能合并进 ipc::commands）
- client-app/src-tauri/src/healthz.rs    ← 删除
- scripts/build-sidecars.sh              ← 删除
- pyproject.toml（双端两个）              ← 删除
```

### 3.3 Cargo workspace

新增仓库根 `Cargo.toml`：

```toml
[workspace]
resolver = "2"
members = [
  "crates/conduit-core",
  "server-app/src-tauri",
  "client-app/src-tauri",
]

[workspace.package]
version = "0.2.0"
edition = "2021"
rust-version = "1.78"
authors = ["TerrellShe"]

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
hyper = { version = "1", features = ["full"] }
hyper-util = { version = "0.1", features = ["tokio"] }
http-body-util = "0.1"
fast-socks5 = "1"
mdns-sd = "0.11"
regex = "1"
globset = "0.4"
ipnet = "2"
moka = { version = "0.12", features = ["future"] }
dashmap = "6"
netdev = "0.41"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
chrono = { version = "0.4", features = ["serde"] }
socket2 = "0.5"
anyhow = "1"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
tracing-appender = "0.2"
specta = { version = "2", features = ["typescript"] }
tauri = { version = "2", features = ["tray-icon"] }
```

---

## 4. 共享 crate `conduit-core` 设计

承担**双端共用**的部分，避免 server-app 与 client-app 重复实现。

### 4.1 模块清单

| 模块 | 内容 | 平移自 |
|---|---|---|
| `mdns.rs` | service type 常量 `_conduit._tcp.local.`、TXT 字段名（name/port/socks/api/vpn/version/pac）、`MdnsServiceInfo` 结构 | server `mdns_advertiser.py` + client `discoverer.py` 中字段约定部分 |
| `pac.rs` | `PacRules` 结构、`load_rules(text)` 解析、`find_proxy(host) -> PacDecision` 决策 | server `pac_engine.py` + client `pac_parser.py` |
| `relay.rs` | `bidirectional_relay(reader, writer, on_progress)` | server / client `relay.py` |
| `events.rs` | `EventBus<T: Clone>` 基于 `tokio::sync::broadcast`，提供 `publish` / `subscribe` | server / client `events_bus.py` |
| `types.rs` | 双端共享 wire types：`DiscoveredServer` / `ConnectionInfo` / `RouteDecision` / `HealthCheckResult` 等。每个都打 `#[derive(Serialize, Deserialize, Type)]`（specta），**Rust 与 TS 类型由 specta 生成保持同步** | TS 端 `types/proxy.ts` + `types/client.ts` |
| `error.rs` | `ConduitError` 通用错误枚举 + `ConduitResult<T>` | 双端 `error.rs` 雏形 |
| `lib.rs` | re-export | — |

### 4.2 PAC 引擎签名草案

```rust
pub struct PacRules {
    pub local_globs: Vec<globset::GlobMatcher>,
    pub local_domains: Vec<String>,
    pub local_nets: Vec<ipnet::IpNet>,
    pub internal_globs: Vec<globset::GlobMatcher>,
    pub internal_domains: Vec<String>,
    pub fallback_globs: Vec<globset::GlobMatcher>,
    pub fallback_domains: Vec<String>,
    pub cn_direct_globs: Vec<globset::GlobMatcher>,
    pub cn_direct_domains: Vec<String>,
    pub proxy_target: String,
    pub proxy_target_with_fallback: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub enum PacDecision {
    Direct,
    Proxy(String),         // "PROXY host:port"
    ProxyOrDirect(String), // "PROXY host:port; DIRECT"
}

impl PacRules {
    pub fn parse(pac_text: &str) -> Self;
    pub fn update_proxy_target(&mut self, host: &str, http_port: u16);
    pub fn find_proxy(&self, host: &str) -> PacDecision;
}
```

### 4.3 EventBus 签名

```rust
pub struct EventBus<T: Clone + Send + 'static> {
    tx: tokio::sync::broadcast::Sender<T>,
}

impl<T: Clone + Send + 'static> EventBus<T> {
    pub fn new(capacity: usize) -> Self;
    pub fn publish(&self, event: T);
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<T>;
}
```

UI 推送方向：`EventBus<ServerEvent>` 在 `proxy::core::ProxyCore` 内，IPC 层 spawn 一个 task 把 `subscribe` 的事件转发到 `app.emit("event-name", payload)`。

---

## 5. server-app 模块设计

### 5.1 ProxyCore（编排器）

```rust
pub struct ProxyCore {
    cfg: Arc<ProxyConfig>,
    rules: Arc<RwLock<PacRules>>,
    bus: EventBus<ServerEvent>,
    registry: Arc<ConnectionRegistry>,
    passive_clients: Arc<PassiveClientRegistry>,
    sampler: TrafficSampler,
    health: HealthCheck,
    advertiser: Option<MdnsAdvertiser>,
    state: RwLock<RuntimeState>, // running flag, started_at, listener handles
}

impl ProxyCore {
    pub fn new(cfg: ProxyConfig) -> Self;
    pub async fn start(&self) -> ConduitResult<()>;
    pub async fn stop(&self) -> ConduitResult<()>;
    pub async fn status(&self) -> ServerStatus;
    pub fn bus(&self) -> &EventBus<ServerEvent>;
}
```

`start()` 顺序：
1. 加载 PAC（`PacRules::parse(include_str!("../../../proxy.pac"))`）
2. spawn HTTP listener（`http::serve(listener, cfg.clone(), rules.clone(), registry.clone())`）
3. spawn SOCKS5 listener
4. 启动 sampler 与 passive_clients ticker
5. 启动 VPN watcher task
6. 注册 mDNS（如 cfg.mdns_enabled）

### 5.2 HTTP forward proxy（hyper 1.x）

`proxy/http.rs`：
- `serve(listener: TcpListener, cfg, rules, registry, passive_clients)`
- 内部 accept → `tokio::spawn(handle_one)`
- `handle_one`：
  - 解析 request
  - 如果是 `GET /proxy.pac` 或 `/wpad.dat`：返回 PAC 文件（带 Content-Type）
  - 如果是 `GET /check?host=xxx`：返回 PAC 决策（JSON）
  - 如果是 `GET /status`：等价 `/api/status`（保留给 LAN 客户端诊断用）
  - 否则按 forward proxy 处理：
    - `CONNECT host:port` → `outbound.rs` 建上游连接 → `relay::bidirectional_relay`
    - `GET http://...` (absolute-URI) → 透明转发

不引入 hudsucker、不引入 noxy（noxy 太新，等成熟）；直接 hyper 手写。

### 5.3 SOCKS5

`proxy/socks5.rs`：
- 使用 `fast_socks5::server::Socks5Server` 的 builder
- 自定义 connector：所有出站走 `outbound::open_with_fallback(host, port, policy)`
- 拦截目标地址：先经 `pac_rules.find_proxy(host)` 决策，再调用 outbound

### 5.4 outbound（DIRECT-first race）

```rust
pub enum Policy { Auto, Direct, ProxyOnly }

pub async fn open_with_fallback(
    host: &str,
    port: u16,
    policy: Policy,
    cfg: &ProxyConfig,
) -> ConduitResult<TcpStream>;
```

- `Auto`：先尝试 DIRECT（`cfg.direct_first_timeout_s`），失败 fallback 默认路由（即 VPN 出口）
- `Direct`：只走物理网卡（`cfg.physical_iface_ip`）
- `ProxyOnly`：只走默认路由

实现细节：用 `socket2::Socket::bind` 绑物理网卡 IP 实现 DIRECT；并发 race 用 `tokio::select!`。

### 5.5 mDNS Advertiser

`proxy/advertiser.rs`：
```rust
pub struct MdnsAdvertiser {
    daemon: mdns_sd::ServiceDaemon,
    info: mdns_sd::ServiceInfo,
}

impl MdnsAdvertiser {
    pub fn new(name: &str, host_ip: IpAddr, http_port: u16, socks: u16,
               api_port: Option<u16>, vpn_on: bool, version: &str) -> ConduitResult<Self>;
    pub fn register(&self) -> ConduitResult<()>;
    pub fn unregister(&self) -> ConduitResult<()>;
    pub fn update_vpn_state(&mut self, vpn_on: bool) -> ConduitResult<()>;
}
```

TXT 字段保持与现状完全一致（以保持 client 兼容）：`name` / `port` / `socks` / `api`（可空，新版没有控制 API 也保留字段为空字符串避免 client 端报错）/ `vpn` / `version` / `pac`。

### 5.6 ConnectionRegistry / TrafficSampler

```rust
pub struct ConnectionRegistry {
    sessions: dashmap::DashMap<String, ConnectionInfo>,
    next_id: AtomicU64,
    bus: EventBus<ServerEvent>,
}

impl ConnectionRegistry {
    pub fn add(&self, peer_ip: IpAddr, proto: Proto, target: String) -> String;
    pub fn update_bytes(&self, sid: &str, sent: u64, recv: u64);
    pub fn remove(&self, sid: &str);
    pub fn snapshot(&self) -> Vec<ConnectionInfo>;
    pub fn len(&self) -> usize;
}

pub struct TrafficSampler { ... }
impl TrafficSampler {
    pub fn start(self, registry: Arc<ConnectionRegistry>);
    pub async fn snapshot_tick(&self) -> TrafficTick;
}
```

EventBus 推 `ServerEvent::ClientConnected` / `ClientDisconnected` / `TrafficTick`。

---

## 6. client-app 模块设计

### 6.1 ClientCore 编排器

```rust
pub struct ClientCore {
    cfg: Arc<ClientConfig>,
    bus: EventBus<ClientEvent>,
    discoverer: Arc<MdnsDiscoverer>,
    route_cache: Arc<RouteCache>,
    route_resolver: Arc<RouteResolver>,
    local_proxy: Arc<LocalProxy>,
    system_proxy: Arc<SystemProxy>,
    connectivity: Arc<ConnectivityDiag>,
    state: RwLock<ClientState>,
}

impl ClientCore {
    pub fn new(cfg: ClientConfig) -> Self;
    pub async fn start(&self) -> ConduitResult<()>;
    pub async fn stop(&self) -> ConduitResult<()>;
    pub async fn connect_to(&self, server_id: &str) -> ConduitResult<()>;
    pub async fn disconnect(&self) -> ConduitResult<()>;
    pub async fn diagnose(&self) -> DiagnoseReport;
}
```

### 6.2 MdnsDiscoverer

```rust
pub struct MdnsDiscoverer {
    daemon: mdns_sd::ServiceDaemon,
    receiver: mdns_sd::Receiver<mdns_sd::ServiceEvent>,
    known_servers_path: PathBuf,    // ~/Library/Application Support/Conduit/known-servers.json
    cache: dashmap::DashMap<String, DiscoveredServer>,
    bus: EventBus<ClientEvent>,
}

impl MdnsDiscoverer {
    pub fn start(&self) -> ConduitResult<()>;
    pub fn list(&self) -> Vec<DiscoveredServer>;
    pub fn forget(&self, server_id: &str);
    pub fn forget_all(&self);
}
```

启动时：
1. 加载 `known-servers.json`（如存在），把历史 server 标 `source: "history"`
2. spawn task 监听 mdns_sd `ServiceEvent::ServiceResolved` → 合并到 cache，标 `source: "mdns"`
3. publish `ClientEvent::ServerDiscovered` / `ServerLost`

### 6.3 LocalProxy（本地 SOCKS5 listener）

```rust
pub struct LocalProxy {
    bind: SocketAddr,                 // 默认 127.0.0.1:7890
    cfg: Arc<ClientConfig>,
    upstream: RwLock<Option<UpstreamServer>>, // 当前 connect_to 的 server
    route_resolver: Arc<RouteResolver>,
    route_cache: Arc<RouteCache>,
    traffic: Arc<TrafficMeter>,
}

impl LocalProxy {
    pub async fn start(&self) -> ConduitResult<()>;
    pub async fn stop(&self) -> ConduitResult<()>;
    pub async fn set_upstream(&self, server: UpstreamServer);
    pub async fn clear_upstream(&self);
}
```

每个进入连接：
1. SOCKS5 握手
2. `route_resolver.resolve(host)` 决策走 DIRECT 还是 upstream
3. 缓存命中直接复用决策；否则走规则评估
4. 建立隧道 → `relay::bidirectional_relay` + `traffic.account(...)`

### 6.4 SystemProxy（macOS networksetup）

不依赖第三方 sysproxy crate，直接 `std::process::Command` 实现 4 个调用，与现状 `system_proxy.py` 等价。

```rust
pub struct SystemProxy { /* ProcessRunner trait for testability */ }

impl SystemProxy {
    pub fn list_services(&self) -> ConduitResult<Vec<String>>;
    pub fn get_socks_state(&self, service: &str) -> ConduitResult<SocksProxyState>;
    pub fn set_socks(&self, service: &str, host: &str, port: u16) -> ConduitResult<()>;
    pub fn enable_socks(&self, service: &str, on: bool) -> ConduitResult<()>;
    pub fn restore_to(&self, service: &str, prev: &SocksProxyState) -> ConduitResult<()>;
}

pub struct SocksProxyState {
    pub enabled: bool,
    pub server: String,
    pub port: u16,
}
```

### 6.5 RouteCache + RouteResolver

`RouteCache`：基于 **moka**，TTL = `cfg.route_cache_ttl_s`（默认 600s），持久化到 `~/Library/Application Support/Conduit/route-cache.json`。

`RouteResolver`：决策树
1. 命中 cache → 返回
2. 评估 PAC 规则（来自 server 的 PAC URL，client 自己也 parse 一份）
3. 直连测试（小 timeout）
4. 失败 fallback upstream
5. 写 cache

---

## 7. Tauri IPC Contract

### 7.1 设计原则

- 用 `#[tauri::command]` 暴露**全部读 + 操作**接口
- 用 `app.emit("event-name", payload)` 推**实时变更**
- 用 **specta** 生成 TS 类型文件（一次性 build script），UI 端 import 即用，编译期保证一致

### 7.2 server-app commands & events

| Command | 等价旧 endpoint | 入参 | 返回 |
|---|---|---|---|
| `get_status` | GET /api/status | — | `ServerStatus` |
| `get_clients` | GET /api/clients | — | `ClientsSnapshot` |
| `get_traffic` | GET /api/traffic | — | `TrafficSnapshot` |
| `stop_proxy` | POST /api/admin/stop | — | `()` |
| `start_proxy` | （新增，旧版没有，sidecar 起就启动）| — | `()` |
| `get_health` | GET /healthz | — | `HealthReport` |
| `client_heartbeat` | POST /api/clients/heartbeat | `{ client_name, version, peer_ip }` | `HeartbeatResponse` |

旧 endpoint `POST /api/clients/heartbeat` 是被 LAN 上的 client 调的（不是 UI），需要保留 HTTP 入口。**保留**：把 `client_heartbeat` 也作为 `proxy/http.rs` 处理的一个 endpoint，跟 `/proxy.pac` `/check` 一起暴露在 0.0.0.0:8080，供 LAN 客户端调用。

| Event（emit） | 等价旧 SSE | Payload |
|---|---|---|
| `server-event` | GET /api/events | `ServerEvent`（enum：ClientConnected / ClientDisconnected / TrafficTick / VpnStateChanged / PassiveClient*） |

### 7.3 client-app commands & events

| Command | 等价旧 endpoint | 入参 | 返回 |
|---|---|---|---|
| `list_servers` | GET /api/servers | — | `Vec<DiscoveredServer>` |
| `forget_server` | POST /api/servers/forget | `{ server_id }` | `()` |
| `forget_all_servers` | POST /api/servers/forget_all | — | `()` |
| `connect_to` | POST /api/connect/{server_id} | `{ server_id }` | `ConnectionInfo` |
| `disconnect` | POST /api/disconnect | — | `()` |
| `get_connection` | GET /api/connection | — | `ConnectionState` |
| `get_traffic` | GET /api/traffic | — | `TrafficSnapshot` |
| `get_cache` | GET /api/cache | — | `RouteCacheSnapshot` |
| `clear_cache` | DELETE /api/cache | — | `()` |
| `diagnose` | GET /api/diagnose | — | `DiagnoseReport` |

| Event（emit） | 等价旧 SSE | Payload |
|---|---|---|
| `client-event` | GET /api/events | `ClientEvent`（enum：ServerDiscovered / ServerLost / Connected / Disconnected / TrafficTick / RouteResolved 等） |

### 7.4 UI 适配

UI 端的改动模式（示例）：

旧（fetch + EventSource）：
```ts
const status = await fetch('http://127.0.0.1:8090/api/status').then(r => r.json());
const es = new EventSource('http://127.0.0.1:8090/api/events');
es.addEventListener('client_connected', ev => { ... });
```

新（Tauri）：
```ts
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const status = await invoke<ServerStatus>('get_status');
const unlisten = await listen<ServerEvent>('server-event', ev => {
  if (ev.payload.kind === 'client_connected') { ... }
});
```

类型 `ServerStatus` / `ServerEvent` 由 specta 在 `cargo build` 时输出到 `server-app/ui/src/generated/bindings.ts`。

---

## 8. 数据模型（serde wire types，定义在 `conduit-core::types`）

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ServerStatus {
    pub running: bool,
    pub version: String,
    pub http_port: u16,
    pub socks5_port: u16,
    pub api_port: u16,           // 保留字段，等于 http_port（因为不再有独立 API）
    pub pac_url: Option<String>,
    pub mdns: MdnsInfo,
    pub vpn: VpnInfo,
    pub lan: LanInfo,
    pub clients_count: usize,
    pub passive_clients_count: usize,
    pub uptime_sec: u64,
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DiscoveredServer {
    pub server_id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub socks: u16,
    pub api: u16,
    pub vpn: bool,
    pub version: String,
    pub pac: String,
    pub source: ServerSource,    // mdns | history | manual
    pub last_seen_at: f64,
    pub healthy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ServerSource { Mdns, History, Manual }

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerEvent {
    ClientConnected { session_id: String, peer_ip: String, proto: String, target: String, since: f64 },
    ClientDisconnected { session_id: String },
    TrafficTick { window_sec: u64, totals: TrafficTotals, per_client: Vec<TrafficPerClient> },
    VpnStateChanged { available: bool, iface: Option<String> },
    PassiveClientUpdated { peer_ip: String, name: String, version: String, last_seen: f64 },
}

// ... 其他 type 同理
```

**所有字段保持 snake_case** 以兼容现有 UI（不需要重写 props 名称）。

---

## 9. 配置管理

### 9.1 ProxyConfig（server）

```rust
#[derive(Debug, Clone, Serialize, Deserialize, clap::Parser)]
pub struct ProxyConfig {
    #[clap(long, default_value = "0.0.0.0")]
    pub bind: IpAddr,
    #[clap(long, default_value = "8080")]
    pub http_port: u16,
    #[clap(long, default_value = "1080")]
    pub socks_port: u16,

    #[clap(long, value_delimiter = ',', default_values_t = vec![
        "192.168.0.0/16".parse().unwrap(),
        "10.0.0.0/8".parse().unwrap(),
        "172.16.0.0/12".parse().unwrap(),
        "127.0.0.0/8".parse().unwrap(),
    ])]
    pub allowed_cidrs: Vec<ipnet::IpNet>,

    #[clap(long, value_delimiter = ',',
           default_values_t = vec![80, 443, 22, 8080, 8443, 8118, 8888, 9000, 9443])]
    pub allowed_connect_ports: Vec<u16>,

    #[clap(long, default_value = "")]
    pub pac_advertised_host: String,

    #[clap(long, default_value = "INFO")]
    pub log_level: String,

    #[clap(long, default_value_t = 1.5)]
    pub direct_first_timeout_s: f64,
    #[clap(long, default_value_t = 300.0)]
    pub direct_cache_ttl_s: f64,
    #[clap(long, default_value = "")]
    pub physical_iface_ip: String,

    #[clap(long, action = clap::ArgAction::SetFalse)]
    pub mdns_enabled: bool,
    #[clap(long, default_value = "")]
    pub mdns_service_name: String,
}
```

### 9.2 加载顺序

1. 默认值（clap 默认）
2. **配置文件**（新增）：`~/.conduit/server-config.toml`，存在则覆盖默认。Tauri 设置面板可写。
3. CLI 参数（dev 模式 `cargo run -- --xxx` 时用）
4. 环境变量 `CONDUIT_*`（最高优先级）

去掉旧的 `--watchdog-ppid`（不再有 sidecar）和 `--api-port`（不再有独立 API 端口）。

---

## 10. 错误模型

```rust
#[derive(Debug, thiserror::Error)]
pub enum ConduitError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid address: {0}")]
    InvalidAddr(String),

    #[error("upstream: {0}")]
    Upstream(String),

    #[error("pac parse: {0}")]
    PacParse(String),

    #[error("mdns: {0}")]
    Mdns(String),

    #[error("system_proxy: {0}")]
    SystemProxy(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("internal: {0}")]
    Internal(String),
}

pub type ConduitResult<T> = Result<T, ConduitError>;
```

Tauri command 返回 `Result<T, String>`，由 `impl From<ConduitError> for String` 适配（保留 `#[error("...")]` 文案给 UI 展示）。

---

## 11. 启动 / 关闭流程

### 11.1 启动

```
main()
  → init tracing-subscriber（stdout + 滚动文件 ~/.conduit/logs/conduit-server.log）
  → 加载 ProxyConfig（默认 + 配置文件 + CLI + ENV）
  → tauri::Builder
       .plugin(tauri_plugin_autostart) // client only
       .setup(|app| {
           let core = ProxyCore::new(cfg);
           app.manage(Arc::new(core));
           tauri::async_runtime::spawn(async move {
               core.start().await.expect("proxy start");
           });
           ipc::events::wire_event_forwarders(app, core.bus());
           tray::install(app);
           Ok(())
       })
       .invoke_handler(tauri::generate_handler![
           ipc::commands::get_status,
           ipc::commands::get_clients,
           ipc::commands::get_traffic,
           ipc::commands::stop_proxy,
           ipc::commands::start_proxy,
           ipc::commands::get_health,
       ])
       .run(...)
```

启动完成后 UI 即可 `invoke('get_status')`，**没有 healthz 轮询等待**——主进程已经活着了，直接就绪。

### 11.2 关闭

- 用户点 quit / Cmd+Q：Tauri lifecycle hook → `core.stop()`（unregister mDNS、close listeners、drain registry、刷 route_cache 到磁盘）→ exit
- 系统 signal SIGTERM：tokio signal handler 调 `core.stop()`
- panic：catch_unwind 包一层，记 log 后 exit

---

## 12. 日志与可观测性

| 维度 | 实现 |
|---|---|
| 日志框架 | `tracing` + `tracing-subscriber`，env-filter 支持 `RUST_LOG` |
| 日志输出 | stdout（dev）+ 滚动文件 `~/.conduit/logs/conduit-{server,client}.log`（每天滚动，保留 7 天）通过 `tracing-appender::rolling` |
| 日志级别 | 默认 INFO；DEBUG 通过 `RUST_LOG=conduit=debug` 打开 |
| 字段 | 用 `tracing` span 携带 session_id / peer_ip / target，方便 grep |
| 指标 | 运行时统计已经在 `ConnectionRegistry` / `TrafficSampler`，通过 `get_status` / `get_traffic` command 暴露给 UI |
| crash 报告 | 暂不做（v0.2.0 后续可接入 sentry-rs） |

---

## 13. 测试策略

### 13.1 单元测试

| 模块 | 测试重点 | 优先级 |
|---|---|---|
| `pac` | 5 段规则全覆盖：local_globs / internal / fallback / cn_direct / 默认。**必须达到 Python 测试 100% case 覆盖** | P0 |
| `mdns` | TXT 字段 encode/decode，service_type 常量 | P0 |
| `system_proxy` | mock ProcessRunner 验证 networksetup 命令拼接、stdout 解析 | P0 |
| `route_resolver` / `route_cache` | TTL 过期、并发读写、持久化 | P0 |
| `outbound` | DIRECT-first race（mock TcpStream） | P1 |
| `connection_registry` | snapshot 一致性、并发 add/remove | P1 |
| `events` | broadcast 多订阅、容量满时 lagged | P1 |

### 13.2 集成测试

`tests/it/` 目录：
- `server_lifecycle.rs`：spawn ProxyCore → CONNECT 到 httpbin.org/get → 断言 200 + body
- `socks5_lifecycle.rs`：通过 fast-socks5 客户端连本机 1080 → 测试相同
- `mdns_e2e.rs`：起 advertiser + discoverer，断言能互相发现（用临时 service_type 避免污染本机其他 conduit）
- `pac_decision.rs`：加载真实 proxy.pac，对几十个 host 做决策对比 Python 端结果

### 13.3 端到端

`scripts/e2e.sh` 改造：
- 不再启 Python sidecar
- 改为 `cargo run --release --bin conduit-server` + `cargo run --release --bin conduit-client`
- 测试矩阵保持：浏览器 / curl / pip / git / VPN on / VPN off

### 13.4 测试代码迁移

旧 4500 行 Python tests：
- 黑盒 / 集成测试约 60% 价值高，迁移到 Rust（约 2700 行 → Rust 估 1500 行）
- 单元测试约 40% 重写 Rust 等价（约 1800 行 → Rust 估 800 行）

---

## 14. 打包与发布

### 14.1 删除项

- `scripts/build-sidecars.sh`
- `pyproject.toml`（双端两个）
- `server-app/src-tauri/binaries-dir/` 与 `client-app/src-tauri/binaries-dir/`
- `tauri.conf.json` 的 `bundle.resources` 中 `binaries-dir/...` 条目

### 14.2 新增 / 修改

- `tauri.conf.json` 的 `bundle.resources` 移除 binaries-dir
- `Cargo.toml` 工作区根
- `scripts/release.sh`：去掉 `bash scripts/build-sidecars.sh` 一行
- `.github/workflows/release.yml`：去掉 PyInstaller 步骤；增加 `cargo build --target` 矩阵（aarch64-apple-darwin / x86_64-apple-darwin / x86_64-pc-windows-msvc / x86_64-unknown-linux-gnu）
- `package.json` scripts：`dev:server` 改为 `pnpm --filter server-app dev:tauri`（直接 tauri dev，不再单跑 sidecar）

### 14.3 体积验证目标

| 平台 | 当前 dmg | 目标 dmg |
|---|---|---|
| macOS arm64 server | ~35MB | **≤ 15MB** |
| macOS arm64 client | ~35MB | **≤ 15MB** |
| 双端总和 | ~70MB | **≤ 30MB** |

---

## 15. 迁移计划（无中间态、激进切换）

> **核心原则**：用户量为 0 → 不做 feature flag、不做并行运行、不留 Python fallback。每个 Sprint 结束 main 分支必须可发可跑，但中间不必"等价"——可以"砍 Python 后跑 Rust"。

### Sprint 1（Week 1）：地基 + PAC + 共享 crate
- [ ] 仓库根新增 Cargo workspace（保留旧 src-tauri 仍可编译）
- [ ] 新建 `crates/conduit-core`，先实现 `pac.rs` + `error.rs` + `events.rs` + `types.rs`
- [ ] 把 server 的 PAC 单元测试用例（约 20 个）翻译成 Rust，**100% pass**
- [ ] 把 `proxy.pac` 通过 `include_str!` 嵌入
- [ ] **里程碑**：`cargo test -p conduit-core` 全绿

### Sprint 2（Week 2）：server-app HTTP/SOCKS5/mDNS 全量
- [ ] `proxy/http.rs`（含 PAC serving + /check + heartbeat endpoint）
- [ ] `proxy/socks5.rs`（fast-socks5 接 outbound）
- [ ] `proxy/outbound.rs`（DIRECT-first race + socket2 bind）
- [ ] `proxy/connections.rs` + `proxy/traffic.rs`
- [ ] `proxy/advertiser.rs`（mdns-sd）
- [ ] `proxy/healthcheck.rs`
- [ ] `ipc/commands.rs` + `ipc/events.rs`
- [ ] **删除** `server-app/core/`、`server-app/src-tauri/src/sidecar.rs`、binaries-dir
- [ ] UI 端：用新 `invoke` / `listen` 替换 `fetch` / `EventSource`，generate bindings
- [ ] **里程碑**：`pnpm tauri build` 出来的 .app 无 sidecar，能完整跑通端到端

### Sprint 3（Week 3-4）：client-app 全量
- [ ] `proxy/discoverer.rs`
- [ ] `proxy/route.rs`（cache + resolver）
- [ ] `proxy/local.rs`（本地 SOCKS5）
- [ ] `proxy/system_proxy.rs`（networksetup）
- [ ] `proxy/connectivity.rs`
- [ ] `proxy/traffic.rs`
- [ ] `ipc/commands.rs` + `ipc/events.rs`
- [ ] **删除** `client-app/core/`、`client-app/src-tauri/src/sidecar.rs`、binaries-dir
- [ ] UI 端：同上替换
- [ ] **里程碑**：双端 .dmg 发 v0.2.0-alpha

### Sprint 4（Week 5-6）：测试 + 打包链 + 发布
- [ ] 删除 `scripts/build-sidecars.sh`、双端 `pyproject.toml`
- [ ] 改造 `scripts/release.sh` / `e2e.sh`
- [ ] GitHub Actions release.yml 加 cargo cross-compile 矩阵
- [ ] 跑完 e2e 全集
- [ ] 发 v0.2.0 正式版
- [ ] **里程碑**：仓库 grep 不到 .py 业务代码

---

## 16. 时间线

| 周 | 主要工作 | 交付 |
|---|---|---|
| W0 | POC（mdns-sd / hyper CONNECT / Tauri Emit / networksetup sandbox / cargo cross） | POC 报告 |
| W1 | Sprint 1：地基 + PAC + conduit-core | PAC 单测全绿 |
| W2 | Sprint 2：server-app 全量 + UI 适配 | server .app 可独立跑 |
| W3-W4 | Sprint 3：client-app 全量 + UI 适配 | client .app 可独立跑，v0.2.0-alpha |
| W5-W6 | Sprint 4：测试 + 打包链 + 发布 | v0.2.0 正式版 |

总计 **6 周全职**（W0 算 0.5 周），含 POC、不含返工与节假日。返工缓冲 +1 周比较稳。

---

## 17. 风险与未决问题

| # | 风险 / 未决 | 等级 | 处理 |
|---|---|---|---|
| R1 | mdns-sd 与 macOS Bonjour Browser 互操作未知 | 🟡 中 | W0 POC-1 验证；若不通 fallback 到 zeroconf-rs |
| R2 | hyper 1.x 处理 CONNECT 隧道时 1GB 大文件吞吐是否达标 | 🟡 中 | W0 POC-2 验证 |
| R3 | macOS Tauri sandbox 是否能调 `/usr/sbin/networksetup` | 🟡 中 | W0 POC-4 验证；最坏开 sandbox entitlement |
| R4 | tauri::Emit 在 1000 conn/s 事件下 UI 是否卡 | 🟢 低 | W0 POC-3 验证；可批量合并事件 |
| R5 | cross-compile macOS x86_64 在 arm64 host 上的产物可用性 | 🟢 低 | W0 POC-5 验证；GitHub Actions runner 选 macos-13 (intel) 兜底 |
| R6 | specta 生成 TS 类型与现有 UI props 不完全对齐 | 🟢 低 | 在 conduit-core::types 加 `#[serde(rename_all = "snake_case")]` 与 UI 已有约定一致 |
| R7 | `tracing-appender::rolling` 在 macOS sandbox 下写 `~/.conduit/logs/` 是否被拦 | 🟢 低 | 实测；最差改写到 `~/Library/Logs/Conduit/` |
| R8 | Tauri command 序列化 `dashmap::DashMap` 不友好 | 🟢 低 | snapshot 转 `Vec<T>` 再序列化 |
| R9 | 旧 known-servers.json / route-cache.json 字段需保持兼容 | 🟢 低 | wire-compatible 字段不变；新版直接读旧文件 |
| R10 | client 端 LAN 上调 server 的 `client_heartbeat` HTTP endpoint 不能丢 | 🟡 中 | 不能，`proxy/http.rs` 必须保留这个 endpoint（在 0.0.0.0:8080 上） |

---

## 附录 A：关键 trait / struct 签名一览

> 仅列出**对外公开**的部分，私有实现细节不展开。

### A.1 `conduit-core` 公共 API

```rust
// PAC
pub struct PacRules { ... }
impl PacRules {
    pub fn parse(text: &str) -> Self;
    pub fn update_proxy_target(&mut self, host: &str, port: u16);
    pub fn find_proxy(&self, host: &str) -> PacDecision;
}

// EventBus
pub struct EventBus<T: Clone + Send + 'static> { ... }
impl<T: Clone + Send + 'static> EventBus<T> {
    pub fn new(capacity: usize) -> Self;
    pub fn publish(&self, event: T);
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<T>;
}

// Relay
pub async fn bidirectional_relay<R1, W1, R2, W2>(
    upstream_r: R1, upstream_w: W1,
    downstream_r: R2, downstream_w: W2,
    on_progress: impl Fn(u64, u64) + Send + Sync,
) -> std::io::Result<(u64, u64)>
where R1: AsyncRead + Unpin + Send, W1: AsyncWrite + Unpin + Send, ... ;

// Errors
pub enum ConduitError { ... }
pub type ConduitResult<T> = Result<T, ConduitError>;
```

### A.2 server-app `proxy::core::ProxyCore`

```rust
pub struct ProxyCore { ... }

impl ProxyCore {
    pub fn new(cfg: ProxyConfig) -> Self;
    pub async fn start(&self) -> ConduitResult<()>;
    pub async fn stop(&self) -> ConduitResult<()>;
    pub async fn status(&self) -> ServerStatus;
    pub fn bus(&self) -> &EventBus<ServerEvent>;
}
```

### A.3 server-app `ipc::commands`

```rust
#[tauri::command]
#[specta::specta]
pub async fn get_status(state: tauri::State<'_, Arc<ProxyCore>>) -> Result<ServerStatus, String>;

#[tauri::command]
#[specta::specta]
pub async fn get_clients(state: tauri::State<'_, Arc<ProxyCore>>) -> Result<ClientsSnapshot, String>;

#[tauri::command]
#[specta::specta]
pub async fn get_traffic(state: tauri::State<'_, Arc<ProxyCore>>) -> Result<TrafficSnapshot, String>;

#[tauri::command]
#[specta::specta]
pub async fn start_proxy(state: tauri::State<'_, Arc<ProxyCore>>) -> Result<(), String>;

#[tauri::command]
#[specta::specta]
pub async fn stop_proxy(state: tauri::State<'_, Arc<ProxyCore>>) -> Result<(), String>;

#[tauri::command]
#[specta::specta]
pub async fn get_health(state: tauri::State<'_, Arc<ProxyCore>>) -> Result<HealthReport, String>;
```

### A.4 client-app `proxy::core::ClientCore`

```rust
pub struct ClientCore { ... }

impl ClientCore {
    pub fn new(cfg: ClientConfig) -> Self;
    pub async fn start(&self) -> ConduitResult<()>;
    pub async fn stop(&self) -> ConduitResult<()>;
    pub async fn connect_to(&self, server_id: &str) -> ConduitResult<ConnectionInfo>;
    pub async fn disconnect(&self) -> ConduitResult<()>;
    pub async fn diagnose(&self) -> ConduitResult<DiagnoseReport>;
    pub fn bus(&self) -> &EventBus<ClientEvent>;
}
```

---

## 附录 B：TS / Rust 类型一致性方案

### B.1 specta 工作流

1. `conduit-core/types.rs` 中所有 wire type derive `specta::Type`
2. server-app/src-tauri 与 client-app/src-tauri 的 `build.rs` 调 `specta::ts::export()` 把当前 binary 的 commands + types 导出为 TS 文件
3. 输出到：
   - `server-app/ui/src/generated/bindings.ts`
   - `client-app/ui/src/generated/bindings.ts`
4. UI import 使用：`import { commands, ServerStatus } from '@/generated/bindings'`
5. CI 步骤：`cargo build` 后 `git diff --exit-code` 检查 bindings.ts 是否提交

### B.2 命名约定

- Rust struct field：`snake_case`（默认 + serde rename_all）
- TS type field：`snake_case`（与 UI 现有 props 对齐）
- Enum tag：`#[serde(tag = "kind", rename_all = "snake_case")]`，TS 端用 discriminated union

### B.3 路径

```
crates/conduit-core/src/types.rs           ← 单一 source of truth（Rust）
       ↓ build.rs
server-app/ui/src/generated/bindings.ts    ← auto-gen, in-repo, gitignored or committed
client-app/ui/src/generated/bindings.ts    ← auto-gen
```

建议 **commit bindings.ts**：方便 PR review 时看到 wire 变化，CI 用 git diff 兜底防漂移。

---

## 附录 C：Cargo.toml 完整版（双端 src-tauri）

```toml
# server-app/src-tauri/Cargo.toml
[package]
name = "conduit-server"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[lib]
name = "conduit_server_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
conduit-core = { path = "../../crates/conduit-core" }
tauri = { workspace = true, features = ["tray-icon"] }
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
hyper.workspace = true
hyper-util.workspace = true
http-body-util.workspace = true
fast-socks5.workspace = true
mdns-sd.workspace = true
dashmap.workspace = true
netdev.workspace = true
clap.workspace = true
chrono.workspace = true
socket2.workspace = true
anyhow.workspace = true
thiserror.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
tracing-appender.workspace = true
specta.workspace = true
```

```toml
# client-app/src-tauri/Cargo.toml
[package]
name = "conduit-client"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[lib]
name = "conduit_client_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
conduit-core = { path = "../../crates/conduit-core" }
tauri = { workspace = true, features = ["tray-icon"] }
tauri-plugin-autostart = "2"
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
hyper.workspace = true
hyper-util.workspace = true
http-body-util.workspace = true
fast-socks5.workspace = true
mdns-sd.workspace = true
dashmap.workspace = true
moka.workspace = true
netdev.workspace = true
clap.workspace = true
chrono.workspace = true
socket2.workspace = true
anyhow.workspace = true
thiserror.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
tracing-appender.workspace = true
specta.workspace = true
```

---

**变更记录**：
- 2026-05-06 v1.0 初稿（基于可行性分析 v1.1 + 不留中间态原则）
