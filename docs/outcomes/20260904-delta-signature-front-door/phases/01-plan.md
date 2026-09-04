# Phase 1 — `smelt explain` delta-signature headline (text + `--json`)

**Outcome:** `docs/outcomes/20260904-delta-signature-front-door/outcome.md`
**Advances:** success criterion 1.

## Objective

Make the model's own delta signature the first line of `smelt explain <model>`'s
maintenance-plan report, in the form `incremental_models.md` §Surface "CLI" already
specifies, and carry the identical fields in `--json`. The model's own `OutputDelta` is
today derivable only for *upstream* edges (`smelt_db::ref_model_edge`); this phase gives
the model itself the same single-owner derivation and renders both surfaces from one
struct so text and JSON cannot drift.

## Spec delta (made first, by the implement step)

1. `docs/specs/incremental_models.md` §Surface "CLI", Headline bullet — state that the
   headline is the report's **first** line, and add the missing third rendering: a
   `general` verdict prints `emits: general (degraded by: <reason>), not
   delta-addressable` (no addressing claim), matching `format_output_delta`'s existing
   vocabulary.
2. `docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report" — document the
   headline line and the `--json` `delta_signature` object as an append-stable addition
   (§Constraints item 5): `shape` (`append_only_window` | `keyed_upsert` | `general`),
   `addressing` (`window` | `key` | `none`), `keys` / `axis` / `degraded_by` (present per
   shape), `slice_bound` + `settle_bound` (present when key temporal locality is
   admitted), and `grain` (the derived friendly label already printed as `derived grain:`).
3. `docs/specs/incremental_models.md` §Known Divergences — the bullet "`smelt explain`
   does not yet print the delta-signature headline" is narrowed: the headline clause is
   deleted; the per-column guarantee summary and derived run shape remain listed (they are
   IS-19, out of scope for this outcome).
4. `docs-site/docs/reference/cli.md` §`smelt explain` — document the headline and the
   `delta_signature` JSON object in the same terms.

## Tests (red first)

- `crates/smelt-db/tests/typed_model_edge.rs::own_output_delta_matches_downstream_edge_view`
  — the new `smelt_db::model_output_delta_for(db, ws, file)` returns exactly the
  `OutputDelta` a downstream model's `model_edges_for` reports for that same upstream
  (single-owner assertion; no second derivation).
- `crates/smelt-cli/tests/explain_maintenance.rs::headline_is_the_reports_first_line`
  — for a keyed example model the first non-empty report line starts with
  `model <name>  (emits: keyed upsert over [` and contains `key-addressed`.
- `…::partition_grain_headline_is_window_addressed` — `examples/timeseries`'
  partition-grain model prints `append-only within a window, window-addressed by <axis>`.
- `…::composed_headline_appends_slice_bound` — a model whose plan carries `key_locality`
  additionally prints `slice-bounded by <axis> under key temporal locality` and its
  settle bound.
- `…::general_headline_names_the_degrading_construct` — a degraded model prints
  `general (degraded by: …), not delta-addressable` and claims no addressing.
- `…::headline_grain_label_matches_derived_grain_row` — the `grain:` clause in the
  headline is the same string the report's `derived grain:` row prints (no second label).
- `…::explain_json_delta_signature_matches_text_headline` — `--json`'s `delta_signature`
  object's fields reproduce every element the text headline renders, for the same model.
- `…::docs_reference_documents_the_headline` — `docs-site/docs/reference/cli.md` mentions
  the `emits:` headline and `delta_signature` in the explain section (doc-sync guard).

## Tasks

1. Extract `ref_model_edge`'s own-shape derivation in `crates/smelt-db/src/lib.rs` into a
   private helper, and expose `pub fn model_output_delta_for(db, ws, file) -> Option<OutputDelta>`
   as a thin Salsa wrapper that folds `derive_workspace_output_deltas` once and calls it
   (Salsa purity rule; the edge path calls the same helper).
2. Add `pub struct DeltaSignatureHeadline` to `crates/smelt-cli/src/explain.rs` — plain
   `Serialize` data (shape, addressing, keys/axis/degraded_by, slice_bound, settle_bound,
   grain) — plus `pub fn delta_signature_headline(shape, key_locality, contract_view)` and
   `render_text(&self) -> String`. `format_output_delta` stays the single vocabulary owner
   for the shape names.
3. Prepend the rendered headline to `build_maintenance_plan_report`'s output (before
   `Maintenance plan: <name>`), taking the shape as a new argument.
4. Add `delta_signature: DeltaSignatureHeadline` to `ExplainMaintenanceJson` and populate
   it in `build_maintenance_plan_json` from the SAME struct instance the text renders.
5. Thread `smelt_db::model_output_delta_for` through `commands/explain.rs::explain_maintenance_plan`
   into both builders.
6. Apply the four spec/doc edits above.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-db --test typed_model_edge --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model --test explain_show_sql --test cli_docs_coverage --quiet 2>&1 | tail -30`

## Commit message

`feat(cli): smelt explain leads with the model's delta-signature headline`
