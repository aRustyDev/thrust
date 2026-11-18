#!/bin/sh
# ADR-0002 / ADR-0007 Confirmation: the dependency-graph check.
#
# core, ports, contracts (and the shared router, thrust-api) must not depend on any runtime or
# platform crate. Prints the evidence (full normal-dependency lists) and exits non-zero on a hit.
# Each forbidden crate is also checked with `cargo tree -i`, which is the authoritative query
# ("is this crate anywhere in the graph?"), on BOTH targets.
#
# The ORM crates (ADR-0044: hand SQL on tokio-postgres, so no sqlx, sea-orm or diesel) are checked separately, over the
# WHOLE workspace: every member, every kind of edge, every feature, both targets.
#
#   scripts/check-deps.sh
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MANIFEST="$ROOT/Cargo.toml"
FORBIDDEN="tokio tokio-util tokio-postgres mio socket2 hyper hyper-util tower-http worker worker-sys wasm-bindgen wasm-bindgen-futures js-sys web-sys deadpool-postgres lapin sqlx lettre"
CLEAN_CRATES="thrust-contracts thrust-ports thrust-core thrust-html thrust-api thrust-plane-types thrust-api-v1 thrust-compat-plane"
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
echo "=== compatibility boundary: thrust-compat-plane stays outward ==="
compat_metadata="$("$ROOT/scripts/rs" cargo metadata --manifest-path "$MANIFEST" --format-version 1 --no-deps --offline 2>&1)"
if ! printf '%s' "$compat_metadata" | python3 -I -c '
import json
import sys

try:
    metadata = json.load(sys.stdin)
except ValueError:
    sys.exit(1)

members = set(metadata["workspace_members"])
sys.exit(not any(package["name"] == "thrust-compat-plane" and package["id"] in members for package in metadata["packages"]))
'; then
  echo "FORBIDDEN: thrust-compat-plane is not a workspace member"
  echo "$compat_metadata" | head -8
  STATUS=1
fi
for crate in thrust-contracts thrust-ports thrust-core; do
  out="$("$ROOT/scripts/rs" cargo tree --manifest-path "$MANIFEST" -p "$crate" -e normal --target all -i thrust-compat-plane 2>&1)"
  case "$out" in
    *"did not match any packages"*|*"nothing to print"*) echo "ok: thrust-compat-plane is not in $crate's graph" ;;
    *) echo "FORBIDDEN: $crate depends on thrust-compat-plane"; echo "$out" | head -8; STATUS=1 ;;
  esac
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
echo "=== adapters-oidc (it ships in both roots; openidconnect brings wasm-bindgen and js-sys on wasm32 through chrono and"
echo "    getrandom, so those two are allowed here and kept out of core. Everything else forbidden is not allowed.) ==="
OIDC_FORBIDDEN="tokio tokio-util tokio-postgres mio socket2 hyper hyper-util tower-http worker worker-sys deadpool-postgres lapin sqlx reqwest"
for bad in $OIDC_FORBIDDEN; do
  out="$("$ROOT/scripts/rs" cargo tree --manifest-path "$MANIFEST" -p thrust-adapters-oidc -e normal --target all -i "$bad" 2>&1)"
  case "$out" in
    *"did not match any packages"*|*"nothing to print"*|"") ;;
    *) echo "FORBIDDEN: thrust-adapters-oidc depends on $bad"; echo "$out" | head -5; STATUS=1 ;;
  esac
done
# the one thing it must not be: part of core's graph (core must stay free of wasm-bindgen)
out="$("$ROOT/scripts/rs" cargo tree --manifest-path "$MANIFEST" -p thrust-core -e normal --target all -i thrust-adapters-oidc 2>&1)"
case "$out" in
  *"did not match any packages"*|*"nothing to print"*|"") echo "ok: thrust-adapters-oidc is not in thrust-core's graph" ;;
  *) echo "FORBIDDEN: thrust-core depends on thrust-adapters-oidc"; STATUS=1 ;;
esac

echo
echo "=== ORM crates (ADR-0044, issue #40): none anywhere in the workspace, on any edge, in any feature set, on either target ==="
# The sections above check only some crates and only `-e normal`, which would pass a `[dev-dependencies] sqlx` or one in a
# crate not named above (adapters-postgres, -cloudflare, -selfhosted, the roots). This one asks the whole workspace:
# `--workspace`, `-e all` (normal, build and dev edges), `--all-features` (an optional dependency behind a feature) and
# `--target all`. cargo-deny's `[bans]` (deny.toml) is the second guard.
# Three answers: absent; present (the tree starts with the crate); or cargo could not answer (offline, a stale lock, a
# manifest error), which is NOT "absent" and fails with its own message so it is never misread as either of the others.
ORM_FORBIDDEN="sqlx sea-orm diesel diesel-async"
for bad in $ORM_FORBIDDEN; do
  out="$("$ROOT/scripts/rs" cargo tree --manifest-path "$MANIFEST" --workspace --all-features -e all --target all -i "$bad" 2>&1)"
  case "$out" in
    *"did not match any packages"*|*"nothing to print"*|"") echo "ok: $bad is not in the workspace graph" ;;
    *"$bad v"*) echo "FORBIDDEN: $bad is in the workspace graph (ADR-0044)"; echo "$out" | head -8; STATUS=1 ;;
    *) echo "ERROR: cargo tree could not say whether $bad is in the workspace graph:"; echo "$out" | head -8; STATUS=1 ;;
  esac
