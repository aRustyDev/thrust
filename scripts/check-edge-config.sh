#!/bin/sh
# The edge's configuration check (ADR-0111, issue #195): `thrust-edge` is a routing Worker in front of production, so its wrangler
# configuration may hold only what routing needs.
#
#   scripts/check-edge-config.sh [dir]     check every wrangler*.toml in dir (default: crates/edge)
#   scripts/check-edge-config.sh --self-test
#                                          run the check against fixtures it must accept and fixtures it must refuse
#
# For every `wrangler*.toml` it fails on
#   * a `main` other than `build/worker/shim.mjs` (worker-build's file; no hand-written entry in front of it);
#   * `workers_dev` or `preview_urls` not set to `false` at the top level: the edge is reached on the shared hostname only, and no
#     version of it may be public at a workers.dev URL;
#   * a top-level key other than name, main, compatibility_date, compatibility_flags, workers_dev and preview_urls (a `route`,
#     `routes`, `kv_namespaces` or any other binding written inline is caught here);
#   * a table other than `[[services]]`, `[[flagship]]`, `[version_metadata]`, `[vars]` and `[secrets]`: an allowlist, so every
#     data binding, assets, placement, a trigger or a route is refused, including kinds Cloudflare adds later;
#   * anything but exactly one `[[services]]` (production), exactly one `[[flagship]]` and exactly one `[version_metadata]`:
#     without the last the edge cannot say which build of it answers (`x-thrust-edge-version`), so no deploy of it could be
#     told from the one it replaced.
# A commented-out line does not count. A check that finds no config to look at would pass vacuously, so that is an error too;
# `--self-test` is the control, run next to the real check in CI.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SHIM="build/worker/shim.mjs"
TOP_KEYS="name main compatibility_date compatibility_flags workers_dev preview_urls"
TABLES="[[services]] [[flagship]] [version_metadata] [vars] [secrets]"

# top_value FILE KEY: the value of a top-level key (before the first table header), without quotes or a trailing comment
top_value() {
  awk -v k="$2" '/^[[:space:]]*\[/ { exit } $0 ~ "^[[:space:]]*" k "[[:space:]]*=" { print; exit }' "$1" |
    sed -e 's/^[^=]*=[[:space:]]*//' -e 's/[[:space:]]*#.*$//' -e 's/^"\(.*\)"$/\1/'
}

check_dir() {
  dir="$1"
  status=0
  found=0
  for cfg in "$dir"/wrangler*.toml; do
    [ -e "$cfg" ] || continue
    found=$((found + 1))
    name="$(basename "$cfg")"
    echo "=== $name ==="
    main="$(top_value "$cfg" main)"
    if [ "$main" = "$SHIM" ]; then echo "ok: main is $SHIM"; else echo "FORBIDDEN: $name: main is '${main:-<none>}', not $SHIM"; status=1; fi
    for key in workers_dev preview_urls; do
      v="$(top_value "$cfg" "$key")"
      if [ "$v" = false ]; then echo "ok: $key = false"; else echo "FORBIDDEN: $name: $key is '${v:-<unset>}', must be false"; status=1; fi
    done
    # top-level keys: the lines before the first table header that assign something
    keys="$(awk '/^[[:space:]]*\[/ { exit } /^[[:space:]]*[A-Za-z0-9_.-]+[[:space:]]*=/ { sub(/^[[:space:]]*/, ""); sub(/[[:space:]]*=.*/, ""); print }' "$cfg")"
    for k in $keys; do
      case " $TOP_KEYS " in *" $k "*) ;; *) echo "FORBIDDEN: $name: top-level key '$k' is not one the edge may have"; status=1 ;; esac
    done
    # tables: every header must be on the allowlist
    headers="$(grep -E '^[[:space:]]*\[' "$cfg" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*#.*$//' -e 's/[[:space:]]//g')"
    for h in $headers; do
      case " $TABLES " in *" $h "*) ;; *) echo "FORBIDDEN: $name: table $h is not one the edge may have (no data, assets, placement, trigger or route)"; status=1 ;; esac
    done
    for t in '[[services]]' '[[flagship]]' '[version_metadata]'; do
      n="$(echo "$headers" | grep -cxF "$t")"
      if [ "$n" -eq 1 ]; then echo "ok: one $t"; else echo "FORBIDDEN: $name has $n $t blocks, must have exactly one"; status=1; fi
    done
  done
  if [ "$found" -eq 0 ]; then
    echo "FORBIDDEN: no wrangler*.toml in $dir: nothing to check"
    status=1
  fi
  return "$status"
}

