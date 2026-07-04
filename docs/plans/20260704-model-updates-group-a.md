# Plan: Model updates — Group A (Rename & ontology landing)

**Date**: 2026-07-04
**Master plan**: [`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) — Group A (phases A1–A4)
**Specs (oracles)**:
- [`docs/specs/models.md`](../specs/models.md) — §"Refresh axis", §"Materialization (storage) modes", §"Constraint violations"
- [`docs/specs/batched_models.md`](../specs/batched_models.md) — §Surface, §Known Divergences
- [`docs/specs/multi_backend.md`](../specs/multi_backend.md) — §"Incremental-view-maintenance capabilities"
- [`docs/specs/materialized_view.md`](../specs/materialized_view.md) — §"No silent fallback"
- [`docs/specs/smelt_yml.md`](../specs/smelt_yml.md), [`docs/specs/data_catalog.md`](../specs/data_catalog.md), [`docs/specs/diagnostics.md`](../specs/diagnostics.md)
**Spec diff**: the 2026-07-04 spec edits (committed in `f056ac35`) that reshaped the refresh axis, renamed `incremental → batched`, removed `materialized_view` from the storage axis, and added the two IVM capability flags. This plan closes the code↔spec gap those edits opened (tracked in each spec's §Known Divergences).
**Tracking branch**: `worktree-incremental`
**Docs**: code+docs

**Key decision (fixed by the user, do not re-open).** A1 **hard-cuts** the old `incremental:` frontmatter block. There is **no** dual-accept deprecation window: a model that still declares an `incremental:` block is a hard error directing the user to `refresh: batched`. Example workspaces are migrated to the new surface in the same phase.

---

## Execution prompt (for a fresh Claude session / the autonomy loop)

You are executing this plan phase by phase. It is the **active sub-plan** registered in
[`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) §"Spawned sub-plans".

**Before touching any code:**
1. Read this entire plan, then read the cited spec sections — they are the correctness oracle. Do not re-open settled spec decisions (esp. the hard-cut).
2. Confirm you are on branch `worktree-incremental`.
3. Find the next `pending` row in the Progress-tracking table below. That is your phase. If every row is `done`, run §Verification, flip this sub-plan's registry Status to `done` in the master, and stop.

**Per phase, run `/smelt:implement`'s loop:** pre-flight (`cargo build`/`cargo test` green except this phase's own red target) → implementer subagent (red-green TDD on the listed tests, real fixtures in `examples/`) → reviewer subagent (material findings only) → iterate → set the row `done` → commit + push with the phase's `Commit.` line. A phase's row lists a **spec increment** where one is pre-authorised; making the cited edits is expected, not scope creep.

**Ordering.** A1 → A2 → A3 → A4. A2 depends on A1 (the `Batched` variant must exist before its diagnostics are renamed). A3 is independent of A1/A2 but sequence it after A2 to keep commits clean. A4 depends on A3 (the storage-axis `MaterializedView` must be gone before `refresh: materialized_view` is wired).

**Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this file only*. Spec + `docs-site/` edits describe the feature as if it always existed; as each phase lands, **remove** the matching §Known-Divergence rename/gap note rather than annotating it with a phase number.

**Block rule.** On a design decision not answered here or by the spec, or a pre-flight red unrelated to this phase's target: set the row `blocked` with a one-line reason, append to §"Blocked phases", restore a clean tree, commit, emit `<<PHASE_BLOCKED>>`. Otherwise emit `<<PHASE_COMPLETE>>`.

---

## Context

The 2026-07-04 spec reshape made the refresh axis a peer enum split by output shape and re-homed
engine-maintained materialized views from storage to `refresh: materialized_view`
(`models.md` §"Refresh axis", §"Materialization (storage) modes"). The implementation lags: today
`RefreshStrategy` is only `{Full, Cumulative}` and the batched mode is selected by a separate
`incremental:` block with `enabled: true`; `Materialization` still carries a `MaterializedView`
variant; and the dialect carries `supports_materialized_views` instead of the two IVM flags. Group A
lands the rename and the ontology so every later group (B, C, D) references the settled surface.

## Scope

