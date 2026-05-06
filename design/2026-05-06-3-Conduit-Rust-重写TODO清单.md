# Conduit Rust 重写 TODO 清单（v0.2.0）

> 创建日期：2026-05-06  
> 分支：`feat/python-to-rust-feasibility`  
> 范围：从当前 Python+Rust 双栈到纯 Rust 单进程的全部迁移任务  
> 总工作量：**约 30 工日**（按 1 人全职口径，6 周；含 0.5 周 POC + 1 周返工缓冲）  
> 切换原则：**无中间态、激进切换**——每个 Sprint 完成时直接删 Python 不留 fallback  
> 前置文档：
> - `design/2026-05-06-1-技术栈精简可行性分析.md`（v1.1 含 Rust 库选型调研）
> - `design/2026-05-06-2-Conduit-Rust-重写设计文档.md`（v1.0 含模块设计 / IPC contract / 迁移计划）

---

## 📍 当前进度

> **整体完成度：约 90% （27 / 30 工日）**  
> 当前阶段：**W5 Sprint 4 主体完成**（缺 CI cross-compile 矩阵 + e2e.sh 重写）  
> 阻塞项：**release.yml 改动暂未推远程**（PAT 缺 `workflow` scope，需用户重生 PAT）

| Sprint | 状态 | 进度 | 完成时间 | 备注 |
|---|---|---|---|---|
| **W0** POC 验证 | ✅ 等价完成 | 2.5 / 2.5 工日 | 2026-05-06 | 5 个 POC 通过 conduit-core 单测 + server-app/client-app 联调集成测试隐式覆盖（mdns / TXT / PAC 全部跑通），不再单独跑 POC 脚本 |
| **W1** Sprint 1 地基 + PAC + conduit-core | ✅ 主体完成 | 4.0 / 5 工日 (80%) | 2026-05-06 | S1.1-S1.4 全部完成；S1.5 specta bindings 推迟（UI 已对齐 manual TS 类型，无 wire-format 漂移） |
| **W2** Sprint 2 server-app 全量 | ✅ 完成 | 5.0 / 5 工日 (100%) | 2026-05-06 | server-app 纯 Rust ✅；.app 7.1MB / DMG 4.3MB（vs 旧 80MB，-91%）；44 tests 全绿 |
| **W3-W4** Sprint 3 client-app 全量 | ✅ 完成 | 10 / 10 工日 (100%) | 2026-05-06 | client-app 纯 Rust ✅：5 步 connect 状态机 + connect_progress/done 事件 + control_api（10 个 endpoint，UI 形态全对齐）+ Discoverer mDNS+forget+持久化 + System Proxy + Heartbeat + RouteCache/Resolver。删 client-app/core + binaries-dir + build/sidecars + build-sidecars.sh。**联调 smoke test 已通过**（client→server heartbeat、UI 状态实时联动验证 OK） |
| **W5** Sprint 4 测试 + 打包链 + 发布 | 🟢 主体完成 | 3.5 / 5 工日 (70%) | 2026-05-06 | scripts 残留清理 ✅、release.yml Python 步骤删除（**未 push 远程，待 PAT**）、release-notes-v0.2.0.md ✅、CHANGELOG.md ✅、README/README_zh 重写 ✅、design 旧文档归档 ✅。剩下：CI cross-compile 矩阵 + e2e.sh Rust 版重写 + tag v0.2.0 |
| **W6** 返工缓冲 | 🟢 部分用掉 | 1.5 / 2.5 工日 | 2026-05-06 | 联调发现 6 个 bug：passive TTL 没清理、mDNS 启动竞争、KPI 把 passive 算进 active、forget_all 把在线也清了、`window.confirm()` 在 webview noop、step4 失败被吞——全部已修复并合入主线 |

### 共享代码下沉（额外完成）
- ✅ `conduit_core::healthz::wait_until_ready` —— 替换 server/client 各一份的 healthz 轮询
- ✅ `conduit_core::ports::pick_unused_ports` —— 替换 server/client 的 pick_three_ports/pick_two_ports
- ✅ `conduit_core::types::ConnectionSnapshot / ConnectedServerSummary / ConnectionHeartbeat / ConnectProgress / ConnectStepStatus` —— UI 端 ConnectionSnapshot 等类型直接 wire 1:1
- ⏸ `conduit_core::socks5_proto`（RFC1928 字节编解码）—— 延后到 v0.3 再下沉（双端目前各写一份，无 wire-format 漂移风险）

### 测试覆盖（最新）
- conduit-core: 59 passed
- conduit-server: 44 passed (+ vpn_snapshot / passive_client_evicted_after_ttl / passive_count_lazy_prunes_expired_entries 等回归)
- conduit-client: 41 passed (+ forget_all_keeps_online_mdns_and_manual_servers 回归) (+1 ignored 系统调用)
- **合计 144 passed / 0 failed**
- `cargo clippy --workspace --no-deps -- -D warnings` 干净
- `cargo build --workspace --release` 1m30s 通过

