# Hermetic contract for the edge's Cloudflare root (ADR-0111): no credentials, no network, a mocked provider.
#   tofu -chdir=infra/edge init -backend=false && tofu -chdir=infra/edge test
# It pins what the edge depends on: the opt-in paths are exactly policy.json's audiences on the shared hostname and nothing
# wider; who they admit is only groups (by id) and the CI token; the Previews admit ONLY the edge's token; and no input can carry
# a person. `apply` runs against the mock only, so computed ids exist to compare.

mock_provider "cloudflare" {}

variables {
  cloudflare_account_id = "0123456789abcdef0123456789abcdef"
}

run "by_default_the_app_and_one_opt_in_path_per_audience_admitting_no_one" {
  command = apply

  assert {
    condition     = cloudflare_flagship_app.thrust.name == "thrust" && cloudflare_flagship_app.thrust.account_id == var.cloudflare_account_id
    error_message = "The Flagship app is `thrust`, in the account given."
  }

  # policy.json's audiences (.github/deploy/flagship/policy.json): a change there must be seen here too
  assert {
    condition     = toset(keys(cloudflare_zero_trust_access_application.opt_in)) == toset(["dev", "rc"])
    error_message = "One opt-in application per policy.json audience (dev, rc), and no other."
  }

  assert {
    condition = alltrue([
      for aud, app in cloudflare_zero_trust_access_application.opt_in :
      app.type == "self_hosted" && length(app.destinations) == 1 && app.destinations[0].type == "public" &&
      app.destinations[0].uri == "thrust.sow.foo/_edge/opt-in/${aud}" && app.app_launcher_visible == false
    ])
    error_message = "Each opt-in application protects exactly thrust.sow.foo/_edge/opt-in/<audience>, a public path, and nothing else."
  }

  assert {
    condition     = alltrue([for app in values(cloudflare_zero_trust_access_application.opt_in) : length(app.policies) == 0])
    error_message = "With no group and no CI token, an opt-in application has no policy, so it admits no one."
  }

  assert {
    condition = (
      length(cloudflare_zero_trust_access_policy.opt_in_people) == 0 && length(cloudflare_zero_trust_access_policy.opt_in_ci) == 0 &&
      length(cloudflare_zero_trust_access_application.previews) == 0 && length(cloudflare_zero_trust_access_policy.edge_previews) == 0
    )
    error_message = "Nothing else is made until its inputs are given."
  }

  assert {
    condition     = output.flagship_app_id == cloudflare_flagship_app.thrust.id && toset(keys(output.opt_in_aud)) == toset(["dev", "rc"]) && output.previews_application_id == null
    error_message = "The outputs name the app, each audience's AUD tag, and no Previews application."
  }
}

run "groups_and_the_ci_token_admit_people_first_then_ci" {
  command = apply

  variables {
    opt_in_group_ids   = { rc = ["0b6f0d7e-1a2b-4c3d-8e9f-0a1b2c3d4e5f", "1c7a1e8f-2b3c-4d5e-9f0a-1b2c3d4e5f60"], dev = [] }
    ci_opt_in_token_id = "2d8b2f90-3c4d-4e5f-8a1b-2c3d4e5f6071"
  }

  assert {
    condition = (
      toset(keys(cloudflare_zero_trust_access_policy.opt_in_people)) == toset(["rc"]) &&
      cloudflare_zero_trust_access_policy.opt_in_people["rc"].decision == "allow" &&
      toset([for r in cloudflare_zero_trust_access_policy.opt_in_people["rc"].include : r.group.id]) == toset(["0b6f0d7e-1a2b-4c3d-8e9f-0a1b2c3d4e5f", "1c7a1e8f-2b3c-4d5e-9f0a-1b2c3d4e5f60"])
    )
    error_message = "rc's people policy allows exactly its two groups, by id; dev, with no group, has none."
  }

  assert {
    condition = (
      length(cloudflare_zero_trust_access_policy.opt_in_ci) == 1 &&
      cloudflare_zero_trust_access_policy.opt_in_ci[0].decision == "non_identity" &&
      length(cloudflare_zero_trust_access_policy.opt_in_ci[0].include) == 1 &&
      one(cloudflare_zero_trust_access_policy.opt_in_ci[0].include).service_token.token_id == "2d8b2f90-3c4d-4e5f-8a1b-2c3d4e5f6071"
    )
    error_message = "The CI policy is non_identity and admits exactly the CI opt-in token."
  }

  assert {
    condition = (
      [for p in cloudflare_zero_trust_access_application.opt_in["rc"].policies : p.id] ==
      [cloudflare_zero_trust_access_policy.opt_in_people["rc"].id, cloudflare_zero_trust_access_policy.opt_in_ci[0].id] &&
      [for p in cloudflare_zero_trust_access_application.opt_in["rc"].policies : p.precedence] == [1, 2]
    )
    error_message = "rc's path applies its people policy first, then the CI token's."
  }

  assert {
    condition = (
      [for p in cloudflare_zero_trust_access_application.opt_in["dev"].policies : p.id] == [cloudflare_zero_trust_access_policy.opt_in_ci[0].id] &&
      [for p in cloudflare_zero_trust_access_application.opt_in["dev"].policies : p.precedence] == [1]
    )
    error_message = "dev's path, with no group, admits only the CI token."
  }

  assert {
    condition     = length(cloudflare_zero_trust_access_application.previews) == 0
    error_message = "The Previews application is not made by the opt-in inputs."
  }
}