done

echo
echo "=== who pulls tokio into the Worker bundle (wasm32, thrust-worker, cloudflare set only) ==="
"$ROOT/scripts/rs" cargo tree --manifest-path "$MANIFEST" -p thrust-worker --no-default-features --target wasm32-unknown-unknown -e normal -i tokio 2>&1 | head -30

echo
echo "=== native-only crates that must NOT be in the Worker bundle (wasm32 graph) ==="
for bad in deadpool-postgres hyper hyper-util mio socket2 tower-http lettre thrust-adapters-selfhosted thrust-server; do
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
echo "=== proof minting (issues #35, #44): Scope::mint only in authorize_with_role (which authorize is), WorkspaceScope::mint only in authorize_workspace, SystemScope::mint only in the composition roots ==="
# thrust-core's unit-test module and each member's own integration tests (`<member directory>/tests/`) may mint either;
# comment lines (doctests included) are skipped. A call spelled through an alias or a qualified path is not caught: the
# complete check is clippy's disallowed-methods. The `lint` job (issue #40) runs clippy, but `disallowed-methods` is not
# configured yet.
# The member directories come from cargo metadata, so a member's tests are exempt at any depth under crates/ (ADR-0009). Not a
# pattern: `^crates/.*/tests/` would also exempt a `src/tests/` module, and the check would stop looking there.
MINT='(^|[^A-Za-z0-9_])(Scope|SystemScope|WorkspaceScope)::mint\('
MEMBER_DIRS="$(
  "$ROOT/scripts/rs" cargo metadata --manifest-path "$MANIFEST" --format-version 1 --no-deps --offline 2> /dev/null \
    | python3 -I -c 'import json, sys
try:
    m = json.load(sys.stdin)
except ValueError:
    sys.exit(0)  # cargo metadata failed: print nothing, and the empty list below fails the check
root = m["workspace_root"].rstrip("/") + "/"
for p in m["packages"]:
    if p["id"] in m["workspace_members"]:
        print(p["manifest_path"][len(root):-len("/Cargo.toml")])'
)"
if [ -z "$MEMBER_DIRS" ]; then
  echo "FAILED: cargo metadata listed no member directory, so no member's tests can be told apart"
  STATUS=1
fi
hits="$(
  awk -v f="crates/core/usecases/src/lib.rs" '
    /^#\[cfg\(test\)\]/ { cfgtest = 1; next }
    cfgtest && /^mod / { intests = 1 }
    { cfgtest = 0 }
    /^[ \t]*\/\// { next }
    match($0, /fn [a-z_0-9]+/) { fn = substr($0, RSTART + 3, RLENGTH - 3) }
    intests { next }
    /SystemScope::mint\(/ { print f ":" NR ":" $0; next }
    /WorkspaceScope::mint\(/ && fn != "authorize_workspace" { print f ":" NR ":" $0; next }
    /(^|[^A-Za-z0-9_])Scope::mint\(/ && fn != "authorize_with_role" { print f ":" NR ":" $0 }
  ' "$ROOT/crates/core/usecases/src/lib.rs"
  grep -rnE "$MINT" "$ROOT/crates" --include='*.rs' \
    | sed "s|^$ROOT/||" \
    | MEMBER_DIRS="$MEMBER_DIRS" awk '
        BEGIN { n = split(ENVIRON["MEMBER_DIRS"], d, "\n") }
        { for (i = 1; i <= n; i++) if (index($0, d[i] "/tests/") == 1) next; print }
      ' \
    | grep -v '^crates/core/usecases/src/lib.rs:' \
    | grep -vE '^[^:]+:[0-9]+:[ \t]*//' \
    | grep -vE '^crates/(server/src/main\.rs|worker/src/lib\.rs|worker/src/memory\.rs):[0-9]+:.*[^A-Za-z0-9_]SystemScope::mint\(' \
    || true
)"
if [ -n "$hits" ]; then
  echo "FORBIDDEN: a proof minted outside its owner"
  echo "$hits"
  STATUS=1
else
  echo "ok: Scope minted only in authorize_with_role, WorkspaceScope only in authorize_workspace, SystemScope only in the composition roots"
fi

exit "$STATUS"
