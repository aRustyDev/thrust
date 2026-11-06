#!/usr/bin/env bash
# Spike 0004: LOCAL-ONLY dev servers.
#   ROLE=edge  (default)  edge Worker is primary (default :8787); Worker B is an auxiliary Worker reachable only
#                         through the service binding.
#   ROLE=space            the stand-in public `space` Worker alone (default :8788).
#
# Makes no remote Cloudflare calls: `--local`, all Cloudflare credentials removed from the environment,
# and an empty throwaway wrangler config dir so a `wrangler login` on this machine cannot be picked up.
#
#   ./dev.sh                              # edge + B on :8787
#   ROLE=space ./dev.sh                   # space Worker on :8788
#   PORT=8790 ./dev.sh --var POLICY_AUD:abc --var TEAM_DOMAIN:https://t.example --var JWKS_URL:http://127.0.0.1:8899/jwks
#
# Extra args are passed to `wrangler dev`.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
scratch="${SPIKE_SCRATCH:-$(mktemp -d)}"
role="${ROLE:-edge}"
mkdir -p "$scratch/xdg" "$scratch/state-$role"

export XDG_CONFIG_HOME="$scratch/xdg"
export WRANGLER_SEND_METRICS=false
unset CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID CLOUDFLARE_API_KEY CLOUDFLARE_EMAIL

case "$role" in
  edge)
    exec wrangler dev --local \
      --port "${PORT:-8787}" \
      --persist-to "$scratch/state-$role" \
      -c "$here/worker/edge/wrangler.jsonc" \
      -c "$here/worker/b/wrangler.jsonc" \
      "$@"
    ;;
  space)
    exec wrangler dev --local \
      --port "${PORT:-8788}" \
      --persist-to "$scratch/state-$role" \
      -c "$here/worker/space/wrangler.jsonc" \
      "$@"
    ;;
  *)
    echo "ROLE must be edge or space" >&2
    exit 2
    ;;
esac
