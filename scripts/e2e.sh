#!/usr/bin/env bash
# e2e.sh —— Conduit Rust 重写版（v0.2+）的端到端冒烟测试。
#
# 与 v0.1 (Python sidecar) 时代的核心区别：
#   - server-app / client-app 都是 Tauri app，必须有 webview，无法 headless 启动
#   - 本脚本不再 spawn / kill 进程，只校验 control API 契约 + 真实 SOCKS5 流量
#
# v0.2 起依赖：bash + curl + jq（**不再依赖 python3**）
#   brew install jq
#
# ─────────────── 跑法 ───────────────
#   1. 在两个 terminal 分别起 dev：
#        pnpm --filter conduit-server tauri dev
#        pnpm --filter conduit-client tauri dev
#   2. 从 client-app 的 Rust 日志里找 `boot socks=... api=NNN`，
#      把 api 端口传给本脚本（server 端 api 端口由 client 通过 mDNS
#      自动发现，无需手动）：
#        CLIENT_API=NNN ./scripts/e2e.sh
#   3. 跳过外部 dev、只跑 headless 内核冒烟：
#        ./scripts/e2e.sh --headless-only
#
# ─────────────── 校验流程 ───────────────
#   step 0  conduit-core socks5_relay_smoke headless（无 webview，纯 Rust）
#   step 1  client /healthz 200
#   step 2  client /api/connection 拿到 idle + socks_port
#   step 3  client /api/servers 在 15s 内通过 mDNS 看到 server
#   step 4  server /healthz 200（端口由 step 3 给出）
#   step 5  server /api/status running=true
#   step 6  POST client /api/connect/<server_id> → state=connected
#   step 7  curl --socks5-hostname client_socks_port → server /healthz
#   step 8  client /api/traffic 累计字节 > 0
#   step 9  POST client /api/disconnect → state=idle
#
# 任何步骤失败立即 exit 1 + 打印调查命令。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

HEADLESS_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --headless-only) HEADLESS_ONLY=1 ;;
    -h|--help)
      sed -n '1,/^set -euo pipefail/p' "$0" | head -n -2
      exit 0
      ;;
    *)
      echo "✗ 未知参数: $arg" >&2
      echo "用法: $0 [--headless-only]" >&2
      exit 2
      ;;
  esac
done

usage_hint() {
  cat >&2 <<'EOF'
hint: 启动顺序 ——
  Term1: pnpm --filter conduit-server tauri dev
  Term2: pnpm --filter conduit-client tauri dev
两个 dev 都打印过 `boot ... api=NNN` 之后再跑：
  CLIENT_API=<client api port> ./scripts/e2e.sh
仅跑无 webview 的内核冒烟（不需要 dev / mDNS / 真实流量）：
  ./scripts/e2e.sh --headless-only
EOF
}

# ---------- helpers ----------
require() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "✗ 需要 $1 但没找到" >&2
    if [[ "$1" == "jq" ]]; then
      echo "  macOS: brew install jq" >&2
    fi
    exit 1
  }
}
require curl
require cargo

http_get() { curl -sS -m 5 "$1"; }
http_post() { curl -sS -m 5 -X POST "$1"; }

# ---------- step 0: headless 内核冒烟（纯 Rust，无 webview / 无外部 dev） ----------
echo "═══ step 0: conduit-core socks5_relay_smoke headless ═══"
if cargo run --quiet -p conduit-core --example socks5_relay_smoke --release 2>&1 | tail -8; then
  echo "  ✓ headless smoke PASS"
else
  echo "  ✗ headless smoke 失败 —— 不要继续跑外部 dev e2e" >&2
  exit 1
fi

if [[ "$HEADLESS_ONLY" -eq 1 ]]; then
  echo ""
  echo "═══ ✓ headless-only 模式 PASS ═══"
  exit 0
fi

# ----- 之后的 step 都需要外部 dev 在跑 + jq parse 控制 API 响应 -----
require jq

CLIENT_API="${CLIENT_API:-}"
if [[ -z "$CLIENT_API" ]]; then
  echo "✗ 需要环境变量 CLIENT_API（client-app dev 启动日志里 \`boot socks=... api=NNN\` 的 api 端口）" >&2
  usage_hint
  exit 1
fi
CLIENT_BASE="http://127.0.0.1:$CLIENT_API"

# ---------- step 1: client healthz ----------
echo ""
echo "═══ step 1: client /healthz ═══"
if ! http_get "$CLIENT_BASE/healthz" | jq -e '.ok == true' >/dev/null 2>&1; then
  echo "  ✗ client /healthz 没返回 ok" >&2
  echo "  curl -sS $CLIENT_BASE/healthz" >&2
  usage_hint
  exit 1
fi
echo "  ✓ client healthz ok"

# ---------- step 2: client connection idle ----------
echo ""
echo "═══ step 2: client /api/connection ═══"
CONN=$(http_get "$CLIENT_BASE/api/connection")
CLIENT_STATE=$(echo "$CONN" | jq -r '.state // ""')
CLIENT_SOCKS=$(echo "$CONN" | jq -r '.socks_port // 0')
echo "  state=$CLIENT_STATE socks_port=$CLIENT_SOCKS"
if [[ "$CLIENT_STATE" != "idle" ]]; then
  echo "  ⚠ client 不在 idle，先 disconnect 再继续" >&2
  http_post "$CLIENT_BASE/api/disconnect" >/dev/null || true
  sleep 1
