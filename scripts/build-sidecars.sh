#!/usr/bin/env bash
# build-sidecars.sh —— 把 server / client 两个 Python sidecar 打成
# PyInstaller --onedir 目录树。
#
# 为什么用 onedir 而不是 onefile：
#   onefile 在 macOS 上每次启动要把 24MB 二进制解压到 /tmp/_MEIxxx/，
#   叠加 Gatekeeper 的安全扫描，冷启动经常 30+ 秒，超出 healthz 超时。
#   onedir 直接保留解压后结构，启动 < 1 秒。
#   参考: https://pyinstaller.org/en/stable/common-issues-and-pitfalls.html
#
# 输出:
#   server-app/src-tauri/binaries-dir/conduit-server-sidecar/
#       ├── conduit-server-sidecar          (主二进制 launcher)
#       └── _internal/                      (libpython, deps, .so 等)
#   client-app/src-tauri/binaries-dir/conduit-client-sidecar/
#       └── 同上结构
#
# 这些目录会通过 tauri.conf.json 的 bundle.resources 一并打入 .app/.dmg。
# Rust 端 sidecar.rs 通过 app.path().resource_dir() 解析定位目录。
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
  local entry script_basename outdir built_dir tauri_binaries_dir target_dir

  case "$app" in
    server)
      entry="server-app/core/proxy_server.py"
      script_basename="conduit-server-sidecar"
      tauri_binaries_dir="server-app/src-tauri/binaries-dir"
      ;;
    client)
      entry="client-app/core/client_main.py"
      script_basename="conduit-client-sidecar"
      tauri_binaries_dir="client-app/src-tauri/binaries-dir"
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
  echo "═══ building $app sidecar (onedir) ═══"
  echo "  entry: $entry"

  outdir="build/sidecars/$app"
  rm -rf "$outdir"
  mkdir -p "$outdir"

  # PyInstaller onedir 模式 + 显式 hidden import (zeroconf 内部动态加载较多模块)
  python3 -m PyInstaller \
    --onedir \
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

  built_dir="$outdir/dist/${script_basename}"
  if [[ ! -d "$built_dir" ]] || [[ ! -f "$built_dir/${script_basename}${EXE_SUFFIX}" ]]; then
    echo "✗ build failed: ${built_dir}/${script_basename}${EXE_SUFFIX} not found" >&2
    exit 1
  fi

  # 把整个 onedir 输出（含 _internal/）替换到 tauri 的 binaries-dir 下
  target_dir="${tauri_binaries_dir}/${script_basename}"
  rm -rf "$target_dir"
  mkdir -p "$tauri_binaries_dir"
  cp -R "$built_dir" "$target_dir"
  chmod +x "$target_dir/${script_basename}${EXE_SUFFIX}"

  local size_mb
  size_mb=$(du -sm "$target_dir" | cut -f1)
  echo "✓ $target_dir  (${size_mb} MB, onedir)"
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
