# Plan: Function call expansion in model contexts — named-arg substitution + struct-returning lowering

**Date**: 2026-05-19
**Spec**: [`docs/specs/functions.md`](../specs/functions.md) §"Known Divergences" — *"End-to-end `smelt build` execution of `smelt.<path>(...)` function calls is incomplete"*
**Spec diff**: none (closing an existing Known Divergence — the spec already mandates the surface; this plan brings the implementation up to it)
**Parent plan**: [`docs/plans/20260519-functions-meta-gaps.md`](20260519-functions-meta-gaps.md) Phase 1
**Tracking PR / branch**: `worktree-web_analytics` (this worktree). PR opens when Phase 1 is complete.
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/functions.md` — it is the correctness oracle. Pay particular attention to §"Function call syntax" (named-arg binding semantics, `param => value`) and Semantics rule 16 (declared return type is authoritative; struct-typed call sites inherit their declared fields). Also read `docs/specs/scoping.md` §"Resolution order" so you understand how parameters bind inside a body. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-web_analytics`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/web_analytics/` or `crates/smelt-cli/tests/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: phase 1 ships named-arg substitution + struct-returning lowering only. **Phase 2 (FROM-position alias synthesis) is out of scope here** — if a fixture needs an alias on a `TableExpr`-returning call, write it explicitly (`FROM smelt.functions.sessionize(...) AS s`).
- Honor architectural invariants from `CLAUDE.md` (pure-function rule for `type_inference.rs`; string-based `substitute_params` lives only in the lowering layer in `crates/smelt-cli/src/compiler.rs`).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/functions.md` and `docs-site/docs/guide/functions.md` describe the feature as if it has always existed — no `### Phase 1 — …` headings, no `(Phase 1)` inline labels.

**Subagent model rule.** Outer orchestrator runs on `opus`. Every delegated subagent — implementer, reviewer, every expert in per-phase reviewer tables — MUST be spawned with `model: "sonnet"`. Do not omit the `model` parameter.

---

## Context

The function surface is fully parsed and type-checked. `SmeltFnExpander` / `SmeltPathCallExpander` substitute positional parameters at SQL-emission time (`crates/smelt-cli/src/compiler.rs::substitute_params`), but named-argument bindings (`param => value`) are dropped — the closure signature carries `_named: Vec<(String, String)>` that is never consumed. Separately, when a `smelt.define` returns `Expr<Struct<{…}>>` (e.g. `parse_event_payload`) and is called in a SELECT-list position with `.*` projection, the body's brace-struct literal `{f: v, …}` substitutes verbatim — DuckDB then materialises a single struct column rather than three named columns.

Both blockers are documented in `examples/web_analytics/functions/{parse_event_payload,sessionize}.sql` comment blocks: the functions are declared canonically but the corresponding silver models inline equivalent SQL. This phase closes the gap so those models can call the declared functions.

## Scope

### In scope (spec coverage)

- **Named-argument substitution** (functions.md §"Function call syntax", Semantics rule 16). When a call passes `param => value`, the lowering layer binds `value` to the parameter `param`'s slot regardless of declaration order, then runs the existing identifier-substitution pass.
- **Struct-returning function lowering at SELECT-list call sites** (functions.md Semantics rule 16; scoping.md §"Resolution order"). When the call's declared return type is `Expr<Struct<{f₁, …, fₙ}>>` and the call appears as a SELECT-list item with a `.*` projection (`smelt.functions.parse_event_payload(payload).*`), the lowering produces `n` separate projected columns named `f₁`…`fₙ` whose value expressions are the corresponding fields of the substituted body's brace-struct literal.
- **`examples/web_analytics/` refactor.** `silver/events_parsed.sql` and `silver/sessions.sql` switch from inlined SQL to declared-function calls. Output schemas stay byte-identical to the snapshot test taken before the refactor.
- **Phase 57 deferred test.** `e2e_passing_clause_substitution_executes` (currently `#[ignore]`d in `crates/smelt-cli/tests/functions_e2e.rs:289`) is un-ignored and made green.
- **Spec edit.** Narrow the *"End-to-end `smelt build` execution of `smelt.<path>(...)` function calls is incomplete"* entry in `functions.md` Known Divergences to scope what remains (cross-engine Spark struct-literal codegen + literal-VALUES models — both handled in Phase 3 of the parent plan).
- **User-doc example.** `docs-site/docs/guide/functions.md` gains a worked struct-returning-function projection example.

