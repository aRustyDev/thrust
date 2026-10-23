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
for bad in deadpool-postgres hyper hyper-util mio socket2 tower-http thrust-adapters-selfhosted thrust-server rusqlite libsqlite3-sys; do
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

echo
echo "=== proof minting (issue #35): Scope::mint only in authorize, SystemScope::mint only in the composition roots ==="
# thrust-core's unit-test module and the integration tests (crates/*/tests) may mint either; comment lines (doctests
# included) are skipped. A call spelled through an alias or a qualified path is not caught: the complete check is
# clippy's disallowed-methods, once the clippy job exists (issue #40).
MINT='(^|[^A-Za-z0-9_])(Scope|SystemScope)::mint\('
hits="$(
  awk -v f="crates/thrust-core/src/lib.rs" '
    /^#\[cfg\(test\)\]/ { cfgtest = 1; next }
    cfgtest && /^mod / { intests = 1 }
    { cfgtest = 0 }
    /^[ \t]*\/\// { next }
    match($0, /fn [a-z_0-9]+/) { fn = substr($0, RSTART + 3, RLENGTH - 3) }
    intests { next }
    /SystemScope::mint\(/ { print f ":" NR ":" $0; next }
    /(^|[^A-Za-z0-9_])Scope::mint\(/ && fn != "authorize" { print f ":" NR ":" $0 }
  ' "$ROOT/crates/thrust-core/src/lib.rs"
  grep -rnE "$MINT" "$ROOT/crates" --include='*.rs' \
    | sed "s|^$ROOT/||" \
    | grep -vE '^crates/[^/]+/tests/' \
    | grep -v '^crates/thrust-core/src/lib.rs:' \
    | grep -vE '^[^:]+:[0-9]+:[ \t]*//' \
    | grep -vE '^crates/(thrust-server/src/main\.rs|thrust-worker/src/lib\.rs|thrust-worker/src/memory\.rs|thrust-worker/src/app_do\.rs):[0-9]+:.*[^A-Za-z0-9_]SystemScope::mint\(' \
    || true
)"
if [ -n "$hits" ]; then
  echo "FORBIDDEN: a proof minted outside its owner"
  echo "$hits"
  STATUS=1
else
  echo "ok: Scope minted only in authorize, SystemScope only in the composition roots"
fi

exit "$STATUS"
