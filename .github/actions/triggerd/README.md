# triggerd

`triggerd` is a local Node 24 GitHub Action that validates a maintained policy,
collects a complete revision-bound pull-request file list, and emits execution
decisions. A `true` rule output means **run that check**; it is not a test
result, authorization decision, or finding.

Inputs are `rules-file` (checkout-relative, default `.github/triggerd.yml`),
optional `scope`, required `github-token`, and strict `full-check` (`true` or
`false`). The action masks the token before source or API work. It accepts
`pull_request`, `push`, `workflow_dispatch`, and `schedule` in this slice.
Manual and scheduled events use full mode; an explicit full check skips diff
collection but still validates the policy and event context. Push uses the
event's exact before/after revision pair; merge queue and Git fallback remain
unavailable.

Outputs are compact JSON strings: `rules` maps every in-scope rule ID to an
actual Boolean, and `details` carries versioned decisions, source provenance,
the exact policy SHA-256, and either a complete change count or a
`not-collected` reason. On a PR, `head` remains the event PR head even when the
checkout is GitHub's synthetic merge revision; `policyRevision`,
`actionRevision`, and `testRevision` record that checkout revision separately.

Collection is bounded to 3,000 files at 100 per API page and verifies metadata
before and after pagination. It retries transport timeouts, 429, and 5xx up to
three attempts with 250ms then 1000ms waits; a longer `Retry-After` is used
only inside the single 120-second action budget. Malformed, stale, denied, or
incomplete sources fail without outputs. Rerun the affected workflow for a
transient failure; use explicit full-check recovery or restore unconditional
execution while retaining the completion gate as later migration tasks specify.

The checked-in distribution is self-contained. Regenerate and verify it with:

```sh
npm run build
npm run check:dist
npm run test:installed
```

`test:installed` uses only `dist/` and synthetic runner files; it is local
distribution evidence, not hosted workflow, permission, protection, pilot, or
cutover evidence.

`dist/gate.js` is the fixed completion-gate entrypoint. It reads only
`RULES_JSON`, `NEEDS_JSON`, and `BINDINGS_JSON` from the environment, requires
the planner and every bound job result, and fails on missing/extra/non-Boolean
decisions, unknown results, selected skips/failures, unselected failures, or a
selected consumer without its selected prerequisite. Bindings are independently
validated against the complete policy rule-ID set; they are never inferred from
the planner output.