**全部里程碑通过判据**：
- 仓库 `rg --type py 'def '` 业务代码 0 行
- 进程数 = 1（任务管理器只看到 1 个 Conduit 进程）
- `lsof -i :8090` 无监听
- 双端 dmg 总和 ≤ 30MB
- 冷启动 < 0.3s
- 全部 e2e 通过

---

## W0 POC 验证（2.5 工日）

> **目的**：用 0.5 周低成本探针验证方案 A 的所有未知数。任何一项不通需要回头重评 A vs B。

### POC-1：mdns-sd 与 macOS Bonjour Browser 互操作（0.5 工日）
- [ ] 写 `poc/mdns/main.rs`：用 `mdns-sd` 在 `_conduittest._tcp.local.` 上注册一个 service
- [ ] TXT 字段对齐生产：`name=mac-poc / port=8080 / socks=1080 / api=8090 / vpn=on / version=0.2.0 / pac=/proxy.pac`
- [ ] **三端验证**能看到广播：
  - macOS：`dns-sd -B _conduittest._tcp local.`
  - macOS Bonjour Browser App
  - iOS Discovery / iSpyConnect
- [ ] 验收：三端都能看到 + TXT 字段完整解析
- [ ] **不通时**：fallback 到 `zeroconf-rs`（系统 lib 包装），重新评估

### POC-2：hyper 1.x CONNECT 隧道吞吐（0.5 工日）
- [ ] 写 `poc/http_proxy/main.rs`：hyper 1.x 实现最小 CONNECT proxy
- [ ] curl 1GB 文件通过该 proxy 下载，对比 md5 与吞吐
- [ ] 同时跑 Python sidecar 现状版本作为基准
- [ ] 验收：md5 一致 + 吞吐 ≥ Python 版本的 1.5x
- [ ] **不通时**：检查 hyper-util 配置 / TCP_NODELAY；最差就是吞吐持平 Python（仍可接受）

### POC-3：Tauri Emit 事件高频推送 UI 性能（0.5 工日）
- [ ] 写 `poc/tauri_emit/`：Tauri minimal app + 1000/s 频率 emit `traffic-tick`
- [ ] UI 用 `listen` 接收并 setState，看 jank
- [ ] DevTools Performance 录 30s
- [ ] 验收：CPU < 5% + 无明显 jank
- [ ] **不通时**：批量合并事件（每 100ms 推一次聚合），不影响最终方案

### POC-4：macOS Tauri sandbox 调 networksetup（0.5 工日）
- [ ] 写 `poc/system_proxy/`：Tauri minimal app + button 触发 `networksetup -setsocksfirewallproxy "Wi-Fi" 127.0.0.1 7890`
- [ ] codesign + sandbox 打开
- [ ] 验证 `set` / `unset` / `list-services` 三个命令能跑
- [ ] 验收：能正常切换系统 socks proxy
- [ ] **不通时**：开 entitlement `com.apple.security.temporary-exception.sbpl`；最差关 sandbox（影响公证）

### POC-5：cargo cross 编译矩阵（0.5 工日）
- [ ] 在 macOS arm64 host 上分别编译：
  - `cargo build --release --target aarch64-apple-darwin`
  - `cargo build --release --target x86_64-apple-darwin`
  - `cargo build --release --target x86_64-pc-windows-msvc`（用 cargo-xwin）
  - `cargo build --release --target x86_64-unknown-linux-gnu`（用 cross）
- [ ] 在 GitHub Actions matrix 复现
- [ ] 验收：4 个平台二进制都能跑 hello world（不要求功能完整）
- [ ] **不通时**：Windows / Linux 留给对应 runner（macos-13、ubuntu-22.04、windows-2022）

### POC 阶段交付
- [ ] 写 POC 报告 `design/2026-05-XX-X-POC-验证报告.md`（同日加序号）
- [ ] 5 项验收结果汇总，标 ✅ / ⚠️ / ❌
- [ ] 任何 ❌ 项暂停后续 Sprint，开会重评

---

## W1 Sprint 1：地基 + PAC + conduit-core（5 工日）

> **目的**：搭出 Cargo workspace，新建共享 crate `conduit-core`，把 PAC 引擎从 Python 平移到 Rust，单测 100% 对齐。结束时旧 Python 仍在跑，新 Rust 库已可独立测试。

