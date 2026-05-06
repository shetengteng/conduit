# Changelog

All notable changes to **Conduit** are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Conduit adheres to [Semantic Versioning](https://semver.org/).

Bilingual (English + 中文) summaries are kept inside each section.

---

## [Unreleased]

(rolling section — additions land here before being snapshotted into the next release)

---

## [0.2.0] — 2026-05-06

> **Pure-Rust rewrite.** The Python sidecar is gone; every Conduit Server / Client is now a single Rust binary inside the Tauri shell. DMG size shrinks ~91 %.
> **纯 Rust 重写。** Python sidecar 整体移除，Conduit Server / Client 现在都是 Tauri 内嵌单 Rust 进程。DMG 体积缩小约 91%。

### Added · 新增
- New shared crate `crates/conduit-core/` holding wire-format types, `EventBus<T>`, `bidirectional_relay` + `ProgressSink`, mDNS TXT codec, `PacRules` engine, `PAC_TEMPLATE` asset, `healthz::wait_until_ready`, `pick_unused_ports` — eliminates duplication between server-app and client-app. // 新建共享 crate `conduit-core`，承载 wire 类型 / EventBus / Relay / mDNS TXT codec / PAC 引擎 / healthz 轮询 / 端口选择，消除两端重复代码。
- `server-app`: in-process `ProxyCore` with hand-rolled HTTP forward proxy, hand-rolled SOCKS5 RFC1928, mDNS advertise, traffic meter, session registry (active + passive), VPN detector, control API (REST + SSE on 127.0.0.1:dynamic). // server-app 引入纯 Rust `ProxyCore`：HTTP 转发、SOCKS5、mDNS、流量统计、会话注册表、VPN 检测、127.0.0.1 控制 API。
- `client-app`: in-process `ClientCore` with mDNS browser + history persistence, route cache + resolver, local SOCKS5 listener (fast-socks5), system-proxy switching, 5-step connect state machine, heartbeat with active server-side passive registration. // client-app 引入纯 Rust `ClientCore`：mDNS 浏览 + 历史持久化、路由缓存、本地 SOCKS5、系统代理、5 步连接、心跳主动登记。
- Server passive-client TTL (30 s) auto-eviction so a closed client no longer lingers forever in the "Standby clients" list. // server 端 passive client 30s TTL 自动清理，关闭的 client 不再永远卡在"待命客户端"列表。
- `ProxyCore::vpn_snapshot()` getter so the mDNS advertiser can read the actual VPN state at startup instead of registering with `vpn=off` default. // ProxyCore 暴露 `vpn_snapshot`，mDNS 启动时读真实 VPN 状态，不再用 `vpn=off` 默认值。
- `Cargo workspace` at the repo root + workspace-pinned dependency versions; `Cargo.lock` is now centralized at the root. // 仓库根新增 Cargo workspace 与 workspace.dependencies；`Cargo.lock` 统一放仓库根。
- `design/2026-05-06-1-技术栈精简可行性分析.md` (Python → JS / Rust feasibility study). // 新增 Python → JS / Rust 可行性分析文档。
- `design/2026-05-06-2-Conduit-Rust-重写设计文档.md` (Rust rewrite design doc, includes IPC contract). // 新增 Rust 重写设计文档。
- `design/2026-05-06-3-Conduit-Rust-重写TODO清单.md` (sprint TODO + progress tracker). // 新增 Rust 重写 TODO 清单。
- `scripts/release-notes-v0.2.0.md` release notes. // v0.2.0 release notes。

### Changed · 变更
- Server "Active clients" KPI now displays **active session count only**; passive heartbeat is shown separately in the subtitle (no more "1 connected but actually nobody is using it"). i18n label changes 已链接客户端 → 活跃客户端 / Connected clients → Active clients. // server "已链接客户端" KPI 改为仅统计 active 会话；passive 心跳单独在副标题展示。
- Client `Discoverer::forget_all()` now keeps online mDNS / manually-added servers and only purges entries whose `source = History`, matching the existing UI label "online servers are not affected". // client `forget_all` 改为只清 history，与 UI 文案"在线 server 不受影响"对齐。
- Client "Clear history" / "Forget single server" confirmation dialogs migrated from `window.confirm()` to shadcn-vue `Dialog` (native `confirm()` is noop in Tauri webview, used to make the buttons look dead). // client 清空历史 / 单条移除的确认弹框从 native confirm 改为 shadcn Dialog（Tauri webview 里 confirm 默认 noop）。
- Client connect step 4 (`switch_endpoint`) no longer swallows `system_proxy.enable` failures: the whole connect now fails with a clear error and `fail_connect` rolls back partial state (heartbeat / system_proxy / local_proxy endpoint). // client 5 步连接 step4 不再吞 `system_proxy.enable` 失败；fail_connect 现在会回滚 heartbeat / system_proxy / local_proxy.endpoint。
- Client `Heartbeat::run_loop` now actively `GET /api/clients/heartbeat` on the server after each successful TCP probe, so the server can register passive clients without dedicated channel. // client Heartbeat 每次 probe 通过后会主动给 server 发 `GET /api/clients/heartbeat`。
- Server mDNS advertiser **subscribes to `EventBus` first, sleeps briefly, then reads `vpn_snapshot()` before initial register** — fixes the startup race where TXT advertised `vpn=off` while the interface was up. // server mDNS 启动顺序调整：先 subscribe → 短等 vpn_detect → 读真实状态 → register。
- Centralised `bump-version.sh` to write workspace `Cargo.toml` and both Tauri `tauri.conf.json` files; `pyproject.toml` / Python `_version.py` removed. // bump-version.sh 统一写 workspace `Cargo.toml` + 两端 `tauri.conf.json`，不再涉及 `pyproject.toml`。
- `scripts/release.sh` no longer calls `build-sidecars.sh`. // release.sh 不再调 build-sidecars.sh。
- `package.json`: client description updated to "pure-Rust runtime"; cleaned up sidecar-related dev scripts. // package.json 描述改为 "pure-Rust runtime"，清理 sidecar 相关 dev scripts。

### Removed · 删除
- All Python sidecar code: `server-app/core/`, `client-app/core/`, both `pyproject.toml`, both `_version.py`. // 删除两端 Python sidecar：`*-app/core/`、`pyproject.toml`、`_version.py`。
- Both `src-tauri/src/sidecar.rs` (Tauri-side sidecar supervisor) and both `src-tauri/src/healthz.rs` (replaced by `conduit_core::healthz`). // 删除两端 `sidecar.rs` 与 `healthz.rs`（被 `conduit_core::healthz` 取代）。
- Both `src-tauri/binaries-dir/` (PyInstaller `--onedir` output directories) and `tauri.conf.json` `bundle.resources` references to them. // 删除两端 PyInstaller onedir 与 tauri.conf 引用。
- `scripts/build-sidecars.sh` (PyInstaller bundle build script). // 删除 PyInstaller 打包脚本。
- Mobile / Windows-Store icon assets that are unused on macOS-only release: `icons/Square*Logo*.png`, `icons/StoreLogo.png`, `icons/android/`, `icons/ios/` (saves ~588 KB across both apps). // 删除 macOS 单平台用不到的图标：`Square*Logo*.png`、`StoreLogo.png`、`android/`、`ios/`（双端共省 ~588KB）。
- Tauri `binaries-dir` from `bundle.resources` and `externalBin` (no more sidecar discovery at runtime). // tauri.conf 移除 binaries-dir / externalBin。

### Fixed · 修复
- "Server stays at 'Waiting for clients to connect' even though the client says it is connected" — root cause: client heartbeat only did TCP probes and never called `/api/clients/heartbeat`. // server "等待客户端接入" 不刷新——根因：client heartbeat 只跑 TCP probe，从不调 `/api/clients/heartbeat`。
- "Active clients = 1 but actually nobody is using the proxy" — KPI was double-counting passive heartbeat as `connected`. Now KPI = active session count only. // "已链接 1 但实际无人使用" — KPI 把 passive 算进去；改为只算 active。
- "After closing the client window, the server still shows the standby client forever" — `SessionRegistry` had no TTL eviction; added 30 s TTL with lazy prune in `passive_count()` / `passive_clients()`. // 关闭 client 后 server 永久显示——SessionRegistry 没 TTL；加 30s lazy prune。
- "Clear history button has no effect" — two stacked bugs: (1) `forget_all` cleared everything including online mDNS servers which mDNS daemon's internal cache resurrected within 1 s, (2) `window.confirm()` is a noop in the Tauri webview so the action never actually fired. Both fixed. // "清空历史按钮没反应" — 两层叠加 bug：(1) `forget_all` 把在线 mDNS 也清了导致 1 秒内被 daemon 内部缓存重新塞回来；(2) `window.confirm()` 在 Tauri webview 里 noop。两层都修。
- "Server mDNS TXT advertised `vpn=off` while `utun5` was actually up" — startup race between `mdns::run` (took 默认 false) and `vpn_detect::run` (publishes initial state into ProxyCore). Fixed by subscribing to `EventBus` before register and reading `vpn_snapshot()` after a short delay. // server mDNS TXT 与实际 VPN 不一致 — 启动竞争；通过先 subscribe + 读 snapshot 修复。
- "Client UI says `connected` but `system_proxy enable failed`" — step 4 was swallowing the error. Now fails the whole connect with a clear message and rolls back partial state. // client 报 connected 但 system_proxy 实际失败 — step4 修复，明确 fail + rollback。

### Bundle size · 包体积
| App | v0.1.x DMG | v0.2.0 DMG | Δ |
|---|---|---|---|
| Conduit Server | ~80 MB | **~4.3 MB** | **−91 %** |
| Conduit Client | ~80 MB | **~5 MB** | **≈−94 %** |

### Quality bar · 质量基线
- `cargo test --workspace --no-fail-fast`: **conduit-core 59 + conduit-server 43 + conduit-client 41 = 143 passed / 0 failed / 2 ignored**. // workspace 测试 143 passed / 0 failed。
- `cargo clippy --workspace --no-deps -- -D warnings` clean. // clippy 0 warning。
- `cargo build --workspace --release` ~1m30s on M1 Pro. // release 编译约 1m30s。

---

## [0.1.4] — 2026-05-04

### Fixed · 修复
- Stop button on the Server toolbar now correctly quits the Tauri main process (the sidecar would shut down but the empty Tauri window used to linger). // server 工具栏停止按钮现在能彻底退出 Tauri 主进程。

---

## [0.1.3] — 2026-05-03

### Fixed · 修复
- Stop flow no longer toasts a failure when the sidecar exits cleanly; once stopped, the server stays stopped instead of being restarted by stale supervision. // 停止流程不再误报失败；停掉后保持 stopped。

---

## [0.1.2] — 2026-05-03

### Added · 新增
- Documented sudo + System Settings fallbacks for the unsigned-dmg Gatekeeper warning. // 文档补充未签名 DMG 的 sudo + 系统设置回退路径。

### Changed · 变更
- Centralised every version literal into a single source so `bump-version.sh` truly writes one place. // 把每个版本字面量集中到单一来源。

### Fixed · 修复
- Cleaned up CI build warnings introduced after upgrading `action-gh-release` to v3. // 清理升级 action-gh-release v3 后的 CI 警告。

---

## [0.1.1] — 2026-05-03

### Added · 新增
- Landing page bilingual switch (zh-CN / en-US) and auto-detection of the latest GitHub release version on the download cards. // 官网中英文切换 + 下载卡片自动检测最新 GitHub Release 版本。
- Client dashboard screenshot card on the landing page. // 官网新增 client dashboard 截图卡片。

### Changed · 变更
- Inline CSS/JS on the landing page split into `assets/`; mismatched screenshot aspect ratios fixed. // 官网 inline CSS/JS 拆分到 assets/，截图比例修正。

### Fixed · 修复
- Discoverer prefers LAN address over loopback when the same hostname resolves to multiple IPs. // mDNS 优先选 LAN 地址而非 loopback。

---

## [0.1.0] — 2026-05-02

> **First public release.** Two macOS desktop apps that turn one Mac's VPN into a shared SOCKS5 / HTTP proxy for every device on the same Wi-Fi.
> **首次公开发布。** 两个 macOS 桌面应用，让一台 Mac 的 VPN 成为同 Wi-Fi 下所有设备的代理网关。

### Added · 新增
- **Conduit Server**: HTTP / SOCKS5 proxy, mDNS broadcast, built-in PAC service (7 + 19 + 14 rule sets), control API, DIRECT-first → VPN-fallback smart routing with 1.5 s head-start, menu-bar tray with status colour, login-time autostart. // server：HTTP/SOCKS5 代理、mDNS 广播、内置 PAC、控制 API、DIRECT-first 智能路由、菜单栏托盘、开机自启。
- **Conduit Client**: mDNS auto-discovery + known-server history, 5-step connect wizard, 1 Hz traffic chart over SSE, per-host route cache (5 min TTL), 5-step diagnostics, macOS system-proxy switching with graceful fallback. // client：mDNS 自动发现 + 历史、5 步连接向导、流量曲线、路由缓存、5 步诊断、系统代理切换。
- Tech stack at v0.1.0: Tauri 2 + Rust shell, Vue 3 + shadcn-vue + Tailwind v4 UI, **Python 3 (aiohttp / asyncio / zeroconf) sidecar packed via PyInstaller `--onedir`**. // 技术栈：Tauri 2 + Rust 外壳、Vue 3 + shadcn-vue + Tailwind v4 UI、Python 3 aiohttp/asyncio/zeroconf sidecar 用 PyInstaller `--onedir` 打包。

### Quality bar · 质量基线
- 210 pytest cases passing (server 82 + client 128). // pytest 210 通过。
- `scripts/e2e.sh` end-to-end smoke 11 s — mDNS → connect → SOCKS5 traffic → diagnose → disconnect. // 端到端 smoke 11 秒走完。
- Verified working on a second Mac (different machine, same Wi-Fi). // 第二台 Mac 验证通过。

---

[Unreleased]: https://github.com/shetengteng/conduit/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/shetengteng/conduit/compare/v0.1.4...v0.2.0
[0.1.4]: https://github.com/shetengteng/conduit/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/shetengteng/conduit/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/shetengteng/conduit/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/shetengteng/conduit/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/shetengteng/conduit/releases/tag/v0.1.0
