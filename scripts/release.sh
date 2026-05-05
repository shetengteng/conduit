#!/usr/bin/env bash
# release.sh —— 打 Conduit Server / Client 的发布包(.dmg / .msi / .deb / .AppImage)。
#
# 流程:
#   1. 跑 build-sidecars.sh 把 Python 打成单二进制 → src-tauri/binaries/
#   2. 跑 pnpm install (确保前端依赖就绪)
#   3. 跑 pnpm tauri build 在每个 app 目录里
#   4. 输出归集到 dist/<app>/<bundle-files>
#
# 跑法:
#   ./scripts/release.sh           # 打两个 app
#   ./scripts/release.sh server    # 只打 server
#   ./scripts/release.sh client    # 只打 client
#
# 注:
#   - 需要先在本机配好 rust + tauri toolchain。
#   - macOS 公证 (notarytool) 需要 APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID
#     env vars; 没配就跳过 (会得到未公证的 .dmg, 用户首次打开要 Ctrl+Click → 打开)。
#   - Windows EV 代码签名需 SIGN_TOOL + cert; 没配就跳过。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() {
  echo "Usage: $0 [server|client]"
  exit 0
}

[[ "${1:-}" == "-h" || "${1:-}" == "--help" ]] && usage

# ---------- 工具检查 ----------
need() {
  command -v "$1" >/dev/null 2>&1 || { echo "✗ $1 not found in PATH" >&2; exit 1; }
}
need pnpm
need cargo
need python3

# ---------- step 1: sidecar ----------
echo "═══ step 1/4: build sidecars ═══"
if [[ $# -gt 0 ]]; then
  ./scripts/build-sidecars.sh "$@"
else
  ./scripts/build-sidecars.sh
fi

# ---------- step 2: pnpm install ----------
echo ""
echo "═══ step 2/4: pnpm install ═══"
if [[ ! -d node_modules ]]; then
  pnpm install
fi

# ---------- step 3: tauri build per app ----------
build_app() {
  local app="$1"
  local dir="${app}-app"
  local product
  case "$app" in
    server) product="Conduit Server" ;;
    client) product="Conduit Client" ;;
    *) echo "✗ unknown app: $app" >&2; exit 1 ;;
  esac

  if [[ ! -d "$dir" ]]; then
    echo "✗ missing $dir" >&2
    exit 1
  fi

  echo ""
  echo "═══ step 3/4: tauri build $product ═══"
  # sidecar 通过 bundle.resources 走 onedir 目录树（见 build-sidecars.sh
  # 顶部注释），不再使用 externalBin 单二进制约定。
  (cd "$dir" && pnpm tauri build)

  # 归集产物
  local bundle_dir="$dir/src-tauri/target/release/bundle"
  if [[ ! -d "$bundle_dir" ]]; then
    echo "⚠ bundle dir not found: $bundle_dir" >&2
    return
  fi

  local out="dist/$app"
  mkdir -p "$out"
  # macOS: dmg + app
  cp -R "$bundle_dir/macos/"*.app "$out/" 2>/dev/null || true
  cp "$bundle_dir/dmg/"*.dmg "$out/" 2>/dev/null || true
  # Linux: deb + AppImage
  cp "$bundle_dir/deb/"*.deb "$out/" 2>/dev/null || true
  cp "$bundle_dir/appimage/"*.AppImage "$out/" 2>/dev/null || true
  # Windows: msi + nsis
  cp "$bundle_dir/msi/"*.msi "$out/" 2>/dev/null || true
  cp "$bundle_dir/nsis/"*.exe "$out/" 2>/dev/null || true

  echo "✓ artifacts -> $out/"
  ls -lh "$out/" | tail -n +2
}

if [[ $# -eq 0 ]]; then
  build_app server
  build_app client
else
  for app in "$@"; do
    build_app "$app"
  done
fi

echo ""
echo "═══ step 4/4: notarization (optional) ═══"
if [[ "$(uname -s)" == "Darwin" ]] && [[ -n "${APPLE_ID:-}" ]]; then
  echo "→ APPLE_ID set, attempting notarization (TODO: implement via xcrun notarytool submit ...)"
  # TODO: 等 Apple Developer 账号到位后填实
  # for dmg in dist/*/*.dmg; do
  #   xcrun notarytool submit "$dmg" --apple-id "$APPLE_ID" \
  #     --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
  #   xcrun stapler staple "$dmg"
  # done
else
  echo "ℹ skipping notarization (set APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID to enable)"
  echo "  未公证的 .dmg 首次双击会被 macOS Gatekeeper 拦截。解决（任选一种）:"
  echo "    a) 终端: sudo xattr -dr com.apple.quarantine \"/Applications/Conduit Server.app\""
  echo "    b) Finder Ctrl+Click .app → 打开 → 确认 (单次)"
  echo "    c) 系统设置 → 隐私与安全性 → 滚到底部点 仍要打开 (macOS 15+ 推荐)"
fi

echo ""
echo "═══ all done ═══"
echo "产物在 dist/<app>/"
