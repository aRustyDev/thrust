#!/bin/sh
# The release-candidate bot (the rc/* release flow, .agents/plans/cicd-review/PLAN.md PR 1). After every push to main it keeps one
# branch, rc/vX.Y.Z, equal to main plus a single commit `chore(release): vX.Y.Z` that writes CHANGELOG.md and the workspace version,
# and one pull request from it to main. Merging that pull request is the release.
#
#   release-pr.sh prepare    in a checkout of main with tags, at MAIN_SHA. Needs no credential. Decides, and writes the files.
#   release-pr.sh publish    makes the commit through the GitHub API (so GitHub signs it), moves the branch, opens or updates the PR.
#
# prepare
#   env:  MAIN_SHA (40 hex, must be HEAD), OUT_DIR (an empty or missing directory), CLIFF_CONFIG (default cliff.toml)
#   reads the remote `origin` for rc/* branches (git ls-remote / fetch; no credential needed for a public repository)
#   prints key=value lines on stdout:
#     action=skip|release|current   release: the branch must move; current: it already holds exactly this commit's content
#     reason=<text>                 why, for the job summary
#     version=vX.Y.Z branch=rc/vX.Y.Z main_tree=<sha> tree=<sha> stale=<space-separated other rc/* branches>   (unless skip)
#   writes OUT_DIR/{CHANGELOG.md,Cargo.toml,Cargo.lock,notes.md,prepare.env} (unless skip)
#   It skips when: the head commit is itself a release (the release workflow tags it and starts this script again); the last
#   release merged into main has no tag yet (the next version cannot be computed from it); nothing since the last tag would go
#   into the changelog (docs, tests, CI and chores do not make a release, see cliff.toml).
#
# publish
#   env:  GH_TOKEN (an App installation token: a push made with the workflow's own token starts no workflow), REPO (owner/name),
#         OUT_DIR (written by prepare), BASE (default main)
#   For action=release: POST git/trees (base main's tree, the three files inline) and refuse unless GitHub's tree is the one
#   prepare computed; POST git/commits; then PATCH (force) or POST the branch ref. For both release and current: open the pull
#   request, or rewrite its body; then close the pull request of every stale rc/* branch with a comment and delete the branch.
#
# Exit status: 0 done (including every skip); 1 a check failed or a call failed; 2 called wrongly.
set -eu
LC_ALL=C
export LC_ALL

die() { echo "release-pr: FAIL: $*" >&2; exit 1; }
usage() { echo "release-pr: $*" >&2; exit 2; }

is_sha() { case "$1" in *[!0-9a-f]*|'') return 1 ;; esac; [ "${#1}" -eq 40 ]; }

# Squeeze runs of blank lines to one and drop trailing blank lines (git-cliff leaves both), so the file is stable.
normalize() { awk 'NF { if (pending && printed) print ""; print; printed = 1; pending = 0; next } { pending = 1 }' ; }

# cliff <out-file> <git-cliff args...>: run git-cliff into a file and fail on ITS status (a pipe would report the last command's).
cliff() {
  cliff_out="$1"; shift
  git-cliff --config "$config" "$@" > "$cliff_out.raw" 2> "$OUT_DIR/.cliff.err" \
    || { cat "$OUT_DIR/.cliff.err" >&2; die "git-cliff $* failed"; }
  normalize < "$cliff_out.raw" > "$cliff_out"
  rm -f "$cliff_out.raw" "$OUT_DIR/.cliff.err"
}

skip() {
  echo "action=skip"
  echo "reason=$*"
  echo "release-pr: skip: $*" >&2
  exit 0
}

