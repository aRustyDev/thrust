#!/bin/sh
# .github/scripts/verify-bundle.sh against a fake `gh` that records what it was asked and answers as a file says (issue #49): the
# verification must ask for every condition (this repository, its deploy.yml, the exact ref, no self-hosted runner), a refusal by gh must
# be a refusal, and a ref that does not deploy (main, a feature branch, a pull request, rc/a/b) is refused before gh is asked. The fake verifies a bundle only if the bundle says so, so a script that stopped asking would be found by the flags it left out.
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

RC=refs/heads/rc/2026-11-15T07-33-32Z
TAG=refs/tags/v0.1.0
for ref in "$RC" refs/heads/rc/manual-1 "$TAG"; do
  # a release candidate is built by deploy.yml, a release by release.yml
  case "$ref" in refs/tags/*) signer=release.yml ;; *) signer=deploy.yml ;; esac
  run "an attested bundle from $ref" 0 "is attested by o/r/.github/workflows/$signer on $ref" "$T/good.tar.gz" o/r "$ref"
  asked "this repository" "--repo o/r"
  asked "$signer as the signer of $ref" "--signer-workflow o/r/.github/workflows/$signer"
  asked "exactly $ref as the source ref" "--source-ref $ref"
  asked "no self-hosted runner" "--deny-self-hosted-runners"
  if grep -q -- "--source-digest" "$FIX/gh.calls"; then bad=$((bad + 1)); echo "BAD  it asked for --source-digest"; else pass=$((pass + 1)); echo "ok   it did not ask for --source-digest"; fi
done

run "a bundle gh refuses (a forged artifact has no attestation)" 1 "NOT VERIFIED" "$T/forged.tar.gz" o/r "$RC"

# a ref that does not deploy is refused, and gh is never asked: even an attested bundle from it is not for an environment
for ref in refs/heads/main refs/heads/feat/x refs/heads/rc/a/b refs/heads/rc/ refs/tags/v0.1.0/x refs/tags/v refs/tags/release-1 \
    refs/pull/1/merge rc/2026-11-15T07-33-32Z v0.1.0 refs/heads/xrc/1 refs/tags/xv1; do
  run "a bundle from $ref" 1 "is not a ref that deploys" "$T/good.tar.gz" o/r "$ref"
  if [ -s "$FIX/gh.calls" ]; then bad=$((bad + 1)); echo "BAD  gh was asked about $ref"; else pass=$((pass + 1)); echo "ok   gh was not asked about $ref"; fi
done

run "no arguments" 2 "usage"
run "no source ref" 2 "usage" "$T/good.tar.gz" o/r
run "a bundle that is not there" 2 "is not a file" "$T/nope.tar.gz" o/r "$RC"
run "a repository that is not owner/repo" 2 "is not owner/repo" "$T/good.tar.gz" justname "$RC"

# gh missing is a usage error and never a pass: no `gh` on the PATH at all
rm -f "$FIX/gh.calls"
# (an empty directory as the whole PATH: on a runner `gh` is in /usr/bin, so a PATH of /usr/bin:/bin would still find it)
mkdir -p "$T/empty"
env -i PATH="$T/empty" /bin/sh "$S" "$T/good.tar.gz" o/r "$RC" > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ] && grep -q "gh is not installed" "$T/out.txt"; then pass=$((pass + 1)); echo "ok   no gh on the PATH is exit 2, not a pass"; else bad=$((bad + 1)); echo "BAD  no gh: exit $got"; sed 's/^/     | /' "$T/out.txt"; fi

echo "passed=$pass bad=$bad"
[ "$bad" = 0 ]
