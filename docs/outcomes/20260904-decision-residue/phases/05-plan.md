# Phase 5 plan — retire per-column `data_latency`; lateness leaves plan derivation

## Objective

Make declared lateness orchestration-only in code, not just in the spec. The per-column
`data_latency` key becomes a hard error with a fix-it naming `mutation_profile.lateness` on the
source; `compute_effective_window` and the runtime windowing chain stop taking a lateness input;
a grep gate pins that no plan-derivation code reads lateness; and `smelt explain` prints a
source's declared lateness labelled as an orchestration-only fact. Advances success criteria 5
and 7 (and the criterion-8 bullets this phase itself closes).

## Spec delta (implement step makes these edits first)

- `docs/specs/models.md` — delete the §Known Divergences bullet "The retired per-column
  `data_latency` key still parses and still widens the run window" (line ~347). The §Surface row
  at line 199 already states the hard error; no edit needed there.
- `docs/specs/model_properties.md` — delete the §Known Divergences bullet
  "`compute_effective_window` still sums declared lateness into the lookback" (line ~419).
- `docs/specs/sources.md` — extend the `MalformedSource` §Diagnostics row (line ~293) with
  "a per-column `data_latency:` inside `columns:` (retired — declare lateness once as
  `mutation_profile.lateness`)". Undocumented-but-parsed today; retiring it silently would leave
  a dead key that fails loud nowhere.
- `docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report" and §"`smelt explain
  --json` output schema` — one sentence each: an inbound **source** edge that declares
  `mutation_profile.lateness` renders an `orchestration-only fact: lateness = <interval>
  (never a plan input)` line, and the JSON edge carries the same as an append-stable
  `lateness` field.

## Tests (red first)

- `smelt-core` `metadata.rs::test_column_data_latency_is_a_hard_error_with_fixit` — a model
  frontmatter `columns: {ts: {data_latency: '3 days'}}` fails to parse; message names
  `mutation_profile.lateness` and the source key, mirroring `batched_subblock_fixit_message`.
- `smelt-core` `metadata.rs::test_contract_frozen_horizon_still_parses_its_interval` — the
  `DataLatency` *grammar* survives (used by `contract.frozen_horizon`); only the column key dies.
- `smelt-core` `sources.rs::test_source_column_data_latency_is_malformed_source` — replaces
  `test_sources_with_data_latency`; the per-column key is a `MalformedSource` with the same fix-it.
- `smelt-db` `tests/retired_data_latency.rs` — `file_diagnostics()` on the broken fixture yields
  one Error naming `mutation_profile.lateness`.
- `smelt-cli` `example_diagnostics.rs::broken_workspace_retired_data_latency` — the
  `examples/broken/` fixture reports on the expected file (LSP-parity channel, mirroring
  `broken_workspace_partition_grain_forbids_metrics`).
- `smelt-logical` `tests/lateness_orchestration_only.rs::no_plan_derivation_reads_lateness` — the
  grep gate: no `.lateness` / `.source_lateness` / `data_latency` *field read* in
  `crates/smelt-logical/src` production text (doc comments and `: None` fixture literals
  excluded), and `compute_effective_window`'s signature carries no latency parameter.
- `smelt-logical` `temporal.rs` — replace `effective_window_sums_lateness_and_reach` with
  `effective_window_is_reach_only`; the other `compute_effective_window` unit tests lose the
  latency argument.
- `smelt-runtime` `tests/partition_axis_windowing.rs` — the integer-axis `data_latency` refusal
  test is deleted (its input no longer exists); the SQL-inferred-lookback and skew refusals stay,
  asserted unchanged.
- `smelt-runtime` `tests/windowing_parity.rs` — the two lateness-widened expectations become
  reach-only; a run over a source declaring `mutation_profile.lateness: '3 days'` produces
  byte-identical batches to the same project with the declaration removed.
- `smelt-cli` `tests/explain_maintenance.rs::explain_prints_lateness_as_orchestration_only` —
  text output carries the labelled line and `--json` the `lateness` field, for a model whose
  source declares it; a source without one prints neither.

## Tasks

1. Spec edits above (spec-first).
2. `smelt-core/src/metadata.rs`: delete `ColumnMetadata::data_latency`; add a pre-validation arm
   in the frontmatter key walk (next to the `batched:` arm) that refuses any `columns:` entry
   declaring `data_latency`, via `MetadataError::YamlParseError(custom(fixit))` — same channel as
   `batched:`, so no new `MetadataError` variant and the exhaustiveness gate is untouched.
   Write the fix-it builder to name the offending column(s).
3. `smelt-core/src/sources.rs`: delete `SourceColumnDef::data_latency` and refuse the key in
   `RawColumn2` with `MalformedSource` + the same fix-it text.
4. `smelt-logical/src/analysis/temporal.rs`: drop `data_latency_days` from
   `compute_effective_window` and `format_explanation`; the lookback is the AST reach alone.
   Update the `EffectiveWindow::lookback_days` doc comment.
5. `smelt-runtime/src/windowing.rs`: remove `data_latency_days` from the parameter chain
   (`compute_incremental_windows`, `_impl`, `compute_integer_windows`, the calendar path and the
   `build_*` helper at ~688) and delete the integer-axis lateness refusal at ~570.
6. `smelt-runtime/src/execute.rs` (~5497/5611) and `smelt-cli/src/commands/explain.rs` (~892):
   delete the `metadata.columns[..].data_latency` reads and the argument they fed.
   Fix `smelt-core/src/graph.rs`, `smelt-ui/src/build.rs`, `smelt-runtime/src/compile.rs`
   struct literals that set `data_latency: None`.
7. `smelt-lsp/src/column_resolution.rs:119`: drop the `data_latency:` special case (the key is
   no longer a legal column sub-key).
8. `examples/broken/models/retired_data_latency.sql` fixture + its `example_diagnostics` entry.
9. `smelt-cli/src/explain.rs`: in the inbound-edge block, print the source's declared
   `mutation_profile.lateness`/`source_lateness` as `orchestration-only fact: lateness = …
   (never a plan input)`; add the append-stable `lateness: Option<String>` to
   `InboundEdgeContract` (`smelt-runtime/src/diagnostics.rs`) so text and `--json` share it.
10. Add the grep gate test file; run `rg -n lateness crates/smelt-logical/src` and confirm every
    surviving hit is a doc comment or a test fixture literal.
11. Confirm `rg -n data_latency examples/ docs-site/docs/` is empty (expected already), and that
    the hand-pasted `smelt explain` excerpts in `docs-site/docs/reference/cli.md` and
    `guide/incremental-models.md` still match the binary — regenerate them if the new line
    appears for their model.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test lateness_orchestration_only --test walk_coverage`
- `cargo test -p smelt-core --lib metadata sources`
- `cargo test -p smelt-db --test retired_data_latency`
- `cargo test -p smelt-runtime --test windowing_parity --test partition_axis_windowing`
- `cargo test -p smelt-cli --test explain_maintenance --test explain_docs_freshness --test example_diagnostics`
- `cargo test -p smelt-lsp --test example_workspaces`
- `cargo test -p smelt-core --test hardening_budget` (baseline unchanged or lowered)

## Commit message

`feat(incremental)!: retire per-column data_latency and remove lateness from plan derivation`
