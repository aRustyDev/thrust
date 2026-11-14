#!/bin/sh
# The CA root the deploy trusts (issue #49): one certificate, and the one whose fingerprint is pinned here. A trust anchor changes only
# through a change that fails this test and has to be edited by a person, with the new fingerprint written next to the old.
#
#   sh .github/scripts/tests/test-deploy-ca.sh
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
PEM="$HERE/.github/deploy/cloudflare-origin-ca-rsa-root.pem"
# the Cloudflare Origin CA RSA root, as published at https://developers.cloudflare.com/ssl/static/origin_ca_rsa_root.pem (2026-11-14)
PINNED="D3:C7:E8:5C:91:70:7F:C0:A1:2A:BC:5D:88:26:67:47:AA:4F:A8:E7:B1:62:F6:33:FF:B3:C9:D9:89:94:76:20"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; bad=0

command -v openssl > /dev/null 2>&1 || { echo "BAD  openssl is not installed"; exit 1; }

fingerprint() { openssl x509 -in "$1" -noout -fingerprint -sha256 2> /dev/null | sed 's/^.*=//' | tr 'a-f' 'A-F'; }
count_certs() { grep -c 'BEGIN CERTIFICATE' "$1"; }
check() { # check <label> <got> <want>
  if [ "$2" = "$3" ]; then pass=$((pass + 1)); echo "ok   $1"; else bad=$((bad + 1)); echo "BAD  $1: got '$2', wanted '$3'"; fi
}

check "the file holds exactly one certificate" "$(count_certs "$PEM")" 1
check "its SHA-256 fingerprint is the published one" "$(fingerprint "$PEM")" "$PINNED"

# the controls: the same two checks, on a certificate that is NOT the pinned one, must disagree
openssl req -x509 -newkey rsa:2048 -nodes -keyout "$T/key.pem" -out "$T/other.pem" -days 1 -subj "/CN=an unrelated certificate" > /dev/null 2>&1
check "CONTROL: an unrelated certificate does not have the pinned fingerprint" "$([ "$(fingerprint "$T/other.pem")" = "$PINNED" ] && echo same || echo different)" different
cat "$PEM" "$T/other.pem" > "$T/two.pem"
check "CONTROL: a file with a second certificate appended is counted as two" "$(count_certs "$T/two.pem")" 2
printf 'not a certificate\n' > "$T/junk.pem"
check "CONTROL: a file that is not a certificate has no fingerprint" "$(fingerprint "$T/junk.pem")" ""

echo "passed=$pass bad=$bad"
[ "$bad" = 0 ]
