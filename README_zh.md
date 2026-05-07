# Conduit

> 零配置局域网 VPN 共享 —— 让一台拥有 VPN 的 Mac 成为同一 Wi-Fi 下其它所有设备的网关。

**官网：** <https://shetengteng.github.io/conduit/> · **语言：** [English](./README.md) · **中文**

Conduit 由两个独立的 macOS 桌面应用组成：

| 应用 | 角色 | 核心能力 |
|---|---|---|
| **Conduit Server** | 运行在持有 VPN 的那台 Mac 上 | HTTP / SOCKS5 代理 + mDNS 广播 + PAC 服务 + 控制 API |
| **Conduit Client** | 运行在每一台希望共享 VPN 的设备上 | mDNS 自动发现、5 步连接向导、智能 per-host 路由缓存、1 Hz 流量曲线、macOS 系统代理切换、系统托盘、5 步诊断、登录自启 |

技术栈：**Tauri 2 + Rust** —— 进程内单运行时 · **Vue 3 + shadcn-vue + Tailwind v4** UI。进程间通信使用 localhost HTTP + Server-Sent Events。_v0.2.0 已彻底移除原 Python sidecar，全部业务逻辑跑在 Tauri 主进程内。_

UI 设计风格：Stripe / Vercel 企业级白底（方案 B），全部图标使用 [RemixIcon](https://remixicon.com/)。

---

## 1. 环境要求

| 软件 | 版本 | 备注 |
|---|---|---|
| macOS | 13+ | Apple Silicon 与 Intel 均可。（Windows / Linux 客户端在 v0.3 计划中。）|
| Node | ≥ 20.10 | `node -v` |
| pnpm | ≥ 9 | `corepack enable pnpm` 或 `npm i -g pnpm@9` |
| Rust 工具链 | ≥ 1.84 | `rustup default stable` |
| 系统授权 | 本地网络 + 系统代理 | 首次启动会弹两次授权框，**全部允许** |

> v0.2.0 起不再需要 Python 或 PyInstaller。

---

## 2. 安装 —— 推荐方式（DMG）

> 只是想**用** Conduit、不打算改代码？直接到 [GitHub Releases](https://github.com/shetengteng/conduit/releases) 下载预编译 DMG。源代码安装见 [§ 3](#3-安装--源代码方式备选)。

### 2.1 按架构选 DMG

| Mac | Server | Client |
|---|---|---|
| Apple Silicon (M1 / M2 / M3 / M4) | `Conduit.Server_<ver>_aarch64.dmg` | `Conduit.Client_<ver>_aarch64.dmg` |
| Intel | `Conduit.Server_<ver>_x64.dmg` | `Conduit.Client_<ver>_x64.dmg` |

体积参考：**Server ≈ 4–5 MB**、**Client ≈ 5–6 MB**（比 v0.1.x Python 版小约 91%）。

### 2.2 标准 DMG 安装

1. 双击 DMG → 把 `.app` 拖进 `/Applications`。
2. Launchpad / Spotlight 搜 **Conduit Server**（或 **Conduit Client**）启动。
3. 第一次运行 macOS 会弹两次授权框（本地网络 + 系统代理），都允许。

### 2.3 ⚠ 未签名 + 暂未公证 —— Gatekeeper 补救命令

DMG 是 **ad-hoc 签名 + 尚未 Apple 公证**（等 Apple Developer ID 到位）。macOS 13+ 第一次打开会被拦截：

> *「Conduit Server」已损坏，无法打开。*
> *无法打开「Conduit Server」，因为无法验证开发者。*

下面三种**任选一种**即可：

#### 方案 A —— 右键打开  *（最简单，单次）*

1. 在 `/Applications` 里找到 `.app`。
2. **右键** → **打开** → 在弹框里再点一次 **打开**。
3. 系统会记住这个豁免，以后双击正常。

#### 方案 B —— 移除 quarantine 属性  *（推荐，复制 / 重新下载后用这条）*

```bash
sudo xattr -dr com.apple.quarantine "/Applications/Conduit Server.app"
sudo xattr -dr com.apple.quarantine "/Applications/Conduit Client.app"
```

注意 `sudo`，会要求输入开机密码。

> macOS 15+ 上偶发 SIP 仍拦截 `xattr`。若仍报错，直接走方案 C。

#### 方案 C —— 系统设置里手动放行  *（macOS 13+）*

第一次打开失败后：

1. **系统设置 → 隐私与安全 → 安全性**。
2. 滑到底部找到 *「`Conduit Server` 因不是来自经过认证的开发者而被阻止使用」*。
3. 点 **仍要打开** → 输入开机密码。

> 项目拿到 Apple Developer ID 之后，`./scripts/release.sh` 会自动调 `xcrun notarytool submit`，上面这些补救都将不再需要。

---

## 3. 安装 —— 源代码方式（备选）

> 只在你打算改代码、跑测试，或者所在平台暂无预编译 DMG 时选这个。

```bash
git clone <repo> conduit && cd conduit
pnpm install            # 6 个 workspace，约 30 秒
```

> 第一次运行时 Tauri 会下载 Rust 依赖并触发 macOS 网络授权弹框，请耐心等待并允许。

---

## 4. 开发模式启动

> 推荐：开两个独立终端，日志互不干扰，方便排查。

### 4.1 启动 Server（共享端）

```bash
cd /path/to/conduit
pnpm dev:server
```

会拉起单个 Tauri 进程（内嵌 `ProxyCore`：HTTP + SOCKS5 + mDNS + 控制 API）。

启动成功的标志：
- 终端打印 `[mdns] advertised <hostname>._conduit._tcp.local. @ <ip>:<port> (vpn=on|off)`
- 顶部状态徽章变绿，三个端口胶囊已填充（HTTP / SOCKS5 / 控制 API）。
- macOS 菜单栏出现 Conduit Server 托盘图标。

### 4.2 启动 Client（接入端）

```bash
cd /path/to/conduit
pnpm dev:client
```

会拉起 Conduit Client Tauri 窗口（内嵌 `ClientCore`：mDNS 浏览 + 本地 SOCKS5 + 路由缓存 + 心跳 + 系统代理控制）。

启动成功的标志：
- 终端打印 `[control_api] listening on http://127.0.0.1:NNNN` 和 `[discoverer] server_discovered: …`
- 「发现」页面在 5–10 秒内出现 Server 卡片。
- macOS 菜单栏出现 Conduit Client 托盘图标。

> **首次运行 macOS 会弹框：「Conduit 想要查找本地网络上的设备」** —— 必须允许，否则收不到 mDNS 广播。

### 4.3 一键同时启动（可选）

```bash
pnpm dev:all
```

不太推荐 —— 两个 Tauri 窗口 + 两个日志混在一起，排查问题会比较吵。

---

## 5. 首次连接（90 秒走通）

1. Server 与 Client 都已启动。
2. 打开 Client 窗口，「发现」页应有一张绿色 Server 卡片。
3. 点击卡片上的「连接」。
4. 5 步进度条走完（约 2-3 秒）。
5. macOS 弹出「Conduit 想要修改系统代理」 → 输入密码允许。
   - 如果拒绝（或在没有 admin 权限的 macOS 13+ 上），第 4 步会**明确报错**；可以在 Client 配置里把 `enable_system_proxy=false` 关掉，然后浏览器手动配 SOCKS5。
6. 打开 google.com —— 已通过共享 VPN 出网。

---

## 6. 自己打 DMG

```bash
./scripts/release.sh                 # pnpm tauri build + 收集到 dist/（冷构建约 3 分钟）
```

产物：

```
dist/server/Conduit Server.app
dist/server/Conduit Server_<ver>_aarch64.dmg     ~4-5 MB
dist/client/Conduit Client.app
dist/client/Conduit Client_<ver>_aarch64.dmg     ~5-6 MB
```

> 同样存在 [§ 2.3](#23--未签名--暂未公证--gatekeeper-补救命令) 的 Gatekeeper 问题。配好 Apple Developer ID 之后这些都可以省掉。

> `scripts/e2e.sh` 当前已标 **DEPRECATED**（基于已删除的 Python sidecar）。Rust 版端到端脚本在 v0.3 backlog。当前手动验收：两个终端各跑 `pnpm tauri dev`，照 `design/2026-05-06-3-Conduit-Rust-重写TODO清单.md` 的 manual checklist 走一遍即可。

---

## 7. 常用命令速查

| 命令 | 作用 |
|---|---|
| `pnpm dev:server` / `pnpm dev:client` | 开发模式启动应用（HMR 热更新） |
| `pnpm build:server` / `pnpm build:client` | 构建 .app + .dmg |
| `pnpm dev:server-ui` / `pnpm dev:client-ui` | 仅启动 Vite 前端（调 UI 时用） |
| `cargo test --workspace` | 跑所有 Rust 测试（143 通过） |
| `cargo clippy --workspace --no-deps -- -D warnings` | Lint 检查（0 warning） |
| `cargo build --workspace --release` | Release 完整构建（M1 Pro 上约 1m30s） |

---

## 8. 仓库布局

```
conduit/
├── Cargo.toml               # Cargo workspace 根
├── crates/
│   └── conduit-core/        # 共享 crate：types / EventBus / Relay / mDNS codec / PAC engine
├── server-app/              # Conduit Server 桌面应用
│   ├── src-tauri/           # Tauri 主进程 + 托盘 + 内嵌 ProxyCore
│   └── ui/                  # Vue 3 + shadcn-vue
├── client-app/              # Conduit Client 桌面应用
│   ├── src-tauri/           # Tauri 主进程 + 托盘 + 自启 + 内嵌 ClientCore
│   └── ui/                  # Vue 3 + shadcn-vue
├── scripts/                 # release.sh / bump-version.sh / publish-release-notes.sh / e2e.sh (已 DEPRECATED)
├── design/                  # 按日期前缀归档的设计文档（v0.2.0 把 Python 时代文档移到 design/archive/）
│   ├── 2026-05-06-1-技术栈精简可行性分析.md
│   ├── 2026-05-06-2-Conduit-Rust-重写设计文档.md
│   └── 2026-05-06-3-Conduit-Rust-重写TODO清单.md
├── CHANGELOG.md             # Keep-a-Changelog 格式
└── package.json             # pnpm workspace 根
```

---

## 9. 故障排查（精简）

| 现象 | 处理 |
|---|---|
| 第一次打开提示「`Conduit Server` 已损坏，无法打开」 | 未签名构建被 Gatekeeper 拦截。跑 `sudo xattr -dr com.apple.quarantine "/Applications/Conduit Server.app"`（Client 同理）。仍报 `Operation not permitted` 走 [§ 2.3 方案 C](#23--未签名--暂未公证--gatekeeper-补救命令)（系统设置里手动放行）。|
| Client「发现」页一直空 | 系统设置 → 隐私与安全 → 本地网络 → 启用 client-app。 |
| 连接卡在第 1 步 | Server 端口被防火墙拦截：`nc -zv <host> <port>` 验证。 |
| 连接第 4 步弹出「Conduit 想要修改系统代理」密码框 | 这是 v0.2.0 起的预期行为：networksetup 设置 SOCKS 系统代理需要管理员权限，输入一次密码即可，macOS keychain 5 分钟内不会再问。取消密码框 → 第 4 步明确报错并自动 rollback partial state（不会留下半连接）。完全不想要弹框：把 client 配置 `enable_system_proxy=false`，浏览器手动配 SOCKS5。 |
| 连接第 4 步失败 `system_proxy enable failed: ... exit status: 14` | 旧版无 admin fallback 时的现象。v0.2.0 已自动 fallback 到 `osascript with administrator privileges`，如仍出现说明 osascript 也失败 —— 通常是用户取消了密码框（错误信息里会显式说 `cancelled by user`）。 |
| Server「活跃客户端」一直 0，但 Client 已连接 | Client 仅发心跳（passive 注册）；KPI 只统计 active SOCKS5/HTTP 会话。浏览器开 google.com 让流量真走起来，数字就会变成 1。 |
| 「待命客户端」列表里关掉的 client 不消失 | v0.2.0 起 30s passive TTL 自动清理；如果仍然不消失，去 server 日志看心跳是否真的断了。 |
| 想推倒重来 | `rm -rf ~/Library/Application\ Support/Conduit/` |

---

## 10. 当前进度

- ✅ v0.2.0 —— 纯 Rust 重写（Python sidecar 移除、单进程、DMG 缩小约 91%）
- ✅ v0.1.x 范围全部保留（M-α / M-β / M-γ / M-δ）
- ✅ `cargo test --workspace`：180 通过（conduit-core 79 含 socks5_proto 下沉 + conduit-server 54 + conduit-client 47） / 0 失败 / 2 忽略
- ✅ `cargo clippy --workspace --no-deps -- -D warnings` 干净
- ⏳ v0.3 backlog：Windows / Linux Client（cross-compile 矩阵）、`tauri-plugin-updater` 自动更新、可编辑设置、Apple 公证

完整版本历史见 [CHANGELOG](./CHANGELOG.md)，重写细节见 [v0.2.0 release notes](./scripts/release-notes-v0.2.0.md)。

---

## 11. 协议

私人项目，暂未公开分发。
