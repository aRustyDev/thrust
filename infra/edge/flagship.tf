# The Flagship app the edge evaluates (ADR-0111): the app only. Every flag inside it is thrust CI's (scripts/flagship.py, from
# .github/deploy/flagship/policy.json), so a `cloudflare_flagship_flag` here would be a second owner fighting the sync: the
# workflow's guard refuses one in this root.
#
# Once the edge binds it, Flagship itself refuses to delete it ("referenced by Worker script(s)", measured by the spike), so a
# stray destroy or replacement fails at Cloudflare. (`prevent_destroy` would say so earlier, but it also stops the contract
# test from tearing its mocked resources down.)
resource "cloudflare_flagship_app" "thrust" {
  account_id = var.cloudflare_account_id
  name       = var.flagship_app_name
}
