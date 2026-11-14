#!/bin/sh
# .github/scripts/fetch-previous-assets.sh against a fake `gh` that serves a list of artifacts and their zips, with the real
# verify-bundle.sh and unpack-bundle.sh behind it (issue #49). The point is what it will NOT take: an artifact that is expired, one made
# by a run on another branch (a fork's pull request can upload one under any name), one without an attestation, one for another commit,
# one with a link in it. Each is shown next to the one it does take.
#
#   sh .github/scripts/tests/test-fetch-previous-assets.sh
#
# FETCH_SCRIPT points the test at a copy of the script, so a deliberately broken copy can be shown to fail it. The copy is put beside the
# real scripts it calls.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
SCRIPTS="$HERE/.github/scripts"
S="${FETCH_SCRIPT:-$SCRIPTS/fetch-previous-assets.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; bad=0
SHA=0123456789abcdef0123456789abcdef01234567
OTHER=fedcba9876543210fedcba9876543210fedcba98

# a copy of the script beside the real ones it calls (the script finds them by its own location)
mkdir -p "$T/scripts"
cp "$S" "$T/scripts/fetch-previous-assets.sh"
cp "$SCRIPTS/verify-bundle.sh" "$SCRIPTS/unpack-bundle.sh" "$T/scripts/"

mkdir -p "$T/bin" "$T/fix"
cat > "$T/bin/gh" <<'EOF'
#!/bin/sh
# fake gh. `gh api repos/o/r/actions/artifacts?name=...` serves $FIX/list.json (or fails when $FIX/list.fail exists);
# `gh api repos/o/r/actions/artifacts/<id>/zip` serves $FIX/<id>.zip; `gh attestation verify` passes when $FIX/attested exists.
case "$1 $2" in
  "api repos/o/r/actions/artifacts?name="*)
    [ -f "$FIX/list.fail" ] && { echo "gh: HTTP 502" >&2; exit 1; }
    cat "$FIX/list.json" ;;
  "api repos/o/r/actions/artifacts/"*"/zip")
    id="${2#repos/o/r/actions/artifacts/}"; id="${id%/zip}"
    [ -f "$FIX/$id.zip" ] || { echo "gh: Not Found (HTTP 404)" >&2; exit 1; }
    cat "$FIX/$id.zip" ;;
  "attestation verify")
    [ -f "$FIX/attested" ] || { echo "gh: no attestations found" >&2; exit 1; } ;;
  *) echo "fake gh: unexpected: $*" >&2; exit 99 ;;
esac
EOF
chmod +x "$T/bin/gh"

# mkbundle <id> <build sha> <with web: yes|no> [symlink]: the zip of an artifact that holds deploy-bundle.tar.gz
mkbundle() {
  python3 - "$T/fix/$1.zip" "$2" "$3" "${4:-}" <<'EOF'
import io, sys, tarfile, zipfile
out, sha, web, extra = sys.argv[1:5]
buf = io.BytesIO()
tf = tarfile.open(fileobj=buf, mode="w:gz")
def add(name, data):
    data = data.encode(); ti = tarfile.TarInfo(name); ti.size = len(data); ti.mode = 0o644; tf.addfile(ti, io.BytesIO(data))
add("./BUILD_SHA", sha + "\n")
add("./build/index.js", "export {}")
if web == "yes":
    add("./web/index.html", "<html>previous</html>")
    add("./web/assets/app-" + sha[:6] + ".js", "// previous chunk\n")
if extra == "symlink":
    ti = tarfile.TarInfo("./web/link"); ti.type = tarfile.SYMTYPE; ti.linkname = "/etc"; tf.addfile(ti)
tf.close()
with zipfile.ZipFile(out, "w") as z:
    z.writestr("deploy-bundle.tar.gz", buf.getvalue())
EOF
}
# artifact <id> <expired true|false> <created_at> <head_branch> [name]: one entry of the list
artifact() { printf '{"id":%s,"name":"%s","expired":%s,"created_at":"%s","workflow_run":{"head_branch":"%s"}}' "$1" "${5:-deploy-bundle-$SHA}" "$2" "$3" "$4"; }
list() { printf '{"total_count":%s,"artifacts":[%s]}' "$#" "$(printf '%s,' "$@" | sed 's/,$//')" > "$T/fix/list.json"; }

run() { # run <label> <want-exit> <want-fragment> [sha]
  label="$1"; want="$2"; frag="$3"; sha="${4:-$SHA}"
  rm -rf "$T/dest"
  PATH="$T/bin:$PATH" FIX="$T/fix" sh "$T/scripts/fetch-previous-assets.sh" o/r "$sha" "$T/dest" > "$T/stdout.txt" 2> "$T/stderr.txt"
  got=$?
  cat "$T/stdout.txt" "$T/stderr.txt" > "$T/out.txt"
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$T/out.txt"; then
    pass=$((pass + 1)); echo "ok   $label (exit $got; saw '$frag')"
  else
    bad=$((bad + 1)); echo "BAD  $label: wanted exit $want + '$frag', got exit $got"; sed 's/^/     | /' "$T/out.txt"
  fi
}
nothing_left() { # nothing_left <label>
  if [ -e "$T/dest/bundle" ]; then bad=$((bad + 1)); echo "BAD  $1: a refused bundle was left unpacked"; else pass=$((pass + 1)); echo "ok   $1: nothing was left unpacked"; fi
}
reset() { rm -f "$T/fix/"*.zip "$T/fix/list.json" "$T/fix/list.fail" "$T/fix/attested"; }

