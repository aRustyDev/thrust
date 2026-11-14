# Files the deploy trusts

`cloudflare-origin-ca-rsa-root.pem` is the Cloudflare Origin CA RSA root certificate. `thrust-pg` serves a certificate that this root
signed (infrastructure repository, ADR 0023), and the deploy's expand migration checks it with `DATABASE_SSLMODE=verify-ca` and this file
as `DATABASE_SSLROOTCERT` (`scripts/migrate-ci.sh`, `thrust_adapters_postgres::tls`).

It is a public certificate and it is byte for byte the one Cloudflare publishes
(<https://developers.cloudflare.com/ssl/static/origin_ca_rsa_root.pem>); the infrastructure repository keeps the same file.
`.github/scripts/tests/test-deploy-ca.sh` pins its SHA-256 fingerprint, so changing the file is a change a reviewer has to look at, and the
`verify-ca` code is tested against exactly these bytes (`crates/thrust-adapters-postgres/tests/tls.rs`).

| | |
|---|---|
| Subject | CloudFlare Origin SSL Certificate Authority |
| Key | RSA 2048, SHA-256 |
| SHA-256 fingerprint | `D3:C7:E8:5C:91:70:7F:C0:A1:2A:BC:5D:88:26:67:47:AA:4F:A8:E7:B1:62:F6:33:FF:B3:C9:D9:89:94:76:20` |
| Not after | 2029-08-15 17:00 UTC |

Before that date the root has to be replaced in the infrastructure repository and here, or the expand migration stops verifying.
