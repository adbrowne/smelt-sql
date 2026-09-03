# Phase 27g summary — runtime dispatch for the keyed-fold `write:` pin

**Shipped:**
- `smelt_db::queries::maintenance::keyed_fold_write_pin(metadata, driving_source)` — the
  `on:`-address-only pin lookup for a whole-row keyed-fold cell, sharing the exact matching
  predicate `matching_write_pin` uses (extracted into a private `write_pin_matching` helper).
- `WindowedKeyedRule::write_group` — a new trait method that resolves the actual
  `StatementGroup` for a `KeyedWriteMechanism` (default: wraps `merge_sql` for `Merge`; panics
  on `StagedCandidate` unless overridden).
- `CumulativeClassification::write_group` override in `cumulative.rs`: the `StagedCandidate` arm
  builds `keyed_fold_candidate_select` → `emit_staged_candidate_conditional`, staged relation
  named `__smelt_staged_<table>`.
- `run_windowed_keyed_maintenance` gains a `write_pin: Option<&'static WritePattern>` parameter;
  resolves the mechanism once via `resolve_keyed_write_mechanism` before the step loop, bailing
  on both `Err(ChoiceRefusal)` and `Ok(None)` before any backend call. The step loop now builds
  `action_group` via `rule.write_group(...)` and threads it unchanged into both the ordinary
  `execute_statement_group` arm and the observed-delta arm.
- `execute_cumulative_aggregate` resolves the driving-source's `write:` pin once and passes it
  through.
- `docs/specs/incremental_models.md` §Known Divergences → "Conditional-maintenance gaps": dropped
  the "no `write:` pin selects…" clause; kept the `supports_fingerprint_sidecar` clause.

**Decisions:**
- The `Grade::Additive` (ledger-folded) branch only wraps a single action statement
  (`Backend::fold_ledger_delta`'s own signature) — added a fail-loud guard refusing a
  multi-statement `action_group` there rather than attempting to mis-wrap it. Not reachable
  today (an additive-graded cell only resolves `StagedCandidate` via an explicit pin, which no
  current fixture exercises together with an additive fold), but documented and guarded rather
  than silently assumed impossible.
- `write_group`'s `slice` parameter is accepted but unused by the `StagedCandidate` arm — key
  temporal locality has no staged-candidate realisation yet; this mechanism is reachable only
  for a bare keyed model until locality composes with it (same scope boundary the plan named).

**For the next planner:**
- The staged-candidate mechanism + key-temporal-locality composition is still open — worth a
  future phase if a `MERGE`-less backend with a locality-sliced keyed model shows up.
- `docs-site/docs/reference/smelt-yml.md` §"Within-family mechanism pins" already documented the
  user-visible semantics from phase 27d — confirmed no edit needed here.
- Test file `crates/smelt-runtime/tests/observed_delta.rs`'s `keyed_fold_suppressed_recording_
  refuses_a_non_duckdb_backend` fixture backend previously had `capabilities()` return
  `unimplemented!()`; now returns `BackendCapabilities::spark()` since the mechanism resolution
  calls `capabilities()` before the dialect check this test targets.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering --test observed_delta` — 14+29+32 passed
- `cargo test -p smelt-logical --lib maintenance::` — 194 passed
- `cargo test -p smelt-db --test maintenance_write_pin_diagnostics` — 5 passed
- `cargo test -p smelt-cli --test cli_unit cumulative_equivalence` — 7 passed
- `cargo test -p smelt-cli --test maintenance_conformance` — 74 passed