### In scope
- **A1** — `refresh: batched` selector + optional `batched:` block; hard-cut the `incremental:` block; the `timeseries:` requirement and the block-without-`refresh` / `refresh`-without-`timeseries` constraint violations (`models.md` §"Constraint violations").
- **A2** — rename the `Incremental`-spelled diagnostic codes + internal config identifiers to `Batched`; downstream spec + `docs-site/` prose sweep; `diagnostics.md` catalogue rows.
- **A3** — remove `MaterializedView` from the storage `Materialization` enum (both the `smelt-core` and `smelt-backend` copies); migration-hint error; `smelt.yml` `default_materialization` + data-catalog sweep.
- **A4** — rename `supports_materialized_views → supports_native_ivm`, add `supports_retraction` (both `false` everywhere), wire the `refresh: materialized_view` hard error when `supports_native_ivm = false`.

### Explicitly deferred
- The batched **eligibility relaxations** (Group B), keyed-mode **maintenance rungs** (Group C), and the new keyed **modes** `versioned`/`latest_value` and the `materialized_view` emit path (Group D). A1 adds `RefreshStrategy::Batched`; A4 makes `refresh: materialized_view` *parse and hard-error*, but the actual emit against an IVM backend is D3.
- Renaming the internal rule module `crates/smelt-logical/src/rules/incremental.rs` and the `IncrementalStrategy` enum is **optional** in A2 — the diagnostic codes and config field names are the user-facing surface that must change; a pure internal file rename may be deferred to avoid churning Group B's live files. If deferred, note it under §"Deferred during implementation".

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| A1    | done    | f72c1d7d | 2026-07-04 |
| A2    | done    | 2e74d516 | 2026-07-04 |
| A3    | pending |        |      |
| A4    | pending |        |      |

---

### Phase A1: `refresh: batched` selector + `batched:` block (hard-cut `incremental:`)

**Goal.** Select the batched mode with `refresh: batched` (implying stored `table`); move the
**existing** config (`unique_key`, `safety_overrides`) into an optional `batched:` block.
Hard-cut the old `incremental:` block: declaring it is an error pointing at `refresh: batched`.

**Pre-conditions.** None (first phase). `RefreshStrategy` today is `{Full, Cumulative}` (`config.rs:26`); the batched mode is selected by `IncrementalConfig { enabled, unique_key, safety_overrides }` (`config.rs:489-500`) whose presence is tied to `timeseries:` at `metadata.rs:412-414`.

> **Scope note.** `nondeterministic_columns` is a **B3** addition — it does not exist today (the current determinism knob is `safety_overrides.allow_nondeterministic`). A1's `batched:` block carries only the fields that exist now (`unique_key`, `safety_overrides`). Do not add `nondeterministic_columns` here.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs` (or `metadata.rs`) unit — `refresh: batched` deserialises to `RefreshStrategy::Batched`; a bare `refresh: foo` still errors listing `batched` among the valid values.
- `crates/smelt-core/src/metadata.rs` unit — a model with `refresh: batched` + `timeseries:` validates; `batched:` block **without** `refresh: batched` is a hard error (`models.md` §"Constraint violations"); `refresh: batched` **without** `timeseries:` errors (`TimeseriesRequiredForBatched` — name lands in A2; A1 may temporarily emit the old code and A2 renames it, OR A1 introduces the new code directly — decide in-phase, prefer introducing `TimeseriesRequiredForBatched` now to avoid a two-step).
- `crates/smelt-core/src/metadata.rs` unit — a model that declares the **old** `incremental:` block is a hard error whose message names `refresh: batched`.
- `crates/smelt-cli/tests/example_diagnostics` (real fixture) — `examples/timeseries/` (and any other workspace using the old `incremental:` block) migrated to `refresh: batched` + `batched:` builds with **zero** diagnostics.

**Implementation shape.**
- Add `RefreshStrategy::Batched` (`config.rs:26`) + its `Deserialize`/`Serialize` arms (`"batched"`).
- Introduce the `batched:` block struct (rehome the `IncrementalConfig` payload fields minus `enabled`); wire it into `ModelMetadata`/`ModelConfig` frontmatter extraction (`metadata.rs`, `config.rs`).
- Frontmatter validation: reject a lone `batched:` block, reject `refresh: batched` without `timeseries:`, reject the retired `incremental:` block with a migration-hint error. Mirror the existing `Cumulative` validation branch (`metadata.rs:389`).
- Migrate every example workspace that used `incremental:` to the new surface.

