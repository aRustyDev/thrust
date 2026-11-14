#!/bin/sh
# .github/scripts/verify-bundle.sh against a fake `gh` that records what it was asked and answers as a file says (issue #49): the
# verification must ask for every condition (this repository, its deploy.yml, main, no self-hosted runner), and a refusal by gh must be a
# refusal. The fake verifies a bundle only if the bundle says so, so a script that stopped asking would be found by the flags it left out.
#
#   sh .github/scripts/tests/test-verify-bundle.sh
#
# VERIFY_SCRIPT points the test at a copy of the script, so a deliberately broken copy can be shown to fail it.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
S="${VERIFY_SCRIPT:-$HERE/.github/scripts/verify-bundle.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
mkdir -p "$T/bin"
cat > "$T/bin/gh" <<'EOF'
#!/bin/sh
# fake gh: records its arguments; `attestation verify <file>` succeeds only when <file> contains the word "attested"
echo "$*" >> "$FIX/gh.calls"
[ "$1 $2" = "attestation verify" ] || { echo "fake gh: unexpected: $*" >&2; exit 99; }
grep -q attested "$3" || { echo "gh: no attestations found" >&2; exit 1; }
EOF
chmod +x "$T/bin/gh"
FIX="$T/fix"; mkdir -p "$FIX"
pass=0; bad=0
printf 'attested bundle\n' > "$T/good.tar.gz"
printf 'forged bundle\n' > "$T/forged.tar.gz"

run() { # run <label> <want-exit> <want-fragment> <bundle> <repo>
  label="$1"; want="$2"; frag="$3"; shift 3
  rm -f "$FIX/gh.calls"
  PATH="$T/bin:$PATH" FIX="$FIX" sh "$S" "$@" > "$T/out.txt" 2>&1
  got=$?
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$T/out.txt"; then
    pass=$((pass + 1)); echo "ok   $label (exit $got; saw '$frag')"
  else
    bad=$((bad + 1)); echo "BAD  $label: wanted exit $want + '$frag', got exit $got"; sed 's/^/     | /' "$T/out.txt"
  fi
}
asked() { # asked <label> <flag text that must be in the call>
  if grep -qF -- "$2" "$FIX/gh.calls" 2> /dev/null; then pass=$((pass + 1)); echo "ok   it asked for $1"; else bad=$((bad + 1)); echo "BAD  it did not ask for $1 ('$2'): $(cat "$FIX/gh.calls" 2> /dev/null)"; fi
}

run "an attested bundle" 0 "is attested by o/r/.github/workflows/deploy.yml on refs/heads/main" "$T/good.tar.gz" o/r
asked "this repository" "--repo o/r"
asked "this repository's deploy workflow as the signer" "--signer-workflow o/r/.github/workflows/deploy.yml"
asked "main as the source ref" "--source-ref refs/heads/main"
asked "no self-hosted runner" "--deny-self-hosted-runners"
if grep -q -- "--source-digest" "$FIX/gh.calls"; then bad=$((bad + 1)); echo "BAD  it asked for --source-digest, which a workflow_run cannot satisfy"; else pass=$((pass + 1)); echo "ok   it did not ask for --source-digest"; fi

run "a bundle gh refuses (a forged artifact has no attestation)" 1 "NOT VERIFIED" "$T/forged.tar.gz" o/r

run "no arguments" 2 "usage"
run "a bundle that is not there" 2 "is not a file" "$T/nope.tar.gz" o/r
run "a repository that is not owner/repo" 2 "is not owner/repo" "$T/good.tar.gz" justname

# gh missing is a usage error and never a pass: no `gh` on the PATH at all
rm -f "$FIX/gh.calls"
# (an empty directory as the whole PATH: on a runner `gh` is in /usr/bin, so a PATH of /usr/bin:/bin would still find it)
mkdir -p "$T/empty"
env -i PATH="$T/empty" /bin/sh "$S" "$T/good.tar.gz" o/r > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ] && grep -q "gh is not installed" "$T/out.txt"; then pass=$((pass + 1)); echo "ok   no gh on the PATH is exit 2, not a pass"; else bad=$((bad + 1)); echo "BAD  no gh: exit $got"; sed 's/^/     | /' "$T/out.txt"; fi

echo "passed=$pass bad=$bad"
[ "$bad" = 0 ]