prepare() {
  [ -n "${MAIN_SHA:-}" ] || usage "MAIN_SHA is not set"
  is_sha "$MAIN_SHA" || usage "MAIN_SHA is not a 40-digit sha: $MAIN_SHA"
  [ -n "${OUT_DIR:-}" ] || usage "OUT_DIR is not set"
  config="${CLIFF_CONFIG:-cliff.toml}"
  [ -f "$config" ] || usage "no git-cliff config at $config"
  command -v git-cliff > /dev/null 2>&1 || die "git-cliff is not installed"
  command -v jq > /dev/null 2>&1 || die "jq is not installed"
  head="$(git rev-parse HEAD)" || die "not in a git checkout"
  [ "$head" = "$MAIN_SHA" ] || usage "HEAD is $head, not MAIN_SHA $MAIN_SHA (git-cliff reads HEAD)"
  if [ -d "$OUT_DIR" ] && [ -n "$(ls -A "$OUT_DIR")" ]; then usage "OUT_DIR $OUT_DIR is not empty"; fi
  mkdir -p "$OUT_DIR"

  subject="$(git log -1 --format=%s "$MAIN_SHA")"
  case "$subject" in
    "chore(release): v"*) skip "the head commit is a release ($subject); the release workflow tags it and runs this again" ;;
  esac

  # The last release merged into main must be tagged, or git-cliff would count its commits again under the next version.
  last_release="$(git log -1 --format=%s --grep='^chore(release): v[0-9]' "$MAIN_SHA")"
  if [ -n "$last_release" ]; then
    last_version="$(echo "$last_release" | sed -n 's/^chore(release): \(v[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\).*/\1/p')"
    [ -n "$last_version" ] || die "cannot read a version from the release commit: $last_release"
    git rev-parse -q --verify "refs/tags/$last_version" > /dev/null \
      || skip "$last_version is merged into main but not tagged yet; the release workflow tags it and runs this again"
  fi

  ctx="$OUT_DIR/.context.json"
  git-cliff --config "$config" --unreleased --context > "$ctx" 2> "$OUT_DIR/.cliff.err" \
    || { cat "$OUT_DIR/.cliff.err" >&2; die "git-cliff --unreleased --context failed"; }
  count="$(jq '[.[].commits | length] | add // 0' "$ctx")" || die "cannot read git-cliff's context"
  rm -f "$ctx"
  [ "$count" -gt 0 ] || skip "nothing since the last release goes into the changelog"

  version="$(git-cliff --config "$config" --bumped-version 2> "$OUT_DIR/.cliff.err")" \
    || { cat "$OUT_DIR/.cliff.err" >&2; die "git-cliff --bumped-version failed"; }
  echo "$version" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' || die "git-cliff gave a version that is not vX.Y.Z: $version"
  number="${version#v}"
  branch="rc/$version"

  # The new section, dated by main's head commit (UTC). git-cliff would date an untagged section with today, so the same main would
  # give a different branch tomorrow, and the date would be when the bot last ran rather than what was released.
  date="$(TZ=UTC git log -1 --format=%cd --date=format-local:%Y-%m-%d "$MAIN_SHA")"
  cliff "$OUT_DIR/notes.raw.md" --unreleased --tag "$version" --strip all
  sed "1s/^## $version ([0-9-]*)\$/## $version ($date)/" "$OUT_DIR/notes.raw.md" > "$OUT_DIR/notes.md"
  rm -f "$OUT_DIR/notes.raw.md"
  [ "$(head -1 "$OUT_DIR/notes.md")" = "## $version ($date)" ] || die "the release notes do not start with ## $version ($date)"

  # The changelog: main's, with the new section put before the newest one, so a released section is never rewritten. The first
  # time, when main has none, it is the header from cliff.toml and the section.
  if git cat-file -e "$MAIN_SHA:CHANGELOG.md" 2> /dev/null; then
    git show "$MAIN_SHA:CHANGELOG.md" > "$OUT_DIR/CHANGELOG.main.md"
    ! grep -q "^## $version " "$OUT_DIR/CHANGELOG.main.md" || die "main's CHANGELOG.md already has a section for $version"
    awk -v notes="$OUT_DIR/notes.md" '
      function put(   l) { while ((getline l < notes) > 0) print l; print ""; done = 1 }
      !done && /^## / { put() }
      { print }
      END { if (!done) { print ""; put() } }
    ' "$OUT_DIR/CHANGELOG.main.md" | normalize > "$OUT_DIR/CHANGELOG.md"
    rm -f "$OUT_DIR/CHANGELOG.main.md"
  else
    cliff "$OUT_DIR/CHANGELOG.raw.md" --unreleased --tag "$version"
    sed "s/^## $version ([0-9-]*)\$/## $version ($date)/" "$OUT_DIR/CHANGELOG.raw.md" > "$OUT_DIR/CHANGELOG.md"
    rm -f "$OUT_DIR/CHANGELOG.raw.md"
  fi
  grep -qx "## $version ($date)" "$OUT_DIR/CHANGELOG.md" || die "the changelog has no section ## $version ($date)"
  [ "$(grep -c "^## $version " "$OUT_DIR/CHANGELOG.md")" = 1 ] || die "the changelog has more than one section for $version"

  # The workspace version: the one `version = "..."` line of [workspace.package], and nothing else in the file.
  git show "$MAIN_SHA:Cargo.toml" > "$OUT_DIR/Cargo.toml.orig" || die "main has no Cargo.toml"
  old="$(awk '/^\[/ { in_pkg = ($0 == "[workspace.package]") } in_pkg && /^version = "/ { v = $3; gsub(/"/, "", v); print v; exit }' "$OUT_DIR/Cargo.toml.orig")"
  [ -n "$old" ] || die "Cargo.toml has no version in [workspace.package]"
  awk -v new="$number" '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") }
    in_pkg && /^version = "/ { print "version = \"" new "\""; n++; next }
    { print }
    END { if (n != 1) exit 1 }
  ' "$OUT_DIR/Cargo.toml.orig" > "$OUT_DIR/Cargo.toml" || die "expected exactly one version line in [workspace.package]"

  # Cargo.lock: every package with no `source` is a workspace member, and each carries the workspace version. All of them move,
  # and nothing from a registry does; anything else is a lockfile this script does not understand.
  git show "$MAIN_SHA:Cargo.lock" > "$OUT_DIR/Cargo.lock.orig" || die "main has no Cargo.lock"
  awk -v old="$old" -v new="$number" '
    function flush(   i, has_source, is_old) {
      if (nlines == 0) return
      has_source = 0; is_old = 0
      for (i = 1; i <= nlines; i++) {
        if (lines[i] ~ /^source = /) has_source = 1
        if (lines[i] == "version = \"" old "\"") is_old = 1
      }
      if (pkg && !has_source) {
        local_pkgs++
        if (is_old) moved++
        for (i = 1; i <= nlines; i++) if (lines[i] == "version = \"" old "\"") lines[i] = "version = \"" new "\""
      }
      for (i = 1; i <= nlines; i++) print lines[i]
      nlines = 0
    }
    /^\[\[package\]\]$/ { flush(); pkg = 1 }
    /^\[/ && !/^\[\[package\]\]$/ { flush(); pkg = 0 }
    { lines[++nlines] = $0 }
    END { flush(); if (local_pkgs == 0 || moved != local_pkgs) exit 1 }
  ' "$OUT_DIR/Cargo.lock.orig" > "$OUT_DIR/Cargo.lock" \
    || die "Cargo.lock: not every workspace package (no source) is at $old"
  rm -f "$OUT_DIR/Cargo.toml.orig" "$OUT_DIR/Cargo.lock.orig"

  # The tree main plus the three files would have, made in a throwaway index so the working tree is not touched.
  idx="$OUT_DIR/.index"
  GIT_INDEX_FILE="$idx" git read-tree "$MAIN_SHA" || die "git read-tree failed"
  for f in CHANGELOG.md Cargo.toml Cargo.lock; do
    blob="$(git hash-object -w "$OUT_DIR/$f")" || die "git hash-object $f failed"
    GIT_INDEX_FILE="$idx" git update-index --add --cacheinfo "100644,$blob,$f" || die "git update-index $f failed"
  done
  tree="$(GIT_INDEX_FILE="$idx" git write-tree)" || die "git write-tree failed"
  rm -f "$idx"
  main_tree="$(git rev-parse "$MAIN_SHA^{tree}")"

  # What origin has: the branch for this version, and every other rc/* branch (stale: superseded by this version).
  heads="$OUT_DIR/.heads"
  git ls-remote --heads origin 'rc/*' > "$heads" || die "git ls-remote origin failed"
  stale=""; current_sha=""
  while read -r sha ref; do
    name="${ref#refs/heads/}"
    if [ "$name" = "$branch" ]; then current_sha="$sha"; else stale="${stale:+$stale }$name"; fi
  done < "$heads"
  rm -f "$heads"

  action=release
  reason="$branch does not exist yet"
  if [ -n "$current_sha" ]; then
    git fetch -q origin "refs/heads/$branch" || die "git fetch $branch failed"
    have_tree="$(git rev-parse "$current_sha^{tree}")"
    have_parent="$(git rev-parse -q --verify "$current_sha^" || true)"
    have_count="$(git rev-list --count "$MAIN_SHA..$current_sha")"
    if [ "$have_tree" = "$tree" ] && [ "$have_parent" = "$MAIN_SHA" ] && [ "$have_count" = 1 ]; then
      action=current; reason="$branch already is main plus this release commit"
    else
      reason="$branch is behind main or holds other content"
    fi
  fi

  {
    echo "action=$action"
    echo "reason=$reason"
    echo "version=$version"
    echo "branch=$branch"
    echo "main_sha=$MAIN_SHA"
    echo "main_tree=$main_tree"
    echo "tree=$tree"
    echo "stale=$stale"
  } > "$OUT_DIR/prepare.env"
  cat "$OUT_DIR/prepare.env"
}

# value <key>: a field of prepare.env
value() { sed -n "s/^$1=//p" "$OUT_DIR/prepare.env"; }

publish() {
  [ -n "${GH_TOKEN:-}" ] || usage "GH_TOKEN is not set"
  [ -n "${REPO:-}" ] || usage "REPO is not set"
  case "$REPO" in */*) ;; *) usage "REPO is not owner/name: $REPO" ;; esac
  [ -n "${OUT_DIR:-}" ] && [ -f "$OUT_DIR/prepare.env" ] || usage "OUT_DIR has no prepare.env"
  base="${BASE:-main}"
  command -v gh > /dev/null 2>&1 || die "gh is not installed"
  command -v jq > /dev/null 2>&1 || die "jq is not installed"

  action="$(value action)"; version="$(value version)"; branch="$(value branch)"
  main_sha="$(value main_sha)"; main_tree="$(value main_tree)"; tree="$(value tree)"; stale="$(value stale)"
  case "$action" in release|current) ;; *) usage "prepare.env says action=$action; nothing to publish" ;; esac
  echo "$version" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' || die "prepare.env has a bad version: $version"
  [ "$branch" = "rc/$version" ] || die "prepare.env's branch $branch is not rc/$version"
  for s in "$main_sha" "$main_tree" "$tree"; do is_sha "$s" || die "prepare.env has a bad sha: $s"; done
  for f in CHANGELOG.md Cargo.toml Cargo.lock notes.md; do [ -s "$OUT_DIR/$f" ] || die "OUT_DIR has no $f"; done

  if [ "$action" = release ]; then
    jq -n --arg base "$main_tree" \
      --rawfile changelog "$OUT_DIR/CHANGELOG.md" --rawfile toml "$OUT_DIR/Cargo.toml" --rawfile lock "$OUT_DIR/Cargo.lock" '
      { base_tree: $base, tree: [
          { path: "CHANGELOG.md", mode: "100644", type: "blob", content: $changelog },
          { path: "Cargo.toml",   mode: "100644", type: "blob", content: $toml },
          { path: "Cargo.lock",   mode: "100644", type: "blob", content: $lock } ] }' > "$OUT_DIR/.tree.json"
    got_tree="$(gh api --method POST "repos/$REPO/git/trees" --input "$OUT_DIR/.tree.json" --jq .sha)" || die "creating the tree failed"
    [ "$got_tree" = "$tree" ] || die "GitHub made tree $got_tree, not the $tree prepare computed; nothing was committed"

    jq -n --arg msg "chore(release): $version" --arg tree "$tree" --arg parent "$main_sha" \
      '{ message: $msg, tree: $tree, parents: [ $parent ] }' > "$OUT_DIR/.commit.json"
    commit="$(gh api --method POST "repos/$REPO/git/commits" --input "$OUT_DIR/.commit.json" --jq .sha)" || die "creating the commit failed"
    is_sha "$commit" || die "GitHub returned a bad commit sha: $commit"

    if gh api "repos/$REPO/git/ref/heads/$branch" --jq .object.sha > /dev/null 2>&1; then
      gh api --method PATCH "repos/$REPO/git/refs/heads/$branch" -f "sha=$commit" -F force=true > /dev/null || die "moving $branch failed"
    else
      gh api --method POST "repos/$REPO/git/refs" -f "ref=refs/heads/$branch" -f "sha=$commit" > /dev/null || die "creating $branch failed"
    fi
    echo "release-pr: $branch -> $commit"
  fi

  {
    cat "$OUT_DIR/notes.md"
    echo
    echo "---"
    echo
    echo "Merging this pull request releases $version: the release workflow tags the merge commit, publishes the release and"
    echo "deploys it to production after review. The branch is rewritten by the release bot after every push to $base; do not push"
    echo "to it."
  } > "$OUT_DIR/.body.md"
  pr="$(gh pr list --repo "$REPO" --head "$branch" --base "$base" --state open --json number --jq '.[0].number // empty')" \
    || die "listing pull requests failed"
  if [ -n "$pr" ]; then
    gh pr edit "$pr" --repo "$REPO" --title "chore(release): $version" --body-file "$OUT_DIR/.body.md" > /dev/null || die "updating #$pr failed"
    echo "release-pr: updated #$pr"
  else
    gh pr create --repo "$REPO" --base "$base" --head "$branch" --title "chore(release): $version" --body-file "$OUT_DIR/.body.md" \
      || die "opening the pull request failed"
  fi

  for old in $stale; do
    case "$old" in rc/v*) ;; *) die "refusing to delete $old: not an rc/v* branch" ;; esac
    old_pr="$(gh pr list --repo "$REPO" --head "$old" --state open --json number --jq '.[0].number // empty')" || die "listing $old's pull request failed"
    if [ -n "$old_pr" ]; then
      gh pr close "$old_pr" --repo "$REPO" --comment "Superseded by $branch: the next release is now $version." > /dev/null || die "closing #$old_pr failed"
    fi
    gh api --method DELETE "repos/$REPO/git/refs/heads/$old" > /dev/null || die "deleting $old failed"
    echo "release-pr: removed $old${old_pr:+ (closed #$old_pr)}"
  done
}

case "${1:-}" in
  prepare) prepare ;;
  publish) publish ;;
  *) usage "usage: release-pr.sh prepare|publish" ;;
esac