# ---- the artifact it takes --------------------------------------------------------------------------------------------------------------
reset; mkbundle 11 "$SHA" yes; touch "$T/fix/attested"
list "$(artifact 11 false 2026-11-14T10:00:00Z main)"
run "a verified bundle of the right commit, with web files" 0 "web=true"
grep -q "^bundle=$T/dest/bundle$" "$T/stdout.txt" && [ -f "$T/dest/bundle/web/index.html" ] && pass=$((pass + 1)) && echo "ok   ...and it is unpacked where it says" || { bad=$((bad + 1)); echo "BAD  the bundle is not where it says"; }
[ "$(wc -l < "$T/stdout.txt" | tr -d ' ')" = 2 ] && pass=$((pass + 1)) && echo "ok   ...and stdout carries only the two key=value lines" || { bad=$((bad + 1)); echo "BAD  stdout has more than bundle= and web=: $(cat "$T/stdout.txt")"; }

reset; mkbundle 12 "$SHA" no; touch "$T/fix/attested"
list "$(artifact 12 false 2026-11-14T10:00:00Z main)"
run "a bundle from before the web app was part of it is not an error" 0 "web=false"

# ---- which artifact: newest, unexpired, from main -------------------------------------------------------------------------------------
reset; mkbundle 21 "$SHA" yes; mkbundle 22 "$SHA" no; touch "$T/fix/attested"
list "$(artifact 21 false 2026-11-14T08:00:00Z main)" "$(artifact 22 false 2026-11-14T09:00:00Z main)"
run "of two good artifacts it takes the newest" 0 "web=false"
reset; mkbundle 23 "$SHA" yes; mkbundle 24 "$SHA" no; touch "$T/fix/attested"
list "$(artifact 23 false 2026-11-14T08:00:00Z main)" "$(artifact 24 true 2026-11-14T09:00:00Z main)"
run "a newer artifact that has expired is skipped for an older one that has not" 0 "web=true"

reset; touch "$T/fix/attested"
list
run "no artifact at all" 3 "no usable artifact"
reset; mkbundle 31 "$SHA" yes; touch "$T/fix/attested"
list "$(artifact 31 true 2026-11-14T10:00:00Z main)"
run "only an expired artifact" 3 "no unexpired"
reset; mkbundle 32 "$SHA" yes; touch "$T/fix/attested"
list "$(artifact 32 false 2026-11-14T10:00:00Z feat/forged)"
run "an artifact made by a run on another branch (a forged one) is not looked at" 3 "from a run on main"
reset; mkbundle 33 "$SHA" yes; touch "$T/fix/attested"
list "$(artifact 33 false 2026-11-14T10:00:00Z main "deploy-bundle-$OTHER")"
run "an artifact of another name is not it" 3 "no usable artifact"
reset; mkbundle 34 "$SHA" yes; touch "$T/fix/attested"
list "$(artifact 34 false 2026-11-14T10:00:00Z main)" "$(artifact 35 false 2026-11-14T11:00:00Z feat/forged)"
run "a newer artifact from another branch does not hide a good one from main" 0 "web=true"

# ---- what a bundle must pass --------------------------------------------------------------------------------------------------------------
reset; mkbundle 41 "$SHA" yes
list "$(artifact 41 false 2026-11-14T10:00:00Z main)"
run "a bundle with no attestation" 4 "did not verify"
nothing_left "a bundle with no attestation"

reset; mkbundle 42 "$OTHER" yes; touch "$T/fix/attested"
list "$(artifact 42 false 2026-11-14T10:00:00Z main)"
run "an attested bundle that is for another commit" 4 "did not unpack"
nothing_left "an attested bundle that is for another commit"

reset; mkbundle 43 "$SHA" yes symlink; touch "$T/fix/attested"
list "$(artifact 43 false 2026-11-14T10:00:00Z main)"
run "an attested bundle with a symlink in it" 4 "did not unpack"
nothing_left "an attested bundle with a symlink"

# ---- GitHub not answering is a refusal, not a "nothing there" ------------------------------------------------------------------------
reset; touch "$T/fix/list.fail"
run "the list of artifacts failing" 4 "could not list the artifacts"
reset; list "$(artifact 51 false 2026-11-14T10:00:00Z main)"
run "an artifact whose download fails" 4 "could not download artifact 51"
reset; printf 'not json' > "$T/fix/list.json"
run "a list that is not JSON" 4 "not the JSON this reads"
reset; printf 'not a zip' > "$T/fix/61.zip"; list "$(artifact 61 false 2026-11-14T10:00:00Z main)"
run "a download that is not a zip" 4 "not a zip file"
reset; python3 -c "import zipfile,sys; z=zipfile.ZipFile(sys.argv[1],'w'); z.writestr('something-else.txt','x'); z.close()" "$T/fix/62.zip"; list "$(artifact 62 false 2026-11-14T10:00:00Z main)"
run "a zip without the bundle in it" 4 "no deploy-bundle.tar.gz"

# ---- usage ----------------------------------------------------------------------------------------------------------------------------------
run "a sha that is not a sha" 2 "not a commit sha" "nope"
sh "$T/scripts/fetch-previous-assets.sh" > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ]; then pass=$((pass + 1)); echo "ok   no arguments is a usage error"; else bad=$((bad + 1)); echo "BAD  no arguments: exit $got"; fi

echo "passed=$pass bad=$bad"
[ "$bad" = 0 ]
