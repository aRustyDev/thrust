variable "cloudflare_account_id" {
  description = "Cloudflare account ID (woven-solutions) that owns the Thrust tunnel and Workers VPC services. Workers VPC services and the Hyperdrive configs that use them must live in the same account as the Workers that bind them."
  type        = string
  nullable    = false

  validation {
    condition     = can(regex("^[0-9a-fA-F]{32}$", var.cloudflare_account_id))
    error_message = "cloudflare_account_id must be a 32-character hexadecimal Cloudflare account ID."
  }
}

variable "tunnel_name" {
  description = "Display name of the dedicated, remotely-managed Cloudflare Tunnel that carries Workers VPC traffic to the Thrust CNPG clusters. Dedicated so no .woven/WARP hostname route, CoreDNS rewrite or Access application is ever attached to it."
  type        = string
  default     = "thrust-pg"
  nullable    = false
}

variable "vpc_services" {
  description = "Workers VPC TCP services, one per Thrust CNPG cluster, keyed by the VPC service name. hostname is the in-cluster read-write Service FQDN (<cluster>-rw.<namespace>.svc.cluster.local) that the tunnel's cloudflared resolves with the connector pod's cluster DNS. A service pins host and port, which is what stops a Worker reaching any other destination (SSRF containment)."
  type = map(object({
    hostname = string
  }))
  default = {
    "thrust-pg"     = { hostname = "thrust-pg-rw.thrust.svc.cluster.local" }
    "thrust-pg-dev" = { hostname = "thrust-pg-dev-rw.thrust.svc.cluster.local" }
  }
  nullable = false

  validation {
    condition = alltrue([
      for name, svc in var.vpc_services :
      can(regex("^[a-z0-9]([a-z0-9-]*[a-z0-9])?$", name)) &&
      can(regex("^[a-z0-9]([a-z0-9-]*[a-z0-9])?-rw\\.[a-z0-9-]+\\.svc\\.cluster\\.local$", svc.hostname))
    ])
    error_message = "Each vpc_services key must be a DNS-label-style name and each hostname the read-write Service FQDN <cluster>-rw.<namespace>.svc.cluster.local (never the -ro/-r replica Services: Hyperdrive writes)."
  }
}

variable "preview_vpc_services" {
  description = "Ephemeral Preview Workers VPC TCP services keyed by the deterministic Preview cluster name. Supply as protected TF_VAR_preview_vpc_services JSON; this map is merged with, never replaces, vpc_services. Every entry is pinned to its own CNPG -rw Service in preview_namespace."
  type = map(object({
    hostname = string
  }))
  default  = {}
  nullable = false

  validation {
    condition = alltrue([
      for name, svc in var.preview_vpc_services :
      can(regex("^thrust-preview-pr-[1-9][0-9]*-[0-9a-f]{12}$", name)) &&
      svc.hostname == "${name}-rw.${var.preview_namespace}.svc.cluster.local"
    ])
    error_message = "Each preview_vpc_services key must be the deterministic thrust-preview-pr-<number>-<12 lowercase hex> cluster name and must pin exactly its own -rw Service in preview_namespace."
  }
}

variable "preview_namespace" {
  description = "Dedicated Kubernetes namespace that contains ephemeral Preview CNPG clusters. The VPC service validator derives the only permitted preview hostname from it."
  type        = string
  default     = "thrust-preview"
  nullable    = false

  validation {
    condition     = can(regex("^[a-z0-9]([a-z0-9-]*[a-z0-9])?$", var.preview_namespace))
    error_message = "preview_namespace must be a DNS-label-style Kubernetes namespace."
  }
}

variable "cert_verification_mode" {
  description = "Workers VPC cert_verification_mode for the tunnel-to-Postgres hop. TESTED LIVE (2026-10-18): against CNPG's private cluster CA, verify_ca FAILED (Hyperdrive create: 'Failed to connect to the provided database: Internal error') and disabled WORKED, so disabled is the default and the live state. The hop is still ENCRYPTED (the origin backend reports TLSv1.3) but the database server is NOT authenticated against our CA. Workers VPC trusts only public CAs and Cloudflare Origin CA and cannot take a custom CA, so verify_full also fails against CNPG's internal CA; verify_ca and verify_full stay allowed values for when the certificate chain changes (hardening is a recorded follow-up). Changing this is an in-place update that the provider reports as an error even when it succeeds: see README.md, section Changing a VPC service."
  type        = string
  default     = "disabled"
  nullable    = false

  validation {
    condition     = contains(["verify_full", "verify_ca", "disabled"], var.cert_verification_mode)
    error_message = "cert_verification_mode must be verify_full, verify_ca or disabled."
  }
}
