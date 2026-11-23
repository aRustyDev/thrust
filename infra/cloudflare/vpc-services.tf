# Workers VPC services (beta) — one TCP service per Thrust CNPG cluster.
#
# A service pins the destination host and port, so a Worker bound to it can reach exactly one
# Postgres and nothing else on the cluster network. (A Workers VPC *network* binding, which
# binds a whole tunnel, is deliberately not used.)
#
# `host.hostname` + `resolver_network` makes the tunnel's cloudflared resolve the name; with no
# resolver_ips it uses the connector pod's own resolver, i.e. cluster DNS, which is what makes
# `<cluster>-rw.<namespace>.svc.cluster.local` resolvable at all.
#
# Creating, updating or deleting a service needs the Connectivity Directory Admin role on the
# identity behind CLOUDFLARE_API_TOKEN. Workers VPC is in beta: fields and the resource may
# change before GA (ADR-0094 fallback: Tunnel + Access application + service token).
#
# THE HYPERDRIVE CONFIGURATION IS NOT DECLARED HERE, ON PURPOSE. The provider does have
# cloudflare_hyperdrive_config with origin.service_id, but its origin.password is a plain
# `required, sensitive` string — not write-only — so declaring it would write the database
# password into this root's R2 state. Secrets never enter state in this repo
# (.claude/rules/security.md). The Hyperdrive configs are created out-of-band, see README.md
# and scripts/hyperdrive-check.sh.

resource "cloudflare_connectivity_directory_service" "postgres" {
  for_each = merge(var.vpc_services, var.preview_vpc_services)

  account_id   = var.cloudflare_account_id
  name         = each.key
  type         = "tcp"
  tcp_port     = 5432
  app_protocol = "postgresql"

  host = {
    hostname = each.value.hostname
    resolver_network = {
      tunnel_id = cloudflare_zero_trust_tunnel_cloudflared.thrust_pg.id
    }
  }

  tls_settings = {
    cert_verification_mode = var.cert_verification_mode
  }
}