**Critical files.**
- `crates/smelt-core/src/config.rs` — `RefreshStrategy`, the `batched:` block struct, `ModelConfig`.
- `crates/smelt-core/src/metadata.rs` — frontmatter extraction, constraint-violation validation.
- `examples/*/` — migrate `incremental:` → `refresh: batched` + `batched:`.
- (Read-only reference) `crates/smelt-logical/src/rules/incremental.rs` — how detection reads the config today.

**Docs touched.**
- `docs/specs/batched_models.md` §Known Divergences — remove the "surface is `batched`; code still spells it `incremental`" note's *selector* clause (the `refresh: batched` selection now matches); leave the diagnostic-code-rename clause for A2.
- `docs/specs/models.md` §Known Divergences — remove the "config keys still spell `incremental`" note's selector portion.
- `docs-site/docs/guide/incremental-models.md` — update the opt-in to `refresh: batched` + `batched:`; drop the `incremental: { enabled: true }` form.

**Review checklist.**
- [ ] Tests above exist and assert the hard-cut error names `refresh: batched`.
- [ ] `models.md` §"Constraint violations" rows for batched are all enforced.
- [ ] Every example workspace migrated; `example_diagnostics` + `-p smelt-lsp --test example_workspaces` green.
- [ ] No dual-accept path left for `incremental:`.
- [ ] Spec/docs edits are timeless (no phase vocabulary).

**Commit.** `feat(core): select batched refresh via 'refresh: batched' + batched: block; hard-cut incremental: block`

---

### Phase A2: Diagnostic-code + config-field rename (`Incremental` → `Batched`)

**Goal.** Rename the user-facing `Incremental`-spelled identifiers to `Batched` and sweep downstream prose.

**Pre-conditions.** A1 landed (`RefreshStrategy::Batched` exists).

**TDD tests to write first.**
- `crates/smelt-core/src/metadata.rs` units — update the two assertions that expect the old code names (`metadata.rs:1500`, `config.rs:1028`) to `TimeseriesRequiredForBatched` / `CumulativeForbidsBatched`; they fail until the enum variants are renamed.
- Diagnostic snapshot / `-p smelt-db` diagnostics tests — regenerate/assert the new code strings.

**Implementation shape.**
- Rename the `DiagnosticCode` enum variants in `crates/smelt-db/src/diagnostics_types.rs`: `TimeseriesRequiredForIncremental` (`:641`), `CumulativeForbidsIncremental` (`:700`), `IncrementalNotBatchSafe` (`:704`, user-visible warning) → their `Batched` spellings.
- Rename the `MetadataError` source variants: `metadata.rs:317`, `:332`; and the rule-side `RuleDiagnosticCode::IncrementalNotBatchSafe` at `rule_diagnostics.rs:50` (emitted `:156`).
- Update every mapping site in `smelt-db/src/lib.rs` (`:1424`, `:1435`, `:1075`) + the exhaustive `map_metadata_error_to_diagnostic` match (compiler-enforced, per CLAUDE.md §Fail-loud). Note `IncrementalRule` gates on `ctx.materialization != "incremental"` (`rules/incremental.rs:129`) — reconcile with the A1 `refresh: batched` selector.
- Internal config type rename (`IncrementalConfig` → `BatchedConfig`) is in scope for the *type name*; the rule-module **file** rename (`rules/incremental.rs`) is **optional** — defer if it collides with Group B's live files (record under §Deferred).
- Update the snapshot/regression assertions in `crates/smelt-cli/tests/example_diagnostics.rs` (`:2162, 2278, 2306, 2440`) and `rule_diagnostics.rs` (`:500, 502`).
- Prose sweep: `timeseries.md`, `sources.md`, `run_state.md`, `planner_integration.md`, `types.md`, `cli.md`, and `docs-site/`.

**Critical files.**
- `crates/smelt-db/src/diagnostics_types.rs` — the `DiagnosticCode` enum.
- `crates/smelt-core/src/metadata.rs` — `MetadataError` variants + emission.
- `crates/smelt-db/src/lib.rs` — mapping sites + `map_metadata_error_to_diagnostic` exhaustive match.
- `crates/smelt-logical/src/rules/rule_diagnostics.rs`, `rules/incremental.rs`, `src/types.rs` — rule code + `IncrementalConfig`/safety types (type rename).
- `docs/specs/diagnostics.md` — **spec increment (pre-authorised)**: rename the §"Incremental" heading → §"Batched", update the three code rows + the `TimeseriesRequiredForIncremental` row under §"Timeseries" and `CumulativeForbidsIncremental` under §"Cumulative aggregate".