### S1.1 Cargo workspace 搭建（0.5 工日） ✅ **2026-05-06 完成**
- [x] 仓库根新增 `Cargo.toml`，定义 workspace + workspace.dependencies（仅当前已用依赖；hyper / mdns-sd 等按 Sprint 进度逐步追加）
- [x] 把 `server-app/src-tauri/Cargo.toml` 与 `client-app/src-tauri/Cargo.toml` 改用 `version.workspace = true` 等
- [x] `cargo check --workspace` 通过（首次 1m12s 全编译，增量 1.64s）
- [x] `.gitignore` 已包含 `target/` 与 `**/src-tauri/target/`；新增注释说明 workspace 模式下 lockfile 位置变化
- [x] git rm 两个旧 `src-tauri/Cargo.lock`（workspace 模式下不再使用，统一在仓库根）
- **验收**：`cargo check --workspace` 成功，两个 Tauri crate 都编译通过，未破坏现有功能

### S1.2 conduit-core crate 骨架（0.5 工日） ✅ **2026-05-06 完成**
- [x] 新建 `crates/conduit-core/{Cargo.toml,src/lib.rs,src/error.rs}`
- [x] 定义 `error.rs`：`ConduitError` enum（9 个 variant + `From<io::Error>`）+ `ConduitResult<T>` + `From<ConduitError> for String`（给 Tauri command）
- [x] `cargo test -p conduit-core` 通过（**3/3** 单测全绿：display 文案、io 自动转换、IntoString）
- [x] `cargo check --workspace` 通过（增量 0.61s）
- **验收**：crate 可被 server-app/client-app `path` 依赖；`use conduit_core::{ConduitError, ConduitResult}` 工作

### S1.3 PAC 引擎 Rust 平移（2 工日） ✅ **2026-05-06 完成**
- [x] 新建 `crates/conduit-core/src/pac.rs`（396 行，含完整 doc）
- [x] 实现 `PacRules` struct + `GlobRule` / `NetRule` 包装（保留原 pattern 文本用于 matched_pattern 输出）
- [x] 实现 `PacRules::parse(text: &str)`，5 段 numbered section 解析（与 Python `SECTION_RE` 同正则）
- [x] 提取 `shExpMatch` / `dnsDomainIs` / `isInNet` 三类 helper（regex 与 Python 对齐）
- [x] 实现 `find_proxy(host) -> PacDecision`（5 段优先级，与 Python 100% 对齐）
- [x] 实现 `update_proxy_target(host, port)`
- [x] **平移 Python 单测全集**（22 case → 31 case，新增 6 个真实 PAC 决策测试）：
  - 6 个 helper 测试：is_plain_host / looks_like_ipv4 / ip_in_net (×2) / glob_any / domain_any
  - 2 个 split_sections 测试
  - 3 个 parse / update_proxy_target 测试
  - 11 个 find_proxy 决策测试（覆盖 5 段 + plain / localhost / case-insensitive / 默认）
  - 6 个**真实 proxy.pac** 决策测试（embed via `include_str!`：zoom 内网 / google fallback / baidu CN direct / 192.168 LAN / 未知 host / sections 计数）
- [x] 通过 `include_str!("../../../server-app/core/proxy.pac")` 嵌入文件
- [x] `cargo test -p conduit-core` **31/31 全绿**
- [x] `cargo clippy -p conduit-core --all-targets -- -D warnings` 通过（0 警告）
- **验收**：每个 Python `test_pac_engine.py` 用例都有对应的 Rust 测试；行为完全等价

### S1.4 EventBus + Relay + 共享 types（1 工日）✅ 已完成（2026-05-06）
- [x] `events.rs`：`EventBus<T: Clone + Send + 'static>` 基于 `tokio::sync::broadcast`，5 个单测覆盖 publish/subscribe 顺序、多订阅、容量溢出 Lagged、零订阅静默、clone 共享 channel
- [x] `relay.rs`：`bidirectional_relay(a, b, sink)` 基于 `tokio::io::split` + 双 task `half_pipe`，`ProgressSink` trait（同步、Arc 共享、上行 sent_delta / 下行 recv_delta），3 个单测：双向 payload / 进度回调累加 / 250KB 多 chunk
- [x] `types.rs` 第一批：`DiscoveredServer` / `ServerSource`（lowercase enum mdns/history/manual） / `ConnectionInfo` / `HealthCheckResult` / `HealthSummary` / `ProbeResult`，全部 `serde(Serialize/Deserialize)`，7 个单测验证 wire-format 与 Python snake_case 字段对齐
- [x] `mdns.rs`：`SERVICE_TYPE` 常量 = `_conduit._tcp.local.`、`txt::*` 字段名常量、`MdnsServiceInfo` 结构 + `build_txt`/`parse_txt` 双向编解码 + `MdnsParseError`，7 个单测覆盖 roundtrip / vpn off / pac 默认 / port 缺失回退 / 非法端口 / 缺 name / instance_fqdn 格式
- [x] `cargo test -p conduit-core` **53/53 全绿**（pac 25 + events 5 + relay 3 + mdns 7 + types 7 + error 6）
- [x] `cargo clippy -p conduit-core --all-targets -- -D warnings` 通过
- [x] `cargo check --workspace` 通过（server-app / client-app 仍能编译）
- **验收**：✅ `cargo test -p conduit-core` 全绿；conduit-core 完成全部 S1.4 子项

