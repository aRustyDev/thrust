# shellcheck shell=bash
# SINGLE SOURCE OF TRUTH for the per-PR connection budget on thrust-pg-dev. Sourced — never
# executed — by BOTH scripts that must agree on it:
#
#   infra/preview/provisioner-bootstrap.sql  stored connection limit for pr_<n> roles
#   infra/cloudflare/hyperdrive.sh       ensure --user pr_<n>: origin_connection_limit
#                                                     defaults to <this> and may not exceed it
#
# WHY THEY MUST AGREE. A per-PR Hyperdrive config authenticates as the per-PR role and pools up to
# its origin_connection_limit backends. If that is larger than the role's CONNECTION LIMIT, the
# excess connection attempts are rejected by Postgres ("too many connections for role") under
# load. Hyperdrive's own default for a paid plan is 60, so omitting the limit is not harmless.
#
# Constraints on the value (Create Hyperdrive API reference, `origin_connection_limit`: "(soft)
# maximum number of connections ... minimum 5; maximum 20 free / 100 paid; default 20 free / 60
# paid"): it must be >= 5. Hyperdrive's limit is SOFT, so it can overshoot slightly — keep the
# role's limit no lower than Hyperdrive's, which is why they are the same number.
#
# Changing it: edit this one line, run both scripts' hermetic tests, then re-run
# update the protected preview_admin.settings table through the trusted operator path, reprovision
# each open PR, and recreate each PR's Hyperdrive configuration to apply the new limit.
# shellcheck disable=SC2034 # consumed by the scripts that source this file
THRUST_PR_ROLE_CONNECTION_LIMIT=8
