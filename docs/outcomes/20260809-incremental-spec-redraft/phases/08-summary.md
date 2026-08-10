# Phase 8 summary — retire declared `grain: key_per_partition` and the dead `Append`/`InsertOverwrite` strategies

## Shipped

- `Grain::deserialize` (`crates/smelt-core/src/config.rs`) rejects `key_per_partition` with a
  message naming the two facts that derive it (`timeseries:` clock, `partition_column ∈
  unique_key`) and `grain: key` as the closest supported declared shape. `Grain::KeyPerPartition`
  the variant, `Serialize`, `Display`, and `derive_grain` are untouched — the label still exists
  as a derived-only classification.
- `IncrementalStrategy::{Append, InsertOverwrite}` deleted (`config.rs`); the dispatch `match` in
  `smelt-backend::Backend::execute_model_incremental` collapsed to the single `DeleteInsert` arm;
  `strategy_label` in `smelt-cli::helpers` collapsed the same way. `Backend::insert_into_from_query`
  / `insert_overwrite` stay on the trait — the capability that would admit those strategies once
  plan derivation selects one (no plan derivation calls them today).
- Spec edits: `docs/specs/incremental_models.md` (strategy-resolution paragraph, two restated
  `Append`-unreachable clauses, the dead-code `InsertOverwrite` Known-Divergence bullet deleted,
  the `key_per_partition` KD bullet rewritten as a derived-label gap, the four-corners table and
  declared-shape prose stop offering `key_per_partition` as writable), `docs/specs/models.md`
  (frontmatter row, derivation table, check-only paragraph, Known-Divergences sentence),
  `docs/specs/diagnostics.md` (declaration refusal documented next to `MaintenanceUnsupportedGrain`),
  `docs/specs/architecture.md` (one stale `IncrementalStrategy::InsertOverwrite` mention fixed for
  correctness — not in the phase's file list but factually wrong once the variant was deleted).
- `examples/timeseries_broken_key_per_partition/models/trajectory.sql` now derives
  `key_per_partition` from `unique_key: [device_id, event_date]` + `timeseries:` instead of
  declaring the grain; comment reworded.
- Test conversions: `smelt-core::config::incremental_strategy_append_and_insert_overwrite_are_gone`
  (extends the old `merge`-rejection test to `append`/`insert_overwrite`); `refresh_axis.rs`'s
  agreeing-assertion test now derives `KeyPerPartition` with no `grain:` written;
  `smelt-cli::tests::incremental::strategies` — the four `Append`/`InsertOverwrite` dispatch tests
  replaced with direct `insert_into_from_query`/`insert_overwrite` trait-method calls (same
  correctness assertions, bypassing `IncrementalStrategy` dispatch); doc-comment updates in
  `statement_parity.rs`, `example_diagnostics.rs`, `explain_model.rs`.
- `crates/smelt-runtime/tests/since_upstream_propagation.rs`'s `trajectory.sql` string fixture
  converted from declaring `grain: key_per_partition` to `unique_key: [user_id, d]`.

## Decisions

- Fixed a gate bug found while making the fixture derive rather than declare:
  `smelt-db::lib.rs`'s `maintenance_plan`/`maintenance_plan_report` (and the `grain: key`-only
  composed-upstream-source lookup inside both) gated entry on the **raw** `metadata.grain` field
  being `Some`, not the resolved label. A model admitted on facts alone (no `grain:` written) got
  no maintenance-plan diagnostics at all — silently, not fail-loud. Switched all four gates to
  `metadata.resolved_grain()`. This wasn't in the phase's file list, but the plan's own test list
  required the derived-only fixture to "stay green, proving the derived path is untouched," and it
  wasn't, until this fix — the derived path was never actually exercised end-to-end before.
- Left `crates/smelt-db/src/lib.rs:2261`'s `contract.frozen_horizon` grain-admissibility check
  (`metadata.grain.unwrap_or(Grain::Key)`) on the raw field — out of this phase's scope (no
  `contract:` fixture exercises it here) but it has the same latent bug; flagged below.

## For the next planner

- `crates/smelt-db/src/lib.rs:2261` (`contract.frozen_horizon` admissibility check) still reads
  the raw `metadata.grain` field and defaults to `Key` when absent — the same class of bug as the
  one fixed here, just not exercised by anything phase 8 touched. Worth an `rg -n 'metadata\.grain'
  crates/smelt-db crates/smelt-cli crates/smelt-runtime` sweep to check for other raw-field reads
  that should route through `resolved_grain()`.
- Row 9 (`batched:` sub-block's remaining `unique_key`/`safety_overrides`) and row 10 (docs-site
  terminology sync + whole-file `§"…"` citation sweep) are unaffected by this phase's edits.
- `docs/outcomes/20260809-incremental-spec-redraft/phases/06-claims.md`'s IC-21 row (`still
  hand-author SQL for the` anchor) reclassified `keep` → `drop`: the Known-Divergence bullet it
  tracked (`production-unreachable InsertOverwrite`) is deleted by this phase since the code it
  described is now actually gone. `06-check.sh`'s two other `gap_claims` failures (IP-02, MP-33)
  are pre-existing — confirmed via `git stash` against phase 8's own changes — and are the
  planner's call on whether to fix or accept as historical fixture drift.

## Gates

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/08-check.sh` → all green.
- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/0{2,3,4,5,7}-check.sh` → all green.
- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/06-check.sh` → green except two
  pre-existing `gap_claims` failures (IP-02, MP-33), confirmed unrelated to this phase.
- `bash .claude/scripts/verify-phase.sh` → ALL GREEN (fmt, clippy zero-warnings, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test example_diagnostics` → 119 passed, 1 ignored (pre-existing).
- `cargo test -p smelt-lsp --test example_workspaces` → 34 passed.
- `cargo test -p smelt-core --test refresh_axis --test source_world_facts` → 23 + 22 passed.
- `cargo test -p smelt-cli --test incremental --features smelt-cli/duckdb` → 48 passed.
- `cargo test -p smelt-cli --test explain_model` → 26 passed.
- `cargo test -p smelt-runtime --test since_upstream_propagation --test statement_parity` → 18 + 23 passed.