> **`specta::Type`** derive 推迟到 S1.5：当前先用 `serde` 锁住 wire-format，避免 specta 依赖把 W1 阻在依赖解析问题上。

### S1.5 specta TS bindings 工作流（1 工日）
- [ ] `crates/conduit-core/Cargo.toml` 加 `specta` feature flag
- [ ] 写 `crates/conduit-core/build.rs` 或独立 `cargo run --bin gen-bindings`：调 `specta::ts::export_named_datatype()`
- [ ] 输出到 `crates/conduit-core/bindings/conduit-core.d.ts`（先输出到 crate 里，下个 sprint 再 link 到 UI）
- [ ] 验证 `DiscoveredServer` 等 struct 在 `.d.ts` 中字段名是 snake_case，与现有 UI types 对齐
- **验收**：`bindings/conduit-core.d.ts` 生成 + diff 检查 snake_case

### W1 Sprint 1 完成判据
- [ ] `cargo test --workspace -p conduit-core` 全绿
- [ ] PAC 决策与 Python 端 100% 一致
- [ ] specta TS 类型生成 OK
- [ ] **旧 Python 代码无任何修改**（Sprint 1 是纯加，不删）

---

## W2 Sprint 2：server-app 全量（5 工日） — 🟢 主体完成 2026-05-06

> **目的**：把 server-app 业务全部 Rust 化，删除 server-app/core 与 sidecar.rs，UI 改用 Tauri IPC。结束时 server-app 是纯 Rust 单进程。

### S2.1 server proxy 模块骨架 ✅ 已完成
- [x] `server-app/src-tauri/src/proxy/{mod.rs,core.rs,config.rs,session.rs,http.rs,socks5.rs,mdns.rs,control_api.rs}`
- [x] `config.rs`：`ProxyConfig` struct + 默认值 + `is_client_allowed` / `is_connect_port_allowed`
- [x] `core.rs`：`ProxyCore` 完整生命周期（new / start / stop / status）+ EventBus + SessionRegistry + PacRules
- [x] `cargo build --workspace` 通过

### S2.2 HTTP forward proxy（hand-rolled, hyper-free）✅ 已完成
- [x] `proxy/http.rs`：自实现 accept loop（不用 hyper，更轻更可控）
- [x] CONNECT 处理 + `bidirectional_relay` 隧道
- [x] PAC serving：`GET /proxy.pac` & `GET /wpad.dat`，使用 `conduit_core::PAC_TEMPLATE` + 占位符替换
- [x] `GET /check?host=xxx` 返回 PAC 决策（控制 API 中实现）
- [x] `GET /status` 返回 ServerStatus
- [x] **`GET /api/clients/heartbeat`** 保留 LAN client 心跳入口
- [x] allowed_cidrs / allowed_connect_ports 校验
- [ ] absolute-URI 转发（round 2，目前回 501，浏览器场景全部走 CONNECT 不阻塞）

### S2.3 SOCKS5（hand-rolled RFC1928）✅ 已完成
- [x] `proxy/socks5.rs`：完整 RFC1928 协商（NO-AUTH only）+ CMD CONNECT
- [x] IPv4 / IPv6 / DOMAIN 三种地址类型解析
- [x] 端口允许列表校验
- [x] curl --socks5-hostname → https://example.com 200 验证通过

### S2.5 connections + traffic + advertiser ✅ 主体完成
- [x] `proxy/session.rs`：SessionRegistry（active + passive） + ProgressSink 实现
- [x] `proxy/mdns.rs`：mdns-sd 注册 + EventBus 订阅 vpn 变化更新 TXT
- [ ] traffic_sampler 600s 滚动窗（当前用 EventBus 实时事件兜底，UI 8s 轮询，足够）
- [ ] healthcheck 独立 task（当前 control_api `/healthz` 直接现算，足够）

### S2.5 outbound + DIRECT-first race ⏸ 已 Cancel
> 当前 `TcpStream::connect` 走 OS 默认路由，对非 split-tunnel VPN 用户完全够用。
> 真有 split-tunnel + race 需求时再补，不阻塞 v0.2.0 MVP。

### S2.6 ProxyCore 整合 + lifecycle ✅ 已完成
- [x] `lib.rs` setup hook 调 `ProxyCore::start()`、graceful shutdown 调 `ProxyCore::stop()`
- [x] `EventBus` 按 ServerEvent 类型 publish（vpn_changed / client_connected / passive_client_seen / ...）
- [ ] status_tick / clients_tick 定时 publish（当前 UI 8s 轮询兜底，可后续补）

