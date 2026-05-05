#!/usr/bin/env bash
#
# bump-version.sh — 一键同步整个 monorepo 的版本号到指定版本。
#
# 触发的所有"产权方"(13 处文件):
#   - 7 个 package.json (root + 2 个 app + 2 个 ui + 2 个 src-tauri 已经被 Cargo 同步)
#   - 2 个 Cargo.toml + 触发 cargo update -p 同步 lock
#   - 2 个 tauri.conf.json
#   - 2 个 pyproject.toml
#
# 触发后,以下"消费者"会自动跟上(不需要手改):
#   - server-app/ui + client-app/ui 通过 vite.config.ts define 注入 __APP_VERSION__,
#     所有 UI 代码用 @/lib/appVersion 拿 APP_VERSION
#   - server-app/core + client-app/core 通过 _version.py 的 importlib.metadata 拿
#     pyproject 里的 version (打包阶段会嵌入)
#   - docs/index.html 在浏览器里 fetch GitHub /releases/latest 拿 tag,跟代码无关
#
# Usage:
#   scripts/bump-version.sh 0.1.2          # bump to 0.1.2 + cargo update + git status
#   scripts/bump-version.sh --dry-run 0.2.0  # 只展示会改的内容
#   scripts/bump-version.sh --check          # 检查现有所有版本号是否一致
#
# 一旦想发布:
#   git add -A && git commit -m "chore(release): bump to v$NEW"
#   git tag -a v$NEW -m "Conduit v$NEW"
#   git push origin main && git push origin v$NEW

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DRY_RUN=0
CHECK_ONLY=0

usage() {
  sed -n '2,28p' "$0"
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --check)   CHECK_ONLY=1; shift ;;
    -h|--help) usage ;;
    *) NEW="$1"; shift ;;
  esac
done

# Files we own: each line is "<path>|<sed-pattern>"
# sed-pattern uses |...| as delimiter so we can keep / clean.
FILES=(
  "package.json|^(  \"version\": \")[^\"]+(\".*)$"
  "server-app/package.json|^(  \"version\": \")[^\"]+(\".*)$"
  "client-app/package.json|^(  \"version\": \")[^\"]+(\".*)$"
  "server-app/ui/package.json|^(  \"version\": \")[^\"]+(\".*)$"
  "client-app/ui/package.json|^(  \"version\": \")[^\"]+(\".*)$"
  "server-app/src-tauri/Cargo.toml|^(version = \")[^\"]+(\".*)$"
  "client-app/src-tauri/Cargo.toml|^(version = \")[^\"]+(\".*)$"
  "server-app/src-tauri/tauri.conf.json|^(  \"version\": \")[^\"]+(\".*)$"
  "client-app/src-tauri/tauri.conf.json|^(  \"version\": \")[^\"]+(\".*)$"
  "server-app/core/pyproject.toml|^(version = \")[^\"]+(\".*)$"
  "client-app/core/pyproject.toml|^(version = \")[^\"]+(\".*)$"
)

# ---------- check ----------
extract_version() {
  local file="$1"
  case "$file" in
    *.json)
      grep -E '^\s*"version":' "$file" | head -1 \
        | sed -E 's/.*"version":[[:space:]]*"([^"]+)".*/\1/'
      ;;
    *.toml)
      grep -E '^\s*version[[:space:]]*=' "$file" | head -1 \
        | sed -E 's/.*version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/'
      ;;
  esac
}

collect_versions() {
  cd "$ROOT"
  local distinct=""
  for entry in "${FILES[@]}"; do
    file="${entry%%|*}"
    v=$(extract_version "$file")
    printf '  %-50s %s\n' "$file" "$v"
    distinct=$(printf '%s\n%s' "$distinct" "$v")
  done
  uniq=$(echo "$distinct" | sort -u | grep -v '^$' | wc -l | tr -d ' ')
  echo
  if [[ "$uniq" == "1" ]]; then
    echo "All ${#FILES[@]} files agree."
  else
    echo "WARNING: $uniq distinct versions detected. Run: scripts/bump-version.sh <target>"
    return 3
  fi
}

if [[ "$CHECK_ONLY" -eq 1 ]]; then
  echo "Current versions across owned files:"
  collect_versions
  exit 0
fi

# ---------- bump ----------
NEW="${NEW:-}"
if [[ -z "$NEW" ]]; then
  echo "error: missing target version" >&2
  usage
fi

if [[ ! "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][A-Za-z0-9.+-]+)?$ ]]; then
  echo "error: '$NEW' is not a valid semver string" >&2
  exit 2
fi

echo "Target version: $NEW"
[[ "$DRY_RUN" -eq 1 ]] && echo "(dry-run)"
echo

cd "$ROOT"

for entry in "${FILES[@]}"; do
  file="${entry%%|*}"
  pattern="${entry#*|}"

  if [[ ! -f "$file" ]]; then
    echo "skip (missing): $file"
    continue
  fi

  old=$(extract_version "$file")

  if [[ "$old" == "$NEW" ]]; then
    printf '  %-50s already %s\n' "$file" "$NEW"
    continue
  fi

  printf '  %-50s %s -> %s\n' "$file" "$old" "$NEW"

  if [[ "$DRY_RUN" -eq 0 ]]; then
    # GNU/BSD sed compatible in-place edit
    if [[ "$file" == *.json ]]; then
      sed -i.bak -E "s|(\"version\": \")[^\"]+\"|\1${NEW}\"|" "$file"
    else
      sed -i.bak -E "s|^(version = \")[^\"]+\"|\1${NEW}\"|" "$file"
    fi
    rm -f "${file}.bak"
  fi
done

if [[ "$DRY_RUN" -eq 0 ]]; then
  echo
  echo "Syncing _version.py fallback strings..."
  for vfile in server-app/core/_version.py client-app/core/_version.py; do
    if [[ -f "$vfile" ]]; then
      sed -i.bak -E "s|^_FALLBACK = \"[^\"]+\"|_FALLBACK = \"${NEW}\"|" "$vfile"
      rm -f "${vfile}.bak"
      printf '  %-50s _FALLBACK -> %s\n' "$vfile" "$NEW"
    fi
  done

  echo
  echo "Syncing Cargo lockfiles..."
  for crate in conduit-server conduit-client; do
    if [[ "$crate" == "conduit-server" ]]; then
      ( cd "$ROOT/server-app/src-tauri" && cargo update -p "$crate" 2>&1 | tail -2 )
    else
      ( cd "$ROOT/client-app/src-tauri" && cargo update -p "$crate" 2>&1 | tail -2 )
    fi
  done

  echo
  echo "Bump complete. Verify:"
  echo "  scripts/bump-version.sh --check"
  echo
  echo "If everything looks right:"
  echo "  git add -A && git commit -m \"chore(release): bump to v${NEW}\""
  echo "  git tag -a v${NEW} -m \"Conduit v${NEW}\""
  echo "  git push origin main && git push origin v${NEW}"
fi