### Explicitly deferred

- **FROM-position alias synthesis** for `TableExpr`-returning calls — Phase 2 of the parent plan. The `silver/sessions.sql` rewrite carries an explicit alias `AS s` until Phase 2 lands.
- **Spark struct-literal lowering** and **literal-VALUES models** — Phase 3 of the parent plan.
- **`.field` (single-field) projection** of a struct-returning call. Only `.*` projection lands in this phase; single-field access (`smelt.functions.parse_event_payload(payload).event_name`) waits until a real fixture demands it.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1.1   | done     | 528f3aa3 | 2026-05-19 |
| 1.2   | done     |        | 2026-05-19 |
| 1.3   | pending  |        |      |
| 1.4   | pending  |        |      |

---

### Phase 1.1: Named-argument substitution in `SmeltPathCallExpander` + `SmeltFnExpander`

**Goal.** Bind named arguments to parameters by name before the identifier-substitution pass runs, so `smelt.functions.safe_divide(numerator => revenue, denominator => cost)` lowers identically to `smelt.functions.safe_divide(revenue, cost)`.

**Pre-conditions.** None — this is the first phase.

**TDD tests to write first.** Listed verbatim — write these as failing tests before any implementation:

- `crates/smelt-cli/src/compiler.rs::tests::substitute_named_args_binds_by_param_name` — unit test on a refactored `substitute_params_with_named(...)` (or equivalent) that builds the positional vector from a mix of positional + named call args and asserts whole-word identifier replacement matches the positional-only path.
- `crates/smelt-cli/src/compiler.rs::tests::substitute_named_args_reorders_independent_of_call_order` — given a 3-parameter signature `[a, b, c]` and a call `(c => 'C', a => 'A', b => 'B')`, the substituted body has every bare `a`/`b`/`c` replaced with `'A'`/`'B'`/`'C'`.
- `crates/smelt-cli/src/compiler.rs::tests::substitute_named_args_mixes_positional_then_named` — positional-first then named (`(x, named => y)`) substitutes correctly.
- `crates/smelt-cli/src/compiler.rs::tests::substitute_named_args_unknown_name_passes_through` — an unknown `unknown_param => value` is non-fatal at the lowering layer (the type-checker has already rejected it via `UnknownPassingParameter`/`MissingArgument`); the lowering returns the body unchanged for that slot so we get a recognisable error from the downstream SQL engine rather than a silent miscompile.
- `crates/smelt-cli/tests/functions_e2e.rs::e2e_passing_clause_substitution_executes` — un-ignore the existing test; populate it with a workspace whose model calls `smelt.functions.safe_divide(numerator => revenue, denominator => cost)`; assert the materialised DuckDB rows match the positional-call snapshot.

