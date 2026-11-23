# Hermetic contract for the Thrust Cloudflare root: no credentials, no network, mocked provider.
#   tofu -chdir=infra/cloudflare test
# Run `tofu init -backend=false` first. This file pins what the VPC services MUST look like so
# a refactor cannot quietly turn a pinned-host TCP service into something wider.

mock_provider "cloudflare" {}

variables {
  cloudflare_account_id = "00000000000000000000000000000000"
}

run "two_pinned_postgres_services_on_one_tunnel" {
  command = plan

  assert {
    condition = (
      length(cloudflare_connectivity_directory_service.postgres) == 2 &&
      alltrue([
        for svc in values(cloudflare_connectivity_directory_service.postgres) :
        svc.type == "tcp" && svc.tcp_port == 5432 && svc.app_protocol == "postgresql"
      ])
    )
    error_message = "Every Thrust VPC service must be a TCP/5432 service with app_protocol postgresql."
  }

  assert {
    condition = (
      cloudflare_connectivity_directory_service.postgres["thrust-pg"].host.hostname == "thrust-pg-rw.thrust.svc.cluster.local" &&
      cloudflare_connectivity_directory_service.postgres["thrust-pg-dev"].host.hostname == "thrust-pg-dev-rw.thrust.svc.cluster.local"
    )
    error_message = "Services must pin the read-write Service FQDN of their own cluster."
  }

  assert {
    condition     = cloudflare_zero_trust_tunnel_cloudflared.thrust_pg.config_src == "cloudflare" && cloudflare_zero_trust_tunnel_cloudflared.thrust_pg.name == "thrust-pg"
    error_message = "The tunnel must be remotely managed and named thrust-pg."
  }

  # TESTED LIVE 2026-10-18 against CNPG's private cluster CA: verify_ca FAILED (Hyperdrive create:
  # "Failed to connect to the provided database: Internal error"), disabled WORKED. The default
  # is therefore the live state, so a plain `tofu plan` proposes no change.
  assert {
    condition = alltrue([
      for svc in values(cloudflare_connectivity_directory_service.postgres) :
      svc.tls_settings.cert_verification_mode == "disabled"
    ])
    error_message = "Both VPC services default to cert_verification_mode = disabled: verify_ca was tested and failed against the private CNPG CA."
  }
}

run "verify_ca_and_verify_full_stay_selectable" {
  command = plan

  variables {
    cert_verification_mode = "verify_ca"
  }

  assert {
    condition     = alltrue([for svc in values(cloudflare_connectivity_directory_service.postgres) : svc.tls_settings.cert_verification_mode == "verify_ca"])
    error_message = "verify_ca must remain an allowed value for when the CNPG certificate chain is hardened."
  }
}

run "a_preview_service_is_added_without_replacing_standard_services" {
  command = plan

  variables {
    preview_vpc_services = {
      "thrust-preview-pr-42-43cba3a2caeb" = {
        hostname = "thrust-preview-pr-42-43cba3a2caeb-rw.thrust-preview.svc.cluster.local"
      }
    }
  }

  assert {
    condition = (
      length(cloudflare_connectivity_directory_service.postgres) == 3 &&
      cloudflare_connectivity_directory_service.postgres["thrust-preview-pr-42-43cba3a2caeb"].host.hostname == "thrust-preview-pr-42-43cba3a2caeb-rw.thrust-preview.svc.cluster.local" &&
      cloudflare_connectivity_directory_service.postgres["thrust-pg"].host.hostname == "thrust-pg-rw.thrust.svc.cluster.local"
    )
    error_message = "A Preview service must be additive and pin exactly its own read-write CNPG hostname."
  }
}

run "rejects_a_preview_service_with_a_noncanonical_name_or_hostname" {
  command = plan

  variables {
    preview_vpc_services = {
      "thrust-preview-pr-42-not-hex" = {
        hostname = "thrust-pg-dev-rw.thrust.svc.cluster.local"
      }
    }
  }

  expect_failures = [var.preview_vpc_services]
}

run "rejects_a_replica_service_host" {
  command = plan

  variables {
    vpc_services = {
      "thrust-pg" = { hostname = "thrust-pg-ro.thrust.svc.cluster.local" }
    }
  }

  expect_failures = [var.vpc_services]
}

run "rejects_an_unknown_verification_mode" {
  command = plan

  variables {
    cert_verification_mode = "off"
  }

  expect_failures = [var.cert_verification_mode]
}

run "rejects_an_empty_or_malformed_account_id" {
  command = plan

  variables {
    cloudflare_account_id = "not-an-account-id"
  }

  expect_failures = [var.cloudflare_account_id]
}
