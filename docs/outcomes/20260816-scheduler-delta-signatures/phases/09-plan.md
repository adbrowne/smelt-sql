# Phase 9 plan — `smelt explain`: the delta-signature headline

## Objective

Make `smelt explain <model>`'s first line the model's **derived delta signature** — what it
emits, how that delta is addressed, the friendly grain label, and the derived run shape —
instead of leading with `Maintenance plan: <model>` and grain. Advances success criterion 5
(the explain verification surface) and criterion 6 (narrowing the "does not yet print the
delta-signature headline" divergence to the guarantee-ledger residue row 10 owns).

## Spec delta

`docs/specs/incremental_models.md` §Surface "CLI", the `smelt explain <model>` **Headline**
bullet:

- Add the **run shape** to the headline's contents: the headline names the model's derived run
  shape alongside the grain label — for `grain: key`, window-forward or snapshot-reconcile
  (`incremental_shapes.md` §"The two run shapes (derived, never declared)"); for
  `grain: partition`, the window sweep over the partition axis. A model whose run shape is not
  derivable prints no run-shape clause rather than a guessed one (the §"Delta signatures"
  never-fabricate rule).
- State the `general` case's headline wording explicitly: `emits: general change,
  whole-table-addressed`, naming the degrading construct — matching the widen-never-narrow
  default the same section already states for an underivable signature.
- Narrow the Known Divergences bullet "**`smelt explain` does not yet print the delta-signature
  headline**" (~L1967) to its residue: the headline and derived run shape now print; the
  per-column guarantee summary does not yet.

## Tests

Red-green, in this order:

1. `smelt-logical` unit (`maintenance::signature`) `keyed_model_headline_names_keys_and_key_addressing`
   — per-group `KeyedUpsert{keys:[order_id]}` renders `emits: keyed upsert over [order_id],
   key-addressed`.
2. `smelt-logical` unit `windowed_model_headline_names_the_axis` — `AppendOnlyWindow{axis:
   order_date}` renders `emits: append-only within a window, window-addressed by order_date`.
3. `smelt-logical` unit `general_model_headline_is_whole_table_addressed_and_names_the_cause`
   — `General{reason}` renders `whole-table-addressed` carrying `reason` verbatim.
4. `smelt-logical` unit `no_derivable_verdict_prints_no_signature` — an empty verdict list
   yields no `emits:` clause (never a fabricated `general`), and a mixed-verdict model renders
   the meet, naming the degrading group.
5. `smelt-logical` unit `composed_model_headline_appends_the_locality_bound` — a keyed model
   with a `KeyLocality` appends its locality slice bound to the keyed headline.
6. `smelt-db` (`tests/integration`, maintenance) `plan_report_carries_own_signature_and_run_shape`
   — `maintenance_plan_report` populates the model's own per-group output-delta verdicts and a
   run shape that is snapshot-reconcile for an unclocked keyed model, window-forward for a
   clocked one.
7. `smelt-cli` (`tests/explain_maintenance.rs`) `explain_headline_is_the_first_line` — the first
   non-empty line of `smelt explain <model>` is the signature headline (grain label and run
   shape present), and `Maintenance plan:` follows it.
8. `smelt-cli` (`tests/explain_model.rs`) `explain_json_carries_signature_headline` — `--json`
   exposes the same signature/addressing/grain/run-shape fields, byte-equal in content to the
   text headline's parts (no CLI-side re-formatting).

## Tasks

1. Land the spec delta above (spec-first; no code yet).
2. New pure `crates/smelt-logical/src/maintenance/signature.rs`: `SignatureHeadline` (emits,
   addressing, grain label, optional run shape, optional locality bound) + a `render()`; derive
   addressing from the per-group `OutputDelta` using the SAME projection rules as
   `edge_type.rs::type_edge` (keyed → key-addressed, window → window-addressed by axis, general
   → whole-table-addressed naming the cause). Export from `maintenance`/`lib.rs`. Tests 1–5.
3. Extract the "this file's own per-group output-delta verdicts" fold in
   `smelt-db/src/lib.rs` (currently inline in `ref_model_edge`'s `output_shape`) into a shared
   helper; call it from both `ref_model_edge` and `maintenance_plan_report`.
4. Add `own_output_delta: Vec<(String, OutputDelta)>` and `run_shape: Option<KeyedRunShape>` to
   `MaintenancePlanResult`; populate `run_shape` from the keyed classification
   `maintenance_plan_report` already runs (`CumulativeClassification::is_snapshot_reconcile`),
   and from the partition-grain sweep where the grain is `partition`. Test 6.
5. `smelt-cli/src/explain.rs::build_maintenance_plan_report`: build the `SignatureHeadline` from
   the plan result (+ `result.plan.key_locality`, + resolved grain) and `writeln!` it as the
   report's first line, above `Maintenance plan:`. No formatting logic in the CLI. Test 7.
6. `build_maintenance_plan_json`: add the same fields (`signature`, `addressing`, `grain`,
   `run_shape`) sourced from the identical `SignatureHeadline` value. Test 8.
7. Update `docs-site/docs/reference/cli.md`'s `smelt explain` output description and any golden
   explain fixture whose leading line moves.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage --quiet`
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model --test explain_show_sql --test explain_probes --quiet`
- `cargo test -p smelt-db --test integration --quiet`
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --quiet`
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches.

## Commit message

`feat(explain): lead the maintenance report with the derived delta-signature headline`
