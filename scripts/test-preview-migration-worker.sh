#!/usr/bin/env bash
# Opt-in Worker-runtime test: a fake, unreachable localhost database; no remote resources.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
worker="$root/crates/preview-migration-executor"
build_bin="${PREVIEW_WORKER_BUILD:-worker-build}"
wrangler_bin="${PREVIEW_WRANGLER:-wrangler}"
python_bin="${PREVIEW_PYTHON:-python3}"
tmp="$(mktemp -d)"
pid=""
cleanup() {
  if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  rm -rf "$tmp"
}
trap cleanup EXIT

# Verify the checked-in deployment template remains unreachable and disabled.
compatibility_date="$("$python_bin" -I - "$worker/wrangler.toml" <<'PY'
import sys, tomllib
with open(sys.argv[1], 'rb') as f:
    cfg = tomllib.load(f)
assert cfg['workers_dev'] is False and cfg['preview_urls'] is False
assert not cfg.get('route') and not cfg.get('routes') and not cfg.get('triggers')
assert cfg['vars']['EXECUTOR_ENABLED'] == 'false'
assert all(cfg['vars'][k] == '' for k in ('PREVIEW_NUMBER', 'PREVIEW_COMMIT', 'PREVIEW_DATABASE'))
assert cfg['hyperdrive'] == [{'binding': 'HYPERDRIVE', 'id': '0' * 32}]
print(cfg['compatibility_date'])
PY
)"
(cd "$worker" && "$root/scripts/rs" "$build_bin" --release .)

# A test-only config avoids rebuilding on every local start; the URL can never reach a real DB.
cat > "$tmp/wrangler.toml" <<EOF
name = "preview-executor-test"
main = "$worker/build/worker/shim.mjs"
compatibility_date = "$compatibility_date"
workers_dev = false
preview_urls = false
[[hyperdrive]]
binding = "HYPERDRIVE"
id = "00000000000000000000000000000000"
localConnectionString = "postgres://pr_1:invalid@127.0.0.1:1/thrust"
EOF
port="$("$python_bin" -I -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')"
url="http://127.0.0.1:$port"
sha=0123456789abcdef0123456789abcdef01234567
start() {
  local enabled="$1" number="$2"
  "$wrangler_bin" dev --config "$tmp/wrangler.toml" --local --port "$port" --ip 127.0.0.1 \
    --inspector-port 0 --show-interactive-dev-session=false \
    --var "EXECUTOR_ENABLED:$enabled" --var "PREVIEW_NUMBER:$number" \
    --var "PREVIEW_COMMIT:$sha" --var "PREVIEW_DATABASE:thrust" > "$tmp/wrangler.log" 2>&1 &
  pid=$!
  for _ in {1..120}; do
    if [[ "$(curl -s -o /dev/null -w '%{http_code}' "$url/other" || true)" == 404 ]]; then return; fi
    if ! kill -0 "$pid" 2>/dev/null; then cat "$tmp/wrangler.log" >&2; return 1; fi
    sleep 0.25
  done
  cat "$tmp/wrangler.log" >&2
  return 1
}
stop() { kill "$pid"; wait "$pid" 2>/dev/null || true; pid=""; }
expect() {
  local want="$1"; shift
  local got
  got="$(curl -s -o /dev/null -w '%{http_code}' "$@")"
  if [[ "$got" != "$want" ]]; then echo "expected HTTP $want, got $got" >&2; cat "$tmp/wrangler.log" >&2; exit 1; fi
}
start false 1
expect 503 -X POST --data '{}' "$url/v1/migrate"
expect 404 "$url/other"
stop
start true 2
# Control: if the binding check is removed this malformed artifact reaches parsing and returns 400.
expect 503 -X POST --data '{}' "$url/v1/migrate"
stop
start true 1
expect 400 -X POST "$url/v1/migrate"
expect 400 -X POST --data '{}' "$url/v1/migrate"
expect 503 -X POST --data-binary "@$root/crates/adapters/postgres/fuzz/seeds/preview_artifact/valid.json" "$url/v1/migrate"
echo "preview migration Worker: closed, binding mismatch, malformed artifact and unavailable database verified"