self_test() {
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  failed=0
  # expect pass|fail LABEL [GREP]: a refusal must also say why (a fixture refused for the wrong reason does not count)
  expect() {
    want="$1"; label="$2"; pattern="${3:-}"
    out="$(check_dir "$tmp/$label" 2>&1)"
    got=$?
    if [ "$want" = pass ] && [ "$got" -eq 0 ]; then
      echo "self-test ok: $label is accepted"
    elif [ "$want" = fail ] && [ "$got" -ne 0 ] && echo "$out" | grep -q -e "$pattern"; then
      echo "self-test ok: $label is refused ($pattern)"
    else
      echo "SELF-TEST FAILED: $label should $want${pattern:+ with '$pattern'}, got exit $got:"
      echo "$out" | sed 's/^/    /'
      failed=1
    fi
  }
  # fixture LABEL: a directory holding a copy of the real template with the changes the shell snippet on stdin makes
  good='name = "e"
main = "build/worker/shim.mjs"
compatibility_date = "2026-11-01"
workers_dev = false
preview_urls = false   # no public version URL

[[services]]
binding = "THRUST"
service = "thrust"

[[flagship]]
binding = "FLAGS"
app_id = "x"

[version_metadata]
binding = "CF_VERSION_METADATA"

[vars]
SHARED_ORIGIN = "https://thrust.example"

[secrets]
required = ["EDGE_PIN_KEY"]
'
  mk() { mkdir -p "$tmp/$1"; printf '%s' "$2" > "$tmp/$1/wrangler.toml"; }
  mk good "$good"
  mkdir -p "$tmp/real"; cp "$ROOT/crates/edge/wrangler.deploy.toml" "$tmp/real/wrangler.deploy.toml"
  mk commented "$good# [[hyperdrive]]
#route = \"x\"
"
  mk workers_dev "$(printf '%s' "$good" | sed 's/^workers_dev = false/workers_dev = true/')
"
  mk no_preview_urls "$(printf '%s' "$good" | sed '/^preview_urls/d')
"
  mk entry "$(printf '%s' "$good" | sed 's|^main = .*|main = "entry.mjs"|')
"
  mk hyperdrive "$good
[[hyperdrive]]
binding = \"HYPERDRIVE\"
id = \"x\"
"
  mk r2 "$good
[[r2_buckets]]
binding = \"B\"
bucket_name = \"b\"
"
  mk assets "$good
[assets]
directory = \".assets\"
"
  mk placement "$good
[placement]
region = \"aws:us-east-1\"
"
  mk routes "$good
[[routes]]
pattern = \"thrust.example\"
custom_domain = true
"
  mk inline_route "route = \"thrust.example/*\"
$good"
  mk two_services "$good
[[services]]
binding = \"OTHER\"
service = \"other\"
"
  mk no_flagship "$(printf '%s' "$good" | sed '/^\[\[flagship\]\]/,/^app_id/d')
"
  mk no_version_metadata "$(printf '%s' "$good" | sed '/^\[version_metadata\]/,/^binding = "CF_VERSION_METADATA"/d')
"
  mk two_version_metadata "$good
[version_metadata]
binding = \"OTHER\"
"
  mk unknown_table "$good
[observability]
enabled = true
"
  mkdir -p "$tmp/second"; printf '%s' "$good" > "$tmp/second/wrangler.toml"; printf '%s\n[[d1_databases]]\nbinding = "D"\n' "$good" > "$tmp/second/wrangler.stage.toml"
  mkdir -p "$tmp/empty"

  expect pass good
  expect pass real
  expect pass commented
  expect fail workers_dev "workers_dev is 'true', must be false"
  expect fail no_preview_urls "preview_urls is '<unset>', must be false"
  expect fail entry "main is 'entry.mjs'"
  expect fail hyperdrive 'table \[\[hyperdrive\]\]'
  expect fail r2 'table \[\[r2_buckets\]\]'
  expect fail assets 'table \[assets\]'
  expect fail placement 'table \[placement\]'
  expect fail routes 'table \[\[routes\]\]'
  expect fail inline_route "top-level key 'route'"
  expect fail two_services 'has 2 \[\[services\]\] blocks'
  expect fail no_flagship 'has 0 \[\[flagship\]\] blocks'
  expect fail no_version_metadata 'has 0 \[version_metadata\] blocks'
  expect fail two_version_metadata 'has 2 \[version_metadata\] blocks'
  expect fail unknown_table 'table \[observability\]'
  expect fail second 'wrangler.stage.toml: table \[\[d1_databases\]\]'
  expect fail empty "nothing to check"
  [ "$failed" -eq 0 ] && echo "self-test passed"
  return "$failed"
}

case "${1:-}" in
  --self-test)
    self_test
    exit $?
    ;;
  *)
    check_dir "${1:-$ROOT/crates/edge}"
    STATUS=$?
    [ "$STATUS" -eq 0 ] && echo "ok: the edge routes and binds nothing else"
    exit "$STATUS"
    ;;
esac
