# Phase 9 summary — `smelt explain`: the delta-signature headline

**Shipped:**
- `crates/smelt-logical/src/maintenance/signature.rs` (new): `KeyedRunShape`
  (`WindowForward`/`SnapshotReconcile`/`PartitionSweep{axis}`),
  `SignatureHeadline`, and pure `derive_signature_headline` — the single
  owner of the headline's formatting, addressing via
  `edge_type::Addressing`'s same three-way mapping. 5 unit tests.
- `crates/smelt-db`: `own_output_delta_verdicts` extracted from
  `ref_model_edge`'s inline fold, now called by both `ref_model_edge` and
  `maintenance_plan_report`. `MaintenancePlanResult` gains
  `own_output_delta: Vec<(String, OutputDelta)>` and
  `run_shape: Option<KeyedRunShape>`, populated in `maintenance_plan_report`
  (run shape from `CumulativeClassification::is_snapshot_reconcile` for
  `grain: key`, from `metadata.timeseries` for `grain: partition`).
- `crates/smelt-cli/src/explain.rs`: `build_maintenance_plan_report` prints
  the headline as the report's first line, above `Maintenance plan:`; new
  `explain_signature_json`/`ExplainSignatureJson` expose the same fields on
  `--json` under a `signature` object. Extracted `locality_route_label` so
  both the headline and the existing "Key temporal locality:" stanza read
  one classification.
- Spec: `incremental_models.md` §Surface "CLI" **Headline** bullet extended
  (run shape, explicit `general` wording); the `does not yet print the
  delta-signature headline` Known Divergences bullet narrowed to the
  per-column-ledger/refusal residue phase 10 owns.
- Docs-site `cli.md` prose + golden fixture updated; tutorial fixture
  (`deduplication.md`) regenerated via
  `python3 examples/web_analytics/generate_tutorial.py`.
- Tests: 5 `smelt-logical` unit, 3 `smelt-db` integration
  (`tests/maintenance_signature.rs`), 2 `smelt-cli` (headline-first-line,
  JSON byte-equal-to-text).

**Decisions:**
- `own_output_delta`/`run_shape` are populated only by
  `maintenance_plan_report` (not `maintenance_plan`'s Salsa-tracked
  refusals-only projection) — matches the plan's task 4, and every other
  `MaintenancePlanResult` construction site (tests, `finish_plan_result`)
  defaults them empty/`None`.
- The mixed-verdict "degrading group" is the highest-rank verdict's own
  group name, tracked alongside the meet rather than re-derived from
  `OutputDelta::meet` (which discards which group won).
- `KeyedRunShape` keeps the plan's literal name even though it also covers
  `grain: partition`'s sweep — "a single run-shape vocabulary, not two" per
  its own doc comment.

**For the next planner:**
- Row 10 (per-column guarantee ledger + pre-execution refusal surfacing) is
  fully unblocked — `SignatureHeadline`/`own_output_delta` give it a
  ready-made per-group verdict source, though the ledger itself (contract ×
  settle bound per column) needs its own new derivation, not reuse of this
  phase's types.
- Not addressed here (correctly out of scope): the per-column guarantee
  summary, and pre-execution refusal surfacing — both explicitly deferred to
  row 10 by the plan.
- No new gaps discovered beyond what phase 8's summary already flagged for
  row 13's close-out sweep.

**Gates:**
- `cargo test -p smelt-logical --test walk_coverage --quiet` — pass (4/4).
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model --test explain_show_sql --test explain_probes --quiet` — pass (26+27+3+6).
- `cargo test -p smelt-db --test integration --quiet` — pass (363/363).
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --quiet` — pass (4+23).
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full
  workspace `cargo test`, `example_diagnostics`).
