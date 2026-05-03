#!/usr/bin/env bash
# build-sidecars.sh —— 把 server / client 两个 Python sidecar 打成
# Tauri externalBin 期望的单二进制。
#
# 输出:
#   server-app/src-tauri/binaries/conduit-server-sidecar-<triple>
#   client-app/src-tauri/binaries/conduit-client-sidecar-<triple>
#
# Tauri externalBin 命名约定: <name>-<rustc-target-triple>
#   macOS arm64  -> aarch64-apple-darwin
#   macOS x64    -> x86_64-apple-darwin
#   Windows x64  -> x86_64-pc-windows-msvc (生成时加 .exe)
#   Linux x64    -> x86_64-unknown-linux-gnu
#
# 依赖:
#   - Python 3.10+ + pip
#   - PyInstaller >= 6.0  (脚本会自动安装到当前 venv;若不在 venv 里会
#     跑 pip install --user --upgrade pyinstaller)
#
# 跑法:
#   ./scripts/build-sidecars.sh           # 打当前平台 server + client
#   ./scripts/build-sidecars.sh server    # 只打 server
#   ./scripts/build-sidecars.sh client    # 只打 client
#
# 注:
#   - 跨平台编译不支持。要打 Windows / Linux 就在那台机器上跑。
#   - macOS 上若需要同时支持 Intel + Apple Silicon,在两台机器上跑
#     再合并到 universal binary (lipo -create);打包脚本暂不自动做,
#     避免拖慢迭代。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# ---------- 平台 → rustc target triple ----------
detect_triple() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin)
      case "$arch" in
        arm64) echo "aarch64-apple-darwin" ;;
        x86_64) echo "x86_64-apple-darwin" ;;
        *) echo "unsupported-darwin-$arch" >&2; exit 1 ;;
      esac
      ;;
    Linux)
      case "$arch" in
        x86_64) echo "x86_64-unknown-linux-gnu" ;;
        aarch64) echo "aarch64-unknown-linux-gnu" ;;
        *) echo "unsupported-linux-$arch" >&2; exit 1 ;;
      esac
      ;;
    MINGW*|MSYS*|CYGWIN*)
      echo "x86_64-pc-windows-msvc"
      ;;
    *) echo "unsupported-os-$os" >&2; exit 1 ;;
  esac
}

TRIPLE="$(detect_triple)"
EXE_SUFFIX=""
case "$TRIPLE" in
  *windows*) EXE_SUFFIX=".exe" ;;
esac

echo "→ target triple: $TRIPLE"

# ---------- 确保 PyInstaller 可用 ----------
ensure_pyinstaller() {
  if python3 -c "import PyInstaller" >/dev/null 2>&1; then
    return
  fi
  echo "→ PyInstaller not found, installing…"
  if [[ -n "${VIRTUAL_ENV:-}" ]] || [[ -n "${PYENV_VERSION:-}" ]]; then
    pip install --upgrade pyinstaller
  else
    pip install --user --upgrade pyinstaller
  fi
}

# ---------- 单个 app 打包 ----------
# args: <app_name>  e.g. server / client
build_one() {
  local app="$1"
  local entry script_basename outdir target_path tauri_binaries_dir

  case "$app" in
    server)
      entry="server-app/core/proxy_server.py"
      script_basename="conduit-server-sidecar"
      tauri_binaries_dir="server-app/src-tauri/binaries"
      ;;
    client)
      entry="client-app/core/client_main.py"
      script_basename="conduit-client-sidecar"
      tauri_binaries_dir="client-app/src-tauri/binaries"
      ;;
    *)
      echo "✗ unknown app: $app" >&2
      exit 1
      ;;
  esac

  if [[ ! -f "$entry" ]]; then
    echo "✗ entry not found: $entry" >&2
    exit 1
  fi

  echo ""
  echo "═══ building $app sidecar ═══"
  echo "  entry: $entry"

  outdir="build/sidecars/$app"
  rm -rf "$outdir"
  mkdir -p "$outdir"

  # PyInstaller 单文件模式 + 显式 hidden import (zeroconf 内部动态加载较多模块)
  python3 -m PyInstaller \
    --onefile \
    --noconfirm \
    --clean \
    --name "$script_basename" \
    --distpath "$outdir/dist" \
    --workpath "$outdir/work" \
    --specpath "$outdir/spec" \
    --hidden-import "zeroconf._handlers.answers" \
    --hidden-import "zeroconf._utils.ipaddress" \
    --hidden-import "zeroconf._utils.name" \
    --hidden-import "zeroconf._utils.net" \
    --hidden-import "zeroconf._utils.time" \
    --hidden-import "aiohttp.resolver" \
    --paths "$(dirname "$entry")" \
    "$entry"

  local built="$outdir/dist/${script_basename}${EXE_SUFFIX}"
  if [[ ! -f "$built" ]]; then
    echo "✗ build failed: $built not found" >&2
    exit 1
  fi

  mkdir -p "$tauri_binaries_dir"
  target_path="${tauri_binaries_dir}/${script_basename}-${TRIPLE}${EXE_SUFFIX}"
  cp "$built" "$target_path"
  chmod +x "$target_path"

  local size_mb
  size_mb=$(du -m "$target_path" | cut -f1)
  echo "✓ $target_path  (${size_mb} MB)"
}

# ---------- 入口 ----------
ensure_pyinstaller

if [[ $# -eq 0 ]]; then
  build_one server
  build_one client
else
  for app in "$@"; do
    build_one "$app"
  done
fi

echo ""
echo "═══ done ═══"
echo "下一步:"
echo "  - 在 server-app/src-tauri/tauri.conf.json 的 bundle 里加 externalBin"
echo "  - sidecar.rs 改成在 release 模式调 ../binaries/conduit-*-sidecar-<triple>"
echo "  - 跑 pnpm tauri build 生成 .app / .dmg"
