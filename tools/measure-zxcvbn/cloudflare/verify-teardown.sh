#!/bin/sh
# After a run of measure-cpu.sh: every thrust-spike-zxcvbn-* Worker must answer "does not exist" (code 10007), and no thrust-spike KV
# namespace may remain. Read-only. Any other answer (an error, a login problem) counts as NOT gone, so a broken check fails safe.
#
#   CLOUDFLARE_ACCOUNT_ID=<id> tools/measure-zxcvbn/cloudflare/verify-teardown.sh <number of rounds>
set -u
: "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID}"
export CLOUDFLARE_ACCOUNT_ID WRANGLER_SEND_METRICS=false NO_COLOR=1
gone=0
left=""
i=0
for w in thrust-spike-zxcvbn-tail $(while [ "$i" -lt "${1:?usage: verify-teardown.sh <number of rounds>}" ]; do i=$((i + 1)); printf 'thrust-spike-zxcvbn-r%02d ' "$i"; done); do
  if wrangler deployments list --name "$w" 2>&1 | grep -q 'code: 10007'; then gone=$((gone + 1)); else left="$left $w"; fi
done
echo "Workers checked: $((gone + $(echo $left | wc -w))); gone (10007): $gone; NOT gone:${left:- none}"
wrangler kv namespace list 2>/dev/null | jq -r '.[].title' | grep -c '^thrust-spike' | sed 's/^/thrust-spike KV namespaces left: /'
wrangler kv namespace list 2>/dev/null | jq 'length' | sed 's/^/KV namespaces in the account: /'
