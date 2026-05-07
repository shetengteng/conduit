#!/usr/bin/env bash
# release.sh —— 打 Conduit Server / Client 的发布包(.dmg / .msi / .deb / .AppImage)。
#
# server-app + client-app 是纯 Rust + TypeScript（in-process ProxyCore /
# ClientCore），打包流程只跑 pnpm install + pnpm tauri build。
#
# 流程:
#   1. pnpm install (确保前端依赖就绪)
#   2. pnpm tauri build 在每个 app 目录里
#   3. 输出归集到 dist/<app>/<bundle-files>
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

# ---------- step 1: pnpm install ----------
echo "═══ step 1/3: pnpm install ═══"
if [[ ! -d node_modules ]]; then
  pnpm install
fi

# ---------- step 2: tauri build per app ----------
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
  echo "═══ step 2/3: tauri build $product ═══"
  # macOS 的 bundle_dmg.sh 在 CI 环境（尤其 Intel runner）上偶发失败：
  # hdiutil 资源紧张 / osascript 调 Finder 失败 / 临时挂载点回收慢等。
  # 失败时重试一次（间隔 5s），并把 cargo-packager / tauri-bundler 的 debug log
  # 打开方便定位；本机首次构建不受影响（一次成功就跳过重试）。
  local build_attempt=0
  local build_max_attempts=2
  while :; do
    build_attempt=$((build_attempt + 1))
    if (cd "$dir" && RUST_LOG="${RUST_LOG:-tauri_bundler=debug,cargo_packager=debug}" pnpm tauri build); then
      break
    fi
    if [[ $build_attempt -ge $build_max_attempts ]]; then
      echo "✗ tauri build $product 连续 ${build_max_attempts} 次失败，放弃" >&2
      exit 1
    fi
    echo "⚠ tauri build $product 第 ${build_attempt} 次失败，5s 后重试..." >&2
    sleep 5
  done

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
echo "═══ step 3/3: notarization (optional) ═══"
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
