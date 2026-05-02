# Conduit

> 局域网内零配置共享上网代理 —— 让一台已连 VPN 的 Mac 把网络分享给同网段其他设备。

Conduit 由两个独立桌面应用组成，分别打包成 macOS `.app`：

| 应用 | 角色 | 关键能力 |
|---|---|---|
| **Conduit Server** | 跑在已连 VPN 的"出口机"上 | HTTP / SOCKS5 代理 + mDNS 广播 + PAC 服务 + 控制 API |
| **Conduit Client** | 跑在每台需要使用代理的内网机上 | mDNS 自动发现 server + 5 步连接进度 + 路由决策缓存 + 1Hz 流量统计 + macOS 系统代理自动切换 + 系统托盘 + 5 项自检 + 开机自启 |

技术栈：**Tauri 2 + Rust** 主进程 + **Vue 3 + shadcn-vue + Tailwind v4** UI + **Python 3 (aiohttp / asyncio / zeroconf)** Sidecar，通过 localhost HTTP + SSE 通信。

UI 风格：Stripe / Vercel 净白企业级 (方案 B)，全图标使用 [RemixIcon](https://remixicon.com/)。

---

## 1. 环境要求

| 软件 | 版本 | 备注 |
|---|---|---|
| macOS | 13+ | M1/M2/Intel 通吃；Linux/Windows 当前不支持 |
| Node | ≥ 20.10 | `node -v` 验证 |
| pnpm | ≥ 9 | `corepack enable pnpm` 或 `npm i -g pnpm@9` |
| Python | ≥ 3.10 | `python3 --version` |
| Rust toolchain | ≥ 1.78 | `rustup default stable` |
| 系统权限 | 本地网络 + 系统代理 | 首次启动 macOS 会两次弹窗，**全部允许** |

Python 依赖由两个 sidecar 自带 `requirements.txt`，第一次跑 `pnpm dev:*` 时会自动 `pip install`。也可手动：

```bash
pip3 install -r server-app/core/requirements.txt
pip3 install -r client-app/core/requirements.txt
```

---

## 2. 一次性安装

```bash
git clone <repo> conduit && cd conduit
pnpm install            # 安 6 个 workspace 的 npm 依赖 + 触发 prepare
```

---

## 3. 开发模式启动

> 推荐两个独立终端各跑一个，方便看日志、Ctrl+C 单独重启。

### 3.1 启动 Server

终端 1：

```bash
cd /path/to/conduit
pnpm dev:server
```

会拉起：
- **Tauri 主窗口**：内嵌 Vue 控制台 (主仪表盘 / 日志 / 设置)
- **Python sidecar** (`server-app/core/proxy_server.py`)：HTTP / SOCKS5 / 控制 API + mDNS 广播

启动成功标志：

- 终端打印 `mDNS advertised: Conduit on _conduit._tcp.local.`
- 主窗口顶部状态徽章变绿 + 三端口胶囊填上数字
- macOS 菜单栏出现 Conduit 图标 (托盘)

可选启动参数（直接给 sidecar）：

```bash
# 自定义 mDNS 广播名
python3 server-app/core/proxy_server.py --mdns-name "工作机"
# 自定义 HTTP 端口
python3 server-app/core/proxy_server.py --http-port 18080
# 关闭 mDNS（仅供隔离调试）
CONDUIT_NO_MDNS=1 pnpm dev:server
```

> v0.1 阶段设置页所有项 **只读展示**，要改请用上面的启动参数。详情见 [验收指南](./design/2026-05-02-1-Conduit-验收指南.md#q7-server-app-设置页输入框点不动是要先停止代理才能改吗)。

### 3.2 启动 Client

终端 2：

```bash
cd /path/to/conduit
pnpm dev:client
```

会拉起：
- **Tauri 主窗口**：发现 / 已连接 / 路由 / 流量 / 诊断 / 设置 六页
- **Python sidecar** (`client-app/core/client_main.py`)：本机 SOCKS5 + 控制 API + mDNS 监听 + 路由决策

启动成功标志：

- 终端打印 `control API listening on 127.0.0.1:NNNN`
- "发现" 页 5–10 秒内出现 server 卡片
- macOS 菜单栏出现 Conduit Client 图标 (托盘)

> **首次启动 macOS 会弹窗 "Conduit 想要查找本地网络上的设备"** —— 必须允许，否则 mDNS 收不到广播。

可选启动参数：

```bash
# 跳过自动切系统代理（自己在浏览器里手动配 SOCKS5 测试）
CONDUIT_NO_SYSTEM_PROXY=1 pnpm dev:client
```

### 3.3 一条命令同时跑（可选）

```bash
pnpm dev:all   # concurrently 启动 server + client，颜色区分
```

不推荐——两个 Tauri 窗口加 sidecar 日志混在一起，调试时不清晰。

---

## 4. 第一次连接（90 秒）

1. 两个 app 都启动
2. 打开 Client 窗口，确认 "发现" 页有一张绿色 server 卡片
3. 点卡片右下 "连接"
4. 看 5 步 stepper 跑完（~2-3 秒）
5. 系统弹出 "Conduit 想要修改系统代理设置" → 输入密码允许
6. 浏览器访问 google.com → 通

完整验收脚本（含路由 / 流量 / 心跳 / 托盘 / 诊断 / 自启 7 大场景）见 **[验收指南](./design/2026-05-02-1-Conduit-验收指南.md)**。

---

## 5. 打包 (DMG)

```bash
pnpm build:server   # → server-app/src-tauri/target/release/bundle/dmg/
pnpm build:client   # → client-app/src-tauri/target/release/bundle/dmg/
```

> ⚠️ **未公证**：可以打包、可以本机运行，但分发给别的 Mac 时对方需要在 "系统设置 → 隐私与安全性" 手动 "仍要打开"。要去掉这个弹窗必须有 Apple Developer ID + `xcrun notarytool submit`，详情看 [Tauri 公证文档](https://v2.tauri.app/distribute/sign/macos/)。

---

## 6. 常用命令

| 命令 | 作用 |
|---|---|
| `pnpm dev:server` / `pnpm dev:client` | 启动各自 app（dev 模式，热重载） |
| `pnpm build:server` / `pnpm build:client` | 出 .app + .dmg（release 模式） |
| `pnpm dev:server-ui` / `pnpm dev:client-ui` | 只跑前端 Vite（用于纯 UI 调试） |
| `pnpm test` | 跑所有 Vitest（如果该 workspace 有的话） |
| `cd server-app/core && python3 -m pytest -q` | server Python 23 个集成测试 |
| `cd client-app/core && python3 -m pytest -q` | client Python 120 个集成测试 |
| `cd client-app/src-tauri && cargo test --lib` | client Rust 2 个单元测试 |
| `cd {server,client}-app/ui && pnpm vue-tsc --noEmit -p tsconfig.app.json` | 前端 TypeScript 类型检查 |

---

## 7. 项目结构

```
conduit/
├── server-app/              # Conduit Server desktop app
│   ├── core/                # Python sidecar: aiohttp + zeroconf
│   ├── src-tauri/           # Rust 主进程
│   └── ui/                  # Vue 3 + shadcn-vue
├── client-app/              # Conduit Client desktop app
│   ├── core/                # Python sidecar: discoverer + connector + cache + meter
│   ├── src-tauri/           # Rust 主进程 + 托盘 + autostart
│   └── ui/                  # Vue 3 + shadcn-vue
├── design/                  # 设计文档（按日期前缀编号）
│   ├── 2026-04-30-5-Conduit-开发TODO清单-进度S6Md-95.md   # TODO + 进度
│   ├── 2026-05-02-1-Conduit-验收指南.md                   # 完整验收手册
│   └── 2026-05-01-prototypes/                            # 4 个 UI 原型 HTML
├── scripts/                 # icons / tray 生成脚本
└── package.json             # workspace root
```

---

## 8. 故障排查（精简版）

| 现象 | 解决 |
|---|---|
| Client "发现" 页一直空 | macOS 设置 → 隐私与安全性 → 本地网络 → 勾上 client-app；或直接看 [验收指南 Q1](./design/2026-05-02-1-Conduit-验收指南.md) |
| 连接 5 步卡在第 1 步 | server 端口被防火墙挡，`nc -zv <host> <port>` 验证 |
| 系统代理切换失败 -setsocksfirewallproxy | macOS 14+ 要 sudo 权限；首次会弹窗，输密码即可。详见 [验收指南 Q3](./design/2026-05-02-1-Conduit-验收指南.md) |
| Server "停止代理" 点了之后没反应 | M-δ 后已修复 (50ms 延迟保证 200 OK 先落) |
| Server 已停止后按钮 disabled | M-δ 后按钮自动切成 "重启代理"，会调 `restart_app` 重启整个 .app |
| 想从头开始 | `pkill -f proxy_server.py; pkill -f client_main.py; rm ~/Library/Application\ Support/Conduit/known-servers.json` |

完整 8 条 FAQ：[验收指南 § 4](./design/2026-05-02-1-Conduit-验收指南.md#4-故障排查)。

---

## 9. 当前进度

- ✅ M-α 骨架 / 端口分配 / 健康探活
- ✅ M-β.1 mDNS 真发现 + known-servers 历史
- ✅ M-β.2 5 步连接 + 系统代理切换 + 心跳
- ✅ M-γ 流量统计 + 路由缓存 + 1Hz SSE
- ✅ M-δ 诊断页 + 系统托盘 + macOS 自启 + 服务端可重启
- ⏳ S4 设置项可编辑 + 持久化 + 热重启
- ⏳ S5 .dmg 公证 + 自动更新 + 多 server 同时连接

总体完成度 **~95%**（v0.1 GA 范围内）。详见 [TODO 清单](./design/2026-04-30-5-Conduit-开发TODO清单-进度S6Md-95.md)。

---

## License

私有项目，未公开。
