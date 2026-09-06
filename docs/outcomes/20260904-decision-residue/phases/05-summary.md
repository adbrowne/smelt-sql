# Phase 5 summary — retire per-column `data_latency`; lateness leaves plan derivation

## Shipped

- `crates/smelt-core/src/metadata.rs`: `ColumnMetadata::data_latency` deleted; a `columns:`
  entry declaring `data_latency` is a hard `YamlParseError` naming `mutation_profile.lateness`
  and the offending column(s) (`column_data_latency_fixit_message`, wired into both the
  single-model and multi-model frontmatter key walks).
- `crates/smelt-core/src/sources.rs`: `SourceColumnDef::data_latency` (legacy aggregate-format
  column) deleted; its `Deserialize` impl refuses the key with the same fix-it text
  (`column_data_latency_retired_message`, shared with `metadata.rs`).
- `crates/smelt-logical/src/analysis/temporal.rs`: `compute_effective_window` drops its
  `data_latency_days` parameter — the effective window is the AST-derived reach alone.
- `crates/smelt-runtime/src/windowing.rs`: `data_latency_days` removed from the whole
  `compute_incremental_windows`/`_impl`/`compute_calendar_windows`/`compute_integer_windows`/
  `compute_incremental_windows_ordered` chain; the integer-axis "nonzero data_latency" refusal
  deleted (the SQL-lookback refusal stays).
- `crates/smelt-runtime/src/execute.rs`, `crates/smelt-cli/src/commands/explain.rs`: the
  `metadata.columns[..].data_latency` reads deleted along with the argument they fed.
- `crates/smelt-runtime/src/diagnostics.rs`: `InboundEdgeContract` gained an optional
  `lateness: Option<String>` field, populated from a source's declared
  `mutation_profile.lateness` in `build_relation_contract`.
- `crates/smelt-cli/src/explain.rs`: prints `orchestration-only fact: lateness = <interval>
  (never a plan input)` under a source inbound edge that declares one; `--json` carries the
  same as `inbound_edges[].lateness`.
- New tests: `smelt-logical/tests/lateness_orchestration_only.rs` (grep gate + a
  `compute_effective_window` signature pin), `smelt-db/tests/retired_data_latency.rs`,
  `examples/broken/models/retired_data_latency.sql` + its `example_diagnostics.rs` entry,
  `explain_maintenance.rs::explain_prints_lateness_as_orchestration_only` +
  `explain_json_carries_lateness` (both against `examples/timeseries`'s `raw.events`, which
  already declared `mutation_profile.lateness: '2 hours'`).
- Spec edits: `models.md`, `model_properties.md` Known-Divergences bullets deleted;
  `sources.md`'s `MalformedSource` row extended; `cli.md`'s explain sections gained the
  text/JSON lateness sentences.
- Regenerated `docs-site/docs/examples/web-analytics/deduplication.md` (tutorial freshness
  gate) and updated the `explain_show_sql_daily_events_golden.txt` fixture — both now carry
  the new orchestration-only line for sources that declare lateness.

## Decisions

- Hard error routed through the existing `MetadataError::YamlParseError(custom(...))` channel
  (the `batched:` retirement's own channel), not a new `MetadataError` variant — keeps the
  exhaustiveness gate untouched.
- `DataLatency` the *grammar* survives (`contract.frozen_horizon` still parses intervals
  through it) — only the per-column key dies, pinned by its own test.
- The undocumented per-column `data_latency` on legacy `SourceColumnDef` (parsed, read by
  nothing) retired in this same phase as a `MalformedSource`-shaped refusal, rather than left
  as a silent dead key.
- `InboundEdgeContract::model` always carries `lateness: None` — lateness is a source-only
  declaration; only `InboundEdgeContract::source` threads a value through.

## For the next planner

- `examples/timeseries/models/sources/raw/events.yml` already declared
  `mutation_profile.lateness: '2 hours'`, which made two golden/tutorial fixtures drift the
  moment the new orchestration-only line landed (`crates/smelt-cli/tests/explain.rs`'s
  `show_sql_output_unchanged` and `tutorial_freshness.rs`'s web-analytics dedupe page). Both
  are fixed here, but any future report-line addition should grep for other `mutation_profile`-
  declaring sources reused by golden fixtures before landing.
- Phase 6 (append-only posture probe: late arrival vs violation) and phase 7 (delete remaining
  divergence bullets, validate all four specs) are still `pending`/unblocked by this phase.
- Not done, out of scope for this phase: any scheduler/`--auto` consumption of lateness — per
  the outcome's own "Out of scope" section, unchanged.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test lateness_orchestration_only --test walk_coverage` — ok.
- `cargo test -p smelt-core --lib metadata` / `--lib sources` — ok.
- `cargo test -p smelt-db --test retired_data_latency` — ok.
- `cargo test -p smelt-runtime --test windowing_parity --test partition_axis_windowing` — ok.
- `cargo test -p smelt-cli --test explain_maintenance --test explain_docs_freshness --test example_diagnostics` — ok.
- `cargo test -p smelt-lsp --test example_workspaces` — ok.
- `cargo test -p smelt-core --test hardening_budget` — ok, baseline unchanged.
