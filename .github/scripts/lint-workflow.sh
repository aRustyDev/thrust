#!/bin/sh
# The rules a workflow that holds credentials must keep, checked on the text (issue #49). Not a YAML parser, and not a substitute for one:
# it reads the layout this repository's workflows use (two-space indent, block `run: |`), which is also what makes a breach easy to see.
#
#   .github/scripts/lint-workflow.sh <file>...
#
# For a workflow (it has a top-level `jobs:`):
#   R1  every `uses:` is a local action (./...) or is pinned to a 40-digit commit sha
#   R2  there is a top-level `permissions: {}`
#   R3  every job has its own `permissions:`
#   R4  no `${{ ... }}` expression inside the text of a `run:` (an input is data, not code: it goes through env)
#   R5  no `pull_request_target`
#   R6  every `actions/checkout` has `persist-credentials: false`
#   R7  `secrets.` only in the jobs named in SECRET_JOBS (default `deploy deploy-do`), `environment:` only there too, and
#       `id-token: write` only in ID_TOKEN_JOBS (default `build`)
# For a composite action (it has a top-level `runs:`): R1, R4 and R6.
#
# Exit status: 0 clean; 1 a rule is broken (file:line: rule: what); 2 called wrongly.
set -u
LC_ALL=C
export LC_ALL

[ $# -ge 1 ] || { echo "usage: lint-workflow.sh <file>..." >&2; exit 2; }
SECRET_JOBS="${SECRET_JOBS:-deploy deploy-do}"
ID_TOKEN_JOBS="${ID_TOKEN_JOBS:-build}"
status=0

for f in "$@"; do
  [ -f "$f" ] || { echo "lint-workflow: $f is not a file" >&2; exit 2; }
  kind=action
  grep -q '^jobs:' "$f" && kind=workflow
  awk -v file="$f" -v kind="$kind" -v secret_jobs="$SECRET_JOBS" -v id_jobs="$ID_TOKEN_JOBS" '
    function bad(rule, msg) { printf "%s:%d: %s: %s\n", file, NR, rule, msg; failed = 1 }
    function indent_of(s) { match(s, /^ */); return RLENGTH }
    function in_list(list, name,   n, a, i) { n = split(list, a, " "); for (i = 1; i <= n; i++) if (a[i] == name) return 1; return 0 }
    function close_checkout() {
      if (pending_checkout && !checkout_ok) bad("R6", "actions/checkout without persist-credentials: false (the step starting at line " checkout_line ")")
      pending_checkout = 0
    }
    function close_job() {
      if (kind == "workflow" && job != "" && !job_has_permissions) bad("R3", "job \"" job "\" has no permissions: of its own")
    }
    BEGIN { in_jobs = 0; job = ""; in_run = 0; failed = 0 }
    {
      line = $0
      stripped = line; sub(/^[ \t]+/, "", stripped)
      blank = (stripped == "")
      ind = indent_of(line)

      # leaving a run block: the first non-blank line indented no deeper than the `run:` key
      if (in_run && !blank && ind <= run_indent) in_run = 0

      # a new step closes the checkout check of the previous one
      if (!blank && stripped ~ /^- /) close_checkout()

      if (kind == "workflow") {
        if (line ~ /^permissions: *\{\} *(#.*)?$/) top_perms = 1
        if (line ~ /^jobs:/) in_jobs = 1
        if (in_jobs && ind == 2 && stripped ~ /^[A-Za-z0-9_-]+:/) {
          close_job()
          job = stripped; sub(/:.*/, "", job)
          job_has_permissions = 0
        }
        if (in_jobs && ind == 4 && stripped ~ /^permissions:/) job_has_permissions = 1
        if (line ~ /pull_request_target/ && stripped !~ /^#/) bad("R5", "pull_request_target runs fork code with secrets")
        if (in_jobs && job != "") {
          if (stripped !~ /^#/ && line ~ /secrets\./ && !in_list(secret_jobs, job)) bad("R7", "secrets. in job \"" job "\", which is not one of: " secret_jobs)
          if (ind == 4 && stripped ~ /^environment:/ && !in_list(secret_jobs, job)) bad("R7", "environment: in job \"" job "\", which is not one of: " secret_jobs)
          if (stripped ~ /^id-token: *write/ && !in_list(id_jobs, job)) bad("R7", "id-token: write in job \"" job "\", which is not one of: " id_jobs)
        }
      }

      # R1: uses
      if (stripped ~ /^(- )?uses:/) {
        u = stripped; sub(/^(- )?uses: */, "", u); sub(/ *#.*$/, "", u)
        if (u !~ /^\.\// && u !~ /@[0-9a-f]{40}$/) bad("R1", "uses: " u " is not pinned to a 40-digit commit sha")
        if (u ~ /^actions\/checkout@/) { pending_checkout = 1; checkout_ok = 0; checkout_line = NR }
      }
      if (pending_checkout && stripped ~ /^persist-credentials: *false/) checkout_ok = 1

      # R4: expressions in run text
      if (stripped ~ /^(- )?run:/) {
        rest = stripped; sub(/^(- )?run: */, "", rest)
        if (rest ~ /\$\{\{/) bad("R4", "an expression in run: text; pass it through env")
        if (rest ~ /^[|>]/) { in_run = 1; run_indent = ind }
      } else if (in_run && !blank && line ~ /\$\{\{/ && stripped !~ /^#/) {
        bad("R4", "an expression in a run: block; pass it through env")
      }
    }
    END {
      close_checkout()
      close_job()
      if (kind == "workflow" && !top_perms) { printf "%s:1: R2: no top-level permissions: {}\n", file; failed = 1 }
      exit failed
    }
  ' "$f" || status=1
done
[ "$status" = 0 ] && echo "lint-workflow: $* clean"
exit "$status"
