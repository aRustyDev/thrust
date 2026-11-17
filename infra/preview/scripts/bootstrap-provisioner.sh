#!/usr/bin/env bash
# One-time operator bootstrap on the fixed development cluster. Requires an existing Secrets
# Manager secret containing the provisioner origin password; never sends the password in argv.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/../pr-role-limits.sh"
secret='/thrust/preview-provisioner-origin-password'
namespace="${NAMESPACE:-thrust}"
kubectl_cmd="${KUBECTL:-kubectl}"
[[ $# == 0 ]] || { echo 'usage: bootstrap-provisioner.sh' >&2; exit 2; }
[[ "$namespace" =~ ^[a-z0-9]([-a-z0-9]*[a-z0-9])?$ ]] || { echo 'invalid namespace' >&2; exit 2; }
[[ "$THRUST_PR_ROLE_CONNECTION_LIMIT" =~ ^([5-9]|[1-9][0-9]|100)$ ]] || { echo 'invalid PR connection limit' >&2; exit 2; }
primary="$("$kubectl_cmd" -n "$namespace" get cluster thrust-pg-dev -o 'jsonpath={.status.currentPrimary}')"
[[ "$primary" =~ ^thrust-pg-dev-[a-z0-9-]+$ ]] || { echo 'invalid or missing dev primary' >&2; exit 3; }
password="$(aws secretsmanager get-secret-value --secret-id "$secret" --query SecretString --output text)"
[[ "$password" =~ ^[A-Za-z0-9_-]{24,128}$ ]] || { echo 'invalid Secrets Manager password' >&2; exit 3; }
# The remote shell reads the two values from stdin, then psql consumes the remaining SQL.
# The password is neither a local nor a remote process argument.
{ printf '%s\n%s\n' "$password" "$THRUST_PR_ROLE_CONNECTION_LIMIT"; cat "$here/../provisioner-bootstrap.sql"; } |
  "$kubectl_cmd" -n "$namespace" exec -i "$primary" -c postgres -- sh -c \
    'IFS= read -r THRUST_PREVIEW_PROVISIONER_PASSWORD && IFS= read -r THRUST_PR_ROLE_CONNECTION_LIMIT && export THRUST_PREVIEW_PROVISIONER_PASSWORD THRUST_PR_ROLE_CONNECTION_LIMIT && exec psql -X -q -v ON_ERROR_STOP=1 -d thrust'
unset password
echo 'Provisioner database bootstrap completed on thrust-pg-dev.'
