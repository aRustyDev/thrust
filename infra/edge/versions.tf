terraform {
  required_version = ">= 1.10.0"

  # Partial backend: key products/thrust/edge/646535719915/terraform.tfstate on Cloudflare R2, a state of its OWN. It must
  # never be the key of another root (see README.md: infra/cloudflare and the infrastructure repository's
  # products/thrust/cloudflare share one key, and each one's apply would destroy what only the other declares).
  backend "s3" {}

  required_providers {
    cloudflare = {
      source = "cloudflare/cloudflare"
      # >= 5.20.0 is the first release carrying cloudflare_flagship_app; the lockfile pins the exact version.
      version = "~> 5.20"
    }
  }
}
