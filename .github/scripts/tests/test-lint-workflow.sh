#!/bin/sh
# .github/scripts/lint-workflow.sh (issue #49): clean on the real deploy workflow and action, and failing, with the right rule named, on a
# copy broken in exactly one way for each rule. A rule that has not been seen to fail is not a rule.
#
#   sh .github/scripts/tests/test-lint-workflow.sh
#
# LINT_SCRIPT points the test at a copy of the script, so a deliberately broken copy can be shown to fail it.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
S="${LINT_SCRIPT:-$HERE/.github/scripts/lint-workflow.sh}"
WF="$HERE/.github/workflows/deploy.yml"
ACT="$HERE/.github/actions/deploy-worker/action.yml"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; bad=0

run() { # run <label> <want-exit> <want-fragment> <file>...
  label="$1"; want="$2"; frag="$3"; shift 3
  sh "$S" "$@" > "$T/out.txt" 2>&1
  got=$?
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$T/out.txt"; then
    pass=$((pass + 1)); echo "ok   $label (exit $got; saw '$frag')"
  else
    bad=$((bad + 1)); echo "BAD  $label: wanted exit $want + '$frag', got exit $got"; sed 's/^/     | /' "$T/out.txt"
  fi
}

run "the real workflow and action are clean" 0 "clean" "$WF" "$ACT"

# make <name> <file> <python expression on `text`>: a copy with one change; the change must change something
make() {
  python3 - "$1" "$2" "$3" <<'EOF'
import sys
name, src, expr = sys.argv[1], sys.argv[2], sys.argv[3]
text = open(src).read()
new = eval(expr, {"text": text})
if new == text:
    print(f"THE CHANGE FOR {name} CHANGED NOTHING")
    sys.exit(3)
open(f"{sys.argv[0] and ''}" + name, "w").write(new)
EOF
}
cd "$T" || exit 1

# R1: unpinned
make r1-tag "$WF" 'text.replace("actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2", "actions/upload-artifact@v7", 1)' > /dev/null || echo "BAD  r1-tag did not change"
run "R1: an action pinned to a tag" 1 "R1: uses: actions/upload-artifact@v7 is not pinned" "$T/r1-tag"
make r1-short "$WF" 'text.replace("actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1", "actions/checkout@3d3c42e", 1)' > /dev/null
run "R1: an action pinned to a short sha" 1 "R1: uses: actions/checkout@3d3c42e is not pinned" "$T/r1-short"
make r1-action "$ACT" 'text.replace("@9000827ccba6bdab643e8b6fd33ac0654aef8333", "@main", 1)' > /dev/null
run "R1: the composite action's own uses are held to it too" 1 "R1: uses: actions/download-artifact@main" "$T/r1-action"

# R2 and R3: permissions
make r2 "$WF" 'text.replace("\npermissions: {}\n", "\n", 1)' > /dev/null
run "R2: no top-level permissions: {}" 1 "R2: no top-level permissions" "$T/r2"
make r3 "$WF" 'text.replace("    permissions:\n      contents: read\n    steps:\n      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1\n        with:\n          ref: ${{ needs.plan.outputs.sha }}\n          persist-credentials: false\n      - uses: actions/download-artifact", "    steps:\n      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1\n        with:\n          ref: ${{ needs.plan.outputs.sha }}\n          persist-credentials: false\n      - uses: actions/download-artifact", 1)' > /dev/null
run "R3: a job without permissions of its own" 1 'R3: job "dry-run" has no permissions' "$T/r3"

# R4: expressions in run text
make r4-block "$WF" 'text + "      - run: |\n          echo \"${{ github.event.inputs.ref }}\"\n"' > /dev/null
run "R4: an expression inside a run block" 1 "R4: an expression in a run: block" "$T/r4-block"
make r4-inline "$WF" 'text + "      - run: echo ${{ github.sha }}\n"' > /dev/null
run "R4: an expression in a one-line run" 1 "R4: an expression in run: text" "$T/r4-inline"
make r4-action "$ACT" 'text.replace("run: scripts/deploy-worker.sh", "run: scripts/deploy-worker.sh ${{ inputs.sha }}", 1)' > /dev/null
run "R4: the same rule on the composite action" 1 "R4: an expression in run: text" "$T/r4-action"

# R5: pull_request_target
make r5 "$WF" 'text.replace("on:\n", "on:\n  pull_request_target:\n", 1)' > /dev/null
run "R5: pull_request_target" 1 "R5: pull_request_target" "$T/r5"

# R6: persist-credentials
make r6 "$WF" 'text.replace("          fetch-depth: 0\n          persist-credentials: false\n", "          fetch-depth: 0\n", 1)' > /dev/null
run "R6: a checkout that keeps the credentials" 1 "R6: actions/checkout without persist-credentials: false" "$T/r6"

# R7: where secrets, environments and id-token may be
make r7-secret "$WF" 'text.replace("    outputs:\n      mode:", "    env:\n      LEAK: ${{ secrets.CLOUDFLARE_API_TOKEN }}\n    outputs:\n      mode:", 1)' > /dev/null
run "R7: a secret in the plan job" 1 'R7: secrets. in job "plan"' "$T/r7-secret"
make r7-env "$WF" 'text.replace("  build:\n    needs: plan\n", "  build:\n    environment: staging\n    needs: plan\n", 1)' > /dev/null
run "R7: an environment on the build job" 1 'R7: environment: in job "build"' "$T/r7-env"
make r7-id "$WF" 'text.replace("    permissions:\n      contents: read\n      checks: read\n", "    permissions:\n      contents: read\n      checks: read\n      id-token: write\n", 1)' > /dev/null
run "R7: id-token: write outside the build job" 1 'R7: id-token: write in job "plan"' "$T/r7-id"

# the rules do not trip on what is allowed
make allowed-comment "$WF" 'text.replace("jobs:\n", "jobs:\n  # note: ${{ secrets.X }} in a comment is not a use, and pull_request_target neither\n", 1)' > /dev/null
run "a comment that names a secret or an expression is not a breach" 0 "clean" "$T/allowed-comment"

# usage
sh "$S" > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ]; then pass=$((pass + 1)); echo "ok   no file is a usage error"; else bad=$((bad + 1)); echo "BAD  no file: exit $got"; fi
sh "$S" "$T/nope.yml" > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ]; then pass=$((pass + 1)); echo "ok   a file that is not there is a usage error"; else bad=$((bad + 1)); echo "BAD  missing file: exit $got"; fi

echo "passed=$pass bad=$bad"
[ "$bad" = 0 ]