**Implementation shape.** Refactor `substitute_params` to accept `Vec<String> positional, Vec<(String, String)> named, &[String] param_names`. Build an ordered `Vec<Option<String>>` indexed by parameter; fill positional slots first (left-to-right) then walk named args and assign each `param_name -> arg_sql` into the matching slot. Pass the resulting `Vec<String>` (with empty strings or the original parameter name for unfilled slots — to be decided in the per-phase plan review) into the existing whole-word `replace_identifier` loop. The `SmeltFnExpander` and `SmeltPathCallExpander` closures stop ignoring `_named` and feed it through.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/compiler.rs` — `substitute_params` signature, callers in `build_emitters`, optional helper `bind_named_args`.
- `crates/smelt-cli/tests/functions_e2e.rs` — un-ignore + fixture for `e2e_passing_clause_substitution_executes`.

**Docs touched (default, unless plan header is `Docs: code-only`).** None in this sub-phase — the user-doc update lands with Phase 1.3 alongside the worked example.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified.
- [ ] Spec rule "Arguments may be positional or named with `param => value`" (functions.md §"Function call syntax") is satisfied.
- [ ] No regression of the existing positional-only path: `cargo test -p smelt-cli --test functions_e2e` continues to pass (excluding the un-ignored test which becomes green here).
- [ ] No scope creep: no edits to `examples/web_analytics/` in this sub-phase.
- [ ] `substitute_params` (or its renamed successor) remains string-based; no AST-level rewrite added to this layer.

**Commit.** `fix(functions): substitute named args in SmeltFnExpander / SmeltPathCallExpander`

---

### Phase 1.2: Struct-returning function lowering — `.*` projection in SELECT lists

**Goal.** When a `smelt.<path>(...)` call's declared return type is `Expr<Struct<{f₁, …, fₙ}>>` and the call appears as a SELECT-list item with `.*` projection, lower it to `n` named projected columns whose value expressions are the corresponding fields of the substituted body's brace-struct literal.

**Pre-conditions.** Phase 1.1 done (the substituted body must already have parameters bound correctly).

**TDD tests to write first.**

- `crates/smelt-dialect/src/printer.rs::tests::struct_returning_call_dot_star_lowers_to_field_projections` — a printer-level unit test: parse `SELECT smelt.functions.parse_event_payload(payload).* FROM e`, wire a `SmeltPathCallExpander` that returns the body `{json_extract_string(payload, '$.event_name') AS event_name, json_extract_string(payload, '$.platform') AS platform, json_extract_string(payload, '$.url') AS url}`, assert printer output projects three separately-aliased columns.
- `crates/smelt-cli/tests/functions_e2e.rs::e2e_struct_returning_fn_dot_star_projection_executes` — workspace with `parse_event_payload` and a model calling `smelt.functions.parse_event_payload(payload).*`; `smelt build` produces a DuckDB table with three Text columns and the expected row values.
- `crates/smelt-cli/tests/functions_e2e.rs::e2e_struct_returning_fn_in_with_clause_passes_through` — a model that calls the same struct-returning function but **not** in a SELECT list with `.*` (e.g. assigns the whole struct to a single aliased column `AS payload_struct`) continues to emit a single struct-typed column.

**Implementation shape.** In `crates/smelt-dialect/src/printer.rs`, when printing a `SMELT_PATH_CALL` whose immediate parent in the CST is a `STAR_QUALIFIER` / `WILDCARD_AFTER_CALL` / equivalent post-`.*` shape, the printer:
1. Asks the path-call expander for the body, as today.
2. Parses the returned body string with `smelt_parser::parse` and locates a top-level `BRACE_STRUCT_LITERAL` (the body must be exactly a brace-struct literal — degraded shape is a planner-time check, out of scope for this phase).
3. Iterates the `STRUCT_FIELD_ITEM` children and emits each as a separate `<value> AS <name>` projection, comma-separated, in place of the original `call.*`.

If the call has no `.*` suffix, fall back to the current verbatim behaviour. If the body is not a brace-struct literal, fall back to verbatim and let downstream surface the error.

The CST shape for `call.*` needs verification — if today's parser does not recognise `<smelt-path-call>.*` distinct from `<smelt-path-call>`, the parser gets a minimal extension to admit a `STAR_PROJECTION` token following a `SMELT_PATH_CALL` inside a `SELECT_ITEM`. The per-phase implementer plan picks the lightest change after a quick parser audit.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-parser/src/parser.rs`, `crates/smelt-parser/src/syntax_kind.rs` — minimal extension to admit `<call>.*` after a `SMELT_PATH_CALL` if not already supported.
- `crates/smelt-dialect/src/printer.rs` — projection logic in `print_node` for the new shape.
- `crates/smelt-cli/src/compiler.rs::build_emitters` — no changes expected (the expander still returns the substituted body string).
- `crates/smelt-cli/tests/functions_e2e.rs` — new tests.

