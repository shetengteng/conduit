#!/usr/bin/env bash
# publish-release-notes.sh —— 把 scripts/release-notes-v<version>.md 推送到
# GitHub Release 的 body，覆盖 GitHub Actions 自动生成的默认 release notes。
#
# 用法:
#   ./scripts/publish-release-notes.sh             # 默认 v0.1.0
#   ./scripts/publish-release-notes.sh v0.1.1      # 指定其它 tag
#
# 前置条件:
#   1. 装 gh CLI:           brew install gh
#   2. 登录:                gh auth login   (走 GitHub.com → HTTPS → 浏览器授权)
#   3. tag 已 push:          git push origin <tag>
#   4. GitHub Actions 已跑完,Release 已被自动创建 (本脚本会轮询等)
#   5. scripts/release-notes-<tag>.md 已存在
#
# 行为:
#   - 等 release 出现 (最多 20 分钟轮询),不出现就退出
#   - 用本地 .md 文件覆盖 release.body
#   - 不动 release 的 assets / draft / prerelease flag

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TAG="${1:-v0.1.0}"
NOTES_FILE="scripts/release-notes-${TAG}.md"
REPO="shetengteng/conduit"
TIMEOUT=1200   # 20 分钟
INTERVAL=20    # 每 20 秒查一次

# ---------- 前置检查 ----------
if ! command -v gh >/dev/null 2>&1; then
  echo "✗ gh CLI not installed. Install with: brew install gh" >&2
  exit 1
fi

if ! gh auth status >/dev/null 2>&1; then
  echo "✗ gh CLI not authenticated. Run: gh auth login" >&2
  exit 1
fi

if [[ ! -f "$NOTES_FILE" ]]; then
  echo "✗ release notes file not found: $NOTES_FILE" >&2
  exit 1
fi

echo "→ tag=$TAG  repo=$REPO  notes=$NOTES_FILE"

# ---------- 等 release 出现 ----------
elapsed=0
while true; do
  if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    echo "✓ release $TAG found"
    break
  fi
  if (( elapsed >= TIMEOUT )); then
    echo "✗ release $TAG not appeared after ${TIMEOUT}s — check $REPO actions tab" >&2
    exit 1
  fi
  echo "  waiting for release $TAG... (${elapsed}s / ${TIMEOUT}s)"
  sleep "$INTERVAL"
  elapsed=$(( elapsed + INTERVAL ))
done

# ---------- 覆盖 release.body ----------
echo "→ updating release notes..."
gh release edit "$TAG" --repo "$REPO" --notes-file "$NOTES_FILE"
echo "✓ release notes updated"

# ---------- 显示链接 ----------
URL="https://github.com/${REPO}/releases/tag/${TAG}"
echo ""
echo "═══ done ═══"
echo "Release page: $URL"