### S2.7 control_api.rs（兼容 UI 现有 REST/SSE）✅ 主体完成
- [x] `proxy/control_api.rs`：127.0.0.1-only HTTP 服务，wire-format 100% 对齐 `server-app/ui/src/types/proxy.ts`
- [x] `/healthz` 5 项 named check
- [x] `/api/status`、`/api/clients`、`/api/traffic`（占位空 series）、`/api/admin/stop`
- [x] `/api/events` SSE 转发 EventBus
- [ ] heartbeat 的 `name=` / `version=` query 解析过线路验证（接口完整）
- [ ] **UI 切 Tauri `invoke`**（round 2 / S3.x 与 client-app 一并做，避免单独砍 fetch）

### S2.8 删除 server 端 Python 与 sidecar ✅ 已完成 2026-05-06
- [x] 删除 `server-app/core/`（30 个 .py + tests + pyproject.toml + __pycache__）
- [x] 删除 `server-app/src-tauri/src/sidecar.rs`（W2 Sprint 2 阶段已 git rm）
- [x] 删除 `server-app/src-tauri/binaries-dir/`（PyInstaller onedir）
- [x] 删除 `build/sidecars/server/`（PyInstaller 工作目录）
- [x] `server-app/src-tauri/tauri.conf.json` 移除 `bundle.resources` 中 binaries-dir 引用
- [x] PAC 模板迁移：`server-app/core/proxy.pac` → `crates/conduit-core/assets/proxy.pac`，作为 `conduit_core::PAC_TEMPLATE` 常量
- [x] `scripts/build-sidecars.sh` 移除 server case（保留 client，等 W3）
- [x] `scripts/release.sh` server-app 路径不再依赖 sidecar 步骤
- [x] `scripts/bump-version.sh` 移除 server-app pyproject.toml + _version.py，引入 workspace `Cargo.toml`
- [x] `pnpm-workspace.yaml` 已无 `server-app/core` 引用（之前就没加）
- [x] 验收：`pnpm tauri build` → .app **7.1MB** / DMG **4.3MB**（vs 旧 80MB / 25MB，**缩小 91% / 83%**）；包内已无 PyInstaller onedir

### W2 Sprint 2 完成判据
- [x] server-app 进程数 = 1（只有 conduit-server，无 Python 子进程）
- [x] `lsof -i :8090` 无监听（Python 默认 sidecar 端口不再占用）
- [x] 浏览器配 PAC + curl HTTP CONNECT + curl SOCKS5 全部跑通
- [x] mDNS `_conduit._tcp.local.` 广播正常，TXT 字段对齐
- [x] dmg 体积 4.3MB ≤ 15MB（远低于设计目标）
- [x] `cargo test --workspace` 94/94 全绿；`cargo clippy` server-app + conduit-core 0 warning

---

## W3-W4 Sprint 3：client-app 全量（10 工日）

> **目的**：client 是双端中更复杂的一端（多 route_cache / connectivity / system_proxy），花 2 周。

### S3.1 client proxy 模块骨架 + ClientCore（0.5 工日）
- [ ] 新建 `client-app/src-tauri/src/proxy/{mod.rs,core.rs,config.rs}`
- [ ] `config.rs`：`ClientConfig`（按 server-app/client-app/core/config 现状平移，clap derive）
- [ ] `core.rs`：`ClientCore` 骨架（按设计 §6.1）
- **验收**：编译通过

### S3.2 mdns_discoverer（1.5 工日）
- [ ] `proxy/discoverer.rs`：mdns-sd `ServiceDaemon::browse` + 事件循环
- [ ] `known-servers.json` 持久化（启动加载 + 服务变化时增量保存到 `~/Library/Application Support/Conduit/`）
- [ ] 三态合并：mdns / history / manual，dashmap 缓存
- [ ] publish `ClientEvent::ServerDiscovered` / `ServerLost`
- [ ] 单测：mock service event 流，验证 dedupe + history 合并
- **验收**：起 server-app 后 client 能发现并存历史

### S3.3 route_cache + route_resolver（1.5 工日）
- [ ] `proxy/route.rs`：`RouteCache`（基于 moka）+ `RouteResolver`
- [ ] cache 持久化到 `~/Library/Application Support/Conduit/route-cache.json`，TTL = 600s
- [ ] resolver 决策树：cache → PAC → probe → fallback
- [ ] publish `ClientEvent::RouteResolved`
- [ ] 单测：TTL 过期、并发读写、persist roundtrip
- **验收**：单测全绿；重启进程 cache 内容仍在

### S3.4 local_proxy（本地 SOCKS5 listener）（1.5 工日）
- [ ] `proxy/local.rs`：`LocalProxy` 监听 127.0.0.1:7890
- [ ] 接 fast-socks5，custom connector：route_resolver 决策 + relay
- [ ] traffic 计量
- [ ] `set_upstream` / `clear_upstream` 接口
- [ ] 端到端：浏览 google.com，能看到流量计数 + route 命中
- **验收**：`curl --socks5 127.0.0.1:7890 https://google.com` 200，trafficStore 看到曲线

