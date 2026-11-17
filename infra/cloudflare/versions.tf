terraform {
  required_version = ">= 1.10.0"

  # Partial backend — key products/thrust/cloudflare/646535719915/terraform.tfstate (Cloudflare R2).
  # The state-key spelling is intentionally retained after this root moved repositories.
  backend "s3" {}

  required_providers {
    cloudflare = {
      source = "cloudflare/cloudflare"
      # >= 5.13.0 is the first release carrying cloudflare_connectivity_directory_service
      # (the Workers VPC service resource). The lockfile pins the exact version.
      version = "~> 5.13"
    }
  }
}
