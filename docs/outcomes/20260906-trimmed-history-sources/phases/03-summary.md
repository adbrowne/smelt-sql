# Phase 03 summary — reach vs. retention, produced by the composition walk

**Shipped:**
- `docs/specs/model_properties.md`: new §"Reach versus retained history" (after §"Unified
  bound / reach derivation") plus a `Retention admissibility` row in the Derived proofs
  table.
- `crates/smelt-logical/src/analysis/retention_reach.rs` (new): `RetentionVerdict`
  (`NoDeclaredBound | Within | Exceeds | UnprovableWithin`), `UnprovableReason`
  (`UnboundedReach | ReachNotDerivable`), `derive_retention_verdicts(sql, ctx, window_age)` —
  a pure fold over `derive_model_bounds`'s own output, no text scan of its own. Registered in
  `analysis/mod.rs`. 9 tests (the plan's 8 walk-side tests + the conversion test).
- `BoundContext` (`analysis/source_bounds.rs`) gained a `retentions: HashMap<String, Seconds>`
  field; its `with_source_retention`/`add_source_retention` setters live in
  `retention_reach.rs` as a second `impl BoundContext` block (see Decisions).
- Fixed the two struct-literal `BoundContext` constructions in
  `crates/smelt-logical/src/rules/incremental.rs` (~1010, ~1328) to carry `retentions`.

**Decisions:** (full reasoning in outcome.md's Decision log, 2026-09-09 phase 3 implement)
- Setters + conversion test placed in the new file, not `source_bounds.rs`, to keep growth
  in the ratcheted file to the unavoidable minimum (+4 lines, not +41).
- Test 8 rewritten from "source absent from SQL text" (impossible — `derive_model_bounds`'s
  existing whole-text top-up always backfills every `ctx.source_partition_cols` entry) to
  "source with a declared retention but no `ctx.source_partition_cols` entry at all", which
  is the real absent-from-the-map case.
- Large-file baseline bumped for `source_bounds.rs` (3367→3371) and `incremental.rs`
  (3430→3432); both were already pinned exactly at baseline, so the plan's own required edits
  (a struct field + its two call-site fixes) regress it by construction.

**For the next planner:**
- Phase 4 (refuse/degrade) consumes `RetentionVerdict` directly — no further shape work
  needed on the verdict itself.
- `derive_retention_verdicts` takes `window_age: Seconds` as a caller-supplied parameter;
  phase 5 (bound-moving-is-an-event) will need to decide where that value comes from at
  plan/run time (likely the gap between the model's authored backfill point and the current
  run) — not decided here, deliberately out of this phase's scope.
- Nothing else surfaced needing a new phase-table row.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy zero-warnings both feature
  sets, full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test walk_coverage --quiet` — 14 passed.
- `cargo test -p smelt-logical --quiet` — all green (retention_reach's 9 new tests included).
- `cargo test -p smelt-core --test source_world_facts --quiet` — 26 passed (unaffected).
- `bash .claude/scripts/large-file-check.sh` — regressed on the two files above; updated via
  `--update` with the sign-off note in Decisions.
