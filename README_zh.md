# Conduit

> 零配置局域网 VPN 共享 —— 让一台拥有 VPN 的 Mac 成为同一 Wi-Fi 下其它所有设备的网关。

**官网：** <https://shetengteng.github.io/conduit/> · **语言：** [English](./README.md) · **中文**

Conduit 由两个独立的 macOS 桌面应用组成：

| 应用 | 角色 | 核心能力 |
|---|---|---|
| **Conduit Server** | 运行在持有 VPN 的那台 Mac 上 | HTTP / SOCKS5 代理 + mDNS 广播 + PAC 服务 + 控制 API |
| **Conduit Client** | 运行在每一台希望共享 VPN 的设备上 | mDNS 自动发现、5 步连接向导、智能 per-host 路由缓存、1 Hz 流量曲线、macOS 系统代理切换、系统托盘、5 步诊断、登录自启 |

技术栈：**Tauri 2 + Rust** 主进程 · **Vue 3 + shadcn-vue + Tailwind v4** UI · **Python 3 (aiohttp / asyncio / zeroconf)** Sidecar。进程间通信使用 localhost HTTP + Server-Sent Events。

UI 设计风格：Stripe / Vercel 企业级白底（方案 B），全部图标使用 [RemixIcon](https://remixicon.com/)。

---

## 1. 环境要求

| 软件 | 版本 | 备注 |
|---|---|---|
| macOS | 13+ | Apple Silicon 与 Intel 均可。Linux/Windows 仅支持 Server，暂无 Client。|
| Node | ≥ 20.10 | `node -v` |
| pnpm | ≥ 9 | `corepack enable pnpm` 或 `npm i -g pnpm@9` |
| Python | ≥ 3.10 | `python3 --version` |
| Rust 工具链 | ≥ 1.78 | `rustup default stable` |
| 系统授权 | 本地网络 + 系统代理 | 首次启动会弹两次授权框，**全部允许** |

Sidecar 的 Python 依赖会在第一次 `pnpm dev:*` 时自动安装。也可以提前手动装：

```bash
pip3 install aiohttp zeroconf
```

---

## 2. 安装 —— 推荐方式（DMG）

> 只是想**用** Conduit、不打算改代码？直接装预编译 DMG。源代码安装见 [§ 3](#3-安装--源代码方式备选)。

### 2.1 预编译产物

跑完 `./scripts/release.sh`（或下载发布包）后产物如下：

```
dist/server/Conduit Server.app
dist/server/Conduit Server_0.1.0_aarch64.dmg     ~30 MB
dist/client/Conduit Client.app
dist/client/Conduit Client_0.1.0_aarch64.dmg     ~30 MB
```

> ⚙ 架构：`aarch64` = Apple Silicon (M1/M2/M3/M4)。Intel Mac 需要在 Intel 主机上从源码构建。

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
xattr -dr com.apple.quarantine "/Applications/Conduit Server.app"
xattr -dr com.apple.quarantine "/Applications/Conduit Client.app"
```

执行完后双击就能正常打开。如果你重新下载或替换了 .app，再跑一次就好。

#### 方案 C —— 系统设置里手动放行  *（macOS 13+）*

第一次打开失败后：

1. **系统设置 → 隐私与安全 → 安全性**。
2. 滑到底部找到 *「`Conduit Server` 因不是来自经过认证的开发者而被阻止使用」*。
3. 点 **仍要打开** → 输入开机密码。

#### 校验

```bash
codesign -dvv "/Applications/Conduit Server.app" 2>&1 | rg "Signature|TeamIdentifier"
# 期望: Signature=adhoc · TeamIdentifier=not set   (← Gatekeeper 拦截就是因为这个)
```

> 项目拿到 Apple Developer ID 之后，`./scripts/release.sh` 会自动调 `xcrun notarytool submit`（[`scripts/release.sh`](./scripts/release.sh) 第 4 步已经留了 env 钩子），上面这些补救都将不再需要。完整公证流程见 [打包与发布说明](./design/2026-05-03-1-Conduit-打包与发布说明.md)。

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

终端 1：

```bash
cd /path/to/conduit
pnpm dev:server
```

会同时拉起：
- **Tauri 主窗口**：Vue 控制台（仪表盘 / 日志 / 设置）
- **Python Sidecar** (`server-app/core/proxy_server.py`)：HTTP / SOCKS5 / 控制 API + mDNS 广播

启动成功的标志：
- 终端打印 `mDNS advertised: Conduit on _conduit._tcp.local.`
- 顶部状态徽章变绿，三个端口胶囊已填充
- macOS 菜单栏出现 Conduit Server 托盘图标

可选 CLI 参数（透传给 Sidecar）：

```bash
python3 server-app/core/proxy_server.py --mdns-name "Workstation"
python3 server-app/core/proxy_server.py --http-port 18080
CONDUIT_NO_MDNS=1 pnpm dev:server      # 关闭 mDNS 用于隔离调试
```

> v0.1 的「设置」页面**只读**，需要修改请用上面的命令行参数覆盖。

### 4.2 启动 Client（接入端）

终端 2：

```bash
cd /path/to/conduit
pnpm dev:client
```

会同时拉起：
- **Tauri 主窗口**：4 个页面（发现 / 已连接 / 诊断 / 设置）
- **Python Sidecar** (`client-app/core/client_main.py`)：本地 SOCKS5 + 控制 API + mDNS 监听 + 智能路由解析

启动成功的标志：
- 终端打印 `control API listening on 127.0.0.1:NNNN`
- 「发现」页面在 5–10 秒内出现 Server 卡片
- macOS 菜单栏出现 Conduit Client 托盘图标

> **首次运行 macOS 会弹框：「Conduit 想要查找本地网络上的设备」** —— 必须允许，否则收不到 mDNS 广播。

可选参数：

```bash
CONDUIT_NO_SYSTEM_PROXY=1 pnpm dev:client    # 跳过自动切换系统代理
```

### 4.3 一键同时启动（可选）

```bash
pnpm dev:all
```

不太推荐 —— 两个 Tauri 窗口 + 两个 Sidecar 日志混在一起，排查问题会比较吵。

---

## 5. 首次连接（90 秒走通）

1. Server 与 Client 都已启动。
2. 打开 Client 窗口，「发现」页应有一张绿色 Server 卡片。
3. 点击卡片上的「连接」。
4. 5 步进度条走完（约 2-3 秒）。
5. macOS 弹出「Conduit 想要修改系统代理」 → 输入密码允许。
   - 如果拒绝（或在没有 admin 权限的 macOS 13+ 上），Client 窗口顶部会出现**琥珀色提示横幅**，告诉你需要在浏览器/应用里手动配置 SOCKS5。
6. 打开 google.com —— 连得通。

---

## 6. 自己打 DMG

`scripts/` 下提供了 3 个生产级脚本：

```bash
./scripts/build-sidecars.sh          # 步骤 1：用 PyInstaller 打包 Python Sidecar（约 50 秒）
./scripts/release.sh                 # 步骤 2：pnpm tauri build + 收集到 dist/（冷构建约 3 分钟）
./scripts/e2e.sh                     # 冒烟测试：11 秒端到端
```

产物（与 § 2 提供的完全一致）：

```
dist/server/Conduit Server.app
dist/server/Conduit Server_0.1.0_aarch64.dmg     ~30 MB
dist/client/Conduit Client.app
dist/client/Conduit Client_0.1.0_aarch64.dmg     ~30 MB
```

> 同样存在 [§ 2.3](#23--未签名--暂未公证--gatekeeper-补救命令) 的 Gatekeeper 问题。脚本结尾会提示对方需要右键 → 打开 *或* 跑 `xattr -dr com.apple.quarantine`。配好 Apple Developer ID 之后这些都可以省掉，完整公证流程见 [打包与发布说明](./design/2026-05-03-1-Conduit-打包与发布说明.md)。

---

## 7. 常用命令速查

| 命令 | 作用 |
|---|---|
| `pnpm dev:server` / `pnpm dev:client` | 开发模式启动应用（HMR 热更新） |
| `pnpm build:server` / `pnpm build:client` | 构建 .app + .dmg |
| `pnpm dev:server-ui` / `pnpm dev:client-ui` | 仅启动 Vite 前端（调 UI 时用） |
| `cd server-app/core && python3 -m pytest -q` | Server pytest 套件（82 用例） |
| `cd client-app/core && python3 -m pytest -q` | Client pytest 套件（128 用例） |
| `cd {server,client}-app/src-tauri && cargo check` | Rust 类型检查 |
| `./scripts/e2e.sh` | 端到端冒烟测试（11 秒） |

---

## 8. 仓库布局

```
conduit/
├── server-app/              # Conduit Server 桌面应用
│   ├── core/                # Python Sidecar：aiohttp + zeroconf
│   ├── src-tauri/           # Rust 主进程 + 托盘
│   └── ui/                  # Vue 3 + shadcn-vue
├── client-app/              # Conduit Client 桌面应用
│   ├── core/                # Python Sidecar：discoverer + connector + cache + meter + diagnose
│   ├── src-tauri/           # Rust 主进程 + 托盘 + 自启（LaunchAgent）
│   └── ui/                  # Vue 3 + shadcn-vue
├── scripts/                 # build-sidecars.sh / release.sh / e2e.sh
├── design/                  # 按日期前缀归档的设计文档
│   ├── 2026-04-30-5-Conduit-开发TODO清单-进度S6Md-95.md   # 完整 TODO + 进度
│   ├── 2026-05-02-1-Conduit-验收指南.md                   # 用户验收手册
│   └── 2026-05-03-1-Conduit-打包与发布说明.md             # 打包与发布说明
└── package.json             # workspace 根
```

---

## 9. 故障排查（精简）

| 现象 | 处理 |
|---|---|
| 第一次打开提示「`Conduit Server` 已损坏，无法打开」 | 未签名构建被 Gatekeeper 拦截。跑 `xattr -dr com.apple.quarantine "/Applications/Conduit Server.app"`（Client 同理）。完整方案见 [§ 2.3](#23--未签名--暂未公证--gatekeeper-补救命令)。|
| Client「发现」页一直空 | 系统设置 → 隐私与安全 → 本地网络 → 启用 client-app。详见 [验收指南 Q1](./design/2026-05-02-1-Conduit-验收指南.md)。|
| 连接卡在第 1 步 | Server 端口被防火墙拦截：`nc -zv <host> <port>` 验证。|
| 出现「未自动切换系统代理」琥珀横幅 | macOS 13+ 切换系统代理需要 admin 权限。要么浏览器手动配 SOCKS5，要么 sudo 启动 Conduit（不推荐）。|
| Server「停止代理」点了没反应 | 已在 M-δ 修复（加 50ms 延时让 HTTP 200 先返回）。|
| 想推倒重来 | `pkill -f proxy_server.py; pkill -f client_main.py; rm ~/Library/Application\ Support/Conduit/known-servers.json` |

完整 FAQ 见 [验收指南 § 4](./design/2026-05-02-1-Conduit-验收指南.md#4-故障排查) 与 [打包发布说明 § 6](./design/2026-05-03-1-Conduit-打包与发布说明.md)。

---

## 10. 当前进度

- ✅ M-α  骨架、端口分配、健康检查
- ✅ M-β.1  mDNS 发现 + 历史 Server 列表
- ✅ M-β.2  5 步连接 + 系统代理切换 + 心跳
- ✅ M-γ  流量曲线 + 路由缓存 + 1 Hz SSE
- ✅ M-δ  诊断页 + 托盘 + macOS 自启 + 已停止后再启动
- ✅ 测试：210 条 pytest 用例全绿（server 82 + client 128）
- ✅ 端到端冒烟（`scripts/e2e.sh`）：mDNS → 连接 → SOCKS5 流量 → 诊断 → 断开，11 秒走完
- ⏳ 可编辑「设置」+ 持久化
- ⏳ Apple 公证 + 自动更新 + 多 Server 并行连接

整体覆盖 v0.1.0 范围 ~98%。详见 [TODO 清单](./design/2026-04-30-5-Conduit-开发TODO清单-进度S6Md-95.md)。

---

## 11. 协议

私人项目，暂未公开分发。
