# Auth is the CLOUDFLARE_API_TOKEN environment variable, never a value in this tree. The token needs, on the account: Flagship
# app Edit, and Access: Apps and Policies Edit (README.md). The R2 state backend is authenticated separately.
provider "cloudflare" {}
