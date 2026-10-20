# ADR index

MADR format (`_template.md`). Directory = architecture group; number is global and permanent.
`accepted` = backed by a recorded owner decision (interview 2026-10-17). `proposed` = options analysed, decision
pending — linked to its `Q-<adr#>` row in `../QUESTIONS.md`. New ADRs take the next free number in the group's range.

| # | Group | Title | Status | Question |
|---|---|---|---|---|
| 0001 | architecture | [Hybrid: keep frontend, rewrite backend](architecture/0001-hybrid-keep-frontend-rewrite-backend.md) | accepted | |
| 0002 | architecture | [Ports and adapters portability](architecture/0002-ports-and-adapters-portability.md) | accepted | |
| 0003 | architecture | [Single-org first, multi-tenant-capable](architecture/0003-tenancy-single-org-multitenant-capable.md) | accepted | |
| 0004 | architecture | [Open-EE superseded; native implementation](architecture/0004-open-ee-superseded-native-implementation.md) | accepted | |
| 0005 | architecture | [Monorepo and package layout](architecture/0005-monorepo-and-package-layout.md) | accepted | |
| 0006 | architecture | [Runtime language and framework](architecture/0006-runtime-language-and-framework.md) | accepted | |
| 0007 | architecture | [Adapter selection mechanism](architecture/0007-adapter-selection-mechanism.md) | accepted | |
| 0008 | architecture | [Feature flags port](architecture/0008-feature-flags-port.md) | accepted | |
| 0010 | ui | [Service-layer adaptation](ui/0010-service-layer-adaptation.md) | superseded by 0034 | |
| 0011 | ui | [Static asset hosting](ui/0011-static-asset-hosting.md) | accepted | |
| 0012 | ui | [Editor and i18n reuse](ui/0012-editor-and-i18n-reuse.md) | accepted | |
| 0013 | ui | [Rust frontend rewrite (Dioxus), multi-target](ui/0013-rust-frontend-dioxus-multitarget.md) | accepted | |
| 0020 | ux | [Parity bar](ux/0020-parity-bar.md) | accepted | |
| 0021 | ux | [Admin (god-mode) and instance config](ux/0021-admin-god-mode-and-instance-config.md) | accepted | |
| 0022 | ux | [Public spaces](ux/0022-public-spaces.md) | accepted | |
| 0030 | api | [Internal API style](api/0030-internal-api-style.md) | superseded by 0034 | |
| 0031 | api | [Public v1 compatibility](api/0031-public-v1-compatibility.md) | accepted | |
| 0032 | api | [Versioning and OpenAPI](api/0032-versioning-and-openapi.md) | accepted | |
| 0033 | api | [Rate limiting and API keys](api/0033-rate-limiting-and-api-keys.md) | accepted | |
| 0034 | api | [Internal API: compat layer + core OpenAPI](api/0034-internal-api-compat-layer-and-core-openapi.md) | accepted | |
| 0040 | data/base | [System-of-record database (+ spike)](data/base/0040-system-of-record-database.md) | proposed | Q-0040 |
| 0041 | data/base | [Tenant isolation strategy](data/base/0041-tenant-isolation-strategy.md) | proposed | Q-0041 |
| 0042 | data/schema | [Schema source and migrations](data/schema/0042-schema-source-and-migrations.md) | accepted | |
| 0043 | data/schema | [IDs, sequencing, soft delete](data/schema/0043-ids-sequencing-and-soft-delete.md) | accepted | |
| 0044 | data/layer | [Query layer / ORM](data/layer/0044-query-layer-orm.md) | accepted | |
| 0045 | data/interfaces | [Repository and unit-of-work ports](data/interfaces/0045-repository-and-unit-of-work-ports.md) | accepted | |
| 0046 | data/contracts | [Event / outbox contract](data/contracts/0046-event-outbox-contract.md) | accepted | |
| 0047 | data/contracts | [File-asset contract](data/contracts/0047-file-asset-contract.md) | accepted | |
| 0048 | data/contracts | [Audit logging](data/contracts/0048-audit-logging.md) | accepted | |
| 0050 | auth | [Identity: OIDC (Zitadel) + Access](auth/0050-identity-oidc-zitadel-access.md) | accepted | |
| 0051 | auth | [Authorization model: RBAC / ReBAC](auth/0051-authorization-model-rbac-rebac.md) | accepted | |
| 0052 | auth | [Sessions and API keys](auth/0052-sessions-and-api-keys.md) | accepted | |
| 0053 | auth | [SCIM provisioning](auth/0053-scim-provisioning.md) | accepted | |
| 0060 | collab | [Yjs collab provider port](collab/0060-yjs-collab-provider-port.md) | accepted | |
| 0061 | collab | [Collab persistence and versioning](collab/0061-collab-persistence-and-versioning.md) | proposed | Q-0061 |
| 0070 | jobs | [Queue port](jobs/0070-queue-port.md) | accepted | |
| 0071 | jobs | [Workflow port](jobs/0071-workflow-port.md), superseded in part (round 29) | accepted | |
| 0072 | jobs | [Cron port](jobs/0072-cron-port.md) | accepted | |
| 0073 | jobs | [Notifications, email, webhooks pipeline](jobs/0073-notifications-email-webhooks-pipeline.md) | accepted | |
| 0080 | storage | [Blob port (R2)](storage/0080-blob-port-r2.md) | accepted | |
| 0081 | storage | [Uploads and exports](storage/0081-uploads-and-exports.md) | accepted | |
| 0082 | storage | [Cache and session port (KV)](storage/0082-cache-and-session-port-kv.md) | accepted | |
| 0083 | search | [Search port](search/0083-search-port.md) | accepted | |
| 0084 | integrations | [AI port](integrations/0084-ai-port.md) | accepted | |
| 0085 | integrations | [GitHub, Slack, Beads, importers](integrations/0085-integrations-github-slack-beads-importers.md) | accepted | |
| 0090 | infra | [Cloudflare topology](infra/0090-cloudflare-topology.md) | accepted | |
| 0091 | infra | [Self-hosted twin](infra/0091-self-hosted-twin.md) | accepted | |
| 0092 | infra | [Secrets and environments](infra/0092-secrets-and-environments.md) | accepted | |
| 0093 | infra | [IaC tooling](infra/0093-iac-tooling.md) | accepted | |
| 0094 | infra | [Postgres hosting: CNPG in woven-o11y via Tunnel and Workers VPC](infra/0094-postgres-hosting-cnpg-tunnel-vpc.md) | accepted | Q-0094.6 |
| 0100 | ci | [Build matrix per adapter set](ci/0100-build-matrix-per-adapter-set.md) | accepted | |
| 0101 | ci | [CI supply chain](ci/0101-ci-supply-chain.md) | accepted | |
| 0110 | cd | [Deploy pipeline](cd/0110-deploy-pipeline.md) | accepted | |
| 0120 | test/models | [Test model and oracle](test/models/0120-test-model-and-oracle.md) | accepted | |
| 0121 | test/patterns | [Port contract tests: one suite, N adapters](test/patterns/0121-port-contract-tests-one-suite-n-adapters.md) | accepted | |
| 0122 | test/patterns | [v1 oracle and parity tests](test/patterns/0122-v1-oracle-and-parity-tests.md) | proposed | Q-0122 |
| 0123 | test/standards | [Test standards and tooling](test/standards/0123-test-standards-and-tooling.md) | accepted | |
| 0130 | observability | [Observability](observability/0130-observability.md) | accepted | |
| 0140 | security | [Licensing (AGPL) and clean-room](security/0140-licensing-agpl-cleanroom.md) | accepted | |
| 0141 | security | [Tenancy and threat model](security/0141-tenancy-and-threat-model.md) | proposed | Q-0141 |

Status counts: 53 accepted · 5 proposed · 2 superseded (60 total). Several `proposed` ADRs carry dated "Update — interview round N"
notes recording partial owner direction; they stay `proposed` until the remaining question is answered.
