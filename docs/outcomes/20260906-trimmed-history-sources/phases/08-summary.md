# Phase 08 summary — Conformance: a trimmed-retention source whose bound advances

**Shipped:**
- `SourceRecipe.retention: Option<RetentionDecl>` (`crates/smelt-maintenance-testkit/src/recipe.rs`)
  plus a `with_retention(days)` builder, defaulted `None` everywhere else so every
  pre-existing recipe renders byte-identically.
- New `crates/smelt-maintenance-testkit/src/retention.rs`: `retained_cutoff(as_of, bound_days)`
  (pure) and `trim_source_to_retention(backend, source, as_of)` (a `DELETE` against
  `main.sources_<name>`, no-op when `retention` is `None`), plus the `RetentionDecl` type and
  its builder impl (kept out of `recipe.rs`/`render.rs`, both already at their large-file
  baseline).
- `render::render_source_yaml` renders a declared retention as a trailing
  `retention: '<n> days'` line.
- `gate::partition_pool::drive_and_assert_collecting` calls `trim_source_to_retention` before
  each of `RunWindow`/`RerunWindow`/`BackfillRegion` (a no-op for every existing caller —
  `FullRefreshRun`/`MigrateModel`'s inner refresh deliberately untouched; that interaction is
  phase 6's own gate, `check_full_refresh_retention`).
- New `crates/smelt-cli/tests/maintenance_conformance/gate/retention_pool.rs`: two generative
  tests (`retention_pool_upholds_equivalence_under_an_advancing_bound`,
  `retention_pool_actually_trims_rows`) and one pinned test
  (`an_aged_backfill_past_the_advancing_bound_refuses_and_leaves_state_unchanged`). Registered
  in `gate/mod.rs`.
- 5 unit tests: `render_source_yaml_emits_a_declared_retention`,
  `render_source_yaml_without_retention_is_byte_identical`,
  `retained_cutoff_is_the_run_clock_minus_the_bound` (all in `retention.rs`'s own test module).

**Decisions:**
- **Two independent clocks, by design.** `smelt-runtime`'s run-time admission check ages a
  run's window against the REAL wall clock (`Utc::now()`), but this harness's schedules use
  fixed/synthetic dates. Anchoring the generative pool's windows a year into the future makes
  `window_age` saturate at zero, so admission never depends on calendar drift; the physical
  trim uses the schedule's OWN clock (each step's own `start`) so rows still depart
  deterministically once `WINDOW_GAP_DAYS` (60) exceeds `RETENTION_DAYS` (30). The pinned
  aged-backfill test instead uses a genuinely-past date (`2000-01-01`) specifically to
  exercise the real-wall-clock refusal path. 2026-09-09.
- **No new oracle transform, no lattice point** (already recorded in `outcome.md` before this
  phase started): `STracker::s_restricted_oracle_sql` materialises its baseline from recorded
  run snapshots, never the physical source, so retention trimming cannot desync the oracle
  from what a technique is expected to still hold — verified true in practice by the green
  equivalence tests.
- Test 5 (`retention_pool_actually_trims_rows`) was split out of test 4 into its own function
  (rather than folded together as first drafted) to keep 1:1 naming with the plan's TDD list;
  both share a `run_case` helper so a case is staged/driven only once between the two runs of
  the deterministic sample (they don't share a sample — each test iterates its own seeded
  `TestRunner`, doubling the drive cost but matching `admission_rate_stays_above_floor`'s own
  precedent of a separate anti-vacuity test).
- `large-file-check.sh` baseline raised for `recipe.rs` (2318→2327), `render.rs`
  (1376→1383), `s_tracker.rs` (1148→1149) — sign-off: the residual growth is a struct field +
  doc comment + 3 call-site literals (recipe.rs), the `render_source_yaml` retention-line
  logic itself (render.rs), and one test-fixture literal field (s_tracker.rs); none of it is
  extractable into `retention.rs` because Rust requires a struct's fields to live with its
  definition. `RetentionDecl` itself and the `with_retention` builder impl block WERE moved
  into `retention.rs` to minimize the residual.

**For the next planner:**
- Row 9 (composed-upstream granularity gap) and rows 10-11 (explain/docs, close-out) remain
  as scoped in the outcome's Decision log from phase 8 planning — untouched by this phase.
- Nothing new discovered that needs a row: the two-clock design fully explains away what
  looked like a potential blocker (real-wall-clock age vs. synthetic schedule dates) with no
  production code change needed.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test maintenance_conformance` — 104 passed.
- `SMELT_CONFORMANCE_CASES=80 cargo test -p smelt-cli --test maintenance_conformance retention_pool` — 3 passed (not seed-lucky).
- `cargo test -p smelt-maintenance-testkit` — 69+4 passed.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — 4+41 passed.
- `cargo test -p smelt-logical --test walk_coverage` — 14 passed.
- `bash .claude/scripts/large-file-check.sh` — OK (baseline updated, sign-off above).
