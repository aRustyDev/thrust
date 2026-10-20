# Contrail plan

Contrail is the MVP of Thrust: Plane rebuilt Cloudflare-native and self-hostable, with a Rust backend, Plane's React
frontend kept for the first release, and a Dioxus clone of the UI after it (SPEC §1, §2). This directory is the
working plan package. Start with `PLAN.md` for what ships in what order, and `PROMPT.md` to begin a working session.
This copy, in `aRustyDev/thrust`, is canonical (`spikes/README.md`).

| Document | What it holds |
|---|---|
| [`PLAN.md`](PLAN.md) | Milestones M0 to M6 with exit criteria, the critical path, the M2 screen list, status and issue numbers. |
| [`SPEC.md`](SPEC.md) | Goals, principles, requirements (R-F, R-P, R-S), the Plane-to-Cloudflare mapping and risks. |
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | Deployed architecture (Cloudflare set and self-hosted twin) and the hexagonal application layout. |
| [`FEATURES.md`](FEATURES.md) | The feature inventory (F-A to F-X) with a tier for every row (P0, P1, P2, drop). |
| [`TRACEABILITY.md`](TRACEABILITY.md) | Features and requirements mapped to ADRs, spikes and test homes, with the gaps. |
| [`DEVIATIONS.md`](DEVIATIONS.md) | The exception register: intentional departures from Plane and the pinned oracle (D-NNN). |
| [`QUESTIONS.md`](QUESTIONS.md) | Open and answered questions (Q-NNNN and the Q-G, Q-F, Q-R families); every open row has an owner and a date. |
| [`PROMPT.md`](PROMPT.md) | The session prompt: reading order, decisions not to reopen, what is still open, and the round log. |
| [`REVIEW-NOTES.md`](REVIEW-NOTES.md) | The independent package review of round 25 and its findings. |
| [`adrs/`](adrs/README.md) | Architecture decision records (MADR) by group, indexed with their status in `adrs/README.md`. |
| [`spikes/`](spikes/README.md) | Time-boxed experiments that gate ADRs, with their order and status in `spikes/README.md`. |
| [`research/`](research/) | Research notes and spike result files (`research/0001` to `0010`). |
| [`reviews/`](reviews/) | The 2026-10-19 Fable architecture review (findings FR-01 to FR-23) and its handoff prompt. |
