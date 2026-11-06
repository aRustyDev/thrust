output "hostname" {
  value = local.hostname
}

output "space_hostname" {
  value = local.space_hostname
}

output "ui_app_aud" {
  description = "AUD tag of the UI app: set it as POLICY_AUD on the edge Worker so /internal/whoami can verify the Access JWT."
  value       = cloudflare_zero_trust_access_application.ui.aud
}

output "bypass_app_aud" {
  value = cloudflare_zero_trust_access_application.bypass.aud
}

output "service_token_client_id" {
  description = "Client ID (not a secret)."
  value       = cloudflare_zero_trust_access_service_token.test.client_id
}

output "service_token_client_secret" {
  description = "SECRET. Read with `tofu output -raw service_token_client_secret` INTO AN ENV VAR inside one shell call; never print or log."
  value       = cloudflare_zero_trust_access_service_token.test.client_secret
  sensitive   = true
}

output "teardown_inventory" {
  description = "Everything this root creates; the runbook's verification step greps the account for the spike-0004 prefix."
  value = {
    custom_domains = [
      cloudflare_workers_custom_domain.edge.hostname,
      cloudflare_workers_custom_domain.space.hostname,
    ]
    access_apps = concat(
      [
        cloudflare_zero_trust_access_application.ui.name,
        cloudflare_zero_trust_access_application.bypass.name,
        cloudflare_zero_trust_access_application.space_bypass.name,
      ],
      [for a in cloudflare_zero_trust_access_application.overlap_gated : a.name],
    )
    policies = [
      cloudflare_zero_trust_access_policy.service_auth.name,
      cloudflare_zero_trust_access_policy.ui_allow.name,
      cloudflare_zero_trust_access_policy.bypass.name,
    ]
    service_token = cloudflare_zero_trust_access_service_token.test.name
  }
}
