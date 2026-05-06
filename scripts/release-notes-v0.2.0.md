# Conduit v0.2.0 — Pure-Rust rewrite, single process, **−91 % bundle size**

> **Same product, leaner runtime.** v0.1.x bundled a Python sidecar via PyInstaller; v0.2.0 ships as a single Rust binary inside the Tauri shell. No Python at install time, no extra process at runtime, ~10× smaller DMG.
> **同款产品，运行时更精简。** v0.1.x 把 Python sidecar 用 PyInstaller 打包进 .app；v0.2.0 整个换成 Rust 单进程，安装时不再带 Python，运行时不再 spawn sidecar，DMG 体积缩小约 10 倍。

**Landing page · 官网**: <https://shetengteng.github.io/conduit/>

---

## What changed in v0.2.0 · v0.2.0 改了什么

### 🦀 Pure Rust runtime · 纯 Rust 运行时
- All proxy logic (HTTP forward proxy, hand-rolled SOCKS5 RFC1928, mDNS advertise/browse, PAC engine, route cache, system-proxy switching, heartbeat) ported from Python to Rust and wired directly into the Tauri main process.
- Removed PyInstaller `--onedir` bundle, removed sidecar supervision logic, removed Python from `tauri.conf.json` `bundle.resources`.
- New shared crate `conduit-core` holds wire-format types, `EventBus<T>`, `bidirectional_relay` + `ProgressSink`, mDNS TXT codec, PAC engine and the PAC asset (`include_str!`).

### 📦 Bundle size · 包体积
| App | v0.1.x DMG | v0.2.0 DMG | Δ |
|---|---|---|---|
| Conduit Server | ~80 MB (PyInstaller onedir) | **~4.3 MB** | **−91 %** |
| Conduit Client | ~80 MB | **~5 MB**¹ | **≈−94 %** |

¹ Client v0.2.0 DMG number measured locally on Apple Silicon dev build; the published DMG may differ slightly after sign + notarize.

### ⚡ Process & startup · 进程与启动
- Task Manager / Activity Monitor: **1 process per app** (was 2 — Tauri main + Python sidecar).
- No `lsof -i :8090` listener (Python sidecar default port is gone).
- Cold start measured under 0.3 s on M1 Pro (was ~1.2 s waiting for sidecar healthz).

### ✨ Behaviour fixes shipped together with the rewrite
- **Server "Active clients" KPI** now reflects active sessions only; passive heartbeat is shown separately in the subtitle (no more "1 connected but actually nobody is using it").
- **Server prunes passive clients after 30 s** of missed heartbeats (Bug: closed client used to stay forever in the "Standby clients" list).
- **mDNS first advertise** now reads the actual VPN state from `ProxyCore::vpn_snapshot()` instead of using `vpn=off` default (Bug: TXT advertised `vpn=off` while interface was `utun5` up).
- **Client "Clear history" dialog** uses shadcn-vue Dialog instead of `window.confirm()` (Bug: native `confirm()` is noop in the Tauri webview, so the button looked dead).
- **Client `forget_all`** now keeps online mDNS / manually-added servers, only purges history (UI label "online servers are not affected" finally matches the implementation).
- **Client connect step 4** no longer swallows `system_proxy enable` failures: the whole connect now fails with a clear error and rolls back partial state (heartbeat / system_proxy / local_proxy endpoint) instead of pretending it's connected.
- **Client heartbeat** now actively `GET /api/clients/heartbeat` so the server can register passive clients and the server UI no longer stays on "waiting for clients".

### 🧪 Tests · 测试
- Workspace `cargo test --workspace`: **conduit-core 59 + conduit-server 43 + conduit-client 41 = 143 passed / 0 failed / 2 ignored**.
- `cargo clippy --workspace --no-deps -- -D warnings` clean.

---

## Installation · 安装

### Pick the right DMG · 按架构下载

| Mac | Server | Client |
|---|---|---|
| Apple Silicon (M1 / M2 / M3 / M4) | `Conduit.Server_0.2.0_aarch64.dmg` | `Conduit.Client_0.2.0_aarch64.dmg` |
| Intel | `Conduit.Server_0.2.0_x64.dmg` | `Conduit.Client_0.2.0_x64.dmg` |

Server goes on the Mac that holds your VPN. Client goes on every device that wants to share that VPN.
持有 VPN 的 Mac 装 Server；其它想共享 VPN 的设备装 Client。

### Install steps · 安装步骤