**Docs touched.**
- `docs/specs/functions.md` §"Known Divergences" — re-word the *"End-to-end execution incomplete"* bullet to scope only the cross-engine cases remaining for Phase 3 (Spark struct-literal lowering + literal-VALUES models). Phrase as a feature description, not a phase log.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified.
- [ ] `call.*` projection produces exactly `n` projected columns named after the struct fields (functions.md Semantics rule 16).
- [ ] Non-`.*` call sites are unchanged.
- [ ] Spec edit is timeless — no `Phase 1`/`Phase 3` vocabulary in `functions.md` body.
- [ ] Architectural invariants honored (no Salsa import added to printer / dialect crates).
- [ ] No scope creep into Phase 2 (no alias synthesis).

**Commit.** `feat(functions): lower struct-returning function .* projection in SELECT lists`

---

### Phase 1.3: Refactor `examples/web_analytics/silver/events_parsed.sql` to call `parse_event_payload`

**Goal.** Replace the three inlined `json_extract_string(payload, '$.…')` calls with a single `smelt.functions.parse_event_payload(payload).*` projection. Output schema and row values stay identical.

**Pre-conditions.** Phases 1.1 and 1.2 done.

**TDD tests to write first.**

- `crates/smelt-cli/tests/example_diagnostics.rs` — already exists workspace-wide. Add no test; verify it passes with the refactored model (no findings on `examples/web_analytics/`).
- `crates/smelt-cli/tests/web_analytics_refactor_snapshot.rs::events_parsed_rowset_equivalent_after_refactor` — a new file-scoped test that snapshots `silver.events_parsed`'s rows from a known datagen seed before vs. after the refactor (uses a deterministic seed under `examples/web_analytics/datagen.yaml`). Asserts row equality.

**Implementation shape.** Edit `examples/web_analytics/models/silver/events_parsed.sql` to:
```sql
SELECT
    event_id,
    device_id,
    user_id,
    CAST(event_date AS DATE) + to_seconds(seconds_in_day) AS event_ts,
    CAST(event_date AS DATE) AS event_date,
    smelt.functions.parse_event_payload(payload).*
FROM smelt.bronze.raw_events
```

