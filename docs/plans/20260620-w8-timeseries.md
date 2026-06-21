# Plan: W8 — Timeseries partition/pruning invariants (D-52)

**Parent (master plan)**: `docs/plans/20260613-spec-impl.md` — the **W8** wave of the spec-remediation backlog. Remediates **D-52** from `docs/research/20260613-spec-remediation-decisions.md` (§"D-52 · [Maj] `timeseries.md` partition/event-time nullability + granularity-vs-partition-type"): two fail-loud validation invariants that prevent silent correctness holes in incremental execution. The spec is already committed; this wave is **code-only**. The autonomy loop works this sub-plan phase by phase and rolls up to the master only when it is exhausted.

**Date**: 2026-06-20
**Spec**: `docs/specs/timeseries.md` §"Validation rules" (rules 7 and 8), §"Constraints & Invariants" (invariants 7 and 8), §"Diagnostic codes" — these are the correctness oracle.
**Spec diff**: none — the two invariants already appear in the spec text as of 2026-06-13. This wave lands the code to match.
**Tracking branch**: `worktree-spec_review`
**Docs**: code-only. P3 (close-out) retracts the "Output-schema-dependent validation rules" Known-Divergence note in `timeseries.md` for the two rules that land here (rules 7 and 8), since those are no longer divergences. The R2-coordinated incremental-execution halves remain as Known Divergences.

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then the spec sections above — they are the correctness oracle; do not re-open the settled decision (D-52 resolved to option A). Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked` rows) using the per-phase routine below (pre-flight → red-green `/smelt:implement` with implementer + reviewer, spec as oracle → verification gates → set the row to `done` + date → commit + push with the phase's commit message). If that was the last `pending` phase, also flip this sub-plan's Status to `done (<today>)` in the master registry row for W8 and commit together. Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>` (record + continue), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>`, or `<<ALL_DONE>>`. A block is recorded and the loop continues — there is no hard-stop.

## Design decisions (resolved — do not re-litigate)

| Dec | One-line contract (the spec is authoritative) |
|-----|-----------------------------------------------|
| **D-52 rule 7** | `partition_column` (and `event_time_column` when it drives pruning) must be **NOT NULL** on the model's output or source's column list → else `MalformedTimeseries`. A NULL partition value silently escapes `>= start AND < end` pruning and is never re-inserted — a correctness hole. |
| **D-52 rule 8** | Sub-day `granularity` (`hour`) requires a timestamp-resolution `partition_column` type (timestamp or timestamptz). Pairing `granularity: hour` with a plain `DATE` partition silently coarsens pruning to whole days → `MalformedTimeseries`. |
| **R2 coordination boundary** | Both rules govern the *declaration* of a timeseries block and fire as static diagnostics during `check_file_diagnostics`. The corresponding incremental-execution changes (how DELETE+INSERT respects the pruning window when partition_column is confirmed non-null, and how sub-day window arithmetic works) belong to the R2 incremental-cadence rewrite, which is **out of scope** for this backlog. Do not touch `smelt-runtime` incremental execution paths in this plan. |
| **Implementation layer** | Rules 7 and 8 require the model's output schema (column types and nullability). The pure `validate_timeseries` function in `smelt-core/src/metadata.rs` operates only on `ModelMetadata` + raw SQL text and cannot see type information. These checks must live in `smelt-db` (the Salsa layer), not in `validate_timeseries`, because `typed_model_schema` / `resolved_model_schema` are Salsa queries. The check function should be a **pure helper** (per the Salsa-purity rule) that accepts the resolved schema and `TimeseriesConfig` and returns `Vec<Diagnostic>` — the Salsa query calls it. |
| **Source-side checks** | For sources, `nullable: false` on the column entry in the YAML `columns:` list is the NOT-NULL signal. Source nullability is already parsed into `SourceInfo`; the NOT-NULL check for sources reads from `SourceInfo.columns[*].nullable`. Type constraint (rule 8) for sources reads from the column's declared type. |

## Per-phase routine

1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. If red on this phase's own acceptance target, proceed. If red on **unrelated** breakage, treat as a block (record + continue).
2. **Red-green `/smelt:implement`.** Write the phase's failing test(s) first, then the implementation, spec as oracle. Implementer pass, then reviewer pass (material findings only).
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test --quiet 2>&1 | tail -40` (green); the dual example gate `cargo test -p smelt-cli --test example_diagnostics` + `cargo test -p smelt-lsp --test example_workspaces`.
4. **Record + commit.** Set the table row to `done` + date; commit + push tests + impl + table together with the phase's commit message. Emit `<<PHASE_COMPLETE>>` (or roll-up on the last phase).

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue, no hard-stop)

