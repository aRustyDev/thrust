#!/bin/sh
# ADR-0002 / ADR-0007 Confirmation: the dependency-graph check.
#
# core, ports, contracts (and the shared router, thrust-api) must not depend on any runtime or
# platform crate. Prints the evidence (full normal-dependency lists) and exits non-zero on a hit.
# Each forbidden crate is also checked with `cargo tree -i`, which is the authoritative query
# ("is this crate anywhere in the graph?"), on BOTH targets.
#
#   scripts/check-deps.sh
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MANIFEST="$ROOT/Cargo.toml"
FORBIDDEN="tokio tokio-util tokio-postgres mio socket2 hyper hyper-util tower-http worker worker-sys wasm-bindgen wasm-bindgen-futures js-sys web-sys deadpool-postgres lapin sqlx"
CLEAN_CRATES="thrust-contracts thrust-ports thrust-core thrust-api"
STATUS=0

for crate in $CLEAN_CRATES; do
  echo "=== $crate: normal dependencies (all targets) ==="
  "$ROOT/scripts/rs" cargo tree --manifest-path "$MANIFEST" -p "$crate" -e normal --target all --prefix none 2>&1 | sort -u
  for bad in $FORBIDDEN; do
    out="$("$ROOT/scripts/rs" cargo tree --manifest-path "$MANIFEST" -p "$crate" -e normal --target all -i "$bad" 2>&1)"
    case "$out" in
      *"did not match any packages"*|*"nothing to print"*|"") ;;
      *) echo "FORBIDDEN: $crate depends on $bad"; echo "$out" | head -5; STATUS=1 ;;
    esac
  done
done

echo
echo "=== adapters-memory (must also be runtime-free: it ships in the Worker and the server) ==="
for bad in $FORBIDDEN; do
  out="$("$ROOT/scripts/rs" cargo tree --manifest-path "$MANIFEST" -p thrust-adapters-memory -e normal --target all -i "$bad" 2>&1)"
  case "$out" in
    *"did not match any packages"*|*"nothing to print"*|"") ;;
    *) echo "FORBIDDEN: thrust-adapters-memory depends on $bad"; STATUS=1 ;;
  esac
done

echo
echo "=== who pulls tokio into the Worker bundle (wasm32, thrust-worker, cloudflare set only) ==="
"$ROOT/scripts/rs" cargo tree --manifest-path "$MANIFEST" -p thrust-worker --no-default-features --target wasm32-unknown-unknown -e normal -i tokio 2>&1 | head -30

echo
echo "=== native-only crates that must NOT be in the Worker bundle (wasm32 graph) ==="
for bad in deadpool-postgres hyper hyper-util mio socket2 tower-http thrust-adapters-selfhosted thrust-server; do
  out="$("$ROOT/scripts/rs" cargo tree --manifest-path "$MANIFEST" -p thrust-worker --no-default-features --target wasm32-unknown-unknown -e normal -i "$bad" 2>&1)"
  case "$out" in
    *"did not match any packages"*|*"nothing to print"*|"") echo "ok: $bad absent from the Worker graph" ;;
    *) echo "FORBIDDEN in Worker graph: $bad"; STATUS=1 ;;
  esac
done

echo
echo "=== worker-only crates that must NOT be in the native server graph ==="
for bad in worker worker-sys wasm-bindgen js-sys web-sys thrust-adapters-cloudflare thrust-worker; do
  out="$("$ROOT/scripts/rs" cargo tree --manifest-path "$MANIFEST" -p thrust-server -e normal -i "$bad" 2>&1)"
  case "$out" in
    *"did not match any packages"*|*"nothing to print"*|"") echo "ok: $bad absent from the server graph" ;;
    *) echo "FORBIDDEN in server graph: $bad"; STATUS=1 ;;
  esac
done

exit "$STATUS"