Remove the outdated comment block in `examples/web_analytics/functions/parse_event_payload.sql` that says the model inlines the SQL.

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/models/silver/events_parsed.sql`
- `examples/web_analytics/functions/parse_event_payload.sql` (comment update only).
- `crates/smelt-cli/tests/web_analytics_refactor_snapshot.rs` (new test file).
- `docs-site/docs/guide/functions.md` (worked example).

**Docs touched.**
- `docs-site/docs/guide/functions.md` — add a "Projecting struct-returning function outputs" section showing the `.*` form, written as if always supported.

**Review checklist** (material findings only):
- [ ] TDD test asserts row equivalence under the deterministic datagen seed.
- [ ] `cargo test -p smelt-cli --test example_diagnostics` reports zero findings on `examples/web_analytics/`.
- [ ] User-doc edit is timeless — no plan vocabulary.
- [ ] No scope creep into `sessions.sql` (that's Phase 1.4).

**Commit.** `refactor(web_analytics): use parse_event_payload struct fn in events_parsed`

---

### Phase 1.4: Refactor `examples/web_analytics/silver/sessions.sql` to call `sessionize`

**Goal.** Replace the three-CTE inlined window-function pipeline in `silver/sessions.sql` with a `smelt.functions.sessionize(...)` named-argument call. Output schema and row values stay identical (within the bounds of the deterministic datagen seed).

**Pre-conditions.** Phase 1.1 done (named-arg substitution). Phase 2 (FROM-position alias synthesis) is **explicitly not** a pre-condition — this phase uses an explicit `AS s` alias at the call site.

**TDD tests to write first.**

- `crates/smelt-cli/tests/web_analytics_refactor_snapshot.rs::sessions_rowset_equivalent_after_refactor` — snapshot `silver.sessions`'s rows before vs. after the refactor under a deterministic seed; assert row-set equality.
- `crates/smelt-cli/tests/example_diagnostics.rs` — passes with zero findings.

**Implementation shape.** `sessions.sql` is rewritten to roughly:
```sql
---
materialization: table
incremental:
  enabled: true
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
---
WITH sessionized AS (
    SELECT *
    FROM smelt.functions.sessionize(
        source => smelt.silver.events_parsed,
        partition_col => device_id,
        ts_col => event_ts,
        platform_col => platform
    ) AS s
),
with_start_date AS (
    SELECT
        device_id, event_ts, event_date, platform, session_seq,
        CAST(FIRST_VALUE(event_ts) OVER (PARTITION BY device_id, session_seq ORDER BY event_ts) AS DATE) AS session_start_date
    FROM sessionized
)
SELECT
    CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_seq AS VARCHAR), '-', CAST(MIN(event_ts) AS VARCHAR)) AS session_id,
    device_id,
    session_seq,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    session_start_date,
    COUNT(*) AS event_count,
    ANY_VALUE(platform) AS platform
FROM with_start_date
GROUP BY device_id, session_seq, session_start_date
```

Note: the existing model uses microsecond arithmetic to avoid a type mismatch between `BIGINT` and `INTERVAL` (see the comment block at lines 26–32 of the current file). The `sessionize` function declared in `functions/sessionize.sql` uses `INTERVAL '30 minutes'` and timestamp subtraction directly. Per-phase implementer plan must confirm that DuckDB accepts the function's typing under the actual `event_ts` source type (`Timestamp` after the Phase 1.3 refactor) — if not, the function signature gets a default-value tweak (still in scope as the function is being made canonical here) or this phase's per-phase plan picks the right `gap` argument to supply explicitly.

Remove the outdated comment block in `examples/web_analytics/functions/sessionize.sql` that says the model inlines the SQL.

**Critical files (allowed to touch in this phase).**
- `examples/web_analytics/models/silver/sessions.sql`
- `examples/web_analytics/functions/sessionize.sql` (comment update + possibly the `gap` default).
- `crates/smelt-cli/tests/web_analytics_refactor_snapshot.rs` (extends Phase 1.3's snapshot file).
- `docs-site/docs/guide/functions.md` (named-argument worked example).

**Docs touched.**
- `docs-site/docs/guide/functions.md` — add a worked named-arg call example, written as if always supported.

**Review checklist** (material findings only):
- [ ] Snapshot test asserts row equivalence under the deterministic datagen seed.
- [ ] Diagnostics gate passes.
- [ ] User-doc edit is timeless — no plan vocabulary.
- [ ] The call site carries an **explicit** alias `AS s` (Phase 2 will remove the need for the alias; this phase does not synthesise one).
- [ ] No scope creep into Phase 2.

**Commit.** `refactor(web_analytics): use sessionize fn with named args in silver/sessions`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets`
- `cargo test --quiet 2>&1 | tail -40`
- `cargo test -p smelt-cli --test example_diagnostics`
- `cargo test -p smelt-cli --test functions_e2e e2e_passing_clause_substitution_executes` — passes (was `#[ignore]`d before this phase).
- `cargo test -p smelt-cli --test web_analytics_refactor_snapshot` — passes; snapshots show identical row sets to the pre-refactor baseline.
- `/smelt:validate functions` — the *"End-to-end execution incomplete"* entry in Known Divergences is narrowed to "cross-engine Spark struct-literal codegen + literal-VALUES models" only (those land in Phase 3 of the parent plan).
