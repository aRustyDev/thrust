# Every input is an identifier, never a person: this repository is public, so who may opt in is an Access group made and kept
# elsewhere, named here only by its id. The ids reach the plan as TF_VAR_* from the protected environment `edge-infrastructure`
# (README.md), not from a file in this tree.

variable "cloudflare_account_id" {
  description = "Cloudflare account ID that owns the edge Worker, the production Worker and their Access applications."
  type        = string
  nullable    = false

  validation {
    condition     = can(regex("^[0-9a-f]{32}$", var.cloudflare_account_id))
    error_message = "cloudflare_account_id must be 32 lowercase hexadecimal digits."
  }
}

variable "flagship_app_name" {
  description = "Name of the Flagship app the edge evaluates. Thrust reads only its id (FLAGSHIP_APP_ID)."
  type        = string
  default     = "thrust"
  nullable    = false

  validation {
    condition     = can(regex("^[A-Za-z0-9_-]{1,64}$", var.flagship_app_name))
    error_message = "flagship_app_name must be 1 to 64 letters, digits, '-' or '_' (Flagship's rule)."
  }
}

variable "shared_hostname" {
  description = "The shared hostname the edge serves (ADR-0111). Its opt-in paths are /_edge/opt-in/<audience>."
  type        = string
  default     = "thrust.sow.foo"
  nullable    = false

  validation {
    condition     = can(regex("^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$", var.shared_hostname))
    error_message = "shared_hostname must be a lowercase DNS name with no scheme, port or path."
  }
}

variable "opt_in_group_ids" {
  description = "Per audience, the Access groups whose members may opt in as a person (an allow policy). Group ids only: the groups, and the people in them, are kept outside this public repository. An audience with no group admits people from none."
  type        = map(list(string))
  default     = {}
  nullable    = false

  validation {
    condition = alltrue(flatten([
      for aud, ids in var.opt_in_group_ids : [
        for id in ids : can(regex("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", id))
      ]
    ]))
    error_message = "opt_in_group_ids holds Access group ids (lowercase UUIDs) only: never an email or a name."
  }
}

variable "ci_opt_in_token_id" {
  description = "The id (a UUID, not the client id or secret) of the service token thrust-ci-opt-in, made by service-token.py. The target deploys opt in with it. Empty: no CI policy on the opt-in paths."
  type        = string
  default     = ""
  nullable    = false

  validation {
    condition     = var.ci_opt_in_token_id == "" || can(regex("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", var.ci_opt_in_token_id))
    error_message = "ci_opt_in_token_id must be empty or a service token id (a lowercase UUID)."
  }
}

variable "edge_previews_token_id" {
  description = "The id of the service token thrust-edge-previews, made by service-token.py: the only credential the Previews' Access application admits, and the one the edge shows. Empty (or production_worker_id empty): no Previews application yet."
  type        = string
  default     = ""
  nullable    = false

  validation {
    condition     = var.edge_previews_token_id == "" || can(regex("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", var.edge_previews_token_id))
    error_message = "edge_previews_token_id must be empty or a service token id (a lowercase UUID)."
  }
}

variable "production_worker_id" {
  description = "The production Worker's id (its script tag, 32 hex digits: the `tag` of thrust-production in GET /accounts/<id>/workers/scripts), whose Previews only the edge may reach. Empty: no Previews application yet."
  type        = string
  default     = ""
  nullable    = false

  validation {
    condition     = var.production_worker_id == "" || can(regex("^[0-9a-f]{32}$", var.production_worker_id))
    error_message = "production_worker_id must be empty or a Worker's script tag (32 lowercase hexadecimal digits)."
  }
}
