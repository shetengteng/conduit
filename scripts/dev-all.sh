#!/usr/bin/env bash
# dev-all.sh — Spin up the entire Conduit dev stack on this machine.
#
# - server-app (Tauri shell + Python sidecar + Vue UI on Vite :1420)
# - client-app (Tauri shell + Python sidecar + Vue UI on Vite :1421)
#
# Uses `concurrently` to run both Tauri dev processes side-by-side.
# Each Tauri dev spawns its own beforeDevCommand (Vite) so we don't
# need to start Vite separately here.
#
# Usage:
#   ./scripts/dev-all.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v pnpm >/dev/null 2>&1; then
  echo "✗ pnpm not found; install via 'corepack enable && corepack prepare pnpm@9 --activate'" >&2
  exit 1
fi

if [[ ! -d node_modules ]]; then
  echo "→ first-time install"
  pnpm install
fi

exec pnpm dev:all
