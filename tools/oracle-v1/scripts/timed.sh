#!/usr/bin/env bash
# timed.sh <label> <command...>
#
# Runs <command>, appends "<label> <seconds>s rc=<n>" to $ORACLE_TIMINGS (default: oracle/.timings.log,
# which is gitignored by oracle/.gitignore), and exits with the COMMAND's own status -- never the
# status of the logging. (A wrapper that ends in `echo` reports the echo's status, which hides failures.)
set -u
label="$1"
shift
timings="${ORACLE_TIMINGS:-$(dirname "$0")/../.timings.log}"
start=$(date +%s)
"$@"
rc=$?
end=$(date +%s)
echo "${label} $((end - start))s rc=${rc}" >> "${timings}"
exit "${rc}"
