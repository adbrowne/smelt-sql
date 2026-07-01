# Plan: Event-time monotonicity trace primitive

**Date:** 2026-07-01
**Spec:** `docs/specs/incremental_models.md` — Semantics §"Event-time monotonicity trace", Design ("Event-time monotonicity is one primitive, inferred where possible, declared where necessary"), Constraints §12, and the associated Known Divergences / Open Questions entries.
**Spec diff:** Adds the normative `EventTimeTrace` primitive — the interval-preimage-is-an-interval / monotone-non-decreasing property, the static monotone whitelist + non-monotone blacklist, the `Traceable`/`StaticSeed`/`NotTraceable` verdict, the one-directional soundness contract (Constraint 12), and the declared escape hatch ("a declaration may only widen eligibility"). Marks the primitive as specified-but-not-yet-emitted under Known Divergences.
**Docs:** code + spec (the spec change above lands first, per spec-first). No user-facing docs-site change in Phase 1–2: the primitive is internal analysis with no user surface until a consumer ships; the first consumer plan carries the docs-site update.
**Design source:** `docs/research/20260701-expanding-incremental-eligibility.md` Part 6 (derivation), Part 7 (external validation — ClickHouse `getMonotonicityForRange`, Iceberg `preserves_order`, Delta generated columns, and the CALM / Rice / Richardson decidability bounds).

## Motivation

Three of the relaxations the eligibility audit works through — `UNION`-branch
partitionability (research Part 2 / §2.5), subquery/CTE pushdown conservatism
(Part 3 / §4.6), and join driving-fact resolution (Part 5 / §5.4) — all block on
the **same** missing analysis: does the model's projected `event_time` trace back,
monotonically, to a real source partition column, and if so to which column, on
which source, under what constant offset? Today no such analysis exists in
`smelt-logical`; the closest thing is the A5 substring test
(`stripped_sql.contains(event_time_column)`, `crates/smelt-logical/src/rules/incremental.rs`),
which proves nothing about monotonicity.

This plan lands the primitive **as a pure, independently-testable analysis with no
consumer wired** — the natural first phase (research §6.5). Shipping it alone keeps
the three future relaxations honest with each other (one analysis, not three
divergent private copies — the syntax-vs-semantics inconsistency research §3.3
exposed), and it can be red-green unit-tested against the §6.2 whitelist/blacklist
and property-tested against the existing DuckDB harness hazards (P3, Q5, J3–J5)
*before* any injection or runtime code changes. Later phases wire the three
consumers; their center of gravity is elsewhere and each is separately gated.

The primitive must respect two architectural invariants (`CLAUDE.md`): **Salsa
purity** (it is a pure function over parser AST + declared context; any `smelt-db`
query is a thin wrapper) and **Layered single-ownership** (analysis lives in
`smelt-logical`, above `smelt-parser`, below `smelt-db`/`smelt-planner`).

---

## Phase 0 — `analyze_select` retains the parsed `Expr` node (prerequisite)

**Files:** `crates/smelt-logical/src/analysis/mod.rs`.

**Why:** `SelectAnalysis` currently keeps each select item as raw `text`
(`SelectItemKind::{GroupByKey,…}.text`) and discards the `Expr` node. The
monotonicity classifier walks the expression tree, so it needs the node (or must
re-parse). Research §6.7 flags this as the plumbing choice; retaining the node is
the "one change, many future analyses benefit" option and is preferred.

**Change:**
- Extend the `SelectItemKind` variants (or add a sibling field) to carry the
  parsed `smelt_parser::Expr` for the select item alongside the existing `text`,
  without removing `text` (existing consumers keep working). Keep the change
  additive and behaviour-preserving for every current caller of `analyze_select`.
- If retaining the node proves invasive (lifetime/ownership friction with the
  Rowan tree), fall back to the documented alternative: the primitive re-parses
  the event-time expression text in isolation. Record which path was taken in the
  spec's `analyze_select` Open Question.

**Tests (red-green, `crates/smelt-logical`):**
- `analyze_select_retains_expr_for_group_key`: a `SELECT DATE_TRUNC('day', ts) AS d, …`
  yields an item whose retained `Expr` re-serialises to the same text.
- Existing `analyze_select` tests still pass unchanged (no regression to `text`).

**Implementer checklist:**
- [ ] Change is additive — no existing field removed, no current caller broken.
- [ ] No new production `unwrap`/`expect` (hardening ratchet); parse failures degrade gracefully, matching `analyze_select`'s existing `Option` return.
- [ ] `cargo fmt --all`; `cargo clippy --all-targets` clean.

