# Phase 13 summary — Write-pin equivalence

**Shipped:**
- `smelt_logical::maintenance::cell_equivalence_proof` (`crates/smelt-logical/src/maintenance/mod.rs`) —
  single-owner proof: a compare-based pattern (`diff_patch`/`keyed_conditional`/`staged_candidate`)
  delegates to `choice::resolve_write_suppression`'s P2/P3 proof; every other registry pattern is `Ok`.
- `MaintenancePlanResult::comparability` (`crates/smelt-db/src/queries/maintenance.rs`) — the derived
  P3 vector, surfaced from `derive_model_maintenance_plan`'s own `model_property_vector` call so
  `write_pin_diagnostics` and `smelt explain` both read it instead of re-walking.
- `write_pin_diagnostics` now threads real `group_columns`/`comparability`/`row_identity` into
  `resolve_write_pin`'s equivalence hook (was `|_pattern| Ok(())`) — a `diff_patch` pin over an
  incomparable column now refuses `MaintenanceWriteAddressingRefused`, naming the column.
- `crates/smelt-runtime/src/maintenance_driver.rs`: both `resolve_write_variant` call sites
  (`resolve_live_column_scoped_cell`, `resolve_live_membership_recompute_cell`) now propagate a
  `ChoiceRefusal` as a real `anyhow` run error instead of `continue`-ing to a silent region-recompute
  fallback.
- `crates/smelt-cli/src/explain.rs`: the write-variant stanza calls the real
  `resolve_write_suppression`/`resolve_write_variant` with `result.comparability` for the
  identity-bearing case too (previously a `facts.has_identity`-only proxy) — an incomparable column
  now prints `write variant: unconditional (not admitted — …)` and a `technique: suppress` pin over
  it propagates as a real `explain` error.
- Spec: `docs/specs/incremental_models.md` §"Per-cell write addressing" → "User pins" states the new
  behaviour; the two matching Known Divergences bullets are deleted.
- Tests: 4 unit tests (`cell_equivalence_proof`), 2 new `smelt-db` integration tests, 1 new `smelt-cli`
  `explain` test, 1 new `smelt-cli` real-execute test (`suppress_pin_keyed_refusal` module).

**Decisions:**
- Compare-based-pattern classification is by registry `pattern.name`, not `WriteSelection` (which
  collapses `keyed`/`keyed_conditional`/`staged_candidate` onto the same `Technique::KeyedFold`
  selection and would blur the distinction this proof needs).
- `MaintenancePlanResult::comparability` is populated by ONE new `model_property_vector` call in
  `derive_model_maintenance_plan`'s success path (empty on every early-refusal return) rather than
  threading the walk result out of `derive_fold_spec`'s own internal call — simpler, and Salsa-purity
  clean since it's still exactly one walk per derivation.
- `smelt explain`'s "no proven row identity" default line changed text from
  `unconditional (default — …)` to `unconditional (not admitted — …)` since the branch is now driven
  by the real `WriteSuppression::Unconditional { why }` payload for BOTH the P2 and P3 cases, not a
  hand-written P2-only sentence. One pre-existing test's assertion string was updated to match; no
  other behavioural regression.

**For the next planner:**
- **Real bug found, out of scope, not fixed:** `smelt_logical::rules::cumulative::group_by_unique_key`
  (and `analyze_select`'s `group_by_exprs`) derives an EMPTY key whenever the `GROUP BY` column is
  literally named `order_id` — confirmed via isolated probe (`customer_id`/`orderid` both work fine,
  `order_id` alone fails). Smells like a keyword/lexer collision on the `ORDER` substring. This silently
  breaks `grain: key` admission for any model grouping by a column named `order_id` — likely worth a
  dedicated bug ticket/plan; several `docs/specs/incremental_models.md` examples use `order_id` as a
  running-example column name, so this may be live in other fixtures too.
- Reaching a genuinely live `Technique::ColumnScopedMerge` dispatch through a real staged
  `execute_project` project is now effectively impossible for a join-shaped model (any join against a
  mutable dimension is membership-sensitive per `docs/plans/20260808-membership-sensitivity.md`) — the
  `smelt-cli` P3-refusal test for the write-*variant* dimension had to use a `grain: key` fold-only
  fixture (`resolve_live_membership_recompute_cell`'s `Technique::DeleteInsert` staged-candidate path)
  instead. Worth noting in `docs/specs/incremental_models.md` if a future reader goes looking for a
  live `ColumnScopedMerge` e2e fixture and can't find one that isn't hand-built.
- Not addressed (genuinely out of this phase's scope per its own text): "a pattern family with no
  comparability-based obligation" residue — every currently-registered pattern except the three
  compare-based ones has zero obligation, so there is no live residue to narrow yet; left the spec
  wording open per the plan's own instruction ("narrow rather than delete" only applies if a residue
  survives — none does today).

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full `cargo test`
  workspace, `example_diagnostics`).
- `cargo test -p smelt-logical --test walk_coverage` — pass (4/4).
- `cargo test -p smelt-db --test maintenance_write_pin_diagnostics` — pass (5/5).
- `cargo test -p smelt-runtime --test technique_lowering --test statement_parity --test execute_parity` —
  pass (31/23/4).
- `cargo test -p smelt-cli --test maintenance_pins --test maintenance_conformance --features duckdb` —
  pass (4/74).
- `python3 examples/web_analytics/generate_tutorial.py` re-run to refresh the one drifted
  `smelt-generate` block (`docs-site/docs/examples/web-analytics/deduplication.md`, a `write variant:`
  line whose text changed).