run "the_previews_admit_only_the_edges_token" {
  command = apply

  variables {
    edge_previews_token_id = "3e9c3a01-4d5e-4f60-9b2c-3d4e5f607182"
    production_worker_id   = "fedcba9876543210fedcba9876543210"
  }

  assert {
    condition = (
      length(cloudflare_zero_trust_access_application.previews) == 1 &&
      length(cloudflare_zero_trust_access_application.previews[0].destinations) == 1 &&
      cloudflare_zero_trust_access_application.previews[0].destinations[0].type == "preview_worker" &&
      cloudflare_zero_trust_access_application.previews[0].destinations[0].worker_id == "fedcba9876543210fedcba9876543210"
    )
    error_message = "The Previews application covers the production Worker's Previews (preview_worker), and only them."
  }

  assert {
    condition = (
      [for p in cloudflare_zero_trust_access_application.previews[0].policies : p.id] == [cloudflare_zero_trust_access_policy.edge_previews[0].id] &&
      cloudflare_zero_trust_access_policy.edge_previews[0].decision == "non_identity" &&
      length(cloudflare_zero_trust_access_policy.edge_previews[0].include) == 1 &&
      one(cloudflare_zero_trust_access_policy.edge_previews[0].include).service_token.token_id == "3e9c3a01-4d5e-4f60-9b2c-3d4e5f607182"
    )
    error_message = "The Previews admit exactly one credential, the edge's token, and no identity."
  }

  assert {
    condition     = output.previews_application_id == cloudflare_zero_trust_access_application.previews[0].id
    error_message = "The output names the Previews application."
  }
}

run "the_previews_need_both_the_token_and_the_worker" {
  command = plan

  variables {
    edge_previews_token_id = "3e9c3a01-4d5e-4f60-9b2c-3d4e5f607182"
  }

  assert {
    condition     = length(cloudflare_zero_trust_access_application.previews) == 0 && length(cloudflare_zero_trust_access_policy.edge_previews) == 0
    error_message = "A token with no Worker to protect makes no Previews application."
  }
}

run "an_email_where_a_group_goes_is_refused" {
  command = plan
  variables {
    opt_in_group_ids = { rc = ["someone@example.com"] }
  }
  expect_failures = [var.opt_in_group_ids]
}

run "a_group_for_an_audience_with_no_path_is_refused" {
  command = plan
  variables {
    opt_in_group_ids = { ops = ["0b6f0d7e-1a2b-4c3d-8e9f-0a1b2c3d4e5f"] }
  }
  expect_failures = [cloudflare_zero_trust_access_application.opt_in]
}

run "a_token_id_that_is_a_client_id_is_refused" {
  command = plan
  variables {
    ci_opt_in_token_id = "0123456789abcdef0123456789abcdef.access"
  }
  expect_failures = [var.ci_opt_in_token_id]
}

run "a_previews_token_id_that_is_not_an_id_is_refused" {
  command = plan
  variables {
    edge_previews_token_id = "thrust-edge-previews"
  }
  expect_failures = [var.edge_previews_token_id]
}

run "a_worker_name_where_its_id_goes_is_refused" {
  command = plan
  variables {
    production_worker_id = "thrust-production"
  }
  expect_failures = [var.production_worker_id]
}

run "a_hostname_with_a_path_is_refused" {
  command = plan
  variables {
    shared_hostname = "thrust.sow.foo/_edge"
  }
  expect_failures = [var.shared_hostname]
}

run "an_app_name_flagship_refuses_is_refused" {
  command = plan
  variables {
    flagship_app_name = "thrust edge"
  }
  expect_failures = [var.flagship_app_name]
}

run "an_account_id_that_is_not_one_is_refused" {
  command = plan
  variables {
    cloudflare_account_id = "acct-1"
  }
  expect_failures = [var.cloudflare_account_id]
}