**Reviewer checklist:**
- [ ] The retained node is the *item* expression, not the whole SELECT — no accidental widening of what's kept.
- [ ] Confirm the fallback (re-parse) decision is recorded in the spec Open Question if taken.

**Commit:** `refactor(smelt-logical): retain parsed Expr on SelectAnalysis items`

---

## Phase 1 — The pure monotonicity module (`trace_event_time`)

**Files:** new `crates/smelt-logical/src/analysis/monotonicity.rs`; register in
`crates/smelt-logical/src/analysis/mod.rs`.

**Change:**
- Add the verdict types and the single entry point (illustrative shape from
  research §6.5):
  ```rust
  pub enum Offset { Seconds(Seconds), Symbolic(String) } // months/years
  pub enum EventTimeTrace {
      Traceable  { source: String, source_column: String, offset: Offset },
      StaticSeed  { reason: String },
      NotTraceable { reason: String },
  }
  pub fn trace_event_time(
      event_time_expr: &smelt_parser::Expr,
      ctx: &crate::analysis::source_bounds::BoundContext,
  ) -> EventTimeTrace;
  ```
- Implement the classifier by walking `event_time_expr` from the projected column
  toward the leaves, per the spec whitelist/blacklist:
  - **Whitelist → recurse, compose offsets:** bare/qualified column (identity),
    `DATE_TRUNC`, `CAST(col AS DATE/TIMESTAMP)`, `date_bin`/`time_bucket`/`FLOOR`,
    `col ± INTERVAL '<const>'` (fold into `Offset`), `col AT TIME ZONE '<const>'`.
    Composition recurses on the *single* column-bearing argument at each layer.
  - **`StaticSeed`:** constant / `NULL` literal, `COALESCE(col, <const>)`.
  - **`NotTraceable` (fail closed):** two column-bearing arguments, `MOD`/`EXTRACT`,
    `CASE`, `GREATEST`/`LEAST(col,const)`, unknown UDF, `NOW`/`CURRENT_DATE`,
    `CAST(col AS VARCHAR)`, any unrecognised head.
  - Resolve the traced leaf column against `ctx.source_partition_cols`; return
    `Traceable{ source, source_column, offset }` where `source_column` is the
    matched `BoundContext` partition column (so the result is directly consumable
    by the existing `BoundResult::Bounded.source_partition_col` machinery).
  - `CAST` is whitelisted **only** for date/timestamp target types.
- Uses `Expr` accessors already exposed by `smelt-parser` (`as_column_ref`,
  `as_function_call`, `as_cast`, `as_binary`, `FunctionCall::name`/`arguments`,
  `CastExpr::expression` + target type). No new crate dependency.

**Tests (red-green, `crates/smelt-logical`):**
- **Whitelist unit tests** (`trace_*_traceable`): each whitelist row of the spec
  returns `Traceable` tracing to the expected column with the expected offset,
  including a 3-layer composition `DATE_TRUNC('day', CAST(event_ts AS TIMESTAMP) + INTERVAL '2 hours')`
  → `Traceable{ source_column: event_ts, offset: +2h }`.
- **Blacklist unit tests** (`trace_*_not_traceable` / `*_static_seed`): each
  blacklist row returns the specified verdict; `COALESCE(col,'1970-01-01')` and a
  bare `NULL`/constant → `StaticSeed`; `EXTRACT(HOUR FROM ts)`, `end_ts - start_ts`,
  `CASE …`, `my_udf(ts)`, `NOW()`, `CAST(ts AS VARCHAR)` → `NotTraceable`.
- **Fail-closed test** (`unknown_head_fails_closed`): a synthetic unrecognised
  function head returns `NotTraceable`, never `Traceable`.
- **Multi-source test** (`two_column_arithmetic_not_traceable`): an expression
  mixing two sources' columns → `NotTraceable` (the join multi-clock case, J4).

**Implementer checklist:**
- [ ] Module is pure — no Salsa, no I/O, no `smelt-db`/`smelt-planner` dependency (Layered single-ownership; verify `cargo tree -p smelt-db -i smelt-planner` unaffected — this crate is below both).
- [ ] Every unrecognised branch returns `NotTraceable` with a `reason` — never a silent `Traceable` (Constraint 12; fail-loud discipline).
- [ ] No new production `unwrap`/`expect` beyond the hardening baseline; classify any as infallible or convert to a verdict.
- [ ] `cargo fmt --all`; `cargo clippy --all-targets` clean.

