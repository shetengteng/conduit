# Conduit

> Zero-config LAN VPN sharing — let one Mac (with VPN) act as the gateway for every other device on the same Wi-Fi.

**Landing page:** <https://shetengteng.github.io/conduit/> · **Languages:** **English** · [中文](./README_zh.md)

Conduit ships as two independent macOS desktop apps:

| App | Role | Key capabilities |
|---|---|---|
| **Conduit Server** | Runs on the Mac that owns the VPN connection | HTTP / SOCKS5 proxy + mDNS broadcast + PAC service + control API |
| **Conduit Client** | Runs on every device that wants to use the shared VPN | mDNS auto-discovery, 5-step connect wizard, smart per-host route cache, 1 Hz traffic chart, macOS system-proxy switching, system tray, 5-step diagnostics, login-time autostart |

Stack: **Tauri 2 + Rust** — single in-process runtime · **Vue 3 + shadcn-vue + Tailwind v4** UI. Communication is localhost HTTP + Server-Sent Events. _v0.2.0 removed the previous Python sidecar; everything now runs inside the Tauri main process._

UI design: Stripe / Vercel-style enterprise white (Plan B), all icons via [RemixIcon](https://remixicon.com/).

---

## 1. Requirements

| Software | Version | Notes |
|---|---|---|
| macOS | 13+ | Apple Silicon & Intel both fine. (Windows / Linux client coming in v0.3.) |
| Node | ≥ 20.10 | `node -v` |
| pnpm | ≥ 9 | `corepack enable pnpm` or `npm i -g pnpm@9` |
| Rust toolchain | ≥ 1.84 | `rustup default stable` |
| OS permissions | Local Network + System Proxy | Two macOS prompts on first launch — **allow both** |

> No Python or PyInstaller required as of v0.2.0.

---

## 2. Install — recommended path (DMG)

> If you only want to **use** Conduit (not develop it), grab the pre-built DMGs from [GitHub Releases](https://github.com/shetengteng/conduit/releases). Source-code install is in [§ 3](#3-install--source-code-alternative).

### 2.1 Pick the right DMG

| Mac | Server | Client |
|---|---|---|
| Apple Silicon (M1 / M2 / M3 / M4) | `Conduit.Server_<ver>_aarch64.dmg` | `Conduit.Client_<ver>_aarch64.dmg` |
| Intel | `Conduit.Server_<ver>_x64.dmg` | `Conduit.Client_<ver>_x64.dmg` |

Typical sizes: **Server ≈ 4–5 MB**, **Client ≈ 5–6 MB** (≈ 91 % smaller than the v0.1.x Python builds).

### 2.2 Standard DMG install

1. Double-click the DMG → drag the `.app` into `/Applications`.
2. Open Launchpad / Spotlight → search for **Conduit Server** (or **Conduit Client**).
3. Allow the two macOS prompts on first launch (Local Network + System Proxy).

### 2.3 ⚠ Unsigned & not-yet-notarized — fix Gatekeeper

The DMGs are **ad-hoc signed and NOT Apple-notarized** (yet — pending Apple Developer ID). On macOS 13+ the first launch is blocked with:

> *"Conduit Server" is damaged and can't be opened.*
> *"Conduit Server" cannot be opened because the developer cannot be verified.*

Pick **one** of the workarounds below.

#### Option A — Right-click → Open  *(simplest, one-time)*

1. Locate the `.app` in `/Applications` (or wherever you dropped it).
2. **Right-click** → **Open** → click **Open** in the warning dialog.
3. macOS remembers this exemption for that specific binary.

#### Option B — Strip the quarantine attribute  *(recommended after copying / re-downloading)*

```bash
sudo xattr -dr com.apple.quarantine "/Applications/Conduit Server.app"
sudo xattr -dr com.apple.quarantine "/Applications/Conduit Client.app"
```

Note the `sudo` — it'll prompt for your login password.

> macOS 15+ occasionally still blocks `xattr` via SIP. If it fails, jump straight to Option C.

#### Option C — Whitelist in System Settings  *(macOS 13+)*

After the first failed launch:

1. **System Settings → Privacy & Security → Security**.
2. Scroll to *"`Conduit Server` was blocked from use because it is not from an identified developer."*
3. Click **Open Anyway** → enter your password.

> Once the project obtains an Apple Developer ID, `./scripts/release.sh` will run `xcrun notarytool submit` automatically.

---

## 3. Install — source-code alternative

> Pick this only if you plan to modify code, run tests, or are on a platform without a pre-built DMG.

```bash
git clone <repo> conduit && cd conduit
pnpm install            # 6 workspaces, ~30s
```

> First run: Tauri will download Rust dependencies and macOS will prompt for network permission. Be patient and click **Allow**.

---

## 4. Run in dev mode

> Recommended: two separate terminals so logs are easy to follow.

### 4.1 Start Server

```bash
cd /path/to/conduit
pnpm dev:server
```

This launches a single Tauri process with the in-process `ProxyCore` (HTTP + SOCKS5 + mDNS + control API).

Success indicators:
- Terminal prints `[mdns] advertised <hostname>._conduit._tcp.local. @ <ip>:<port> (vpn=on|off)`
- Top status badge turns green; three port pills filled in (HTTP / SOCKS5 / control API).
- Conduit icon appears in the macOS menu bar (tray).

### 4.2 Start Client

```bash
cd /path/to/conduit
pnpm dev:client
```

This launches the Conduit Client Tauri window with the in-process `ClientCore` (mDNS discoverer + local SOCKS5 + route cache + heartbeat + system-proxy controller).

Success indicators:
- Terminal prints `[control_api] listening on http://127.0.0.1:NNNN` and `[discoverer] server_discovered: …`
- "Discovery" page shows server cards within 5–10 seconds.
- Conduit Client icon appears in the macOS menu bar.

> **First launch macOS will prompt: "Conduit wants to find devices on your local network"** — must allow, otherwise mDNS broadcasts won't reach the client.

### 4.3 Run both at once (optional)

```bash
pnpm dev:all
```

Not recommended — two Tauri windows + two log streams interleaved.

---

## 5. First connection (90 seconds)

1. Both apps running.
2. Open the Client window. The "Discovery" page should show a green server card.
3. Click "Connect" on the card.
4. Watch the 5-step stepper finish (~2-3 s).
5. macOS prompts "Conduit wants to modify system proxy" → enter password to allow.
   - If you decline (or you're on macOS 13+ without admin rights), the connect step 4 will fail with a clear error; just relaunch Client with `enable_system_proxy=false` in your config and configure SOCKS5 manually in your browser.
6. Open google.com — it works through the shared VPN.

---

## 6. Build the DMGs yourself

```bash
./scripts/release.sh                 # pnpm tauri build + collect into dist/ (~3 min cold)
```

Output:
```
dist/server/Conduit Server.app
dist/server/Conduit Server_<ver>_aarch64.dmg     ~4-5 MB
dist/client/Conduit Client.app
dist/client/Conduit Client_<ver>_aarch64.dmg     ~5-6 MB
```

> Same Gatekeeper caveat as [§ 2.3](#23--unsigned--not-yet-notarized--fix-gatekeeper). Apple Developer ID + `xcrun notarytool submit` will remove this once configured.

> `scripts/e2e.sh` is currently marked **DEPRECATED** (it targeted the removed Python sidecar). The Rust-version end-to-end script is on the v0.3 backlog. For now use `pnpm tauri dev` in two terminals plus the manual checklist in `design/2026-05-06-3-Conduit-Rust-重写TODO清单.md`.

---

## 7. Common commands

| Command | Effect |
|---|---|
| `pnpm dev:server` / `pnpm dev:client` | Run app in dev mode (HMR) |
| `pnpm build:server` / `pnpm build:client` | Build .app + .dmg |
| `pnpm dev:server-ui` / `pnpm dev:client-ui` | Front-end Vite only (UI tweaking) |
| `cargo test --workspace` | Run all Rust tests (143 passing) |
| `cargo clippy --workspace --no-deps -- -D warnings` | Lint check (0 warning) |
| `cargo build --workspace --release` | Full release build (~1m30s on M1 Pro) |

---

## 8. Repo layout

```
conduit/
├── Cargo.toml               # Cargo workspace root
├── crates/
│   └── conduit-core/        # Shared crate: types / EventBus / Relay / mDNS codec / PAC engine
├── server-app/              # Conduit Server desktop app
│   ├── src-tauri/           # Tauri main + tray + in-process ProxyCore
│   └── ui/                  # Vue 3 + shadcn-vue
├── client-app/              # Conduit Client desktop app
│   ├── src-tauri/           # Tauri main + tray + autostart + in-process ClientCore
│   └── ui/                  # Vue 3 + shadcn-vue
├── scripts/                 # release.sh / bump-version.sh / publish-release-notes.sh / e2e.sh (DEPRECATED)
├── design/                  # Date-prefixed design docs (Python era moved to design/archive/ in v0.2.0)
│   ├── 2026-05-06-1-技术栈精简可行性分析.md
│   ├── 2026-05-06-2-Conduit-Rust-重写设计文档.md
│   └── 2026-05-06-3-Conduit-Rust-重写TODO清单.md
├── CHANGELOG.md             # Keep-a-Changelog format
└── package.json             # pnpm workspace root
```

---

## 9. Troubleshooting (short)

| Symptom | Fix |
|---|---|
| `"Conduit Server" is damaged and can't be opened` on first launch | Unsigned-build Gatekeeper. Run `sudo xattr -dr com.apple.quarantine "/Applications/Conduit Server.app"` (and same for Client). If you still get `Operation not permitted`, use [§ 2.3 Option C](#23--unsigned--not-yet-notarized--fix-gatekeeper) (whitelist in System Settings). |
| Client "Discovery" page stays empty | Settings → Privacy & Security → Local Network → enable client-app. |
| Connect stalls on step 1 | Server port blocked by firewall: `nc -zv <host> <port>` to verify. |
| Connect step 4 fails with `system_proxy enable failed: ... exit status: 14` | macOS `networksetup -setsocksfirewallproxy` requires admin / Tauri sandbox is denying it. Either grant admin via System Settings, or set `enable_system_proxy=false` in client config and configure SOCKS5 manually in your browser. |
| Server "Active clients" stays 0 even though the client connected | Client only sends a heartbeat (passive registration); the KPI counts active SOCKS5/HTTP sessions only. Open google.com through the proxy and the number should bump to 1. |
| "Standby clients" list shows a closed client forever | Should self-evict within 30 s as of v0.2.0 (passive-client TTL). If not, check the server log for missed heartbeats. |
| Wipe everything and start over | `rm -rf ~/Library/Application\ Support/Conduit/` |

---

## 10. Status

- ✅ v0.2.0 — Pure-Rust rewrite (Python sidecar removed, single process per app, ~91 % smaller DMG)
- ✅ M-α / M-β / M-γ / M-δ scope retained from v0.1.x
- ✅ `cargo test --workspace`: 143 passing (conduit-core 59 + conduit-server 43 + conduit-client 41) / 0 failed / 2 ignored
- ✅ `cargo clippy --workspace --no-deps -- -D warnings` clean
- ⏳ v0.3 backlog: Windows / Linux client (cross-compile matrix), `tauri-plugin-updater` auto-update, editable Settings, Apple notarization

See the [CHANGELOG](./CHANGELOG.md) for the full per-version history and the [v0.2.0 release notes](./scripts/release-notes-v0.2.0.md) for the rewrite details.

---

## 11. License

Private project, not yet publicly distributed.
