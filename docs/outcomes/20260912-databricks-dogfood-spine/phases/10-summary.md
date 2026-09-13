# Phase 10 summary — evidence banked

## Shipped

- `docs/handoffs/2026-09-13-databricks-findings.md` — the findings handoff: 6 defects fixed in
  place (dialect-dispatched hash, `DROP_COMMAND_TYPE_MISMATCH`, source-cast spelling,
  `epoch_us`, `LAG`/`LEAD` frame elision, the succession `state_downgraded` dispatch +
  ledger-free rebuild), 7 defects recorded not fixed in priority order (led by
  `gold.events_enriched`'s registered divergence and the O(source)-per-window succession
  rebuild cost), the one registered divergence with its bound, the Free Edition constraints
  that shaped the design, an open Catalog Commits design question flagged for planner triage,
  and a "next steps" punch-list for a follow-on `databricks-correctness` outcome.
- `docs/specs/multi_backend.md` §Known Divergences: replaced the stale "inherited, not
  independently verified" capability-matrix entry with what the live sweep actually measured;
  added entries for the succession ledger-free rebuild's O(source) cost and
  `gold.events_enriched`'s plan-derivation-time downgrade (the fingerprint-sidecar section's
  stated `UnsupportedOnBackend` runtime refusal no longer describes this cell's actual path
  since phase 7b).
- `docs-site/docs/guide/targets.md`: new `### Databricks` section (after BigQuery) — target
  shape, refused foreign keys, `${ENV}`-only token rule, credentials, cross-engine-exchange
  gap, and a `#### Free Edition constraints` subsection sourced from `free-edition-facts.md`.
- `docs/ROADMAP.md` item 11 revised: Databricks is now a distinct, live-verified backend;
  what remains (Metrics DSL/metrics-view compatibility, native IVM, paid-tier compute) restated
  as the item's actual remaining scope.
- `.gitignore`: `.env` added next to the local-runtime block, with a one-line comment naming it
  as the wizard library's default `ENV_FILE`. Confirmed untracked (`git ls-files .env` — empty).
- New tests: `crates/smelt-core/tests/databricks_docs_freshness.rs` (2 tests — the docs-site
  section names every `DATABRICKS_FOREIGN_KEYS` entry and states the bare-hostname rule, scoped
  to the `### Databricks` section specifically so a key name shared with Spark/BigQuery's own
  fields can't pass vacuously) and `crates/smelt-cli/tests/databricks_findings_handoff.rs`
  (3 tests — the handoff names every relation either live report measured as diverging and
  every excluded model, and the spec no longer claims the capability column is unverified).

## Decisions

- Scoped the docs-site freshness test's key/host-rule checks to the `### Databricks` section
  alone, not the whole `targets.md` file — an initial version passed vacuously because
  `warehouse`/`format`/`database` are also Spark's own field names, appearing in the Spark
  section regardless of whether a Databricks section existed at all.
- Added a Known Divergences entry for `gold.events_enriched`'s plan-derivation-time downgrade
  rather than editing §"The fingerprint sidecar capability" directly — the plan's spec delta
  names Known Divergences as the only spec edit for this phase, and that section's stated
  runtime `UnsupportedOnBackend` refusal is accurate for the flagless-`mutable_snapshot` leg it
  describes; it just doesn't describe this cell's actual (now different, since phase 7b) path.

## For the next planner

- The follow-on `databricks-correctness` outcome (not created here, per the plan's brief) has
  its punch-list in the handoff's "Next steps" section, headed by the Catalog Commits triage
  question — worth spec/plan attention before any further `Technique` downgrade work touches
  the succession or merge-ledger machinery.
- Row 11 (the Databricks Asset Bundle / scheduled-job criterion) is the only remaining row in
  this outcome; it is live-gated and unaffected by this phase.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-core --test databricks_docs_freshness` — 2/2.
- `cargo test -p smelt-cli --test databricks_findings_handoff` — 3/3.
- `cargo test -p smelt-cli --test docs_front_door` — 6/6, unchanged.
- `cargo test -p smelt-cli --test github_activity_dual_target --test github_activity_dbx_oracle`
  — 26/26 and 18/18, unchanged by this phase.
- `git ls-files .env` — empty (not tracked).
