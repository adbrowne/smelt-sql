# Phase 15 plan — bank the DuckDB-half evidence

## Objective

Write `docs/handoffs/2026-09-08-github-activity-findings.md`: the interim, DuckDB-half
findings document. It carries the four measured root causes, the five registered
divergences, and the requirements this pipeline places on the two feature outcomes.
Advances criterion 8 (evidence banked) as far as it can go without a warehouse, and
unblocks the harvest phases of `bigquery-correctness`, `external-dag-steps` and
`trimmed-history-sources`, all three of which are parked on a document that does not
exist yet. Phase 16 extends the same file with the live-run half.

## Spec delta

None. No user-visible feature behaviour changes; this phase produces a handoff document
plus one drift gate. (`examples/github_activity/README.md` may gain a pointer line.)

## Tests

Red-green, in `crates/smelt-cli/tests/github_activity_oracle.rs` (registry lives in
`github_activity_support/mod.rs`):

1. `every_registry_entry_is_named_in_the_findings_handoff` — every `DIVERGENCE_REGISTRY`
   entry's relation name appears verbatim in `docs/handoffs/2026-09-08-github-activity-findings.md`;
   fails naming the missing relation. Red first: write the test before the handoff exists.
2. `findings_handoff_names_no_unknown_relation` — the reverse leg: every relation name the
   handoff's divergence table claims is registered actually resolves to a registry entry, so
   a renamed or retired entry cannot leave a stale row in the document.
3. `findings_handoff_declares_its_interim_status` — the document carries an explicit
   "DuckDB half only / live half pending" marker, so a downstream harvest phase cannot
   mistake it for the complete criterion-8 artifact.

These are cheap string-level gates over a committed doc — the point is drift, not prose
quality. Keep them in the existing binary; do not add a new test target.

## Tasks

1. Write the failing tests (1–3) against the not-yet-existing handoff path; confirm red.
2. Harvest the evidence, reading only in-repo sources: `phases/0{2,3,4,6,8,9}-summary.md`,
   this outcome's "## Decision log", and `examples/github_activity/README.md`. Do not
   re-derive or re-measure anything — every number in the handoff must be traceable to one
   of those, cited by file.
3. Write `docs/handoffs/2026-09-08-github-activity-findings.md` with these sections:
   - **Status** — interim: DuckDB half only, live-BigQuery half is phase 16 of this
     outcome; name the gate (no provisioned project) so a reader knows why.
   - **The four root causes**, each with the model, the code location already identified,
     and the evidence: (a) succession naming-tie fold (`silver_repo_naming`,
     `silver_actor_naming`); (b) enrichment freeze — no `UpstreamMutation(gold.repo_dim)`
     cell derived, `crates/smelt-logical/src/maintenance/derive/model_edge.rs`, staleness
     strictly non-decreasing across all 30 windows, zero rows ever heal; (c) oracle
     windowing gap — `compute_calendar_windows` in `crates/smelt-runtime/src/windowing.rs`
     rebases only at the outer edges of a multi-day invocation, so the *full-refresh oracle*
     under-counts `silver_actor_sessions`; (d) mart repair gap —
     `marts_daily_active_contributors` never revisits an already-written partition.
   - **The five registered divergences**, as a table: relation, `Bound` shape
     (`FoldEquality` / `StaleButHistoricallyValid` / `MonotoneDivergence` + side), root
     cause, and whether it is a smelt defect or a fixture artifact.
   - **Latent, unmeasured** — `emit_succession_full_rebuild` never runs the clock-tie probe,
     so a content-*disagreeing* tie would corrupt a full refresh undetected (this fixture
     measured zero disagreeing ties).
   - **Requirements handed to `20260906-external-dag-steps`** — what the loader being
     external-by-convention costs today, and what a `produced_by:` declaration would have to
     express to replace it, derived from `scripts/bq-dogfood-loader.sh`'s actual shape.
   - **Requirements handed to `20260906-trimmed-history-sources`** — the loader's 45-day
     `partition_expiration_days` bound and its derivation (30-day fixture + backfill
     headroom), and the inert, contradictory `retention: '90 days'` field on both
     `models/sources/raw/github_events{,_arrival}.yml`, which that outcome must reconcile.
   - **Punch-list for `20260906-bigquery-correctness`** — one numbered item per root cause,
     each stating fix-or-register and the model that provokes it.
4. Correct the stale comment on `models/sources/raw/github_events{,_arrival}.yml`'s
   `retention:` field — it claims to match the loader's trim and does not (45 vs 90). Change
   the *comment* only, not the value; the value is `trimmed-history-sources`' call.
5. Append a dated one-line pointer to the "## Decision log" of each of
   `docs/outcomes/20260906-bigquery-correctness/outcome.md`,
   `docs/outcomes/20260906-external-dag-steps/outcome.md` and
   `docs/outcomes/20260906-trimmed-history-sources/outcome.md`: the interim handoff exists
   at this path, covers the DuckDB half only, and its live half lands in phase 16. Do not
   edit their phase tables — reshaping those outcomes is their own planners' job.
6. Add a pointer line from `examples/github_activity/README.md`'s "Trusting the numbers"
   section to the handoff, so the findings are reachable from the example.
7. Write `phases/15-summary.md`.

## Verification

- `cargo test -p smelt-cli --test github_activity_oracle` — the three new tests green
  alongside the existing suite (the per-window sweep is ~108s; budget for it).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN, no ratchet lowered.
- `bash .claude/scripts/large-file-check.sh` — OK.
- Manual: confirm every numeric claim in the handoff appears in a cited summary or the
  decision log; an uncited number is a defect in this phase.

## Commit message

`outcome(bigquery-dogfood-spine): bank the DuckDB-half findings handoff`
