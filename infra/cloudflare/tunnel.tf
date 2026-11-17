# Dedicated remotely-managed tunnel for the Thrust CNPG clusters (ADR 0022, plane-byd.4).
#
# It lives in THIS product root, on R2 state, and not in organization/cloudflare: that root has a
# LOCAL backend that may only be applied from the canonical checkout, and it already owns the
# woven-o11y WARP tunnel. A tunnel that exists only to carry Workers VPC traffic for one product
# has no reason to share state, an apply path or a blast radius with it. Same call as
# shared/cloudflare/proton.
#
# !! DO NOT add a cloudflare_zero_trust_tunnel_cloudflared_config resource. That object has
# !! single-PUT replace semantics, and Workers VPC does not use it: routing is the VPC service's
# !! host + port, not tunnel ingress rules.
#
# !! DO NOT add a hostname route, a private-network route or an Access application. No .woven
# !! hostname is involved (Workers reach the database through a VPC service, not WARP), so the
# !! CoreDNS / external-dns rules do not apply and a route here would only widen the tunnel.
#
# !! Deliberately NO token data source and NO token output. This root's state lives on R2 and a
# !! token data source would persist the connector credential into it. The operator fetches the
# !! token once through the API into AWS Secrets Manager (README.md) and ESO delivers it to the
# !! in-cluster connector (still owned by the infrastructure repo's products/thrust/kube/cloudflared.tf).

resource "cloudflare_zero_trust_tunnel_cloudflared" "thrust_pg" {
  account_id = var.cloudflare_account_id
  name       = var.tunnel_name
  config_src = "cloudflare" # remotely managed: token-based connector in-cluster
}