Set the row to `blocked` with a one-line reason; append a dated entry to §"Blocked phases"; restore the tree to a clean committed state; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:

- The check requires data only available inside the R2 incremental-execution internals (e.g. the effective filter_range at runtime). Record the gap under §Deferred and move on — the static declaration-time check is what this plan delivers.
- `typed_model_schema` returns all-Unknown types for the model under test (type inference not yet available for that shape). Narrow the test to a model whose type can be inferred; note the limitation as a Known Divergence.
- The spec is genuinely ambiguous for a real case the phase hits. Record the question; do not guess.
- Pre-flight red on unrelated breakage this phase didn't introduce.

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| P1 | NOT-NULL invariant: nullable `partition_column` or pruning `event_time_column` → `MalformedTimeseries` | done | D-52 rule 7 | feat(db): reject nullable timeseries partition/pruning columns (D-52 rule 7) | 2026-06-22 |
| P2 | Granularity-vs-partition-type: sub-day `granularity` with `DATE` partition → `MalformedTimeseries` | done | D-52 rule 8 | feat(db): reject sub-day granularity with date-resolution partition column (D-52 rule 8) | 2026-06-22 |
| P3 | Close-out: retract KD note for rules 7+8, registry row, ROADMAP | pending | D-52 close-out | docs(spec-impl): close out W8 — timeseries partition invariants; retract KD (D-52) | |

---

### Phase P1: NOT-NULL invariant for partition/pruning columns (D-52 rule 7)

**Goal.** When a model's `timeseries:` block names a `partition_column`, that column must be NOT NULL in the model's inferred output schema. When `event_time_column` is distinct from `partition_column` (i.e., it drives pruning independently), it must also be NOT NULL. A nullable partition or pruning column silently escapes the `>= start AND < end` pruning window and is never deleted or re-inserted — a correctness hole for incremental execution. Violation → `MalformedTimeseries`. (Spec `timeseries.md` §"Validation rules" rule 7; Constraint 7.)

For external sources, the check reads `nullable: false` from the source's YAML `columns:` list. For models, it reads nullability from `typed_model_schema`.

**Implementation shape.** Add a pure function `check_timeseries_nullability(ts: &TimeseriesConfig, schema: &ModelSchema) -> Vec<MalformedTimeseries-style messages>` in `smelt-db/src/queries/check_types.rs` (or a new `check_timeseries.rs` sub-module under `queries/`). Wire it into `check_file_diagnostics` in `smelt-db/src/lib.rs` after `typed_model_schema` is already available — following the existing `check_type_diagnostics` call pattern. Add a parallel path for sources that reads `SourceInfo.columns`.

**Critical files.**
- `crates/smelt-db/src/lib.rs` — `check_file_diagnostics` (where the new call is wired in)
- `crates/smelt-db/src/queries/check_types.rs` (or new `queries/check_timeseries.rs`) — the pure check function
- `crates/smelt-core/src/config.rs` — `TimeseriesConfig` struct (read-only)
- `crates/smelt-types/src/lib.rs` — `TypedColumn.nullable` (the nullability field)

**One test idea.** In `crates/smelt-db/tests/model_frontmatter_diagnostics.rs`: construct a model whose SELECT projects `partition_date` as `COALESCE(CAST(ts AS DATE), DATE '2020-01-01')` (nullable output) with `partition_column: partition_date` in the timeseries block — the COALESCE result is nullable because of the outer expression. Assert `MalformedTimeseries` fires. Also add a negative test: `DATE_TRUNC('day', ts)` is nullable (SQL NULL propagation) — assert it fires too. Add a positive test where the column is provably NOT NULL (literal or CASE with exhaustive ELSE). Also add an `examples/timeseries_broken_nullable_partition/` fixture for the `example_diagnostics` e2e gate.

**Commit.** `feat(db): reject nullable timeseries partition/pruning columns (D-52 rule 7)`

---

### Phase P2: Sub-day granularity requires timestamp-resolution partition type (D-52 rule 8)

**Goal.** When `granularity` is `hour` (a sub-day value), `partition_column` must be a timestamp-resolution type (timestamp or timestamptz). Pairing `granularity: hour` with a `DATE` partition type silently coarsens pruning to whole days — `DATE` cannot represent hour boundaries. Violation → `MalformedTimeseries`. (Spec `timeseries.md` §"Validation rules" rule 8; Constraint 8.)

`granularity: hour` is currently the only sub-day granularity in the closed enum (`Granularity::Hour`). The check is: if `ts.granularity == Granularity::Hour`, then `partition_column`'s inferred type must be `DataType::Timestamp { .. }` (not `DataType::Date`). Unknown types are skipped (avoid false positives when inference is unavailable).

