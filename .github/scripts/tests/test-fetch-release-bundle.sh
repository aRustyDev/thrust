#!/bin/sh
# .github/scripts/fetch-release-bundle.sh against a fake `gh` that serves a release as files say (the release flow, PR 3). The point is
# what it will NOT take: a draft, a release that is not immutable, one with no bundle or two, a download GitHub's release attestation
# does not vouch for, a tag that is not a release tag. Each is shown next to the one it takes.
#
#   sh .github/scripts/tests/test-fetch-release-bundle.sh
#
# FETCH_RELEASE_SCRIPT points the test at a copy of the script, so a deliberately broken copy can be shown to fail it.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
S="${FETCH_RELEASE_SCRIPT:-$HERE/.github/scripts/fetch-release-bundle.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
FIX="$T/fix"; mkdir -p "$T/bin" "$FIX"
pass=0; bad=0

cat > "$T/bin/gh" <<'EOF'
#!/bin/sh
# fake gh: `release view` serves $FIX/release.json (fails if $FIX/view.fail); `release download` writes the bundle into --dir (fails if
# $FIX/download.fail, writes an empty file if $FIX/download.empty); `release verify-asset` passes unless $FIX/verify.fail.
echo "$*" >> "$FIX/gh.calls"
case "$1 $2" in
  "release view")
    [ -f "$FIX/view.fail" ] && { echo "release not found" >&2; exit 1; }
    cat "$FIX/release.json" ;;
  "release download")
    [ -f "$FIX/download.fail" ] && { echo "HTTP 502" >&2; exit 1; }
    dir=""; prev=""
    for a in "$@"; do [ "$prev" = "--dir" ] && dir="$a"; prev="$a"; done
    [ -n "$dir" ] || { echo "fake gh: no --dir" >&2; exit 99; }
    if [ -f "$FIX/download.empty" ]; then : > "$dir/deploy-bundle.tar.gz"; else echo bundle > "$dir/deploy-bundle.tar.gz"; fi ;;
  "release verify-asset")
    [ -f "$FIX/verify.fail" ] && { echo "no attestation matches" >&2; exit 1; }
    exit 0 ;;
  *) echo "fake gh: unexpected: $*" >&2; exit 99 ;;
esac
EOF
chmod +x "$T/bin/gh"

# release <tagName> <isDraft> <isImmutable> <asset names...>
release() {
  tag="$1"; draft="$2"; imm="$3"; shift 3
  assets=""; for a in "$@"; do assets="$assets{\"name\":\"$a\"},"; done
  printf '{"tagName":"%s","isDraft":%s,"isImmutable":%s,"assets":[%s]}' "$tag" "$draft" "$imm" "${assets%,}" > "$FIX/release.json"
}
reset() { rm -f "$FIX"/*; rm -rf "$T/dest"; }

run() { # run <label> <want-exit> <want-fragment> <tag> [repo]
  label="$1"; want="$2"; frag="$3"; tag="$4"; repo="${5:-o/r}"
  PATH="$T/bin:$PATH" FIX="$FIX" sh "$S" "$repo" "$tag" "$T/dest" > "$T/out.txt" 2>&1
  got=$?
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$T/out.txt"; then
    pass=$((pass + 1)); echo "ok   $label (exit $got; saw '$frag')"
  else
    bad=$((bad + 1)); echo "BAD  $label: wanted exit $want + '$frag', got exit $got"; sed 's/^/     | /' "$T/out.txt"
  fi
}
fact() { # fact <label> <command...>: a check that must hold
  label="$1"; shift
  if "$@"; then pass=$((pass + 1)); echo "ok   $label"; else bad=$((bad + 1)); echo "BAD  $label"; fi
}

# ---- the one it takes ---------------------------------------------------------------------------------------------------------------
reset; release v0.1.0 false true deploy-bundle.tar.gz sbom.spdx.json SHA256SUMS
run "a published, immutable release with its bundle" 0 "$T/dest/deploy-bundle.tar.gz" v0.1.0
fact "...the bundle is where it says" test -s "$T/dest/deploy-bundle.tar.gz"
fact "...it asked GitHub's release attestation about that file" grep -qF -- "release verify-asset v0.1.0 $T/dest/deploy-bundle.tar.gz --repo o/r" "$FIX/gh.calls"
fact "...it downloaded only the bundle" grep -qF -- "--pattern deploy-bundle.tar.gz" "$FIX/gh.calls"

# ---- what it refuses ------------------------------------------------------------------------------------------------------------------
reset; release v0.1.0 true true deploy-bundle.tar.gz
run "a draft" 1 "is a draft" v0.1.0
fact "...nothing was downloaded from a draft" sh -c "! grep -q 'release download' '$FIX/gh.calls'"
reset; release v0.1.0 false false deploy-bundle.tar.gz
run "a release that is not immutable" 1 "is not immutable" v0.1.0
reset; release v0.1.0 false null deploy-bundle.tar.gz
run "a release whose immutability is unknown" 1 "is not immutable" v0.1.0
reset; release v0.1.0 false true sbom.spdx.json
run "a release with no bundle" 1 "has 0 assets named" v0.1.0
reset; release v0.1.0 false true deploy-bundle.tar.gz deploy-bundle.tar.gz
run "a release with two bundles" 1 "has 2 assets named" v0.1.0
reset; release v0.2.0 false true deploy-bundle.tar.gz
run "a release that is not the tag asked for" 1 "is not v0.1.0" v0.1.0
reset; release v0.1.0 false true deploy-bundle.tar.gz; touch "$FIX/verify.fail"
run "a download GitHub's release attestation does not vouch for" 1 "is not v0.1.0's attested asset" v0.1.0
fact "...and the unvouched file is not left behind" test ! -e "$T/dest/deploy-bundle.tar.gz"
reset; release v0.1.0 false true deploy-bundle.tar.gz; touch "$FIX/view.fail"
run "a release that cannot be read" 1 "could not read the release" v0.1.0
reset; release v0.1.0 false true deploy-bundle.tar.gz; touch "$FIX/download.fail"
run "a download that fails" 1 "could not download" v0.1.0
reset; release v0.1.0 false true deploy-bundle.tar.gz; touch "$FIX/download.empty"
run "a download that is empty" 1 "empty or missing" v0.1.0
reset; printf 'not json' > "$FIX/release.json"
run "a release that is not JSON" 1 "refused" v0.1.0

# a tag that is not a release tag is refused before GitHub is asked anything
for tag in main v v1/x release-1 refs/tags/v0.1.0 xv1; do
  reset; release "$tag" false true deploy-bundle.tar.gz
  run "the tag '$tag'" 1 "is not a release tag" "$tag"
  fact "...gh was not asked about '$tag'" test ! -s "$FIX/gh.calls"
done

# ---- usage ----------------------------------------------------------------------------------------------------------------------------
reset
PATH="$T/bin:$PATH" FIX="$FIX" sh "$S" > "$T/out.txt" 2>&1; got=$?
fact "no arguments is a usage error" test "$got" = 2
run "a repository that is not owner/repo" 2 "is not owner/repo" v0.1.0 justname

echo "passed=$pass bad=$bad"
[ "$bad" = 0 ]
