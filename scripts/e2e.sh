#!/usr/bin/env bash
# e2e.sh —— Conduit Rust 重写版（v0.2+）的端到端冒烟测试。
#
# 与 v0.1 (Python sidecar) 的核心区别：现在 server-app / client-app 都是
# 独立的 Tauri app，带 webview，无法 headless 启动。本脚本不再负责
# spawn / kill 进程，只校验两个 dev 实例的 control API 契约 + 一次真实
# SOCKS5 流量穿越。
#
# ─────────────── 跑法 ───────────────
#   1. 在两个 terminal 分别起 dev：
#        pnpm --filter conduit-server tauri dev
#        pnpm --filter conduit-client tauri dev
#   2. 从 client 端 dev 的 Rust 日志里找 `boot socks=... api=NNN` 一行，
#      把 api 端口传给本脚本（server 端 api_port 通过 client 的 mDNS
#      发现，无需手动）：
#        CLIENT_API=NNN ./scripts/e2e.sh
#
# ─────────────── 校验流程 ───────────────
#   1. client /healthz 200
#   2. client /api/connection 拿到 idle 状态
#   3. client /api/servers 在 15s 内通过 mDNS 看到 server
#   4. server /healthz 200（端口由步骤 3 给出的 api_port）
#   5. server /api/status running=true
#   6. POST client /api/connect/<server_id> → state=connected
#   7. 真实 SOCKS5 流量：curl --socks5 client_socks_port → server /healthz
#   8. client /api/traffic 累计字节 > 0
#   9. POST client /api/disconnect → state=idle
#
# 任何步骤失败立即 exit 1 + 打印调查命令。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CLIENT_API="${CLIENT_API:?需要环境变量 CLIENT_API（client-app dev 启动日志里 \`boot socks=... api=NNN\` 的 api 端口）}"
CLIENT_BASE="http://127.0.0.1:$CLIENT_API"

usage_hint() {
  cat >&2 <<'EOF'
hint: 启动顺序——
  Term1: pnpm --filter conduit-server tauri dev
  Term2: pnpm --filter conduit-client tauri dev
两个 dev 都打印过 `boot ... api=NNN` 之后再跑：
  CLIENT_API=<client api port> ./scripts/e2e.sh
EOF
}

# ---------- helpers ----------
require() {
  command -v "$1" >/dev/null 2>&1 || { echo "✗ 需要 $1 但没找到" >&2; exit 1; }
}
require curl
require python3

http_get() {
  curl -sS -m 5 "$1"
}
http_post() {
  curl -sS -m 5 -X POST "$1"
}

# ---------- step 1: client healthz ----------
echo "═══ step 1: client /healthz ═══"
if ! http_get "$CLIENT_BASE/healthz" | grep -q '"ok"'; then
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
CLIENT_STATE=$(echo "$CONN" | python3 -c "import sys,json; print(json.load(sys.stdin).get('state',''))")
CLIENT_SOCKS=$(echo "$CONN" | python3 -c "import sys,json; print(json.load(sys.stdin).get('socks_port',0))")
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
for ((i=0; i<30; i++)); do
  RESP=$(http_get "$CLIENT_BASE/api/servers" || true)
  read -r SERVER_ID SERVER_API_PORT < <(echo "$RESP" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    for s in d.get('servers', []):
        if s.get('source') == 'mdns' and s.get('api_port', 0) > 0:
            print(s.get('server_id', ''), s.get('api_port', 0))
            break
except Exception:
    pass
")
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
if ! http_get "$SERVER_BASE/healthz" | grep -q '"ok"'; then
  echo "  ✗ server /healthz 没返回 ok（端口=$SERVER_API_PORT）" >&2
  exit 1
fi
echo "  ✓ server healthz ok"

# ---------- step 5: server status running ----------
echo ""
echo "═══ step 5: server /api/status running ═══"
STATUS=$(http_get "$SERVER_BASE/api/status")
RUNNING=$(echo "$STATUS" | python3 -c "import sys,json; print(json.load(sys.stdin).get('running', False))")
HTTP_PORT=$(echo "$STATUS" | python3 -c "import sys,json; print(json.load(sys.stdin).get('http_port', 0))")
echo "  running=$RUNNING http_port=$HTTP_PORT"
[[ "$RUNNING" == "True" ]] || { echo "  ✗ server 不是 running 状态" >&2; exit 1; }

# ---------- step 6: client connect ----------
echo ""
echo "═══ step 6: POST /api/connect/$SERVER_ID ═══"
CONNECT_URL="$CLIENT_BASE/api/connect/$(python3 -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1], safe=''))" "$SERVER_ID")"
RESP=$(http_post "$CONNECT_URL")
echo "  resp: $RESP"
NEW_STATE=$(echo "$RESP" | python3 -c "import sys,json; print(json.load(sys.stdin).get('state',''))")
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
DOWN=$(echo "$TRAFFIC" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(d.get('total_downlink', d.get('downlink', 0)))
")
echo "  total_downlink=$DOWN bytes"
[[ "$DOWN" -ge 1 ]] || { echo "  ✗ client traffic 累计为 0" >&2; exit 1; }

# ---------- step 9: disconnect ----------
echo ""
echo "═══ step 9: POST /api/disconnect ═══"
DC=$(http_post "$CLIENT_BASE/api/disconnect")
echo "  resp: $DC"
sleep 1
FINAL_STATE=$(http_get "$CLIENT_BASE/api/connection" \
  | python3 -c "import sys,json; print(json.load(sys.stdin).get('state',''))")
echo "  final state=$FINAL_STATE"
[[ "$FINAL_STATE" == "idle" ]] || { echo "  ✗ disconnect 后没回到 idle" >&2; exit 1; }

echo ""
echo "═══ ✓ end-to-end smoke test PASSED ═══"
echo "  client_api=$CLIENT_API  server_api=$SERVER_API_PORT"
echo "  server_id=$SERVER_ID  http_port=$HTTP_PORT  socks_port=$CLIENT_SOCKS"
