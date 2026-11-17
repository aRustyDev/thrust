#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/bin"
cat >"$tmp/bin/aws" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$@" >>"$TRACE"
printf '%s\n' "${TEST_PASSWORD:-0123456789abcdef0123456789abcdef}"
SH
cat >"$tmp/bin/kubectl" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$@" >>"$TRACE"
if [[ " $* " == *' get cluster '* ]]; then
  printf '%s' "${TEST_PRIMARY:-thrust-pg-dev-1}"
else
  IFS= read -r password
  IFS= read -r limit
  [[ "$password" == 0123456789abcdef0123456789abcdef && "$limit" == 8 ]]
  grep -q 'CREATE FUNCTION preview_admin.provision' || exit 1
fi
SH
chmod +x "$tmp/bin/aws" "$tmp/bin/kubectl"
export PATH="$tmp/bin:$PATH" TRACE="$tmp/trace"
bash "$here/bootstrap-provisioner.sh" >"$tmp/out"
grep -q 'thrust-pg-dev-1' "$tmp/trace"
if grep -q '0123456789abcdef0123456789abcdef' "$tmp/trace" "$tmp/out"; then echo 'password leaked' >&2; exit 1; fi
if TEST_PRIMARY=thrust-pg-1 bash "$here/bootstrap-provisioner.sh" >/dev/null 2>&1; then echo 'production primary accepted' >&2; exit 1; fi
if TEST_PASSWORD=bad bash "$here/bootstrap-provisioner.sh" >/dev/null 2>&1; then echo 'bad password accepted' >&2; exit 1; fi
echo 'bootstrap wrapper tests: PASS'
