#!/bin/sh
# .github/scripts/unpack-bundle.sh against archives built member by member (issue #49): the bundle it must accept, and each kind of archive
# it must refuse, with the check that a refused archive wrote nothing. A refusal that was not seen to refuse is not a lock.
#
#   sh .github/scripts/tests/test-unpack-bundle.sh
#
# UNPACK_SCRIPT points the test at a copy of the script, so a deliberately broken copy can be shown to fail it.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
S="${UNPACK_SCRIPT:-$HERE/.github/scripts/unpack-bundle.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; bad=0
SHA=0123456789abcdef0123456789abcdef01234567
OTHER=fedcba9876543210fedcba9876543210fedcba98

# make <name> <python statements that add members to `tf` (a TarFile) with helpers add_file/add_dir>
make() {
  python3 - "$T/$1.tar.gz" "$2" <<'EOF'
import io, sys, tarfile
out, body = sys.argv[1], sys.argv[2]
tf = tarfile.open(out, "w:gz")
def add_file(name, data):
    data = data.encode() if isinstance(data, str) else data
    ti = tarfile.TarInfo(name); ti.size = len(data); ti.mode = 0o644
    tf.addfile(ti, io.BytesIO(data))
def add_dir(name):
    ti = tarfile.TarInfo(name); ti.type = tarfile.DIRTYPE; ti.mode = 0o755
    tf.addfile(ti)
def add_special(name, kind, link=""):
    ti = tarfile.TarInfo(name); ti.type = kind; ti.linkname = link; ti.mode = 0o755
    tf.addfile(ti)
exec(body)
tf.close()
EOF
}

run() { # run <label> <want-exit> <want-fragment> <archive> <expected sha>
  label="$1"; want="$2"; frag="$3"; arc="$4"; sha="$5"
  rm -rf "$T/dest"
  sh "$S" "$T/$arc.tar.gz" "$T/dest" "$sha" > "$T/out.txt" 2>&1
  got=$?
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$T/out.txt"; then
    pass=$((pass + 1)); echo "ok   $label (exit $got; saw '$frag')"
  else
    bad=$((bad + 1)); echo "BAD  $label: wanted exit $want + '$frag', got exit $got"; sed 's/^/     | /' "$T/out.txt"
  fi
}
empty_dest() { # empty_dest <label>: a refused archive extracted nothing
  if [ -z "$(find "$T/dest" -type f 2> /dev/null | head -1)" ]; then pass=$((pass + 1)); echo "ok   $1: nothing was extracted"; else bad=$((bad + 1)); echo "BAD  $1: a refused archive left files behind"; fi
}

GOOD="add_dir('./'); add_dir('./build/'); add_dir('./build/worker/'); add_file('./build/worker/shim.mjs', 'export {}'); add_file('./BUILD_SHA', '$SHA\n'); add_dir('./web/'); add_file('./web/index.html', '<html>')"
make good "$GOOD"
run "a bundle of plain files, for the commit it is for" 0 "unpacked" good "$SHA"
[ -f "$T/dest/web/index.html" ] && [ -f "$T/dest/build/worker/shim.mjs" ] && pass=$((pass + 1)) || { bad=$((bad + 1)); echo "BAD  the files are not where the archive put them"; }

run "a bundle built for another commit" 1 "not from $OTHER" good "$OTHER"

make no-sha "add_dir('./'); add_file('./web/index.html', '<html>')"
run "a bundle with no BUILD_SHA" 1 "no BUILD_SHA" no-sha "$SHA"

make wrong-sha "add_file('./BUILD_SHA', '$OTHER\n'); add_file('./web/index.html', '<html>')"
run "a BUILD_SHA that names another commit" 1 "was built from '$OTHER'" wrong-sha "$SHA"

make absolute "add_file('./BUILD_SHA', '$SHA\n'); add_file('$T/escaped-absolute.txt', 'x')"
run "a member with an absolute path" 1 "absolute path" absolute "$SHA"
empty_dest "an absolute path"
[ ! -e "$T/escaped-absolute.txt" ] && pass=$((pass + 1)) && echo "ok   an absolute path: nothing was written there" || { bad=$((bad + 1)); echo "BAD  an absolute path was written"; }

make dotdot "add_file('./BUILD_SHA', '$SHA\n'); add_file('../escaped-dotdot.txt', 'x')"
run "a member with a .. in its path" 1 "'..'" dotdot "$SHA"
empty_dest "a .. path"
[ ! -e "$T/escaped-dotdot.txt" ] && pass=$((pass + 1)) && echo "ok   a .. path: nothing was written outside" || { bad=$((bad + 1)); echo "BAD  a .. path escaped"; }

make symlink "add_file('./BUILD_SHA', '$SHA\n'); add_special('./web', tarfile.SYMTYPE, '/etc')"
run "a symlink" 1 "not a plain file" symlink "$SHA"
empty_dest "a symlink"

make hardlink "add_file('./BUILD_SHA', '$SHA\n'); add_file('./web/index.html', '<html>'); add_special('./web/copy.html', tarfile.LNKTYPE, './web/index.html')"
run "a hard link" 1 "link" hardlink "$SHA"
empty_dest "a hard link"

# A tar that lists a hard link with the type of a regular file and says `link to <target>` after the name (implementations differ in
# how they show one): the type check cannot see it, so the text check has to. A `tar` on the PATH that rewrites the type letter
# stands in for that tar, and the archive is the one above.
REAL_TAR="$(command -v tar)"
mkdir -p "$T/shim"
printf '#!/bin/sh\ncase " $* " in\n  *" -tvzf "*) "%s" "$@" | sed "s/^h/-/" ;;\n  *) exec "%s" "$@" ;;\nesac\n' "$REAL_TAR" "$REAL_TAR" > "$T/shim/tar"
chmod +x "$T/shim/tar"
rm -rf "$T/dest"
PATH="$T/shim:$PATH" sh "$S" "$T/hardlink.tar.gz" "$T/dest" "$SHA" > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 1 ] && grep -q "is a link" "$T/out.txt"; then pass=$((pass + 1)); echo "ok   a hard link a tar lists as a regular file is still refused, by its text (exit 1; saw 'is a link')"; else bad=$((bad + 1)); echo "BAD  the text check on a hard link listed as a regular file: exit $got"; sed 's/^/     | /' "$T/out.txt"; fi
empty_dest "a hard link listed as a regular file"

make fifo "add_file('./BUILD_SHA', '$SHA\n'); add_special('./web/pipe', tarfile.FIFOTYPE)"
run "a fifo" 1 "not a plain file" fifo "$SHA"
empty_dest "a fifo"

make empty "pass"
run "an empty archive" 1 "empty" empty "$SHA"

printf 'this is not an archive\n' > "$T/junk.tar.gz"
run "a file that is not an archive" 1 "cannot be read" junk "$SHA"

# usage
sh "$S" > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ]; then pass=$((pass + 1)); echo "ok   no arguments is a usage error"; else bad=$((bad + 1)); echo "BAD  no arguments: exit $got"; fi
sh "$S" "$T/good.tar.gz" "$T/dest" short > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ]; then pass=$((pass + 1)); echo "ok   an expected sha that is not a sha is a usage error"; else bad=$((bad + 1)); echo "BAD  a bad sha: exit $got"; fi
sh "$S" "$T/nope.tar.gz" "$T/dest" "$SHA" > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ]; then pass=$((pass + 1)); echo "ok   a bundle that is not there is a usage error"; else bad=$((bad + 1)); echo "BAD  a missing bundle: exit $got"; fi

echo "passed=$pass bad=$bad"
[ "$bad" = 0 ]
