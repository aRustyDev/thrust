# None of these is a secret. Where each one goes is in README.md.

output "flagship_app_id" {
  description = "The Flagship app's id: thrust's repository variable FLAGSHIP_APP_ID (which turns flagship.yml on) and the edge's binding."
  value       = cloudflare_flagship_app.thrust.id
}

output "opt_in_aud" {
  description = "Per audience, the AUD tag of its opt-in application: the environment `edge`'s THRUST_ACCESS_AUD_<AUDIENCE>."
  value       = { for aud, app in cloudflare_zero_trust_access_application.opt_in : aud => app.aud }
}

output "previews_application_id" {
  description = "The id of the Access application on the production Worker's Previews, or null while it is not made."
  value       = local.previews ? cloudflare_zero_trust_access_application.previews[0].id : null
}
