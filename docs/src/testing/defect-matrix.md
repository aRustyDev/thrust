# Defect matrix

Each row is a class of defect that matters for thrust. **Primary** is the technique expected to catch it first;
**backstop** is an independent technique that catches it if the primary test is wrong, missing or vacuous. Names in
code spans are real tests, `mutant_*` cargo features or scripts in the repo today. A row with no primary is a gap and
is listed in [Status](status.md).

| Id | Defect class | Primary | Backstop | Target (owner) |
|---|---|---|---|---|
| X1 | Cross-tenant read or write | contract `cross_tenant_access_is_rejected_and_has_no_effect` | planted `mutant_ignore_tenant`; `pg_adapter` `membership_distinguishes_member_non_member_and_other_tenant` | property "user never sees rows outside scope" (`TRACEABILITY.md` R-S1) |
| X2 | Role or membership rule wrong | contract `non_members_and_guests_are_forbidden_to_write`, `unauthenticated_requests_are_rejected`, `internal_routes_require_the_admin_token` | paired allow/deny in the same tests | v1 differential (ADR-0122) |
| X3 | Adapter sets diverge | contract suite over 4 targets (`scripts/contract-all.sh`) | `pg_adapter` against real Postgres | model-based port tests (ADR-0121) |
| X4 | Sequence ids duplicated or reused | contract `concurrent_creates_never_duplicate_a_sequence_id`, `soft_delete_hides_rows_and_never_reuses_sequence_ids` | planted `mutant_dup_sequence`; `pg_adapter` `concurrent_allocation_is_unique_contiguous_and_serialised` | 10k-create concurrency test (R-F2) |
| X5 | Event lost or processed twice (outbox, queue, sweeper) | contract `outbox_event_reaches_the_queue_consumer_exactly_once`, `redelivered_event_is_processed_once`, `sweeper_publishes_rows_the_fast_path_did_not` | planted `mutant_no_activity_dedupe`; `pg_adapter` `outbox_is_atomic_with_the_entity_and_dispatch_is_recorded` | model-based (ADR-0121) |
| X6 | Transaction or pool state leaks | `pg_adapter` `an_abandoned_transaction_does_not_poison_the_pool`, `rollback_is_atomic_and_unique_violation_is_a_conflict` | planted `mutant_discard_noop` | |
| X7 | Migration or SQL wrong | `pg_adapter` (runs migrations in a container) | `native-pg` and `worker-cf` contract cells | migration up/down checks *(no owner)* |
| X8 | wasm32 and native behave differently | contract suite on `native-*` vs `worker-*` targets | `scripts/check-deps.sh` on both targets | wasm32 unit tests on workerd (ADR-0123) |
| X9 | Worker binding or platform semantics | `worker_only.rs` (`worker_room_*`, `worker_cron_*`, `worker_bindings_*`) | stage-2 runs against a deployed Worker (`scripts/stage2-contract.sh`) | |
| X10 | Layering violated (platform crate in core) | `scripts/check-deps.sh` | | `cargo deny` in CI (R-P1) |
| X11 | Editor-document conversion wrong | golden corpus `parity_stage1_yjs_binary_and_json`, `parity_stage2_json_to_html`, `parity_stage3_html_to_json` | differential `oracle_stage3_fuzz_html`; controls `mutated_schema_is_detected_by_the_corpus_and_by_the_dump_diff`, `comparator_discriminates_between_different_documents` | |
| X12 | Sanitiser lets XSS through | `allow_list_preservation_of_editor_html`, `oracle_v1_write_with_sanitiser_matches_python_plus_js` | must-fail `unsanitised_payload_is_caught` | sanitiser fuzzing |
| X13 | Panic or blow-up on hostile input | `mutated_binaries_never_panic`, `oracle_mutated_html_never_panics`; contract `invalid_input_is_a_client_error` | contract `panic_is_contained_and_the_target_keeps_serving` (contains the damage) | coverage-guided fuzzing (`rust-test-fuzz`) |
| X14 | Public v1 API incompatible with Plane | **none** | | schema diff + corpus + nightly differential (ADR-0122) |
| X15 | Pure domain logic wrong | **none** (no unit tests yet) | contract suite, indirectly | example + property tests (`rust-test-logic`) |
| X16 | A test passes without checking anything | count guard and target guard in `scripts/contract.sh`; `stage3_not_vacuous` | planted mutants (`scripts/mutants.sh`) | cargo-mutants per crate (`rust-test-mutation`) |
| X17 | Nondeterminism or flakiness | Rules 2 and 6 (no retries, hermetic default) | `scripts/flake-hunt.sh` | proptest regression files |
| X18 | UI regression | **none** (no UI crate yet) | | Dioxus SSR tests, pixel diff, Playwright (ADR-0123) |
| X19 | Performance or cold-start regression | `scripts/startup-check.sh`, `scripts/stage2-latency.sh`, `thrust-contract-tests` bench (measure only) | | budgets enforced as gates *(no owner)* |
| X20 | Input validation or rate limiting wrong | contract `invalid_input_is_a_client_error`, `create_is_rate_limited_per_user` | `worker_bindings_rate_limiting_binding` | property tests on validators |

## Technique ownership

Who teaches each technique. A technique with no owning skill can still be used: copy the shape of the in-repo example
on its page.

| Technique | Owning skill | Status |
|---|---|---|
| [Example and parameterized](techniques/example-and-parameterized.md) | `rust-test-logic` | see page |
| [Property](techniques/property.md) | `rust-test-logic` | see page |
| [Model-based](techniques/model-based.md) | `rust-test-logic` | see page |
| [Golden fixtures](techniques/golden-fixtures.md) | `rust-test-logic` (harness); `ts-to-rust` owns fixtures recorded from a TypeScript port | see page |
| [Differential](techniques/differential.md) | `rust-test-logic` (`oracle_` against Node); none for differential fuzzing or the v1 oracle | see page |
| [Contract suite](techniques/contract-suite.md) | **none** (G-1) | see page |
| [Real backends](techniques/real-backends.md) | **none** (G-1) | see page |
| [Fuzz](techniques/fuzz.md) | `rust-test-fuzz` | see page |
| [Mutation](techniques/mutation.md) | `rust-test-mutation` (cargo-mutants); none for planted mutants (G-2) | see page |
| [App and end-to-end](techniques/app-and-e2e.md) | `rust-test-app`; none for browsers | see page |
| [Architecture gates](techniques/architecture-gates.md) | **none** | see page |
| [Performance](techniques/performance.md) | **none** | see page |
