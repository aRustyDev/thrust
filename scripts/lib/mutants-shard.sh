# shellcheck shell=sh
# Which rows of scripts/mutants.sh one run is responsible for. Sourced by scripts/mutants.sh; tested by .github/scripts/tests/test-mutants-shards.sh.
#
# The rows cannot share a checkout (each rebuilds `thrust-server` with a mutant's cargo feature into one `target/`, and moves a JUnit report from a
# shared path), so the job runs as a matrix of runners instead, each with a `target/` of its own. Every runner walks the WHOLE script and runs only the
# rows at its positions: row k (1-based, in the order the script lists them) belongs to shard (k - 1) mod SHARDS. Nothing is decided by name, so a
# row added anywhere is picked up, and the rows of one loop land on different shards, which evens the work (the kinds of row cost 10 to 30 s each).
#
#   MUTANTS_SHARDS   how many shards the rows are split over (default 1: every row, as when one runner ran them all)
#   MUTANTS_SHARD    this run's index, 0 to SHARDS - 1 (default 0)
#   MUTANTS_LIST     when set (to anything), run nothing: print `<row number> <shard> <row>` for every row, and exit 0
#
# Every row starts with `want <label>`; a row that does not would run on every shard. `shard_finish` catches that: it compares the rows this run
# selected with the verdicts it recorded, and a row outside `want` makes the second number larger. It also fails a shard that selected no row (more
# shards than rows, or a wrong index), which would pass for having killed every mutant it was given.

# shard_init: read and check the two numbers. Returns 2 (and says why) when they are not a shard of a split. Neither set is the whole run; one
# without the other is an error, because `MUTANTS_SHARDS=4` alone would run shard 0 on every runner and drop three quarters of the rows.
shard_init() {
  if [ -z "${MUTANTS_SHARDS:-}${MUTANTS_SHARD:-}" ]; then
    SHARD_N=1; SHARD_I=0
  elif [ -z "${MUTANTS_SHARDS:-}" ] || [ -z "${MUTANTS_SHARD:-}" ]; then
    echo "mutants.sh: MUTANTS_SHARDS and MUTANTS_SHARD go together (got '${MUTANTS_SHARDS:-}' and '${MUTANTS_SHARD:-}')" >&2
    return 2
  else
    SHARD_N="$MUTANTS_SHARDS"; SHARD_I="$MUTANTS_SHARD"
  fi
  # whole numbers, no sign and no leading zero (the shell's arithmetic would read 08 as octal, and dash stops on it)
  for si_v in "$SHARD_N" "$SHARD_I"; do
    case "$si_v" in
      *[!0-9]* | 0?*) echo "mutants.sh: MUTANTS_SHARDS ('$SHARD_N') and MUTANTS_SHARD ('$SHARD_I') must be whole numbers without a leading zero" >&2; return 2 ;;
    esac
  done
  if [ "$SHARD_N" -lt 1 ] || [ "$SHARD_I" -ge "$SHARD_N" ]; then
    echo "mutants.sh: MUTANTS_SHARD=$SHARD_I is not one of the $SHARD_N shards (0 to $((SHARD_N - 1)))" >&2
    return 2
  fi
  ROW_N=0
  ROWS_SELECTED=0
}

# want <label>: counts the row, and says whether this run runs it. Always returns 1 in list mode, after printing the row.
want() {
  ROW_N=$((ROW_N + 1))
  w_shard=$(((ROW_N - 1) % SHARD_N))
  if [ -n "${MUTANTS_LIST:-}" ]; then
    printf '%s %s %s\n' "$ROW_N" "$w_shard" "$1"
    return 1
  fi
  if [ "$w_shard" -eq "$SHARD_I" ]; then
    ROWS_SELECTED=$((ROWS_SELECTED + 1))
    # A row's own output goes to a file, so the job log would say nothing about a row that is running (or hung). This line is its start: the log's
    # timestamps give each row's time, and the last one is the row a cancelled job was in.
    echo "row $ROW_N: $1"
    return 0
  fi
  return 1
}

# shard_finish <verdicts recorded>: ends a list run, and fails (returns 5, as scripts/contract.sh does for a vacuity guard) a run whose
# selection and verdicts disagree, or that selected nothing.
shard_finish() {
  if [ -n "${MUTANTS_LIST:-}" ]; then exit 0; fi
  if [ "$ROWS_SELECTED" -eq 0 ]; then
    echo "VACUITY GUARD: shard $SHARD_I of $SHARD_N selected no row (the script has $ROW_N); a run that ran nothing has killed nothing" >&2
    return 5
  fi
  if [ "$1" -ne "$ROWS_SELECTED" ]; then
    echo "VACUITY GUARD: shard $SHARD_I of $SHARD_N selected $ROWS_SELECTED rows and recorded $1 verdicts; a row that is not behind \`want\` runs on every shard, and one that recorded nothing was never judged" >&2
    return 5
  fi
  return 0
}
