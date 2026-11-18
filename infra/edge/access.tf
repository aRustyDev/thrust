# Access for the edge (ADR-0111; measured by the live spike, research/0016):
#
#   * One application per opt-in audience, on <shared_hostname>/_edge/opt-in/<audience>, and nothing else: a path application
#     protects only its path, so the rest of the hostname stays open. The audiences are policy.json's, the same list the edge's
#     ACCESS_AUD_<AUDIENCE> variables must match (scripts/flagship.py validate checks those). Each admits the people of its
#     Access groups (allow) and the CI opt-in service token (non_identity). With neither, it admits no one.
#   * One application on the production Worker's Previews (destination preview_worker), admitting ONLY the edge's service token,
#     so a Preview can be reached only through the edge, and the pin and Flagship cannot be bypassed.
#
# The service tokens themselves are made by service-token.py, not here: the provider keeps a token's client_secret in state, and
# this state is unencrypted R2. Only their ids come in.

locals {
  policy    = jsondecode(file("${path.module}/../../.github/deploy/flagship/policy.json"))
  audiences = toset(local.policy.audiences)
  # the audiences with at least one group: an allow policy with no include would admit no one, so none is made
  people   = { for aud, ids in var.opt_in_group_ids : aud => ids if length(ids) > 0 }
  ci       = var.ci_opt_in_token_id != ""
  previews = var.edge_previews_token_id != "" && var.production_worker_id != ""
}

resource "cloudflare_zero_trust_access_policy" "opt_in_people" {
  for_each   = local.people
  account_id = var.cloudflare_account_id
  name       = "thrust-edge-opt-in-${each.key}-people"
  decision   = "allow"
  include    = [for id in each.value : { group = { id = id } }]
}

resource "cloudflare_zero_trust_access_policy" "opt_in_ci" {
  count      = local.ci ? 1 : 0
  account_id = var.cloudflare_account_id
  name       = "thrust-edge-opt-in-ci"
  decision   = "non_identity"
  include    = [{ service_token = { token_id = var.ci_opt_in_token_id } }]
}

resource "cloudflare_zero_trust_access_application" "opt_in" {
  for_each             = local.audiences
  account_id           = var.cloudflare_account_id
  name                 = "thrust-edge-opt-in-${each.key}"
  type                 = "self_hosted"
  destinations         = [{ type = "public", uri = "${var.shared_hostname}/_edge/opt-in/${each.key}" }]
  app_launcher_visible = false
  policies = concat(
    contains(keys(local.people), each.key) ? [{ id = cloudflare_zero_trust_access_policy.opt_in_people[each.key].id, precedence = 1 }] : [],
    local.ci ? [{ id = cloudflare_zero_trust_access_policy.opt_in_ci[0].id, precedence = contains(keys(local.people), each.key) ? 2 : 1 }] : [],
  )

  lifecycle {
    precondition {
      condition     = alltrue([for aud in keys(var.opt_in_group_ids) : contains(local.audiences, aud)])
      error_message = "opt_in_group_ids names an audience policy.json does not have; the edge has no opt-in path for it."
    }
  }
}

resource "cloudflare_zero_trust_access_policy" "edge_previews" {
  count      = local.previews ? 1 : 0
  account_id = var.cloudflare_account_id
  name       = "thrust-edge-previews"
  decision   = "non_identity"
  include    = [{ service_token = { token_id = var.edge_previews_token_id } }]
}

resource "cloudflare_zero_trust_access_application" "previews" {
  count                = local.previews ? 1 : 0
  account_id           = var.cloudflare_account_id
  name                 = "thrust-production-previews"
  type                 = "self_hosted"
  destinations         = [{ type = "preview_worker", worker_id = var.production_worker_id }]
  app_launcher_visible = false
  policies             = [{ id = cloudflare_zero_trust_access_policy.edge_previews[0].id, precedence = 1 }]
}