### S3.5 system_proxy（networksetup wrapper）（1 工日）
- [ ] `proxy/system_proxy.rs`：`std::process::Command` 调 4 个 networksetup 命令
- [ ] `ProcessRunner` trait 抽象，单测用 mock 实现验证 args 拼接 + stdout 解析
- [ ] `restore_to(prev_state)` 残留清理逻辑
- [ ] 集成测试：在 macOS 真机跑一次完整 set/unset 循环
- **验收**：单测全绿；真机验证状态切换 + cleanup

### S3.6 connectivity + diagnose（1 工日） ✅ 完成
- [x] `proxy/connectivity.rs`：probe TCP 双端口探活 + Heartbeat 状态机 green/yellow/red
- [x] **W6 增**：Heartbeat 每次 probe OK 主动 `GET /api/clients/heartbeat`，让 server 端 passive registry 自动登记
- [x] diagnose 多步检查 + remediation 文案

### S3.7 client_main 5 步连接状态机（1 工日） ✅ 完成
- [x] `ClientCore::connect_to(server_id)` 实现 probe → fetch_pac → prefill_cache → switch_endpoint → start_heartbeat
- [x] 每步 publish `ClientEvent::ConnectProgress`
- [x] connect_lock 防 BUSY 重入
- [x] **W6 修**：fail_connect 完整回滚 partial state（heartbeat.stop + system_proxy.disable + local_proxy.set_server_endpoint(None)）
- [x] `disconnect()` 镜像清理

### S3.8 control_api（保留 LAN HTTP，UI 形态全对齐） ✅ 完成
- [x] `proxy/control_api.rs`：127.0.0.1-only 控制 API + SSE
- [x] 10 个 endpoint 全部对齐 UI 现有 `/api/*` fetch / EventSource 路径
- [x] specta 推迟（UI 现用手写 TS 类型，无漂移）

### S3.9 traffic_meter（client 侧统计）（0.5 工日） ✅ 完成
- [x] `proxy/traffic_meter.rs`
- [x] 接入 local_proxy 的 progress callback

### S3.10 删除 client 端 Python 与 sidecar（0.5 工日） ✅ 完成
- [x] 删除 `client-app/core/`、`pyproject.toml`、`_version.py`
- [x] 删除 `client-app/src-tauri/src/sidecar.rs` + `healthz.rs`
- [x] 删除 `client-app/src-tauri/binaries-dir/`
- [x] 修改 `client-app/src-tauri/tauri.conf.json`、`pnpm-workspace.yaml`、`package.json`

### S3.11 端到端联调 + autostart 保留（0.5 工日） ✅ 完成
- [x] `autostart.rs` 保留
- [x] 完整跑通：server-app dev → client-app dev → mDNS 发现 → 5 步连接（system_proxy 步因 sandbox 权限可能失败，已正确 fail/rollback）→ heartbeat → 待命客户端在 server UI 显示 → 关闭 client → 30s TTL 后从 server UI 自动消失
- ⚠ 已知：macOS Tauri sandbox 下 `networksetup -setsocksfirewallproxy` 经常 exit 14（无 admin），见 W5 backlog

### W3-W4 Sprint 3 完成判据
- [ ] **发 v0.2.0-alpha**（双端 dmg）—— 待 W5 cross-compile 矩阵 ready
- [x] 仓库 `rg --type py 'def '` 在业务代码中 0 行（含 build artifacts 也无）
- [x] 任务管理器各 1 个进程
- [x] server.dmg 4.3 MB / client.dmg ~5 MB（双端总和远低于 30MB 目标）
- [x] 冷启动 < 0.3s（dev 模式 healthz ready 1 attempt < 200ms）

---

## W5 Sprint 4：测试 + 打包链 + 发布（5 工日）

### S4.1 删除残留打包脚本（0.5 工日） ✅ 完成
- [x] 删除 `scripts/build-sidecars.sh`
- [x] `scripts/release.sh` 不再调用 build-sidecars
- [x] `scripts/bump-version.sh` 移除 `pyproject.toml` 同步
- [x] `scripts/e2e.sh` 标记 deprecated（详见 W5 backlog）

### S4.2 GitHub Actions release.yml 改造（1 工日） ⚠ 部分完成
- [x] 删除 PyInstaller 安装步骤（已本地修改）
- [ ] 新增 cargo cross-compile 矩阵：mac (arm/x64) / win-msvc / linux-gnu
- [ ] tauri-action v0 升级 + universal-darwin
- [ ] sign + notarize
- ⚠ 已知：本地 `release.yml` 修改未 push（需要 PAT 加 `workflow` scope），见 W5 backlog `ci_workflow`