**Docs touched.**
- `docs/specs/diagnostics.md` — as above (must agree with the enum in the same commit).
- `docs/specs/batched_models.md` §Known Divergences — remove the diagnostic-code-rename clause of the "still spells it `incremental`" note (now resolved).
- `docs/specs/models.md` §Known Divergences — remove the "diagnostics still spell `incremental`" note.
- `docs-site/` — sweep `incremental`-spelled diagnostic references.

**Review checklist.**
- [ ] `map_metadata_error_to_diagnostic` still exhaustive; build green.
- [ ] `diagnostics.md` rows match the emitted codes exactly.
- [ ] No production identifier emits `TimeseriesRequiredForIncremental`/`CumulativeForbidsIncremental`.
- [ ] Optional file rename either done or recorded under §Deferred.
- [ ] Spec/docs edits timeless.

**Commit.** `refactor(diagnostics): rename Incremental→Batched diagnostic codes + config types; sweep downstream docs`

---

### Phase A3: Remove `materialized_view` from the storage axis

**Goal.** `Materialization` enum → `{ View | Table | Ephemeral }`. `materialization: materialized_view`
becomes an unknown-value error suggesting `refresh: materialized_view`.

**Pre-conditions.** A2 landed (clean commit boundary).

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs` unit — `materialization: materialized_view` fails deserialisation with a message naming `refresh: materialized_view` as the replacement; `table`/`view`/`ephemeral` still parse.
- Data-catalog test — catalog serialization no longer lists a `materialized_view` storage value.

**Implementation shape.**
- Drop the `MaterializedView` variant from `Materialization` (`config.rs:64`) and its `Deserialize`/`Serialize` arms; add the migration hint to the error string (mirror the existing removed-value hints at `config.rs:85–88`).
- Drop the parallel variant in `crates/smelt-backend/src/types.rs:28` (and `MaterializationStrategy` at `:96` if it carries a MV arm); fix `crates/smelt-backend/src/lib.rs:134` (the `supports_materialized_views` branch on the storage value — this becomes A4's `refresh` concern; here just remove the storage-value branch, replacing with `Table` handling or a TODO A4 wires).
- Sweep `default_materialization` validation (`smelt.yml`) and any printer/DDL branch that emitted a materialized view from the storage value.

**Critical files.**
- `crates/smelt-core/src/config.rs` — `Materialization` (`:63-71`) + Deserialize/Serialize.
- `crates/smelt-backend/src/types.rs` (`:28-58`, the `{Table,View,MaterializedView}` copy), `crates/smelt-backend/src/lib.rs` (`:115-204`, `:134` branch).
- `crates/smelt-cli/src/docs.rs` (`CatalogModel`, `:44/171`) + `crates/smelt-ui/src/types.rs` (`:17,46`) — catalog serialization.
- `crates/smelt-cli/tests/materialization_parity.rs` (`:8-11,120-121`) — the MV-fallback parity test; update/retire.

**Docs touched.**
- `docs/specs/models.md` §"Materialization (storage) modes" — already `{view,table,ephemeral}`; remove any residual §Known-Divergence storage-`materialized_view` note.
- `docs/specs/smelt_yml.md`, `docs/specs/data_catalog.md` — remove §Known-Divergence notes about the storage `materialized_view` value.
- `docs-site/docs/guide/materializations.md` — drop `materialized_view` as a storage option; point to `refresh: materialized_view`.

**Review checklist.**
- [ ] `materialization: materialized_view` errors with the migration hint.
- [ ] Both `Materialization` copies + catalog output drop the variant.
- [ ] `cargo test` green (no residual match arm).
- [ ] Spec/docs §Known-Divergence notes removed.

**Commit.** `refactor(core): drop materialized_view from the storage Materialization axis; add migration hint`

---

### Phase A4: IVM capability flags + `refresh: materialized_view` hard error

**Goal.** Rename `supports_materialized_views → supports_native_ivm`; add `supports_retraction`
(both `false` on every current backend); make `refresh: materialized_view` a hard error naming the
reason when `supports_native_ivm = false`.

**Pre-conditions.** A3 landed (storage-axis MV gone); A1 landed (`RefreshStrategy` exists to add the `MaterializedView` refresh value to).

**TDD tests to write first.**
- `crates/smelt-dialect` unit / capability-conformance suite — every backend advertises `supports_native_ivm = false` and `supports_retraction = false`.
- `crates/smelt-cli` (or `smelt-db`) real-fixture test — a model with `refresh: materialized_view` on DuckDB errors with the `materialized_view.md` §"No silent fallback" message: *"requires native incremental-view maintenance; this engine has none — use `refresh: cumulative`…"*. It does **not** silently become `cumulative` or a full table.

**Implementation shape.**
- Add `RefreshStrategy::MaterializedView` (parses `"materialized_view"`), plus the keyed-mode constraint-violation rows it triggers (`models.md` §"Constraint violations": forbids `timeseries:` and `batched:` block).
- Rename `BackendCapabilities.supports_materialized_views → supports_native_ivm` at `crates/smelt-dialect/src/dialect.rs:67` and every default (lines ~118, 151, 179, 207); add `supports_retraction: false` beside each. Note the DuckDB/Spark defaults were `false`, and the `:207` default was `true` — set it `false` (no backend advertises native IVM today; `multi_backend.md` §IVM).
- Wire the hard error: when a model is `refresh: materialized_view` and the resolved backend's `supports_native_ivm` is `false`, emit the hard error (surface at validation/planning, per the `materialized_view.md` contract). This replaces the removed A3 storage-value branch at `smelt-backend/src/lib.rs:134`.

**Critical files.**
- `crates/smelt-dialect/src/dialect.rs` — `BackendCapabilities` (`:29-100`, flag at `:67`) + per-backend defaults (`duckdb()` `:104`, `spark_delta()` `:136`, `spark_parquet()` `:164`, and the `:207` constructor that currently sets `true` → make `false`).
- `crates/smelt-dialect/tests/capability_conformance.rs` — matrix cells (`:112-117`) + the exhaustive-destructure guard (`:183-197`, will fail to compile until `supports_retraction` is listed).
- `crates/smelt-backend-duckdb/src/lib.rs` (`:562` `capabilities`, test `:838-846`).
- `crates/smelt-core/src/config.rs` / `metadata.rs` — `RefreshStrategy::MaterializedView` + constraint violations.
- `crates/smelt-backend/src/lib.rs` — the hard-error wiring (replaces the A3-removed `:134` branch).

**Docs touched.**
- `docs/specs/multi_backend.md` §Known Divergences — remove the "`supports_native_ivm`/`supports_retraction` specified but unwired; old flag not renamed" note.
- `docs/specs/materialized_view.md` §Known Divergences — narrow the "not implemented" note to the *emit* path (A4 lands the parse + hard error; the emit against an IVM backend is D3).
- `docs-site/` — note `refresh: materialized_view` errors on engines without native IVM.

**Review checklist.**
- [ ] Capability suite asserts both flags `false` on all backends.
- [ ] `refresh: materialized_view` on DuckDB is the exact spec hard error; no silent fallback.
- [ ] `multi_backend.md` §Known-Divergence note removed; `materialized_view.md` narrowed to the emit gap.
- [ ] Spec/docs edits timeless.

**Commit.** `feat(dialect): rename supports_materialized_views→supports_native_ivm, add supports_retraction; hard-error refresh: materialized_view`

---

## Blocked phases

(none yet)

## Deferred during implementation

(Append-only.)

## Verification

- `cargo test` (workspace) green; `cargo clippy --all-targets` clean; `cargo fmt --all -- --check`.
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` — migrated example workspaces build with zero diagnostics.
- No production identifier emits the old `incremental`/storage-`materialized_view` spelling (`rg` over `crates/` excluding tests/docs).
- `/smelt:validate models`, `/smelt:validate batched_models`, `/smelt:validate multi_backend`, `/smelt:validate materialized_view` report zero drift for the surfaces this group touches; every 2026-07-04 §Known-Divergence rename/ontology note is removed as its phase lands.
