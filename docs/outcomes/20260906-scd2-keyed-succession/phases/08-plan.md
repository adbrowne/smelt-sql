# Phase 8 plan — Explain surface for the succession grain

## Objective

Make `smelt explain <model>` (text and `--json`) render a succession model's grain,
`(k, t)` identity, technique, run axis vs clock and partitioning posture, fixed execution
postures, pre-window filter, and the tombstone ledger as internal state — plus the
`keyed_succession` delta-signature headline. Advances success criterion 8; the last
criterion before fixture/docs (9) and close (10).

## Spec delta

None. Phase 1 already pinned the whole surface: `docs/specs/cli.md` §"Succession grain"
(the eight text lines and the `--json` `succession` object) and the same file's
§"Delta-signature headline" (`event history keyed by [<k>], event-addressed by (<k>, <t>)`
and the `keyed_succession`/`event` `delta_signature` values). This phase implements it.

## Tests

Text (`crates/smelt-cli/tests/explain_maintenance/succession.rs`, new module wired into
`main.rs`; stage an arrival-partitioned succession project via a new `support.rs` helper
`stage_succession_project` modelled on `stage_delta_type_project`):

1. `succession_cell_prints_grain_identity_and_technique` — `grain: succession`,
   `identity: (customer_id, effective_ts)`, `technique: succession-patch` in that order.
2. `succession_cell_prints_run_axis_and_clock_for_an_arrival_partitioned_source` —
   `run axis: ingested_date (arrival-partitioned)` and `clock: effective_ts`.
3. `succession_cell_prints_event_time_partitioning_when_axis_equals_clock` — same project
   with the source's `partition_column == event_time_column` renders
   `(event-time-partitioned)`.
4. `succession_cell_prints_fixed_execution_postures` — `posture: re-run tolerant;
   order-independent but serial`.
5. `succession_cell_prints_pre_window_filter_only_when_declared` — the clamped model
   prints `pre-window filter: <sql>`; the unclamped one prints no such line.
6. `succession_cell_prints_the_tombstone_ledger_as_internal_state` — `internal state:
   tombstone ledger customer_history__tombstones (customer_id, effective_ts) — not part
   of the model's public schema`.
7. `succession_headline_is_event_addressed` — the report's first line reads
   `(emits: event history keyed by [customer_id], event-addressed by (customer_id,
   effective_ts); grain: …)`.
8. `non_succession_model_prints_no_succession_lines` — a keyed-upsert model in the same
   project prints none of the seven lines (no leakage).

JSON (`crates/smelt-cli/tests/explain_model/json_output.rs` or a sibling module in
`explain_maintenance/succession.rs` driving `--json`):

9. `succession_json_object_carries_every_field` — `succession.key_columns`,
   `clock_column`, `run_axis`, `partitioning == "arrival"`, `lead_columns`,
   `lag_columns`, `delete_flag`, `pre_window_filter`,
   `tombstone_ledger.{table,columns}`, `rerun_tolerant/order_independent/concurrent`.
10. `succession_json_omits_absent_optional_fields` — no `pre_window_filter` and no
    `delete_flag` key at all (not `null`) for a model declaring neither.
11. `succession_json_delta_signature_is_keyed_succession` — `delta_signature.shape ==
    "keyed_succession"`, `addressing == "event"`, `keys`, `axis == run axis`.
12. `non_succession_json_omits_the_succession_key` — the key is absent, never `null`.

Unit (`crates/smelt-runtime/src/maintenance_driver/succession/tests.rs`):

13. `run_axis_classifies_arrival_vs_event_time_partitioning` — the new shared
    axis-resolution helper returns `Arrival`/`EventTime` from the driving source's own
    `timeseries`, and `None` when the source is unresolvable.

## Tasks

1. Add `SuccessionAxis { column, partitioning }` + `pub fn resolve_succession_run_axis(
   recipe, source_infos) -> Option<SuccessionAxis>` to
   `crates/smelt-runtime/src/maintenance_driver/succession/mod.rs`, classifying
   `partitioning` by `ts.partition_column == ts.event_time_column`; refactor
   `resolve_live_succession_cell` to consume it so the bare-name source matching has one
   owner (test 13).
2. Expose the grain's fixed postures from `smelt-logical` —
   `maintenance::succession::SUCCESSION_POSTURES` (rerun-tolerant, order-independent,
   non-concurrent) with the `incremental_shapes.md` §"Run shape and late events" citation —
   so explain reads them rather than restating spec text (maintenance-plan purity).
3. Add `SuccessionExplainView` to `crates/smelt-cli/src/explain.rs` beside
   `DeltaSignatureHeadline`: built once from `(recipe, run axis, model db name)` via
   `smelt_logical::maintenance::emit::tombstone_table_name`, with `render_text_lines()`
   and a `Serialize` JSON projection (`skip_serializing_if = "Option::is_none"` on every
   optional field) — one struct, both surfaces.
4. Build the view once in `crates/smelt-cli/src/commands/explain.rs` from
   `result.succession_recipe` + `source_infos`, and thread `Option<&SuccessionExplainView>`
   into both `build_maintenance_plan_report` and `build_maintenance_plan_json`.
5. Render the seven text lines directly after the ordinary cell block for the
   `Technique::SuccessionPatch` cell, in spec order, omitting valueless lines (tests 1–6, 8).
6. Add the `succession` field to `ExplainMaintenanceJson` (tests 9, 10, 12).
7. Widen `delta_signature_headline` with an `Option<&SuccessionExplainView>` argument that
   takes precedence over `own_output_delta`, producing `shape: "keyed_succession"`,
   `addressing: "event"`, `keys`, `axis`, and the `event history keyed by …,
   event-addressed by (…)` `render_text` arm (tests 7, 11). Do **not** add an
   `OutputDelta::KeyedSuccession` variant — derived output facts for a succession model's
   consumers are out of scope for this outcome.
8. Add `stage_succession_project` (and the clamped / event-time-partitioned variants) to
   `crates/smelt-cli/tests/explain_maintenance/support.rs`; keep every new file under the
   large-file baseline.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model --test explain
  --test cli_unit --quiet`
- `cargo test -p smelt-cli --test cli_docs_coverage --test explain_docs_freshness --quiet`
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --quiet` and
  `cargo test -p smelt-runtime --lib maintenance_driver::succession --quiet`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`feat(succession): render the succession grain in smelt explain (text and --json)`