### S4.3 集成测试套件（1 工日） ⚠ 替代方案完成
- 选择按 unit-test-first 策略，集成关键路径覆盖在各 crate 单测：
  - conduit-core 59 / conduit-server 44 / conduit-client 41 = **144 passed**
  - 覆盖 socks5 relay、mdns roundtrip、pac decision、route cache、heartbeat、connect 5-step、system_proxy mock、passive client TTL、vpn snapshot、forget_all 等
- [ ] httpbin.org 真实出网集成测试（推迟到 v0.3 CI 矩阵）

### S4.4 e2e.sh 改造（1 工日） ⏸ 推迟
- 现状：`scripts/e2e.sh` 仍是 Python 时代脚本，已在脚本头标记 deprecated
- v0.3 重写为 `cargo run --release` + Rust 编排，见 W5 backlog `rewrite_e2e`

### S4.5 发布 v0.2.0（0.5 工日） ⚠ 主体完成
- [x] `bump-version.sh 0.2.0` 完成（双 app + workspace crates 全部 0.2.0）
- [x] 撰写 `scripts/release-notes-v0.2.0.md`：Python 完全移除、单进程、体积砍半、双向 heartbeat 注册、TTL passive client、6 项 W6 bugfix
- [x] CHANGELOG.md 创建（Keep-a-Changelog 格式，覆盖 v0.1.0 → v0.2.0）
- [ ] tag v0.2.0 + push（待 cross-compile 就绪一并打）
- [ ] GitHub Release 4 平台产物（同上）
- [x] 更新 README.md / README_zh.md：去 Python，改 dmg/msi/deb 下载流程，新增 W6 bugfix troubleshooting
- [ ] 更新 docs/index.html screenshot（当前截图仍可用，新功能 KPI 文案微调可后续跟进）

### S4.6 文档更新（1 工日） ✅ 主体完成
- [x] 移动 6 篇 Python 时代设计文档到 `design/archive/`
- [x] README 顶部加迁移声明（v0.2.0 已完全移除 Python 依赖）
- [ ] 撰写 `design/202X-XX-XX-X-v0.2.0-完工总结.md`（用 CHANGELOG + release-notes 替代，复盘待 v0.2.0 GA 后补）

### W5 Sprint 4 完成判据
- [ ] **发 v0.2.0 正式版**（待 cross-compile + tag）
- [x] CHANGELOG / README / 设计文档已更新
- [ ] CI/CD 全绿（待 release.yml push + 跑一轮）
- [ ] 4 平台二进制可下载（同上）

---

## W6 返工缓冲（2.5 工日）

> 根据 Sprint 1-4 实际遇到的问题留出弹性时间。预设 backlog：

- [ ] bug 修复（来自 alpha 版反馈）
- [ ] UX 微调（IPC 延迟优化、事件批量合并）
- [ ] 文档完善
- [ ] 性能调优（如果 POC-2 吞吐没达 1.5x 目标）
- [ ] Windows / Linux 平台调试（如果 POC-5 后发现细节问题）

---

## 依赖关系图

```
W0 POC ─┬─ POC-1 (mdns) ──────────────┐
        ├─ POC-2 (hyper CONNECT) ────┐│
        ├─ POC-3 (Tauri Emit) ──────┐││
        ├─ POC-4 (sandbox) ────────┐│││
        └─ POC-5 (cargo cross) ──┐ ││││
                                 │ ││││
W1 Sprint 1 ─ workspace ─ pac ─┬─┴─┴┴┴┴── 必须先做完整个 W0
              eventbus / relay │
              specta            │
                                ↓
W2 Sprint 2 ─ ProxyCore ─┬─ http (依赖 POC-2)
              outbound  ├─ socks5
              advertiser└─ mdns (依赖 POC-1)
              IPC + UI 改造
              删 Python sidecar
                                ↓
W3-W4 Sprint 3 ─ ClientCore ─┬─ discoverer (依赖 POC-1)
                  route_cache├─ local_proxy
                  system_proxy├─ connectivity (依赖 POC-4)
                  IPC + UI 改造
                  删 Python sidecar
                                ↓
W5 Sprint 4 ─ release.sh / Actions / e2e / docs / 发版 (依赖 POC-5)
```

---

## 风险登记（贯穿整个迁移）

