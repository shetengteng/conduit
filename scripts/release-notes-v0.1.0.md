# Conduit v0.1.0 — First public release

> **Zero-config LAN VPN sharing.** Two macOS desktop apps that turn one Mac's VPN into a shared SOCKS5 / HTTP proxy for every device on the same Wi-Fi.
> **零配置局域网 VPN 共享。** 两个 macOS 桌面应用，让一台 Mac 的 VPN 自动成为同一 Wi-Fi 下所有设备的代理网关。

**Landing page** · **官网**: <https://shetengteng.github.io/conduit/>

---

## Highlights · 亮点

### Conduit Server
- HTTP / SOCKS5 proxy on the VPN-holding Mac (default ports 15723 / 22142)
- mDNS broadcast so clients auto-discover within 5–10 s on the same Wi-Fi
- Built-in PAC service (7 + 19 + 14 rule sets) and control API
- DIRECT-first → VPN-fallback smart routing with 1.5 s head-start
- Menu-bar tray with status colour, login-time autostart

### Conduit Client
- mDNS auto-discovery + known-server history
- 5-step connect wizard (port → probe → system proxy → heartbeat → ready)
- 1 Hz traffic chart over Server-Sent Events
- Per-host route cache with 5-minute TTL
- 5-step diagnostics (port / mDNS / heartbeat / system proxy / egress)
- macOS system-proxy switching with graceful fallback (amber banner if `networksetup` lacks admin)

### Tech stack · 技术栈
- **Tauri 2 + Rust** — main process, window, tray, autostart, sidecar supervision
- **Vue 3 + shadcn-vue + Tailwind v4** — Stripe / Vercel-style enterprise white UI
- **Python 3 (aiohttp / asyncio / zeroconf)** — sidecar (HTTP, SOCKS5, mDNS, route cache)
- **PyInstaller `--onedir`** — bundles Python runtime; no system Python required at install time

### Quality bar · 质量基线
- ✅ 210 pytest cases passing (server 82 + client 128)
- ✅ End-to-end smoke (`scripts/e2e.sh`) finishes in 11 s — mDNS → connect → SOCKS5 traffic → diagnose → disconnect
- ✅ Verified working on a second Mac (different machine, same Wi-Fi)

---

## Installation · 安装

### Pick the right DMG · 按架构下载

| Mac | Server | Client |
|---|---|---|
| Apple Silicon (M1 / M2 / M3 / M4) | `Conduit.Server_0.1.0_aarch64.dmg` | `Conduit.Client_0.1.0_aarch64.dmg` |
| Intel | `Conduit.Server_0.1.0_x64.dmg` | `Conduit.Client_0.1.0_x64.dmg` |

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

Re-run after any re-download / replace.
每次重新下载或替换 .app 都需要再跑一次。

---

## First connection · 首次连接（90 秒）

1. Open **Conduit Server** on the VPN-holding Mac → wait for the green status badge
   在持有 VPN 的 Mac 打开 **Conduit Server**，等顶部徽章变绿
2. Open **Conduit Client** on any other device → the Discovery page shows the server in 5–10 s
   在其它设备上打开 **Conduit Client**，发现页 5–10 秒内出现 Server 卡片
3. Click **Connect** → walk through the 5-step wizard (~2-3 s)
   点 **Connect**，等 5 步向导走完（约 2-3 秒）
4. Allow the system-proxy prompt (or skip it — the amber banner will tell you the SOCKS5 host/port to set manually)
   允许系统代理弹窗（如果拒绝，琥珀色横幅会告诉你手动配 SOCKS5 的主机+端口）
5. Open google.com — works through the shared VPN
   打开 google.com，已通过共享 VPN 出网

---

## Known limitations · 已知限制

| Item · 项目 | Status · 状态 | Plan · 后续 |
|---|---|---|
| Apple notarization · Apple 公证 | Pending Developer ID · 等开发者账号 | Once obtained, `scripts/release.sh` auto-runs `xcrun notarytool` · 账号到位后自动启用 |
| client-app on Windows / Linux · Client 端 Win / Linux 版 | macOS-only · 仅 macOS | v0.2 |
| Auto-update · 自动更新 | Not yet · 暂无 | v0.2 (`tauri-plugin-updater`) |
| Editable Settings page · 可编辑设置页 | Read-only · 只读 | v0.2 |

For the full list see the [packaging guide](https://github.com/shetengteng/conduit/blob/main/design/2026-05-03-1-Conduit-%E6%89%93%E5%8C%85%E4%B8%8E%E5%8F%91%E5%B8%83%E8%AF%B4%E6%98%8E.md).
完整清单见 [打包发布说明](https://github.com/shetengteng/conduit/blob/main/design/2026-05-03-1-Conduit-%E6%89%93%E5%8C%85%E4%B8%8E%E5%8F%91%E5%B8%83%E8%AF%B4%E6%98%8E.md)。

---

## Verifying downloads · 校验下载

Each DMG can be verified with:
每个 DMG 都可以用以下命令校验：

```bash
shasum -a 256 ~/Downloads/Conduit.Server_0.1.0_aarch64.dmg
```

Expected SHA-256 (this release · 本次发版):

| File · 文件 | SHA-256 |
|---|---|
| `Conduit.Server_0.1.0_aarch64.dmg` | `3baa6d375a7f11abf126fad6813e0d86dd5c36a474e16ce1c1a12dfbc0a5306e` |
| `Conduit.Server_0.1.0_x64.dmg`     | `41f66449d3bcfa8fc048a2af58194cfee14794c3e7bef026a7c4f02feaab6fc0` |
| `Conduit.Client_0.1.0_aarch64.dmg` | `4c04c99dd08d57f8a746700b35571450797aba157bba40a8f45c77406f58b001` |
| `Conduit.Client_0.1.0_x64.dmg`     | `f6a31989ece58c91f956ea83a06e33f038b7d37d15edc76faf8a352826290620` |

---

## Documentation · 文档

- [README (English)](https://github.com/shetengteng/conduit/blob/main/README.md)
- [README (中文)](https://github.com/shetengteng/conduit/blob/main/README_zh.md)
- [Packaging & release guide · 打包发布说明](https://github.com/shetengteng/conduit/blob/main/design/2026-05-03-1-Conduit-%E6%89%93%E5%8C%85%E4%B8%8E%E5%8F%91%E5%B8%83%E8%AF%B4%E6%98%8E.md)
- [Acceptance guide · 验收指南](https://github.com/shetengteng/conduit/blob/main/design/2026-05-02-1-Conduit-%E9%AA%8C%E6%94%B6%E6%8C%87%E5%8D%97.md)

---

## Acknowledgements · 致谢

- [Tauri](https://tauri.app/) — Rust + webview shell that makes the 30 MB DMG possible
- [shadcn-vue](https://www.shadcn-vue.com/) — minimal Tailwind component library used for the entire UI
- [Vue 3](https://vuejs.org/) + [Vite](https://vitejs.dev/) — front-end stack
- [aiohttp](https://docs.aiohttp.org/) + [zeroconf](https://github.com/jstasiak/python-zeroconf) — Python sidecar networking

---

**Full changelog · 完整提交历史**: [main commits](https://github.com/shetengteng/conduit/commits/main)
