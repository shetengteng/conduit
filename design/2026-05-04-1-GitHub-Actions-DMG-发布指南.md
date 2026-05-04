# Conduit GitHub Actions DMG 发布指南

> 日期：2026-05-04
> 适用范围：仓库 [shetengteng/conduit](https://github.com/shetengteng/conduit) 的 macOS DMG 自动化打包
> 工作流文件：`.github/workflows/release.yml`

---

## 1. 两种触发方式

| 触发方式 | 适用场景 | 是否创建 GitHub Release | 产物去哪里 |
|---|---|---|---|
| **Tag push (`v*`)** | 正式发版 / 公开下载 | ✅ 自动创建 Release，附 4 个 dmg | Releases 页 |
| **`workflow_dispatch`(手动)** | 内部验证 / 给另一台 Mac 装一下 | ❌ 不创建 release | 该 run 的 Artifacts 区，14 天保留 |

每次触发都会跑两个并行 job，分别在 `macos-15`(Apple Silicon, aarch64) 和 `macos-15-intel`(Intel, x86_64) 上构建，每个 job 出 2 个 dmg(server + client)，共 **4 个 dmg / 一次触发**。

---

## 2. 方式 A:Tag push 自动发版

适合给团队 / 外部用户用。

### 步骤

```bash
cd /Users/TerrellShe/Documents/personal/tt-projects/conduit

# 1. 确认本地干净 + 与远端同步
git status
git fetch origin
git log --oneline origin/main..HEAD   # 看本地有没有未 push 的 commit
git push                              # 如果有就 push 上去

# 2. 决定版本号(语义化版本)
#    - 修 bug   → patch:0.1.0 → 0.1.1
#    - 加功能   → minor:0.1.0 → 0.2.0
#    - 不兼容   → major:0.1.0 → 1.0.0
VERSION=v0.1.1

# 3. 打 tag(推荐 annotated tag,带描述)
git tag -a $VERSION -m "Release $VERSION"

# 4. push tag → 触发 workflow
git push origin $VERSION
```

### 跑完之后

- 浏览器打开 <https://github.com/shetengteng/conduit/actions> 看 build-dmg 进度(约 10 分钟)
- 全部 ✅ 之后,<https://github.com/shetengteng/conduit/releases> 会自动出现 `v0.1.1`,附:
  - `Conduit Server_0.1.1_aarch64.dmg`
  - `Conduit Server_0.1.1_x86_64.dmg`
  - `Conduit Client_0.1.1_aarch64.dmg`
  - `Conduit Client_0.1.1_x86_64.dmg`
- Release notes 由 GitHub 自动从 commit 历史生成

### 撤销 / 重发

如果想重发同一个版本(罕见):

```bash
git tag -d v0.1.1                  # 本地删 tag
git push origin :refs/tags/v0.1.1  # 远端删 tag
# 顺便去 GitHub Releases 页把那个 release 也手动删了
# 然后重新打 tag
```

---

## 3. 方式 B:workflow_dispatch 手动触发(仅验证)

适合改完代码不打 tag、只想拉个 dmg 给另一台 Mac 装一下试试。

### 步骤

1. 浏览器打开:
   <https://github.com/shetengteng/conduit/actions/workflows/release.yml>
2. 右上角点 `Run workflow`(灰色按钮)
3. 弹出小框,选项:
   - **Branch**: 默认 `main`,通常不改
   - **Which app to build**: 三选一
     - `both`: 同时构建 server + client(默认,推荐)
     - `server`: 只构建 server
     - `client`: 只构建 client
4. 点绿色的 `Run workflow`
5. 列表第一行就是这次 run,点进去看进度

### 跑完之后

- run 详情页底部有 `Artifacts` 板块,列出:
  - `conduit-aarch64.zip`(包含 dmg,Apple Silicon Mac 用)
  - `conduit-x86_64.zip`(包含 dmg,Intel Mac 用)
- 点击 zip 名字直接下载到本地,**注意只有登录 GitHub 的人能下载 artifact**,匿名访客只能用 release(方式 A)

### 适合什么时候用

- 只是想测一下打包能不能跑通
- 给同事或另一台测试机装一下,但还没准备好正式发版
- 改完 workflow 文件想跑一遍看会不会挂

---

## 4. 在另一台 Mac 上安装与验证

无论方式 A 还是 B,拿到 dmg 之后流程一样。

### 4.1 选对架构

```bash
# 在目标 Mac 上跑,看是 Apple Silicon 还是 Intel
uname -m
```

| `uname -m` 输出 | 下哪个 dmg |
|---|---|
| `arm64` | 文件名带 `aarch64` |
| `x86_64` | 文件名带 `x86_64` |

### 4.2 安装

1. 双击 dmg → Finder 弹出挂载窗口
2. 把 `Conduit Server.app` / `Conduit Client.app` **拖到** `Applications` 别名文件夹
3. 在 Finder 里把刚挂载的卷弹出(右键 → 推出)

### 4.3 解除 Gatekeeper quarantine(关键!!!)

第一次打开会被拦,因为 dmg 没有 Apple Developer 公证。**已经 ad-hoc 签过名了,所以不会报 "App Is Damaged"**,但仍然有 quarantine 标记。**两选一**:

**方法 A:终端命令(推荐,一行搞定)**

```bash
xattr -dr com.apple.quarantine "/Applications/Conduit Server.app"
xattr -dr com.apple.quarantine "/Applications/Conduit Client.app"
```

**方法 B:Finder 手动**

- 在 Finder 里找到 `/Applications/Conduit Server.app`
- 按住 `Ctrl` 键点击 → 选 `打开`
- 弹出警告 → 点 `打开`(只有第一次需要,之后双击就行)

### 4.4 启动验证

#### Conduit Server

```bash
open -a "Conduit Server"
```

期望:

- 窗口立即出现(默认 `visible: true`),不卡在 BootScreen
- 状态从 `正在启动代理引擎` → 几秒后切到主界面
- 主界面显示 `运行中`,有 LAN IP / SOCKS5 端口 / VPN 隧道 等信息
- 顶部菜单栏右侧有图标(系统盘菜单)
- 关闭窗口(红圆点)→ 应用隐藏到系统盘,不退出
- 真正退出走系统盘菜单 → `退出 Conduit Server` 或 `cmd-Q`

如果要看 sidecar 日志:

```bash
tail -f ~/.conduit/logs/proxy.log
```

#### Conduit Client

需要先有 server 在同一局域网运行,client 才能 mDNS 发现。

```bash
open -a "Conduit Client"
```

期望:

- 客户端启动后自动发现局域网内 server(几秒)
- 显示 server 的 LAN IP + 端口
- 点连接 → 切到 connected view
- macOS 系统代理被设置(System Settings → Network → Wi-Fi → Details → Proxies 里能看到 Auto Proxy)

---

## 5. 参考:版本号与 tag 关系

| Tauri / package.json version | git tag | dmg 文件名 |
|---|---|---|
| `0.1.0` | `v0.1.0` | `Conduit Server_0.1.0_aarch64.dmg` |
| `0.1.1` | `v0.1.1` | `Conduit Server_0.1.1_aarch64.dmg` |

dmg 文件名里的版本号来自:

- `package.json` 的 `version` 字段
- `server-app/src-tauri/tauri.conf.json` 的 `version`
- `client-app/src-tauri/tauri.conf.json` 的 `version`

**这三处必须保持同步**,正式发版前手工更新一遍。后续可以加个 `pnpm bump-version` 脚本统一 sync,目前不做。