fi
if [[ "$CLIENT_SOCKS" -le 0 ]]; then
  echo "  ✗ client 没有有效 socks_port" >&2
  exit 1
fi

# ---------- step 3: mDNS 发现 server ----------
echo ""
echo "═══ step 3: 等 client 通过 mDNS 发现 server (最多 15s) ═══"
SERVER_ID=""
SERVER_API_PORT=""
for ((i = 0; i < 30; i++)); do
  RESP=$(http_get "$CLIENT_BASE/api/servers" || true)
  read -r SERVER_ID SERVER_API_PORT < <(
    echo "$RESP" | jq -r '
      (.servers // [])
      | map(select(.source == "mdns" and ((.api_port // 0) > 0)))
      | first
      | if . == null then "" else "\(.server_id // "") \(.api_port // 0)" end
    '
  )
  if [[ -n "$SERVER_ID" && "$SERVER_API_PORT" -gt 0 ]]; then
    echo "  ✓ 发现 server_id=$SERVER_ID api_port=$SERVER_API_PORT"
    break
  fi
  sleep 0.5
done
if [[ -z "$SERVER_ID" ]]; then
  echo "  ✗ 15s 内未通过 mDNS 看到任何 server" >&2
  echo "  --- $CLIENT_BASE/api/servers ---" >&2
  http_get "$CLIENT_BASE/api/servers" >&2 || true
  echo >&2
  echo "  hint: 检查 server-app dev 是否在跑、macOS 本地网络权限、防火墙" >&2
  exit 1
fi
SERVER_BASE="http://127.0.0.1:$SERVER_API_PORT"

# ---------- step 4: server healthz ----------
echo ""
echo "═══ step 4: server /healthz ═══"
if ! http_get "$SERVER_BASE/healthz" | jq -e '.ok == true' >/dev/null 2>&1; then
  echo "  ✗ server /healthz 没返回 ok（端口=$SERVER_API_PORT）" >&2
  exit 1
fi
echo "  ✓ server healthz ok"

# ---------- step 5: server status running ----------
echo ""
echo "═══ step 5: server /api/status running ═══"
STATUS=$(http_get "$SERVER_BASE/api/status")
RUNNING=$(echo "$STATUS" | jq -r '.running // false')
HTTP_PORT=$(echo "$STATUS" | jq -r '.http_port // 0')
echo "  running=$RUNNING http_port=$HTTP_PORT"
[[ "$RUNNING" == "true" ]] || {
  echo "  ✗ server 不是 running 状态" >&2
  exit 1
}

# ---------- step 6: client connect ----------
echo ""
echo "═══ step 6: POST /api/connect/$SERVER_ID ═══"
# server_id 用 jq 做最简 percent-encode（jq @uri 对路径段够用）
ENCODED_ID=$(printf '%s' "$SERVER_ID" | jq -Rr @uri)
RESP=$(http_post "$CLIENT_BASE/api/connect/$ENCODED_ID")
echo "  resp: $RESP"
NEW_STATE=$(echo "$RESP" | jq -r '.state // ""')
if [[ "$NEW_STATE" != "connected" ]]; then
  echo "  ✗ /api/connect 没切到 connected (state=$NEW_STATE)" >&2
  exit 1
fi
echo "  ✓ client connected"

# ---------- step 7: SOCKS5 流量穿越 ----------
echo ""
echo "═══ step 7: 通过 client SOCKS5 ($CLIENT_SOCKS) 请求 server /healthz ═══"
TARGET="http://127.0.0.1:$SERVER_API_PORT/healthz"
for i in 1 2 3; do
  curl -sS --socks5-hostname "127.0.0.1:$CLIENT_SOCKS" \
    -m 5 -o /dev/null -w "  req${i}=%{http_code} size=%{size_download}B " \
    "$TARGET" || echo "  req${i}=FAIL"
done
echo
sleep 2

# ---------- step 8: traffic > 0 ----------
echo ""
echo "═══ step 8: client /api/traffic 累计字节 ═══"
TRAFFIC=$(http_get "$CLIENT_BASE/api/traffic")
DOWN=$(echo "$TRAFFIC" | jq -r '.total_downlink // .downlink // 0')
echo "  total_downlink=$DOWN bytes"
[[ "$DOWN" -ge 1 ]] || {
  echo "  ✗ client traffic 累计为 0" >&2
  exit 1
}

# ---------- step 9: disconnect ----------
echo ""
echo "═══ step 9: POST /api/disconnect ═══"
DC=$(http_post "$CLIENT_BASE/api/disconnect")
echo "  resp: $DC"
sleep 1
FINAL_STATE=$(http_get "$CLIENT_BASE/api/connection" | jq -r '.state // ""')
echo "  final state=$FINAL_STATE"
[[ "$FINAL_STATE" == "idle" ]] || {
  echo "  ✗ disconnect 后没回到 idle" >&2
  exit 1
}

echo ""
echo "═══ ✓ end-to-end smoke test PASSED ═══"
echo "  client_api=$CLIENT_API  server_api=$SERVER_API_PORT"
echo "  server_id=$SERVER_ID  http_port=$HTTP_PORT  socks_port=$CLIENT_SOCKS"
