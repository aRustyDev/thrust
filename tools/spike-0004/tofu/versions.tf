# Spike 0004 THROWAWAY OpenTofu root.
#
# - NOT an infra-repo root. No remote backend: state is a LOCAL file whose path run_platform.sh supplies with
#   `-backend-config=path=...` and points OUTSIDE the git worktree (default ~/.local/state/spike-0004/), because a
#   worktree can be pruned while resources still exist and a state file beside a worktree is lost with it
#   (same hazard as the estate's local-backend roots). It holds the service-token secret: mode 700 dir, never commit.
# - Run it only after the owner has named the sandbox account + zone (research/0006 runbook).
# - Every resource is named with the prefix "spike-0004-". Teardown: `tofu destroy`, then `wrangler delete`
#   for both Workers; verification steps are in the runbook.
# - This session only ran `tofu init -backend=false` + `tofu validate`. No plan/apply was run.

terraform {
  required_version = ">= 1.6.0"

  # Partial configuration: `path` is supplied at init time (see run_platform.sh). Local backend only.
  backend "local" {}

  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 5.0"
    }
  }
}

# Credentials: CLOUDFLARE_API_TOKEN in the environment (read from 1Password inside ONE shell call; see runbook).
# Never put a token in this file or in a tfvars file.
provider "cloudflare" {}
