variable "account_id" {
  description = "Sandbox Cloudflare account id (owner supplies)."
  type        = string
}

variable "zone_id" {
  description = "Zone id of the sandbox zone that will hold spike-0004.<zone_name> (owner supplies)."
  type        = string
}

variable "zone_name" {
  description = "Zone apex, e.g. sandbox.example. The public hostname is <hostname_prefix>.<zone_name>."
  type        = string
}

variable "hostname_prefix" {
  description = "Left-most label of the spike hostname. Keep spike-0004 so teardown verification can grep for it."
  type        = string
  default     = "spike-0004"

  validation {
    condition     = startswith(var.hostname_prefix, "spike-0004")
    error_message = "hostname_prefix must start with spike-0004 so the teardown verification can find leftovers."
  }
}

variable "edge_worker_name" {
  description = "Name of the already-deployed edge Worker (wrangler deploy runs BEFORE tofu apply)."
  type        = string
  default     = "spike-0004-edge"
}

variable "space_hostname_prefix" {
  description = "Left-most label of the stand-in public `space` hostname (ADR-0022): host-wide Access bypass on its own custom domain."
  type        = string
  default     = "spike-0004-space"

  validation {
    condition     = startswith(var.space_hostname_prefix, "spike-0004")
    error_message = "space_hostname_prefix must start with spike-0004 so the teardown verification can find leftovers."
  }
}

variable "space_worker_name" {
  description = "Name of the already-deployed stand-in space Worker."
  type        = string
  default     = "spike-0004-space"
}

variable "test_identity_email" {
  description = "Email of the test identity allowed through the UI Allow policy (owner supplies; needs an IdP / OTP login configured on the Access org)."
  type        = string
}

variable "service_token_duration" {
  description = "Lifetime of the throwaway service token."
  type        = string
  default     = "168h"
}

variable "enable_overlap_experiment" {
  description = "Create an extra GATED app at <host>/api/gated/* that sits UNDER the bypass app's /api/* to measure which Access app wins on overlap (docs: the more specific rule wins, nothing is inherited)."
  type        = bool
  default     = true
}