1. Double-click the DMG → drag the `.app` into `/Applications`
   双击 DMG → 把 `.app` 拖进 `/Applications`
2. Open the app from Launchpad / Spotlight
   从 Launchpad 或 Spotlight 启动应用
3. Allow the two macOS prompts on first launch (**Local Network** + **System Proxy**)
   首次启动允许两个授权弹窗（**本地网络** + **系统代理**）

### ⚠ First-launch Gatekeeper fix · 首次启动 Gatekeeper 拦截

These DMGs are **ad-hoc signed** (no Apple Developer ID yet). On macOS 13+ you'll see one of:

> *"Conduit Server" is damaged and can't be opened.*
> *"Conduit Server" cannot be opened because the developer cannot be verified.*

Run **once** in Terminal after dragging into `/Applications`:
拖进 `/Applications` 后在终端**跑一次**：

```bash
xattr -dr com.apple.quarantine "/Applications/Conduit Server.app"
xattr -dr com.apple.quarantine "/Applications/Conduit Client.app"
```

Or right-click the `.app` → **Open** → **Open** in the warning dialog.
或者右键 `.app` → **打开** → 在警告弹窗里再点 **打开**。

---

## Upgrading from v0.1.x · 从 v0.1.x 升级

If you already had v0.1.x installed:

1. Quit the running Server / Client (menu-bar tray → Quit).
2. Drag the new `.app` from the v0.2.0 DMG into `/Applications`, replacing the old one.
3. (Recommended) After the first v0.2.0 launch, you can safely delete the leftover Python user data:
   ```bash
   rm -rf "~/Library/Application Support/Conduit/cache"   # old Python route cache
   ```
   v0.2.0 stores its route cache in `route-cache.json` (Rust serde), no migration needed for `known-servers.json`.

No config-file format change. Tray menu, autostart, system-proxy hooks all behave the same.

---

## Known limitations · 已知限制

| Item · 项目 | Status · 状态 | Plan · 后续 |
|---|---|---|
| Apple notarization · Apple 公证 | Pending Developer ID · 等开发者账号 | Once obtained, `scripts/release.sh` auto-runs `xcrun notarytool` |
| Windows / Linux client · Win/Linux 客户端 | macOS-only this release · 本版仅 macOS | v0.3 (cross-compile matrix in CI) |
| Auto-update · 自动更新 | Not yet · 暂无 | v0.3 (`tauri-plugin-updater`) |
| Editable Settings page · 可编辑设置页 | Read-only · 只读 | v0.3 |

---

## Verifying downloads · 校验下载

Each DMG can be verified with:
每个 DMG 都可以用以下命令校验：

```bash
shasum -a 256 ~/Downloads/Conduit.Server_0.2.0_aarch64.dmg
```

> SHA-256 sums for this release will be appended here once the GitHub Actions workflow finishes building all four DMGs. (TBD until CI pipeline is green.)

---

## Documentation · 文档

- [README (English)](https://github.com/shetengteng/conduit/blob/main/README.md)
- [README (中文)](https://github.com/shetengteng/conduit/blob/main/README_zh.md)
- [Rust rewrite design doc · Rust 重写设计文档](https://github.com/shetengteng/conduit/blob/main/design/2026-05-06-2-Conduit-Rust-%E9%87%8D%E5%86%99%E8%AE%BE%E8%AE%A1%E6%96%87%E6%A1%A3.md)
- [Feasibility study · 可行性分析](https://github.com/shetengteng/conduit/blob/main/design/2026-05-06-1-%E6%8A%80%E6%9C%AF%E6%A0%88%E7%B2%BE%E7%AE%80%E5%8F%AF%E8%A1%8C%E6%80%A7%E5%88%86%E6%9E%90.md)

---

## Acknowledgements · 致谢

- [Tauri](https://tauri.app/) — Rust + webview shell
- [shadcn-vue](https://www.shadcn-vue.com/) — Tailwind component library used for the entire UI
- [Vue 3](https://vuejs.org/) + [Vite](https://vitejs.dev/) — front-end stack
- [hyper](https://hyper.rs/) / [tokio](https://tokio.rs/) / [mdns-sd](https://github.com/keepsimple1/mdns-sd) — Rust networking libraries that replaced the Python sidecar
- All v0.1.x users who tried the early Python build and helped surface edge cases the rewrite now handles natively

---

**Full changelog · 完整提交历史**: [v0.1.4...v0.2.0](https://github.com/shetengteng/conduit/compare/v0.1.4...v0.2.0)
