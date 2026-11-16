#!/bin/sh
# SPIKE for stage 0.2 of the CI split. Not for merging.
# What the top-level `gate` would decide about ONE called workflow under each design, and whether that is what it should decide.
# The leg's job fails when the decision is not the expected one, so a design that mis-judges a scenario shows as a red `top` job.
#
#   naive      the calling job's result only (what gate sees today of a job that calls a workflow)
#   a          the same, the callee having its own verdict job that fails the callee when the jobs were wrong
#   aplus      the calling job's result AND the callee's `verdict` output being `pass`
#   b          the calling job's result AND the callee's reported results, judged here with the same rules
#   allsgreen  the same as a, the verdict job being re-actors/alls-green
#
# Environment: DESIGN SCENARIO PLAN CALLEE_RESULT CALLEE_VERDICT CALLEE_RESULTS
set -eu

decision=fail
case "$DESIGN" in
  naive | a | allsgreen)
    [ "$CALLEE_RESULT" = success ] && decision=pass
    ;;
  aplus)
    [ "$CALLEE_RESULT" = success ] && [ "$CALLEE_VERDICT" = pass ] && decision=pass
    ;;
  b)
    if [ "$CALLEE_RESULT" = success ] && [ -n "$CALLEE_RESULTS" ] \
      && NEEDS_JSON="$CALLEE_RESULTS" KNOWN="a b c" sh .github/scripts/spike-judge.sh > /dev/null 2>&1; then
      decision=pass
    fi
    ;;
  *)
    echo "unknown design '$DESIGN'" >&2
    exit 2
    ;;
esac

expected=fail
[ "$SCENARIO" = green ] && expected=pass

echo "LEG design=$DESIGN scenario=$SCENARIO callee=$CALLEE_RESULT verdict=${CALLEE_VERDICT:-none} results=$([ -n "$CALLEE_RESULTS" ] && echo yes || echo none) decision=$decision expected=$expected"
if [ "$decision" = "$expected" ]; then
  echo "LEG-RESULT match"
else
  echo "LEG-RESULT MISMATCH: the gate would $decision and should $expected" >&2
  exit 1
fi