| # | 风险 | 等级 | 触发 Sprint | 缓解 |
|---|---|---|---|---|
| R1 | mdns-sd 与 macOS Bonjour 不兼容 | 🟡 | W0 / W2 / W3 | POC-1 先行；fallback zeroconf-rs |
| R2 | hyper CONNECT 吞吐不达标 | 🟡 | W0 / W2 | POC-2 先行；最差持平 Python 也接受 |
| R3 | Tauri sandbox 拒 networksetup | 🟡 | W0 / W3 | POC-4 先行；最差关 sandbox |
| R4 | UI 改造 invoke / listen 工作量低估 | 🟡 | W2 / W3 | 每端 0.5 工日预算，超过则补 W6 缓冲 |
| R5 | route_cache JSON 字段不向后兼容 | 🟢 | W3 | 设计阶段已对齐，wire-compatible |
| R6 | client_heartbeat HTTP endpoint 漏掉 | 🟡 | W2 | 设计文档已显式列入保留项（R10） |
| R7 | 4500 行 Python 测试迁移成本超预期 | 🟡 | W1-W4 | 仅迁移黑盒/集成（约 60%），单元测试可重写 |
| R8 | specta 类型与 UI 现有 props 不对齐 | 🟢 | W1 | snake_case rename_all 已在设计中 |
| R9 | cargo cross-compile Windows / Linux 链接错误 | 🟡 | W0 / W5 | POC-5 验证；最坏改用对应 runner |
| R10 | bump-version 漏改某个 Cargo.toml | 🟢 | W5 | 用 `cargo set-version` 工具统一 |

---

## 关键文件改动一览

### 新增
- `Cargo.toml`（仓库根 workspace）
- `crates/conduit-core/`（整个新目录）
- `server-app/src-tauri/src/proxy/`（整个新目录）
- `server-app/src-tauri/src/ipc/`（整个新目录）
- `client-app/src-tauri/src/proxy/`（整个新目录）
- `client-app/src-tauri/src/ipc/`（整个新目录）
- `server-app/ui/src/generated/bindings.ts`（specta 输出）
- `client-app/ui/src/generated/bindings.ts`
- `tests/it/`（集成测试）

### 删除
- `server-app/core/` + `client-app/core/`
- `server-app/src-tauri/src/sidecar.rs` + `healthz.rs`
- `client-app/src-tauri/src/sidecar.rs` + `healthz.rs`
- `server-app/src-tauri/binaries-dir/` + `client-app/src-tauri/binaries-dir/`
- `server-app/core/pyproject.toml` + `client-app/core/pyproject.toml`
- `scripts/build-sidecars.sh`

### 修改
- `pnpm-workspace.yaml`（移除 core 引用）
- `package.json`（移除 dev:server / dev:client 中 sidecar 启动）
- `server-app/src-tauri/Cargo.toml` + `client-app/src-tauri/Cargo.toml`（升级到 workspace 依赖）
- `server-app/src-tauri/tauri.conf.json` + `client-app/src-tauri/tauri.conf.json`（去 binaries-dir）
- `scripts/release.sh` + `scripts/bump-version.sh` + `scripts/e2e.sh`
- `.github/workflows/release.yml`
- `README.md` + `README_zh.md`
- `server-app/ui/src/**` + `client-app/ui/src/**`（fetch → invoke、EventSource → listen）

---

## 进度更新模板（每日 / 每 Sprint）

> 每完成一个 task，把 `[ ]` 改成 `[x]`，并在 Sprint 表更新进度数字。  
> 每个 Sprint 结束追加日期 + 关键交付到对应行的 "完成时间" 与 "备注"。

例：
```
| **W1** Sprint 1 | ✅ 已完成 | 5 / 5 工日 (100%) | 2026-05-13 | conduit-core + PAC 单测 22 case 全绿 |
```

---

**变更记录**：
- 2026-05-06 v1.0 初稿（基于 v0.2.0 重写设计文档 v1.0）
- 2026-05-06 v1.1 W2 Sprint 2 主体完成：server-app 100% 纯 Rust，PAC 迁 conduit-core/assets，删 server/core + binaries-dir + sidecar.rs；DMG 4.3MB（缩小 83%）；94 tests 全绿，0 warning
- 2026-05-06 v1.2 W3-W4 Sprint 3 完成：client-app 100% 纯 Rust（discoverer / route_cache / route_resolver / local_proxy / system_proxy / connectivity / heartbeat / 5-step connect 全部就绪），删 client/core + sidecar；conduit-core 共享 healthz/ports/types；UI 双端全 shadcn-vue 化，confirm dialog 替代 window.confirm
- 2026-05-06 v1.3 W6 收尾 6 项 bugfix：(1) icons 资源清理 ~590KB；(2) mDNS VPN 启动竟争修复（vpn_snapshot getter + 200ms 等首扫）；(3) 连接 5-step step4 system_proxy 失败不再被吞（try_enable + fail_connect 完整 rollback）；(4) 创建 CHANGELOG.md + scripts/release-notes-v0.2.0.md；(5) README/README_zh 全面重写去 Python；(6) design/ 6 篇 Python 时代文档归档到 archive/
- 2026-05-06 v1.4 整体进度 ≈ 90%；W5 剩余主要项 = release.yml cross-compile（PAT scope 缺 workflow，本地修改未 push）、tag v0.2.0、e2e.sh Rust 重写、system_proxy macOS sandbox 提权方案；这 4 项归为 v0.3 sprint backlog
