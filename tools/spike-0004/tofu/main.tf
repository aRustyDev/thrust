locals {
  hostname       = "${var.hostname_prefix}.${var.zone_name}"
  space_hostname = "${var.space_hostname_prefix}.${var.zone_name}"

  # Path-scoped bypass destinations. Access app paths (docs: cloudflare-one/access-controls/policies/app-paths):
  #   - `host/api/*` covers /api/<something> but NOT /api itself, so the exact form is listed as well;
  #   - whether a trailing `/*` covers DEEP paths (/api/a/b/c) is NOT stated by the docs: harness checks B3/B4/B7 measure it;
  #   - whether bare `/api` also matches `/api/` is unstated: harness measures it.
  bypass_prefixes = ["/api", "/auth", "/live"]
  bypass_destinations = flatten([
    for p in local.bypass_prefixes : ["${local.hostname}${p}", "${local.hostname}${p}/*"]
  ])
}

# --------------------------------------------------------------------------------------------------------
# Public hostname: Worker custom domain. Creates the DNS record + certificate itself; the Worker must
# already be deployed (`wrangler deploy` first). Custom Domain = the Worker IS the origin and ALL paths of
# the exact hostname go to it (docs: workers/configuration/routing/custom-domains).
# --------------------------------------------------------------------------------------------------------
resource "cloudflare_workers_custom_domain" "edge" {
  account_id = var.account_id
  zone_id    = var.zone_id
  hostname   = local.hostname
  service    = var.edge_worker_name
}

# --------------------------------------------------------------------------------------------------------
# Stand-in for the public `space` hostname (ADR-0022): its own Worker on its own custom domain, host-wide Bypass.
# Spike step 6: the narrow bypass on THIS hostname must not expose the product hostname's gated routes.
# --------------------------------------------------------------------------------------------------------
resource "cloudflare_workers_custom_domain" "space" {
  account_id = var.account_id
  zone_id    = var.zone_id
  hostname   = local.space_hostname
  service    = var.space_worker_name
}

# --------------------------------------------------------------------------------------------------------
# Test credentials for the GATED paths. The secret lands in LOCAL STATE (client_secret is computed and
# sensitive): state is gitignored and destroyed with the root. Never print it; the runbook reads it into
# the environment inside one shell call.
# --------------------------------------------------------------------------------------------------------
resource "cloudflare_zero_trust_access_service_token" "test" {
  account_id = var.account_id
  name       = "spike-0004-test-token"
  duration   = var.service_token_duration
}

# --------------------------------------------------------------------------------------------------------
# Policies (v5: account-level reusable policies referenced from apps by id + precedence).
# Docs: Bypass and Service Auth are evaluated BEFORE Allow/Block regardless of the order shown.
# --------------------------------------------------------------------------------------------------------
resource "cloudflare_zero_trust_access_policy" "service_auth" {
  account_id = var.account_id
  name       = "spike-0004-service-auth-test-token"
  decision   = "non_identity" # UI label: "Service Auth"
  include = [{
    service_token = { token_id = cloudflare_zero_trust_access_service_token.test.id }
  }]
}

resource "cloudflare_zero_trust_access_policy" "ui_allow" {
  account_id = var.account_id
  name       = "spike-0004-ui-allow-test-identity"
  decision   = "allow"
  include = [{
    email = { email = var.test_identity_email }
  }]
}

resource "cloudflare_zero_trust_access_policy" "bypass" {
  account_id = var.account_id
  name       = "spike-0004-bypass-everyone"
  decision   = "bypass" # disables ALL Access enforcement AND logging for matching traffic (docs)
  include = [{
    everyone = {}
  }]
}

# --------------------------------------------------------------------------------------------------------
# App 1: the UI gate. Destination is the bare hostname = every path, unless a MORE SPECIFIC app matches.
# Service Auth lets the harness authenticate non-interactively; Allow lets a human test identity in.
# --------------------------------------------------------------------------------------------------------
resource "cloudflare_zero_trust_access_application" "ui" {
  account_id       = var.account_id
  name             = "spike-0004-ui"
  type             = "self_hosted"
  session_duration = "1h"

  destinations = [{
    type = "public"
    uri  = local.hostname
  }]

  policies = [
    { id = cloudflare_zero_trust_access_policy.service_auth.id, precedence = 1 },
    { id = cloudflare_zero_trust_access_policy.ui_allow.id, precedence = 2 },
  ]
}

# --------------------------------------------------------------------------------------------------------
# App 2: the bypass paths (/api, /auth, /live and their /* forms). More specific than App 1, so by the
# documented rule it wins for those paths and inherits nothing from App 1.
# --------------------------------------------------------------------------------------------------------
resource "cloudflare_zero_trust_access_application" "bypass" {
  account_id = var.account_id
  name       = "spike-0004-bypass"
  type       = "self_hosted"

  destinations = [for d in local.bypass_destinations : { type = "public", uri = d }]

  policies = [
    { id = cloudflare_zero_trust_access_policy.bypass.id, precedence = 1 },
  ]
}

# --------------------------------------------------------------------------------------------------------
# App 3: the public space hostname. Destination is the bare hostname = every path of THAT hostname only.
# The product hostname's apps (App 1/2) must not match it, and this app must not match the product hostname.
# --------------------------------------------------------------------------------------------------------
resource "cloudflare_zero_trust_access_application" "space_bypass" {
  account_id = var.account_id
  name       = "spike-0004-space-bypass"
  type       = "self_hosted"

  destinations = [{
    type = "public"
    uri  = local.space_hostname
  }]

  policies = [
    { id = cloudflare_zero_trust_access_policy.bypass.id, precedence = 1 },
  ]
}

# --------------------------------------------------------------------------------------------------------
# Overlap experiment: a GATED app nested under the bypass app's /api/*. Docs: "the more specific rule takes
# precedence and no rule is inherited". Harness check O1 asserts /api/gated/* is refused while /api/ping stays open.
# Remove with -var enable_overlap_experiment=false.
# --------------------------------------------------------------------------------------------------------
resource "cloudflare_zero_trust_access_application" "overlap_gated" {
  count = var.enable_overlap_experiment ? 1 : 0

  account_id       = var.account_id
  name             = "spike-0004-overlap-gated-under-api"
  type             = "self_hosted"
  session_duration = "1h"

  destinations = [
    { type = "public", uri = "${local.hostname}/api/gated/*" },
    { type = "public", uri = "${local.hostname}/api/gated" },
  ]

  policies = [
    { id = cloudflare_zero_trust_access_policy.service_auth.id, precedence = 1 },
  ]
}
