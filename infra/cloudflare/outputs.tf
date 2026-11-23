output "thrust_tunnel_id" {
  description = "Tunnel ID of the dedicated thrust-pg tunnel. Needed once to fetch the connector token (README.md). Not a secret."
  value       = cloudflare_zero_trust_tunnel_cloudflared.thrust_pg.id
}

output "vpc_service_ids" {
  description = "Workers VPC service IDs keyed by durable or canonical Preview cluster service name. Pass the matching ID to `wrangler hyperdrive create --service-id`. Not secrets."
  value       = { for name, svc in cloudflare_connectivity_directory_service.postgres : name => svc.service_id }
}
