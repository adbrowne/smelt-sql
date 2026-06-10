# Plan: Source `timeseries:` parsing + filter pushdown in the incremental path (BUG-072/073)

**Date**: 2026-06-10
**Spec**: [`docs/specs/sources.md`](../specs/sources.md) §"Source YAML shape" / §"Source with `timeseries:` declaration", [`docs/specs/incremental_models.md`](../specs/incremental_models.md) §Semantics "Source-filter pushdown"
**Spec diff**: none — implements existing normative spec (ledger BUG-072 + BUG-073 in `docs/bug-hunt/2026-05-30-findings.md`)
**Tracking PR / branch**: `worktree-test_features`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec sections named in the header — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-test_features`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/` or a hermetic TempDir workspace.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` — **Fail-loud discipline** (unknown keys must not be silently dropped) and **Run Pipeline Parity** (pushdown lands in `smelt-runtime`, shared by CLI and UI) are load-bearing here.
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*.

---

## Context

A `timeseries:` block on a per-entity source YAML is silently dropped: `RawSourceYaml`/`SourceInfo` (`crates/smelt-core/src/sources.rs`) have no `timeseries` field and no unknown-key handling, so serde discards the key with no diagnostic (BUG-072 — a fail-loud violation). Independently, the incremental execute loop (`crates/smelt-runtime/src/execute.rs`, batch loop near line 795) calls only `inject_time_filter`; `inject_source_filters` (`transformer.rs`) is consumed only by `cumulative.rs`, so even a parsed source declaration would never produce the pushdown `incremental_models.md` §"Source-filter pushdown" specifies (BUG-073). The `source_timeseries` map handed to the planner (`execute.rs:492-510`) is built from model frontmatter blocks only. Correctness is unaffected (the outer partition-column WHERE clamps output) — pushdown constrains the *source read*.

## Scope

### In scope (spec coverage)
- `sources.md` §"Source YAML shape": `timeseries:` key parsed into `SourceInfo`; unrecognised source-YAML keys are loud, per the unknown-key doctrine (`architecture.md` §Constraints #8 as applied to sources — match the severity convention the doctrine specifies for user-authored YAML).
- `sources.md` §"Source with `timeseries:` declaration": a declaring source becomes a pushdown target for downstream incremental models.
- `incremental_models.md` §"Source-filter pushdown": `WHERE c >= run_start - before AND c < run_end + after` injected per `Bounded(c, before, after)` source reference in the incremental execute path.

### Explicitly deferred
- Pushdown for non-incremental (plain table/view) builds — the spec scopes pushdown to incremental execution; cumulative already has it.
- LSP surfacing of source `timeseries:` (hover/goto) beyond what falls out of parsing.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 99278422 | 2026-06-11 |
| 2     | pending  |        |      |
| 3     | pending  |        |      |

### Phase 1: Parse `timeseries:` on source YAML, loudly

**Goal.** `SourceInfo` carries `timeseries: Option<TimeseriesConfig>` parsed from the per-entity YAML, and an unrecognised key in a source YAML produces a diagnostic instead of silent serde drop.

**Pre-conditions.** Tree green.

**TDD tests to write first.**
- `crates/smelt-core/src/sources.rs::tests::source_yaml_timeseries_block_parses` — a source YAML with `timeseries: {partition_column, granularity}` yields `SourceInfo.timeseries == Some(...)`.
- `crates/smelt-core/src/sources.rs::tests::source_yaml_unknown_key_is_loud` — a source YAML with `timseries:` (typo) surfaces an error/diagnostic naming the key (red today: silently accepted).
- `crates/smelt-cli/tests/example_diagnostics.rs` stays green; if a fixture example gains a `timeseries:`-declaring source (`examples/` real-fixture rule), it is diagnostic-clean.

**Implementation shape.** Add `timeseries: Option<TimeseriesConfig>` to `RawSourceYaml` + `SourceInfo` (`smelt-core/src/sources.rs`), reusing `smelt_core::config::TimeseriesConfig` (already `deny_unknown_fields`). Unknown-key handling per the doctrine: follow the mechanism model sidecars use (`MalformedSource` is the owning code per `sources.md` §Diagnostic codes) — route through `discover_source_infos`'s error path so both CLI and LSP surfaces see it.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/sources.rs` — struct fields + parse + unknown-key handling
- `crates/smelt-db/src/...` — only if the diagnostic wiring requires a mapping entry
- `examples/...` — one fixture source gains a `timeseries:` block (pick the workspace Phase 3's e2e test will reuse)

**Docs touched.**
- `docs-site/docs/guide/sources.md` — document the `timeseries:` key on sources (timeless feature description)

**Review checklist** (material findings only):
- [ ] TDD tests exist and assert what's specified
- [ ] Unknown-key handling matches the fail-loud doctrine and the severity the spec prescribes
- [ ] No scope creep into the execute path (Phases 2–3)
- [ ] Spec + docs-site edits are timeless

**Commit.** `feat(sources): parse timeseries declarations on source YAML, reject unknown keys loudly (BUG-072)`

### Phase 2: Source declarations feed the planner's source-timeseries map

**Goal.** The `source_timeseries` map built in `execute.rs` (lines ~492-510) merges `SourceInfo.timeseries` declarations with the existing model-frontmatter entries, so a declaring source is a pushdown candidate for bound derivation.

**Pre-conditions.** Phase 1 done.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/` (or the existing execute-path test home — follow `execute_parity.rs` conventions): a project whose incremental model reads a `timeseries:`-declaring source shows that source in the derived bounds / plan output (e.g. via the explain/`--show-plan` surface or a unit on the map-building function extracted for testability). Red: today the map only contains model-frontmatter entries.
- Precedence case: a source declared in *both* a model `timeseries:` frontmatter context and source YAML — assert the documented precedence (source YAML is authoritative for its own partition column; if the spec is silent, pause per the execution prompt rather than guessing).

**Implementation shape.** Extract the map-building block into a pure helper (`fn build_source_timeseries_map(graph, source_infos) -> SourceTimeseriesMap`) in `smelt-runtime`, merge `SourceInfo.timeseries`, and call it from `execute_project`. Source infos are already discovered on the execute path (verify; if not, thread them through `ExecuteRequest`/setup the same way seeds are).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/execute.rs` — map construction
- `crates/smelt-runtime/src/...` — new helper home if extracted

**Docs touched.**
- none beyond Phase 1's (internal plumbing) — note in PR description instead

**Review checklist** (material findings only):
- [ ] Helper is pure and unit-tested; `execute_project` calls it (Run Pipeline Parity — both consumers inherit)
- [ ] Precedence behaviour is spec-anchored, not invented
- [ ] No scope creep into Phase 3 (no `inject_source_filters` call yet)

**Commit.** `feat(runtime): merge source YAML timeseries declarations into the planner source map (BUG-072)`

### Phase 3: Wire `inject_source_filters` into the incremental execute path

**Goal.** Each incremental batch execution pushes per-source `WHERE c >= run_start - before AND c < run_end + after` filters onto declaring source reads, per `incremental_models.md` §"Source-filter pushdown".

**Pre-conditions.** Phases 1–2 done.

**TDD tests to write first.**
- `crates/smelt-cli/tests/source_pushdown_e2e.rs::incremental_run_pushes_source_filter` — hermetic DuckDB workspace: incremental model reading a `timeseries:`-declaring source; run a window and assert the *compiled* SQL for the batch carries the source filter (surface it via the dry-run/verbose plan output or a runtime test against `inject_source_filters`' composed output), and the run's results are byte-identical to the pre-pushdown run (per-partition equivalence — pushdown is an optimisation, not a semantics change).
- `crates/smelt-runtime` unit: batch loop composes `inject_time_filter` + `inject_source_filters` in the documented order with the write window vs run window distinction respected (`incremental_models.md` §Semantics 1–3 — DELETE covers write window; source filters derive from run window + per-source bounds).
- `cargo test -p smelt-cli --test example_diagnostics` + incremental example suites stay green.

**Implementation shape.** In the incremental batch loop (`execute.rs` ~line 795), after `inject_time_filter`, derive the per-source bound map (the planner's bound-derivation output already feeds batch classification — reuse it, do not re-derive) and call `transformer::inject_source_filters` before compile. Mirror `cumulative.rs:130`'s usage.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/execute.rs` — batch loop
- `crates/smelt-runtime/src/transformer.rs` — only if the signature needs the bound-map shape adjusted

**Docs touched.**
- `docs/specs/incremental_models.md` — if §Known Divergences carries a "pushdown not wired" note after Phase 1–2 land, retire it; otherwise confirm §Semantics already matches (no edit)
- `docs-site/docs/guide/incremental-models.md` — mention source-read narrowing for declaring sources (timeless)

**Review checklist** (material findings only):
- [ ] Equivalence assertion present (pushdown changes reads, not results)
- [ ] Bound derivation reused from the planner, not re-implemented (Salsa purity / single-source rules)
- [ ] Run Pipeline Parity — change lands in `smelt-runtime`, no CLI-side fork
- [ ] Spec + docs-site edits are timeless

**Commit.** `feat(runtime): push source timeseries filters into incremental batch reads (BUG-073)`

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `cargo test -p smelt-cli --test source_pushdown_e2e`
- `cargo test -p smelt-core` (source YAML parsing + unknown-key)
- `cargo test -p smelt-runtime --test execute_parity`
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces`
- `/smelt:validate sources` and `/smelt:validate incremental_models` report no drift on the pushdown sections
