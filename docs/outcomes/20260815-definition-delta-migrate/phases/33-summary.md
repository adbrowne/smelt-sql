**Shipped:**
- Spec delta landed first (`docs/specs/incremental_models.md`): the override-ladder bullet now
  states the write-suppression dimension is consulted by every write consumer that can
  suppress, keyed-fold included; the "Override-ladder reach (Open Question)" bullet is deleted
  from Known Divergences; a new §Future Extensions entry ("Cost-model input for the
  write-suppression dimension") carries the residual undecided cost-model question forward.
- `crates/smelt-db/src/queries/maintenance.rs`: new `keyed_fold_effective_override(metadata,
  driving_source) -> EffectiveOverride` — matches a `cells[]` entry by `on:` address alone
  (mirroring `keyed_fold_write_pin`'s whole-row addressing), not `effective_override`'s
  per-column-group `matching_cell`, which would never match a whole-row cell's typically-empty
  `columns`.
- `crates/smelt-runtime/src/cumulative.rs`: `resolve_cumulative_write_suppression` now takes
  `&EffectiveOverride`, folds `resolve_write_variant(raw, &Trigger::NewData{source}, false,
  overrides)`, and returns `Result<WriteSuppression, ChoiceRefusal>`. Both live call sites
  (`execute_cumulative_aggregate`'s windowed-forward path, `execute_snapshot_reconcile`'s
  `else` arm) now resolve `keyed_fold_effective_override` and propagate a refusal as a
  run-failing `anyhow` error.
- `crates/smelt-runtime/src/diagnostics.rs`: the `Technique::KeyedFold` arm of
  `build_technique_statements` (the `smelt explain --show-sql` preview) folds the same
  override via `resolve_write_variant`, replacing the stale comment that said the live path
  never folds it in — preview/live parity restored.

**Decisions:**
- The structural first-build/steady-state half of the ladder is unreachable by construction on
  the keyed-fold route: both call sites only resolve suppression once the target table already
  exists (first build takes `emit_create_table_as`, never a suppressible merge). Passed
  `Trigger::NewData{..}` with `ledger_catch_up: false` always — documented as a derivation from
  route structure in the resolver's own doc comment, not an assumption.
- The cost-model-needs-change-ratio-statistics residual moved to §Future Extensions rather than
  staying a Known Divergence — it's a genuine undecided widening, not a gap in what's shipped.
- Dated one-liner appended to outcome.md "## Decision log": 2026-09-03 — override ladder's
  write-suppression dimension now reaches the keyed-fold consumer; structural first-build half
  is unreachable by construction there.

**For the next planner:**
- Row 33's `phases/33-plan.md` existed and was fully executable this iteration — the prior
  "missing plan file" blocker (see outcome.md's 2026-09-03 Blocked entries for phases 32/33)
  had already been resolved by a PLAN step between iterations. Nothing further needed for that
  process gap.
- Test 6 (`explain_show_sql_keyed_fold_honours_the_pin`) and the new `cumulative.rs` unit tests
  (2-5) were placed as unit/integration tests co-located with the existing suppression tests
  rather than in a fresh e2e `execute_project` harness — the resolver-level wiring is exercised
  directly, and `maintenance_conformance`/`statement_parity` already provide the full-stack
  equivalence check. No follow-up scoped from this choice.
- No other un-laddered `resolve_write_suppression` consumer was found (task 6's cross-check):
  `smelt-cli/src/explain.rs`'s plan-cell-driven `KeyedFold`/`ColumnScopedMerge` preview and
  `resolve_cell_write_suppression` both already fold the ladder in.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test technique_lowering` — 32 passed.
- `cargo test -p smelt-runtime --test diagnostics` — 16 passed (incl. new pin test).
- `cargo test -p smelt-runtime --test statement_parity` — 33 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 74 passed.
- `rg -n 'Open Question' docs/specs/incremental_models.md` — only the section heading
  ("Known Divergences / Open Questions") remains; no bullet-level tag.
