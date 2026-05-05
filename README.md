# Conduit

> Zero-config LAN VPN sharing — let one Mac (with VPN) act as the gateway for every other device on the same Wi-Fi.

**Landing page:** <https://shetengteng.github.io/conduit/> · **Languages:** **English** · [中文](./README_zh.md)

Conduit ships as two independent macOS desktop apps:

| App | Role | Key capabilities |
|---|---|---|
| **Conduit Server** | Runs on the Mac that owns the VPN connection | HTTP / SOCKS5 proxy + mDNS broadcast + PAC service + control API |
| **Conduit Client** | Runs on every device that wants to use the shared VPN | mDNS auto-discovery, 5-step connect wizard, smart per-host route cache, 1 Hz traffic chart, macOS system-proxy switching, system tray, 5-step diagnostics, login-time autostart |

Stack: **Tauri 2 + Rust** main process · **Vue 3 + shadcn-vue + Tailwind v4** UI · **Python 3 (aiohttp / asyncio / zeroconf)** sidecar. Communication is localhost HTTP + Server-Sent Events.

UI design: Stripe / Vercel-style enterprise white (Plan B), all icons via [RemixIcon](https://remixicon.com/).

---

## 1. Requirements

| Software | Version | Notes |
|---|---|---|
| macOS | 13+ | Apple Silicon & Intel both fine. Linux/Windows server only (no client yet). |
| Node | ≥ 20.10 | `node -v` |
| pnpm | ≥ 9 | `corepack enable pnpm` or `npm i -g pnpm@9` |
| Python | ≥ 3.10 | `python3 --version` |
| Rust toolchain | ≥ 1.78 | `rustup default stable` |
| OS permissions | Local Network + System Proxy | Two macOS prompts on first launch — **allow both** |

Sidecar Python deps install on first `pnpm dev:*`. Manual install is also fine:

```bash
pip3 install aiohttp zeroconf
```

---

## 2. Install — recommended path (DMG)

> If you only want to **use** Conduit (not develop it), grab the pre-built DMGs. Source-code install is in [§ 3](#3-install--source-code-alternative).

### 2.1 Pre-built artifacts

After running `./scripts/release.sh` (or downloading the release ZIP) you get:

```
dist/server/Conduit Server.app
dist/server/Conduit Server_0.1.0_aarch64.dmg     ~30 MB
dist/client/Conduit Client.app
dist/client/Conduit Client_0.1.0_aarch64.dmg     ~30 MB
```

> ⚙ Architecture: `aarch64` = Apple Silicon (M1/M2/M3/M4). For Intel Macs build from source on an Intel host.

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
# After dragging into /Applications:
xattr -dr com.apple.quarantine "/Applications/Conduit Server.app"
xattr -dr com.apple.quarantine "/Applications/Conduit Client.app"
```

That's it — double-click works normally afterwards. Re-run the command if you ever re-download or replace the app.

#### Option C — Whitelist in System Settings  *(macOS 13+)*

After the first failed launch:

1. **System Settings → Privacy & Security → Security**.
2. Scroll to *"`Conduit Server` was blocked from use because it is not from an identified developer."*
3. Click **Open Anyway** → enter your password.

#### Verify

```bash
codesign -dvv "/Applications/Conduit Server.app" 2>&1 | rg "Signature|TeamIdentifier"
# expected: Signature=adhoc · TeamIdentifier=not set   (← that's why Gatekeeper complains)
```

> Once the project obtains an Apple Developer ID, `./scripts/release.sh` will run `xcrun notarytool submit` automatically (the env-var hooks are already in [`scripts/release.sh`](./scripts/release.sh) step 4) and these workarounds will become unnecessary. See the [packaging guide](./design/2026-05-03-1-Conduit-打包与发布说明.md) for the full notarization flow.

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

Terminal 1:

```bash
cd /path/to/conduit
pnpm dev:server
```

This launches:
- **Tauri main window** with the Vue console (Dashboard / Logs / Settings)
- **Python sidecar** (`server-app/core/proxy_server.py`): HTTP / SOCKS5 / control API + mDNS broadcast

Success indicators:
- Terminal prints `mDNS advertised: Conduit on _conduit._tcp.local.`
- Top status badge turns green; three port pills filled in
- Conduit icon appears in the macOS menu bar (tray)

Optional CLI flags (passed to the sidecar):

```bash
python3 server-app/core/proxy_server.py --mdns-name "Workstation"
python3 server-app/core/proxy_server.py --http-port 18080
CONDUIT_NO_MDNS=1 pnpm dev:server      # disable mDNS for isolated debugging
```

> v0.1 keeps Settings page **read-only**; pass flags above to override.

### 4.2 Start Client

Terminal 2:

```bash
cd /path/to/conduit
pnpm dev:client
```

This launches:
- **Tauri main window**: 4 pages (Discovery / Connected / Diagnose / Settings)
- **Python sidecar** (`client-app/core/client_main.py`): local SOCKS5 + control API + mDNS listener + smart route resolver

Success indicators:
- Terminal prints `control API listening on 127.0.0.1:NNNN`
- "Discovery" page shows server cards within 5–10 seconds
- Conduit Client icon appears in the macOS menu bar

> **First launch macOS will prompt: "Conduit wants to find devices on your local network"** — must allow, otherwise mDNS broadcasts won't reach the client.

Optional flags:

```bash
CONDUIT_NO_SYSTEM_PROXY=1 pnpm dev:client    # skip auto system-proxy switching
```

### 4.3 Run both at once (optional)

```bash
pnpm dev:all
```

Not recommended — two Tauri windows + two sidecar log streams interleaved.

---

## 5. First connection (90 seconds)

1. Both apps running.
2. Open the Client window. The "Discovery" page should show a green server card.
3. Click "Connect" on the card.
4. Watch the 5-step stepper finish (~2-3s).
5. macOS prompts "Conduit wants to modify system proxy" → enter password to allow.
   - If you decline (or you're on macOS 13+ without admin rights), an **amber banner appears at the top of the Client window** telling you to manually configure SOCKS5 in your browser/app.
6. Open google.com — it works.

---

## 6. Build the DMGs yourself

Three production-ready scripts under `scripts/`:

```bash
./scripts/build-sidecars.sh          # Step 1: bundle Python sidecars via PyInstaller (~50s)
./scripts/release.sh                 # Step 2: pnpm tauri build + collect into dist/ (~3 min cold)
./scripts/e2e.sh                     # Smoke test: 11s end-to-end check
```

Output (this is exactly what § 2 ships):
```
dist/server/Conduit Server.app
dist/server/Conduit Server_0.1.0_aarch64.dmg     ~30 MB
dist/client/Conduit Client.app
dist/client/Conduit Client_0.1.0_aarch64.dmg     ~30 MB
```

> Same Gatekeeper caveat as [§ 2.3](#23--unsigned--not-yet-notarized--fix-gatekeeper). The end-of-script line tells you the recipient must right-click → Open *or* run the `xattr -dr com.apple.quarantine` command. Apple Developer ID + `xcrun notarytool submit` will remove this once configured — see the [packaging guide](./design/2026-05-03-1-Conduit-打包与发布说明.md) for the full flow.

---

## 7. Common commands

| Command | Effect |
|---|---|
| `pnpm dev:server` / `pnpm dev:client` | Run app in dev mode (HMR) |
| `pnpm build:server` / `pnpm build:client` | Build .app + .dmg |
| `pnpm dev:server-ui` / `pnpm dev:client-ui` | Front-end Vite only (UI tweaking) |
| `cd server-app/core && python3 -m pytest -q` | Server pytest suite (82 tests) |
| `cd client-app/core && python3 -m pytest -q` | Client pytest suite (128 tests) |
| `cd {server,client}-app/src-tauri && cargo check` | Rust type check |
| `./scripts/e2e.sh` | End-to-end smoke (11s) |

---

## 8. Repo layout

```
conduit/
├── server-app/              # Conduit Server desktop app
│   ├── core/                # Python sidecar: aiohttp + zeroconf
│   ├── src-tauri/           # Rust main + tray
│   └── ui/                  # Vue 3 + shadcn-vue
├── client-app/              # Conduit Client desktop app
│   ├── core/                # Python sidecar: discoverer + connector + cache + meter + diagnose
│   ├── src-tauri/           # Rust main + tray + autostart (LaunchAgent)
│   └── ui/                  # Vue 3 + shadcn-vue
├── scripts/                 # build-sidecars.sh / release.sh / e2e.sh
├── design/                  # Date-prefixed design docs
│   ├── 2026-04-30-5-Conduit-开发TODO清单-进度S6Md-95.md   # Complete TODO + progress
│   ├── 2026-05-02-1-Conduit-验收指南.md                   # User acceptance manual
│   └── 2026-05-03-1-Conduit-打包与发布说明.md             # Packaging & release guide
└── package.json             # Workspace root
```

---

## 9. Troubleshooting (short)

| Symptom | Fix |
|---|---|
| `"Conduit Server" is damaged and can't be opened` on first launch | Unsigned-build Gatekeeper. Run `xattr -dr com.apple.quarantine "/Applications/Conduit Server.app"` (and same for Client). Full options in [§ 2.3](#23--unsigned--not-yet-notarized--fix-gatekeeper). |
| Client "Discovery" page stays empty | Settings → Privacy & Security → Local Network → enable client-app. See [acceptance guide Q1](./design/2026-05-02-1-Conduit-验收指南.md). |
| Connect stalls on step 1 | Server port blocked by firewall: `nc -zv <host> <port>` to verify. |
| Amber banner: "system proxy not auto-switched" | macOS 13+ needs admin to call `networksetup`. Either configure SOCKS5 manually in your browser, or launch Conduit with sudo (not recommended). |
| Server "Stop proxy" did nothing | Fixed in M-δ (50ms delay so HTTP 200 lands first). |
| Wipe everything and start over | `pkill -f proxy_server.py; pkill -f client_main.py; rm ~/Library/Application\ Support/Conduit/known-servers.json` |

For the full FAQ see the [acceptance guide § 4](./design/2026-05-02-1-Conduit-验收指南.md#4-故障排查) and the [packaging guide § 6](./design/2026-05-03-1-Conduit-打包与发布说明.md).

---

## 10. Status

- ✅ M-α  Skeleton, port allocation, health check
- ✅ M-β.1  mDNS discovery + known-server history
- ✅ M-β.2  5-step connect + system proxy switch + heartbeat
- ✅ M-γ  Traffic chart + route cache + 1 Hz SSE
- ✅ M-δ  Diagnose page + tray + macOS autostart + restart-from-stopped
- ✅ Tests: 210 passing pytest cases (server 82 + client 128)
- ✅ End-to-end smoke (`scripts/e2e.sh`): mDNS → connect → SOCKS5 traffic → diagnose → disconnect, all within 11s
- ⏳ Editable Settings + persistence
- ⏳ Apple notarization + auto-update + multi-server simultaneous connect

Overall ~98% of v0.1.0 scope. See the [TODO list](./design/2026-04-30-5-Conduit-开发TODO清单-进度S6Md-95.md).

---

## 11. License

Private project, not yet publicly distributed.
