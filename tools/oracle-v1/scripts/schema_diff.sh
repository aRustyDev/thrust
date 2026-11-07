#!/usr/bin/env bash
# Schema diff job: breaking-change detection with oasdiff (ADR-0032 Confirmation, ADR-0122).
#
#   oracle/scripts/schema_diff.sh <pinned-schema.json> <result.json>
#
# This is the CI step that will compare the Rust v1 schema against the frozen Plane schema.
# Here it is proved against the pinned schema itself, with three controls:
#   self     schema vs itself                 -> must report NO breaking change  (exit 0)
#   altered  six kinds of deliberate break    -> must report breaking changes    (exit 1)
#   benign   additive-only change             -> must NOT be reported as a break (exit 0)
# If `altered` exits 0, the diff job cannot see breaks and is VOID.
#
# oasdiff is pinned by image digest. NEVER pass --open: it uploads the specs to oasdiff.com.
# The utoipa code-first generation check is NOT done here: it needs the Rust v1 slice (wave 2).
set -uo pipefail

SCHEMA="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
OUT="$2"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$ROOT/oracle/.work/schemadiff"
IMAGE='tufin/oasdiff:v1.33.0@sha256:6263a96dd2ef0726c54e21fea9b8e1607eac4841add0079324b424c1f52b819c'
rm -rf "$WORK" && mkdir -p "$WORK"
cp "$SCHEMA" "$WORK/base.json"

uv run "$ROOT/oracle/harness/alter_schema.py" "$WORK/base.json" \
  --breaking "$WORK/breaking.json" --benign "$WORK/benign.json" || exit 2

oasdiff() { docker run --rm -v "$WORK:/specs:ro" "$IMAGE" breaking "$@" --fail-on ERR --format json; }

oasdiff /specs/base.json /specs/base.json > "$WORK/self.out" 2> "$WORK/self.err"
SELF_RC=$?
oasdiff /specs/base.json /specs/breaking.json > "$WORK/breaking.out" 2> "$WORK/breaking.err"
BREAK_RC=$?
oasdiff /specs/base.json /specs/benign.json > "$WORK/benign.out" 2> "$WORK/benign.err"
BENIGN_RC=$?

uv run "$ROOT/oracle/scripts/schema_diff_check.py" \
  --self "$WORK/self.out:$SELF_RC" --breaking "$WORK/breaking.out:$BREAK_RC" \
  --benign "$WORK/benign.out:$BENIGN_RC" --mutations "$WORK/breaking.json.mutations.json" \
  --oasdiff-image "$IMAGE" --out "$OUT"
