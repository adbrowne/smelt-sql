## Drift Report: timeseries

**Spec**: docs/specs/timeseries.md (last_reviewed: 2026-05-21)
**Date**: 2026-05-31
**Phase**: C4 (feature sweep)

### Automated checks
- cargo fmt — PASS (`--check` clean)
- cargo clippy — PASS (no changes this phase; pre-flight green)
- cargo test — PASS (pre-flight `cargo test --quiet` green baseline)
- example_diagnostics — PASS (87 passed, 1 ignored; incl. `timeseries_broken_incremental_without_timeseries`, `timeseries_no_diagnostics`)
- example_workspaces (LSP) — PASS (27 passed)
- smelt-runtime — PASS

### Surface drift
- ✅ `timeseries:` frontmatter block (`event_time_column`, `partition_column`, `granularity`) — present: `TimeseriesConfig` (`crates/smelt-core/src/config.rs:354`), `ModelMetadata.timeseries` (`metadata.rs:137`). Docs-site `docs/reference/timeseries.md` documents the same keys consistently.
- ✅ `granularity` closed enum (`hour|day|week|month|quarter|year`) — `Granularity` (`config.rs:296`). Docs-site matches.
- ✅ Source-YAML `timeseries:` block — parsed via `SourceInfo` path; docs-site documents it.
- ✅ `MalformedTimeseries` / `TimeseriesRequiredForIncremental` diagnostic codes exist (`diagnostics_types.rs:596,600`) and are mapped into `file_diagnostics` (`smelt-db/src/lib.rs:1025`).
- ✅ Timeless-oracle: no `Phase [A-Z0-9]` leaks in spec body or docs-site reference page.
- ❌ **Diagnostic table — "unknown key → `MalformedTimeseries`"**: `TimeseriesConfig` lacks `#[serde(deny_unknown_fields)]` (unlike sibling `ModelMetadata`/`IncrementalConfig`), so an unknown key in the `timeseries:` block is silently accepted. See **BUG-025**. (`config.rs:354`)
- ❌ **Diagnostic table — "missing required key / `granularity` not in enum → `MalformedTimeseries`"**: these are serde-level parse errors of the frontmatter, not `validate_timeseries` outputs. They are **not** surfaced as `MalformedTimeseries` anywhere — `smelt-db/src/lib.rs:865` only bridges `Generates*` variants; a generic serde error falls through and the model's *entire* frontmatter is silently dropped (`discovery.rs:264-270`). See **BUG-023**. (Concretely: `granularity: fortnight` builds a `materialization: table` model as a **VIEW**, exit 0, no diagnostic, no warning.)
- ⚠️ Surface `week_start` "One of: `monday`, `sunday`" — `Weekday` enum (`config.rs:208`) accepts all seven days; `validate_timeseries` checks only week_start-requires-week (rule 6), not the monday/sunday closure. See **BUG-026**. (`week_start` is itself a Known Divergence — "not yet implemented".)
- ⚠️ References → User docs says "to be authored alongside the migration plan" but `docs-site/docs/reference/timeseries.md` already exists and conforms. Stale spec self-reference (References section, not body) — recommend a one-line spec touch in the post-sweep pass.

### Semantics drift
- ✅ Rule "incremental: without timeseries: → `TimeseriesRequiredForIncremental`" — `validate_timeseries` (`metadata.rs:344`); test `timeseries_broken_incremental_without_timeseries` (example_diagnostics.rs:2200).
- ✅ Rule "timeseries: on ephemeral/test → `MalformedTimeseries`" — `metadata.rs:354-369`; unit test `metadata.rs:1323`.
- ✅ Rule 6 "week_start requires granularity: week → `MalformedTimeseries`" — `metadata.rs:373`.
- ⚠️ Rule 1 "partition_column must appear in the SELECT output (and GROUP BY)" — enforced only by a weak substring heuristic (`metadata.rs:381-395`, `upper_body.contains(col)`); not a true SELECT-list parse (code comment admits this). Accepts a column that appears only in a comment / non-projection position. Known approximation, low severity.
- ✅ Rules 2, 3, 4 (event-time projection, event-time type, partition-column type) — explicitly deferred per spec §Known Divergences ("Output-schema-dependent validation rules"); not implemented, consistent with the spec.
- ❌ **`MalformedTimeseries` / `TimeseriesRequiredForIncremental` do not gate the CLI run/build pipeline.** They fire in `file_diagnostics`/LSP, but `run.rs:683-689` only gates on `UnknownSmeltFn`. A model with a malformed-but-parseable timeseries block (week_start-on-day, partition-absent) builds exit 0. Concretely demonstrated (built as BASE TABLE despite `MalformedTimeseries`). See **BUG-024** (BUG-006 class).

### Invariant drift
- ✅ Granularity closure (Invariant 3) — enum-enforced at the type level.
- ✅ Sources never run (Invariant 5) — no execution path consumes source `timeseries:`.
- ❌ The frontmatter-fragility behaviour (BUG-023) is the timeseries manifestation of the **Unified-frontmatter** parity gap already logged as BUG-016: a parse error in any one frontmatter sub-block silently discards the whole block. Architectural (workspace-loading-parity seam).

### Freshness
- last_reviewed: 2026-05-21
- most recent code change to Reference → Code paths: 2026-05-23 (`web_analytics` incremental conversion, #122) touching `metadata.rs`/`config.rs`
- Verdict: **mostly fresh**; the migration that the spec's Known Divergences describe as "upcoming" is in fact landed (the `timeseries:` block exists and is validated). The §Known Divergences "Migration from nested incremental:" and References → User docs ("to be authored") notes are stale — recommend a `/smelt:spec timeseries` touch in the post-sweep pass (do not auto-edit). Cross-references BUG-004.

### Summary
- Drift items: 4 ledger findings (BUG-023..BUG-026), all `needs-review`; plus 2 minor stale spec self-references (folded into Freshness).
- Recommended next step: batch BUG-023/024 with the systemic frontmatter-fragility + diagnostic-gating decision (BUG-006/016 family) in the post-sweep human pass; BUG-025/026 are contained block-validation gaps to resolve alongside. No spec edited and no code changed in-loop (all findings intersect the systemic gating decision or a deferred feature).