**Reviewer checklist:**
- [ ] The soundness direction holds: read each whitelist arm and confirm it is monotone non-decreasing on *all* target backends (the intersection rule), not just DuckDB.
- [ ] `CAST` arm rejects non-temporal targets; `col AT TIME ZONE` DST plateau reasoning is sound (never decreasing).
- [ ] Offset composition folds constants correctly; month/year stays `Symbolic`, not silently coerced to `Seconds`.
- [ ] Verdict `reason` strings name the offending sub-expression (usable in a future diagnostic).

**Commit:** `feat(smelt-logical): add pure event-time monotonicity trace primitive`

---

## Phase 2 — Property-test the primitive against the DuckDB harness

**Files:** new test module under `crates/smelt-logical/tests/` (or a `#[cfg(test)]`
property section), reusing the existing harness SQL at
`docs/research/harness/20260701-{union,subquery,join}_incremental.sql`.

**Why:** The harness already reproduces exactly the hazards the primitive must
catch — P3 (`UNION` NULL/constant seed), Q5 (non-commuting subquery body), J3–J5
(timeseries-dim-as-lookup, multi-clock fact join, fan-out). Wiring them as the
oracle proves the primitive's verdict lines up with the empirically-measured
incremental ≡ full boundary, not just with hand-written expectations.

**Change:**
- For each harness scenario, assert the primitive's verdict on the scenario's
  event-time projection matches the measured outcome: the safe rows (P1/P2, Q1–Q4,
  J1/J2 → 0 violations) trace to `Traceable`; the hazard rows (P3, Q5a/Q5b,
  J3/J4/J5 → non-zero violations) trace to `StaticSeed` / `NotTraceable`.
- Where a scenario is a pushdown-commutation fact rather than a pure-expression
  fact (Q5, J-cases), assert the primitive returns the *conservative* verdict
  (does not push) rather than proving the full commutation — the primitive's job
  is the expression-level trace; the consumer applies it.

**Tests (red-green):**
- `harness_union_null_branch_is_static_seed` (P3).
- `harness_subquery_limit_and_frame_not_traceable` (Q5a/Q5b) — via the event-time
  projection through the non-transparent body.
- `harness_join_ts_dim_and_multiclock_not_pushable` (J3/J4).

**Implementer checklist:**
- [ ] Property tests gated behind the DuckDB dev-dependency the other property tests already use; `DUCKDB_LIB_DIR` respected.
- [ ] Deterministic — no reliance on wall-clock or RNG in the oracle.

**Reviewer checklist:**
- [ ] Each asserted verdict matches the research §2.3/§3.5/§5.5 violation counts.
- [ ] No hazard scenario is asserted `Traceable` (the one-way soundness contract).

**Commit:** `test(smelt-logical): property-test monotonicity trace against DuckDB harness`

---

## Later phases (scoped, not landed here) — wiring the three consumers

These are separate follow-on plans; each carries its own spec-diff and docs-site
update. Listed here so the primitive's output type stays designed for them.

- **Consumer A — `UNION`-branch partitionability (research §2.5, E1).** Call the
  primitive per branch; a branch returning `StaticSeed` is the P3 hazard and is
  named + rejected; all-`Traceable` branches unlock Strategy A wrap-and-filter.
- **Consumer B — subquery/CTE pushdown conservatism (research §4.6, B4/E2).**
  Resolve the outer `event_time` through the body; `Traceable → source-column`
  licenses the push, `NotTraceable →` stay at the outer clamp. Unifies B4 + E2 into
  one body classifier, closing the CTE bypass (§3.3).
- **Consumer C — join driving-fact resolution (research §5.4).** Call the primitive
  on the model `event_time` against every join input; `Traceable` to exactly one
  input is the driving fact (window only that scan); two is the multi-clock hazard
  (J4, reject); zero is a dim-side clock (reject). Replaces the A5 substring test.

---

## Verification gates

- `cargo test -p smelt-logical 2>&1 | tail -40`
- `cargo test -p smelt-logical --test '*monoton*' 2>&1 | tail -40` (property suite)
- `cargo test -p smelt-core --test hardening_budget 2>&1 | tail -20` (no new unwrap/expect/println regressions)
- `cargo tree -p smelt-db -i smelt-planner` unchanged (Layered single-ownership: this crate sits below both, adds no path)
- `cargo fmt --all`; `cargo clippy --all-targets 2>&1 | tail -30`
- `cargo test -p smelt-cli --test example_diagnostics 2>&1 | tail -20` (no example regressions — the primitive is unwired, so this must stay green with zero behaviour change)

## Progress

| Phase | Status |
|-------|--------|
| 0 — `analyze_select` retains `Expr` node | pending |
| 1 — pure `monotonicity.rs` (`trace_event_time`) | pending |
| 2 — property-test against DuckDB harness | pending |
| A/B/C — consumer wiring | deferred to follow-on plans |
