# Auth is the CLOUDFLARE_API_TOKEN environment variable, never a value in this tree. The token
# needs account-scoped Cloudflare Tunnel Edit plus the Workers VPC (connectivity directory)
# write permission — see README.md for the exact scopes and who can hold them. The R2 state
# backend is authenticated separately by the woven-r2-tofu AWS profile.
provider "cloudflare" {}