**Implementation shape.** Add a pure function `check_timeseries_granularity_type(ts: &TimeseriesConfig, schema: &ModelSchema) -> Vec<message>` alongside the P1 function, or extend the same helper with a second pass. Wire into the same `check_file_diagnostics` call site. For sources: read the declared column type from `SourceInfo.columns`.

**Critical files.**
- Same as P1: `crates/smelt-db/src/lib.rs`, the check helper, `crates/smelt-core/src/config.rs` (`Granularity::Hour`), `crates/smelt-types/src/lib.rs` (`DataType::Date` vs `DataType::Timestamp { .. }`)

**One test idea.** In `crates/smelt-db/tests/model_frontmatter_diagnostics.rs`: construct a model with `granularity: hour` and a `SELECT DATE_TRUNC('day', ts) AS partition_date` (DATE output) for `partition_column: partition_date` — assert `MalformedTimeseries`. Negative test: same model with `CAST(ts AS TIMESTAMP)` partition — assert no diagnostic. Also add `examples/timeseries_broken_hour_date_partition/` fixture for the e2e gate.

**Commit.** `feat(db): reject sub-day granularity with date-resolution partition column (D-52 rule 8)`

---

### Phase P3: Close-out (KD retraction, registry, ROADMAP)

**Goal.** Retract the now-satisfied portion of the "Output-schema-dependent validation rules" Known-Divergence note in `docs/specs/timeseries.md`. Specifically: rules 7 and 8 are no longer divergences — remove them from the KD list entry (rules 2, 3, 4 remain deferred alongside the R2 migration). Flip this sub-plan's registry row in `docs/plans/20260613-spec-impl.md` to `done (<today>)`. Add a `docs/ROADMAP.md` line marking W8 complete.

**Critical files.**
- `docs/specs/timeseries.md` — KD retraction (rules 7 and 8 removed from the "Output-schema-dependent" item)
- `docs/plans/20260613-spec-impl.md` — W8 registry row status
- `docs/ROADMAP.md` — completion line

**Commit.** `docs(spec-impl): close out W8 — timeseries partition invariants; retract KD (D-52)`

---

## Deferred

- **Incremental-execution enforcement (R2 boundary).** D-52 is tagged `↔R2`. The static diagnostic checks that land here (rules 7 and 8) are declaration-time validation — they fire when a user misconfigures a timeseries block. The corresponding runtime enforcement — ensuring the DELETE+INSERT incremental strategy reads the confirmed-non-null partition column in the filter predicate, and that sub-day window arithmetic uses the correct timestamp boundary arithmetic — belongs to the R2 incremental-cadence rewrite (`docs/research/20260521-incremental-as-planner-rule.md`). That rewrite restructures `smelt-runtime` incremental execution paths and is explicitly excluded from this backlog's scope. Do not touch `crates/smelt-runtime/` in this plan.
- **Migration from nested `incremental:` form.** The timeseries block and associated validation are still in a migration state (see timeseries.md §Known Divergences first bullet). Rules 7 and 8 implemented here apply to the new-form `timeseries:` block. The legacy nested form migration is out of scope.
- **Validation rules 2, 3, 4** (`event_time_column` projection and type constraints, `partition_column` type constraint beyond rule 8). These also require output-schema data. They remain in the "Output-schema-dependent" KD entry post-W8 and are tracked in `docs/plans/20260521-incremental-timeseries-and-derived-bounds.md`.
- **Source-side NOT-NULL check if SourceInfo does not carry per-column nullability.** If `SourceInfo.columns` does not yet surface the `nullable: bool` from the YAML `columns:` list, narrow P1 to model-output-only in that phase and note the source-side gap as a follow-up rather than blocking.

## Blocked phases

Append-only log. None yet.

## Verification

- `cargo test --quiet 2>&1 | tail -40` green.
- `cargo test -p smelt-db --test model_frontmatter_diagnostics` green (the new `MalformedTimeseries` cases for rules 7 and 8).
- `cargo test -p smelt-cli --test example_diagnostics` green (new broken-timeseries fixture workspaces included).
- `cargo test -p smelt-lsp --test example_workspaces` green.
- `cargo clippy --all-targets` zero warnings.
- Manual smoke: a model with `granularity: hour` and a DATE partition column produces a `MalformedTimeseries` diagnostic in the LSP; a model with a nullable partition column produces the same; both are surfaced by `smelt build`.
- `/smelt:validate timeseries` reports no behavioural drift on rules 7 and 8.
