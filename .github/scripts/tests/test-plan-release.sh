#!/bin/sh
# .github/scripts/plan-release.sh: "does this run of release.yml release, and what" over EVERY combination of event, ref and commit,
# with the expected answer written out here as a separate rule, not read from the script.
#
#   sh .github/scripts/tests/test-plan-release.sh
#
# PLAN_SCRIPT points the test at a copy of the script, so a deliberately broken copy can be shown to fail it.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
SCRIPT="${PLAN_SCRIPT:-$HERE/.github/scripts/plan-release.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; failn=0

GIT_CONFIG_GLOBAL=/dev/null; GIT_CONFIG_SYSTEM=/dev/null
export GIT_CONFIG_GLOBAL GIT_CONFIG_SYSTEM

R="$T/repo"
mkdir -p "$R"
cd "$R" || exit 1
git init -q -b main . || exit 1
git config user.name "plan test"; git config user.email "plan-test@example.invalid"; git config commit.gpgsign false; git config tag.gpgsign false
echo 1 > f; git add -A; git commit -q -m m1; M1="$(git rev-parse HEAD)"
echo 2 > f; git add -A; git commit -q -m m2; M2="$(git rev-parse HEAD)"
git checkout -q -b side "$M1"; echo side > g; git add -A; git commit -q -m s1; S1="$(git rev-parse HEAD)"
git checkout -q main
git update-ref refs/remotes/origin/main "$M2"
# release-plz makes annotated tags; a lightweight one must work the same
git tag -a -m "v0.1.0" v0.1.0 "$M2"
git tag v0.2.0 "$M1"
git tag v0.3.0 "$S1"
git tag v1/x "$M2"
git tag v "$M2"
git tag release-1 "$M2"

# plan <KEY=VALUE>...: the script's stdout and status, in a clean environment
plan() { env -i PATH="$PATH" HOME="$T" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null "$@" sh "$SCRIPT" 2> "$T/err"; }
field() { echo "$1" | grep "^$2=" | cut -d= -f2-; }
check() { # check <label> <got> <want>
  if [ "$2" = "$3" ]; then pass=$((pass + 1)); else failn=$((failn + 1)); echo "BAD  $1: got '$2', wanted '$3'"; fi
}
# the commit each tag names, stated here on its own
commit_of() {
  case "$1" in
    v0.1.0|v1/x|v|release-1) echo "$M2" ;;
    v0.2.0) echo "$M1" ;;
    v0.3.0) echo "$S1" ;;
    *) echo "" ;;
  esac
}

n=0
for event in workflow_dispatch pull_request push release schedule ""; do
  for ref in refs/tags/v0.1.0 refs/tags/v0.2.0 refs/tags/v0.3.0 refs/tags/v1/x refs/tags/v refs/tags/release-1 refs/tags/v9.9.9 \
             refs/heads/v0.1.0 refs/heads/main refs/heads/rc/x refs/pull/1/merge ""; do
    # short: hex, but not a whole sha
    for sha_name in M1 M2 S1 bad short none; do
      case "$sha_name" in
        M1) sha="$M1" ;; M2) sha="$M2" ;; S1) sha="$S1" ;; bad) sha=nope ;; short) sha="$(echo "$M2" | cut -c1-12)" ;; *) sha="" ;;
      esac
      n=$((n + 1))
      out="$(plan EVENT_NAME="$event" RUN_REF="$ref" GITHUB_SHA="$sha")"; rc=$?
      # the rules, stated here on their own (a branch named like a tag is a branch)
      want_rc=0; want=""; want_tag=""; want_sha=""
      sha_ok=yes; case "$sha_name" in bad|short|none) sha_ok=no ;; esac
      tag="${ref#refs/tags/}"
      case "$event" in
        "") want_rc=2 ;;
        pull_request) if [ "$sha_ok" = yes ]; then want=dry-run; want_sha="$sha"; else want_rc=2; fi ;;
        workflow_dispatch)
          if [ "$sha_ok" = no ]; then want_rc=2
          else
            case "$ref" in refs/tags/v0.1.0|refs/tags/v0.2.0|refs/tags/v0.3.0|refs/tags/v9.9.9) is_rel=yes ;; *) is_rel=no ;; esac
            if [ "$is_rel" = no ] || [ "$(commit_of "$tag")" != "$sha" ] || [ "$sha" = "$S1" ]; then want_rc=1
            else want=release; want_tag="$tag"; want_sha="$sha"
            fi
          fi ;;
        *) want=skip ;;
      esac
      label="($event,$ref,$sha_name)"
      check "$label exit" "$rc" "$want_rc"
      if [ "$want_rc" = 0 ]; then
        check "$label mode" "$(field "$out" mode)" "$want"
        check "$label tag" "$(field "$out" tag)" "$want_tag"
        check "$label sha" "$(field "$out" sha)" "$want_sha"
        check "$label has a reason" "$([ -n "$(field "$out" reason)" ] && echo yes)" yes
      else
        check "$label prints nothing on stdout when it does not plan" "$out" ""
        check "$label says why on stderr" "$([ -s "$T/err" ] && echo yes)" yes
      fi
    done
  done
done
echo "ok   $n combinations"

# the cases the matrix holds, named, so a reader sees them: the release of an annotated tag, and of a lightweight one
out="$(plan EVENT_NAME=workflow_dispatch RUN_REF=refs/tags/v0.1.0 GITHUB_SHA="$M2")"
check "an annotated tag on main releases" "$(field "$out" mode):$(field "$out" tag)" "release:v0.1.0"
out="$(plan EVENT_NAME=workflow_dispatch RUN_REF=refs/tags/v0.2.0 GITHUB_SHA="$M1")"
check "a lightweight tag on main releases" "$(field "$out" mode):$(field "$out" tag)" "release:v0.2.0"
# a tag off main is refused, and the reason says so
out="$(plan EVENT_NAME=workflow_dispatch RUN_REF=refs/tags/v0.3.0 GITHUB_SHA="$S1")"; rc=$?
check "a tag off main: exit" "$rc" 1
check "a tag off main: the reason" "$(grep -c 'not a commit of main' "$T/err")" 1
# with no main to compare with, it is a usage error, not a pass
out="$(plan EVENT_NAME=workflow_dispatch RUN_REF=refs/tags/v0.1.0 GITHUB_SHA="$M2" MAIN_REF=origin/not-a-branch)"; rc=$?
check "no main ref is a usage error" "$rc" 2
# main is what the caller says it is: with main at M1, M2 is not on it
out="$(plan EVENT_NAME=workflow_dispatch RUN_REF=refs/tags/v0.1.0 GITHUB_SHA="$M2" MAIN_REF="$M1")"; rc=$?
check "a commit after main's tip is refused" "$rc" 1

echo "passed=$pass bad=$failn"
[ "$failn" = 0 ]
