# Plan: Event-time monotonicity trace primitive (build-and-test-first)

**Date:** 2026-07-02
**Supersedes:** [`docs/plans/20260701-monotonicity-primitive.md`](20260701-monotonicity-primitive.md) — same primitive, reordered to *build + test extensively before resolving the spec's open questions or wiring any consumer*, and revised for four owner decisions (2026-07-02): ClickHouse-style verdict struct, reject-nullable-leaf, tree-annotation (not source-rewrite) injection direction, retain-parsed-`Expr`.
**Spec:** `docs/specs/incremental_models.md` — Semantics §"Event-time monotonicity trace", Constraint 12, Design §"Event-time monotonicity is one primitive…", and the four Open Questions under Known Divergences.
**Spec diff:** *No spec change in Phases 0–3.* The normative section already exists (committed `8ae98b8b`, self-marked "specified but not yet emitted"). This plan lands and tests the implementation first; **Phase 4** then edits the spec to *resolve* the four Open Questions (verdict shape, static-vs-declared boundary, injection direction, `analyze_select` node retention) with the answers the tests validated, and flips the Known-Divergence note from "not yet emitted" to "structural trace emitted; consumers pending".
**Docs:** code + spec. No `docs-site/` change: the primitive is internal analysis with no user surface until a consumer ships; the first consumer plan carries the docs-site update.
**Design source:** `docs/research/20260701-expanding-incremental-eligibility.md` Part 6 (derivation), Part 7 (external validation — ClickHouse `getMonotonicityForRange`, Iceberg `preserves_order`, Delta generated columns; CALM / Rice / Richardson decidability bounds).

## Owner decisions folded in (2026-07-02)

1. **Test before spec/integration.** The pure primitive is built and exhaustively unit- + property-tested (Phases 1–2) and the nullability gate proven (Phase 3) *before* the spec's Open Questions are resolved (Phase 4) and *before* any consumer (`UNION`/subquery/join) is wired (deferred).
2. **No string-matching shortcuts.** The classifier walks the parsed `smelt_parser::Expr` tree structurally; it must not fall back to substring tests like the A5 `stripped_sql.contains(event_time_column)` guard. Phase 0 retains the parsed node so the primitive never re-parses.
3. **ClickHouse-style verdict struct** (Open Question 5 → *yes, full 4-field now*): the traced chain carries `Monotonicity { is_monotonic, is_positive, is_always_monotonic, is_strict }` up front, so descending clocks / named-DST-zone (`is_always_monotonic = false`) / exact-endpoint relaxations need no later type change.
4. **Reject nullable leaf columns** (Open Question 1 → *reject*): a `Traceable` structural verdict whose leaf source column is not provably non-null is downgraded to `NotTraceable`. Nullability lives in `smelt-db` (above `smelt-logical`), so the gate is a **thin `smelt-db` query** wrapping the pure `smelt-logical` trace — the existing "Salsa wrapper assembles inputs, calls the pure function" pattern. *(Recommended home; awaiting owner confirmation vs. promoting `smelt-planner` to a production `smelt-db` dep, or a new `smelt-analysis` crate.)*
5. **Injection is tree-annotation, not source-rewrite** (Open Question 4 → *reject the "track back to a source location" framing*): the trace's `(source, source_column, offset)` is a **semantic** target, not a text span. Consumers annotate the logical/physical tree and let the printer emit SQL; they never compute how to edit source text. This plan only guarantees the trace is expressed semantically so the future annotation-injection redesign can consume it — that redesign is deferred (roadmap).

## Layering (why the split is what it is)

`smelt-core/parser/types` → **`smelt-logical`** → **`smelt-db`** (depends on smelt-logical, owns all type inference + nullability) → **`smelt-planner`** (sibling of smelt-db above smelt-logical; dev-dep only on smelt-db; owns rule *application* + `plan_printer.rs`).

- **Pure structural trace** → `smelt-logical` (`analysis/monotonicity.rs`). Consumes `smelt_parser::Expr` + the existing `BoundContext`; no type info; fully testable in isolation. Respects **Salsa purity** and **Layered single-ownership** (`cargo tree -p smelt-db -i smelt-planner` unchanged — this module is below both).
- **Nullability gate** → `smelt-db` thin query (Phase 3). The one place with both the logical trace and column nullability. This is the "layer on top" the owner intuited — it already exists.
- **Consumers / injection** → `smelt-planner`, later (deferred).

---

## Phase 0 — `analyze_select` retains the parsed `Expr` node (prerequisite)

**Files:** `crates/smelt-logical/src/analysis/mod.rs`.

**Why:** `SelectAnalysis` keeps each select item as raw `text` (`SelectItemKind::{GroupByKey,…}.text`) and discards the `Expr`. The classifier walks the expression tree; retaining the node means it never re-parses (decision 2). Research §6.7 flags this; retaining is the "one change, many future analyses benefit" option.

**Change:**
- Extend each `SelectItemKind` variant (or add a sibling field) to carry the parsed `smelt_parser::Expr` alongside the existing `text`, without removing `text`. Additive and behaviour-preserving for every current `analyze_select` caller.
- If retaining the node is genuinely blocked by Rowan lifetime/ownership friction, fall back to re-parsing the event-time expression text in isolation **and record that in the spec's `analyze_select` Open Question** — but the tree-retention path is the intended one.

**Tests (red-green, `crates/smelt-logical`):**
- `analyze_select_retains_expr_for_group_key`: `SELECT DATE_TRUNC('day', ts) AS d, …` yields an item whose retained `Expr` re-serialises to the same text.
- Every existing `analyze_select` test passes unchanged.

**Implementer checklist:**
- [ ] Additive — no field removed, no caller broken.
- [ ] No new production `unwrap`/`expect` (hardening ratchet); parse failures degrade to the existing `Option` return.
- [ ] `cargo fmt --all`; `cargo clippy --all-targets` clean.

**Reviewer checklist:**
- [ ] The retained node is the *item* expression, not the whole SELECT.
- [ ] Fallback (re-parse) decision, if taken, is recorded in the spec Open Question.

**Commit:** `refactor(smelt-logical): retain parsed Expr on SelectAnalysis items`

---

## Phase 1 — The pure structural trace (`trace_event_time`) with a ClickHouse 4-field verdict

**Files:** new `crates/smelt-logical/src/analysis/monotonicity.rs`; register in `analysis/mod.rs`; re-export from `lib.rs`.

**Change — the shape (decision 3):**

```rust
/// Constant temporal shift folded out of a monotone chain (col ± INTERVAL const).
pub enum Offset { Seconds(Seconds), Symbolic(String) /* months/years */ }

/// ClickHouse-style verdict for the traced chain (getMonotonicityForRange).
pub struct Monotonicity {
    pub is_monotonic: bool,        // chain is monotone over the value range
    pub is_positive: bool,         // direction: non-decreasing (true) vs non-increasing
    pub is_always_monotonic: bool, // monotone across the whole domain, not just a sub-range
    pub is_strict: bool,           // strictly injective vs weakly (plateaus, e.g. DATE_TRUNC)
}

pub enum EventTimeTrace {
    /// Monotone non-decreasing image of `source_column` on `source`, shifted by `offset`.
    Traceable { source: String, source_column: String, offset: Offset, monotonicity: Monotonicity },
    /// Constant or NULL-injecting — static seed, not a partitionable stream (§2.5/P3).
    StaticSeed { reason: String },
    /// Cannot prove monotone traceability — conservative; the consumer must not push (§4.6).
    NotTraceable { reason: String },
}

pub fn trace_event_time(
    event_time_expr: &smelt_parser::Expr,
    ctx: &crate::analysis::source_bounds::BoundContext,
) -> EventTimeTrace;
```

Whitelist entries set `is_monotonic = true`, `is_positive = true`, `is_always_monotonic = true`; `is_strict = true` for identity/`INTERVAL`-shift/fixed-offset-TZ, `false` for the many-to-one `DATE_TRUNC`/`CAST(_ AS DATE)`/`time_bucket`/`FLOOR`-to-grid forms. The fields are populated even though the forward-only consumers currently read only `is_monotonic && is_positive` — they exist so a named-DST-zone (`is_always_monotonic = false`) or descending clock (`is_positive = false`) is a *data* difference, not a type change.

**Change — the classifier:** walk `event_time_expr` from the projected column toward the leaves per the spec whitelist/blacklist, using `smelt-parser`'s existing `Expr` accessors (`as_column_ref`, `as_function_call`, `as_cast`, `as_binary`, `FunctionCall::name`/`arguments`, `CastExpr::expression` + target type). No new crate dependency.
- **Whitelist → recurse on the single column-bearing argument, compose offsets** (`Offset` folds `Seconds`; month/year stays `Symbolic`): bare/qualified column, `DATE_TRUNC`, `CAST(col AS DATE/TIMESTAMP)`, `date_bin`/`time_bucket`/`FLOOR`-to-grid, `col ± INTERVAL '<const>'`, `col AT TIME ZONE '<fixed-offset const>'`.
- **`StaticSeed`:** constant / `NULL` literal, `COALESCE(col, <const>)`.
- **`NotTraceable` (fail closed):** two column-bearing arguments, `MOD`/`EXTRACT`, `CASE`, `GREATEST`/`LEAST(col,const)`, unknown UDF, `NOW`/`CURRENT_DATE`, `CAST(col AS VARCHAR)`, `col AT TIME ZONE '<named DST zone>'`, any unrecognised head.
- Resolve the traced leaf column against `ctx.source_partition_cols`; a `Traceable` `source_column` is the matched `BoundContext` partition column, so it is directly consumable by `BoundResult::Bounded.source_partition_col`.
- `CAST` is whitelisted **only** for date/timestamp targets.

**Tests (red-green, `crates/smelt-logical`):**
- **Whitelist** (`trace_*_traceable`): each spec whitelist row → `Traceable` to the expected column + offset + `Monotonicity` fields, incl. the 3-layer composition `DATE_TRUNC('day', CAST(event_ts AS TIMESTAMP) + INTERVAL '2 hours')` → `Traceable{ source_column: event_ts, offset: +2h, is_strict:false }`.
- **Strictness** (`date_trunc_is_weakly_monotonic`): `DATE_TRUNC` → `is_strict = false`; `col + INTERVAL '1 day'` → `is_strict = true`.
- **Blacklist** (`trace_*_not_traceable` / `*_static_seed`): each blacklist row returns the specified verdict; `COALESCE`/`NULL`/constant → `StaticSeed`; `EXTRACT(HOUR …)`, `end_ts - start_ts`, `CASE`, `my_udf(ts)`, `NOW()`, `CAST(ts AS VARCHAR)`, named-DST-zone → `NotTraceable`.
- **Fail-closed** (`unknown_head_fails_closed`): a synthetic unrecognised function head → `NotTraceable`, never `Traceable`.
- **Multi-source** (`two_column_arithmetic_not_traceable`): columns from two sources → `NotTraceable` (join multi-clock, J4).

**Implementer checklist:**
- [ ] Pure — no Salsa, no I/O, no `smelt-db`/`smelt-planner` dependency (verify `cargo tree -p smelt-db -i smelt-planner` unaffected).
- [ ] No substring/text fallback anywhere in the classifier (decision 2) — it walks `Expr` only.
- [ ] Every unrecognised branch → `NotTraceable` with a `reason` naming the offending sub-expression (Constraint 12; fail-loud).
- [ ] No new production `unwrap`/`expect` beyond baseline.
- [ ] `cargo fmt --all`; `cargo clippy --all-targets` clean.

**Reviewer checklist:**
- [ ] Soundness direction: each whitelist arm is monotone non-decreasing on **all** target backends (intersection rule), not just DuckDB.
- [ ] `CAST` rejects non-temporal targets; named-DST-zone is `NotTraceable` (the instant→local map decreases at fall-back — research §6.2 watch-point c).
- [ ] Offset folding: constants fold; month/year stays `Symbolic`, never silently coerced to `Seconds`.
- [ ] `Monotonicity` fields match the arm (weak vs strict, always-monotonic true for every current whitelist entry).

**Commit:** `feat(smelt-logical): add pure event-time monotonicity trace primitive`

---

## Phase 2 — Property-test the primitive against the DuckDB harness (per-backend structure)

**Files:** new test module under `crates/smelt-logical/tests/`, reusing the harness SQL at `docs/research/harness/20260701-{union,subquery,join}_incremental.sql`.

**Why:** The harness already reproduces exactly the hazards the primitive must catch — P3 (`UNION` NULL/constant seed), Q5 (non-commuting subquery body), J3–J5 (timeseries-dim-as-lookup, multi-clock fact join, fan-out). Wiring them as the oracle proves the verdict lines up with the *empirically measured* incremental ≡ full boundary, not just hand-written expectations. The owner asked for **per-backend** validation of the static-vs-declared boundary, so the oracle trait is structured to add Spark/Postgres targets later (the existing `duckdb_oracle.rs` trait pattern), running DuckDB now.

**Change:**
- For each harness scenario assert the primitive's verdict on that scenario's event-time projection matches the measured outcome: safe rows (P1/P2, Q1–Q4, J1/J2 → 0 violations) → `Traceable`; hazard rows (P3, Q5a/Q5b, J3/J4/J5 → non-zero violations) → `StaticSeed` / `NotTraceable`.
- Where a scenario is a pushdown-commutation fact rather than a pure-expression fact (Q5, J-cases), assert the *conservative* verdict (does not push) — the primitive's job is the expression-level trace; the consumer applies commutation.
- Structure the oracle behind the trait so a `#[cfg(feature = "spark")]` (or Postgres) target can assert the same whitelist is monotone there too, validating the "intersection of all backends" rule empirically per-backend.

**Tests (red-green):**
- `harness_union_null_branch_is_static_seed` (P3).
- `harness_subquery_limit_and_frame_not_traceable` (Q5a/Q5b) via the event-time projection through the non-transparent body.
- `harness_join_ts_dim_and_multiclock_not_pushable` (J3/J4).

**Implementer checklist:**
- [ ] Property tests gated behind the DuckDB dev-dependency the other property tests already use; `DUCKDB_LIB_DIR` respected.
- [ ] Deterministic — no wall-clock/RNG in the oracle.
- [ ] Oracle trait leaves a documented seam for Spark/Postgres targets (per-backend rule).

**Reviewer checklist:**
- [ ] Each asserted verdict matches the research §2.3/§3.5/§5.5 violation counts.
- [ ] No hazard scenario is asserted `Traceable` (one-way soundness contract).

**Commit:** `test(smelt-logical): property-test monotonicity trace against DuckDB harness`

---

## Phase 3 — Nullability gate in `smelt-db` (reject nullable leaf)

**Files:** `crates/smelt-db/src/` (new thin query beside the existing `detect_builtin_rules` call site, `lib.rs:~1530`); reuse `schema.rs` / `type_inference/` for leaf-column nullability.

**Why (decision 4):** The pure trace cannot see nullability (it is below smelt-db). A `Traceable` verdict whose leaf source column can be `NULL` silently drops those rows from every incremental window while a full refresh keeps them (the §2.5/P3 hazard in column form). The gate resolves the leaf column's nullability from smelt-db's schema and **downgrades `Traceable → NotTraceable{ reason: "event-time leaf column <c> is nullable" }`**. Syntactic NULL forms are already `StaticSeed` in Phase 1; this closes the *semantic* nullable-column gap.

**Change:**
- Add a thin smelt-db query `trace_event_time_checked(model, event_time_expr) -> EventTimeTrace` that (a) calls the pure `smelt_logical::trace_event_time`, and (b) if `Traceable`, resolves `source_column`'s `nullable` flag via the model/source `ResolvedSchema` and downgrades when nullable-or-unknown. Fail closed: an unresolvable leaf column (schema unknown) → downgrade, never keep `Traceable`.
- Keep the pure `smelt-logical` verdict unchanged and independently returnable — the gate is *additive* composition, not a rewrite.

**Tests (red-green, `crates/smelt-db`):**
- `nullable_leaf_downgraded_to_not_traceable`: a source whose partition/event-time column is nullable → `Traceable` from the pure fn, `NotTraceable` after the gate.
- `non_null_leaf_stays_traceable`: a `NOT NULL` leaf column keeps `Traceable`.
- `unresolvable_leaf_fails_closed`: schema unknown for the leaf → `NotTraceable`.

**Implementer checklist:**
- [ ] Gate is a thin Salsa wrapper — no analysis logic duplicated from smelt-logical (Salsa purity rule).
- [ ] Fail closed on unknown nullability.
- [ ] `cargo tree -p smelt-db -i smelt-planner` still shows no production path.

**Reviewer checklist:**
- [ ] The downgrade only ever *narrows* (`Traceable → NotTraceable`), never widens (one-way soundness, Constraint 12).
- [ ] Nullability is read from smelt-db's inferred schema, not re-derived textually.

**Commit:** `feat(smelt-db): nullability-gate the event-time monotonicity trace`

---

## Phase 4 — Resolve the spec Open Questions from what the tests validated

**Files:** `docs/specs/incremental_models.md` (Known Divergences + Design + Constraints); `docs/research/20260701-expanding-incremental-eligibility.md` (§6.7 Open Questions).

**Why:** Per the owner's "test before changing the spec" (decision 1), the spec's four Open Questions are resolved only now, with the answers the implementation proved out.

**Change (spec edits):**
- **Verdict shape** → resolved: adopt the ClickHouse 4-field `Monotonicity` struct + trace + three-way classification (Phase 1 shape). Replace the "open whether to carry direction/strictness" note with the shipped shape.
- **Static-vs-declared boundary** → resolved: the full static whitelist ships as classification; declared guarantees only *widen* eligibility; the declared-monotonicity property is trusted **for correctness**, so it warrants a stricter opt-in than the existing property booleans (record the chosen gate — e.g. `unstable_`-style flag — once the first declared consumer lands). Per-backend validation is the standing property suite (Phase 2).
- **Nullability** → resolved: reject nullable-leaf `Traceable` verdicts (Phase 3 gate in smelt-db); update the Known-Divergence entry from "open question" to "gated in smelt-db".
- **Injection direction** → resolved: the trace is a *semantic* `(source, source_column, offset)` target consumed by **tree-annotation** injection; the "track back to a source location / rewrite source text" framing is rejected. Point the note at the deferred annotation-injection redesign.
- **`analyze_select` node retention** → resolved: nodes are retained (Phase 0); the primitive never re-parses.
- Flip the "specified but not yet emitted" Known-Divergence to: **structural trace emitted (`smelt-logical`) + nullability-gated (`smelt-db`); the three consumers (UNION/subquery/join) remain pending.**

**Tests:** `cargo test -p smelt-cli --test example_diagnostics` (no behaviour change — the primitive is still unwired into any user-visible gate). `/smelt:validate incremental_models` shows no new drift.

**Implementer checklist:**
- [ ] Timeless-oracle rule: spec edits describe behaviour, not phases; phase vocabulary stays in this plan.
- [ ] Research §6.7 open questions marked resolved with links back here.

**Reviewer checklist:**
- [ ] Each of the five resolutions matches what the code actually does.

**Commit:** `docs(spec): resolve event-time monotonicity open questions from tested primitive`

---

## Deferred to follow-on plans (each carries its own spec-diff + docs-site update)

Listed so the primitive's output type stays designed for them; **not** landed here.

- **Consumer A — `UNION`-branch partitionability (research §2.5, E1).** Per-branch trace; `StaticSeed` branch is the P3 hazard (named + rejected); all-`Traceable` unlocks Strategy A.
- **Consumer B — subquery/CTE pushdown conservatism (research §4.6, B4/E2).** Resolve outer `event_time` through the body; `Traceable → source-column` licenses the push, `NotTraceable →` stay at the outer clamp. Unifies B4 + E2, closes the CTE bypass (§3.3).
- **Consumer C — join driving-fact resolution (research §5.4).** Trace against every join input; exactly-one `Traceable` = driving fact; two = multi-clock (J4, reject); zero = dim-side clock (reject). Replaces the A5 substring test.
- **Tree-annotation injection redesign (Part 4, decision 5).** Replace the textual `inject_time_filter` / `inject_source_filters` (`transformer.rs:65,272`) with logical/physical-tree annotation consumed by `smelt-planner`'s `plan_printer.rs`. The consumers annotate the tree with the trace's semantic target; the printer emits SQL. No consumer computes source-text edits. **Roadmap item added.**
- **Retain-parsed-AST cleanup sweep.** Phase 0 does this for `analyze_select`; other analyses still re-scan raw text (`analysis/mod.rs` clause string-scanning, `source_bounds.rs` textual `INTERVAL`/`RANGE` recognition, `rules/incremental.rs` `Frontmatter::strip` + re-scan, the `temporal.rs` re-parse comments). **Roadmap item added.**
- **smelt-planner parallel-copy consolidation.** `smelt-planner` still carries a parallel copy of `analysis/*`, `rules/*`, `logical.rs`, `graph.rs`; the smelt-logical extraction is incomplete. Relevant to where any future type-aware analysis moves. **Roadmap item added.**

---

## Verification gates

- `cargo test -p smelt-logical 2>&1 | tail -40`
- `cargo test -p smelt-logical --test '*monoton*' 2>&1 | tail -40` (property suite)
- `cargo test -p smelt-db 2>&1 | tail -40` (Phase 3 nullability gate)
- `cargo test -p smelt-core --test hardening_budget 2>&1 | tail -20` (no new unwrap/expect/println regressions)
- `cargo tree -p smelt-db -i smelt-planner` unchanged (Layered single-ownership)
- `cargo fmt --all`; `cargo clippy --all-targets 2>&1 | tail -30`
- `cargo test -p smelt-cli --test example_diagnostics 2>&1 | tail -20` (primitive unwired → must stay green with zero behaviour change)

## Progress

| Phase | Status |
|-------|--------|
| 0 — `analyze_select` retains `Expr` node | done |
| 1 — pure `monotonicity.rs` (`trace_event_time`, 4-field verdict) | pending |
| 2 — property-test against DuckDB harness (per-backend seam) | pending |
| 3 — nullability gate in `smelt-db` (reject nullable leaf) | pending |
| 4 — resolve spec open questions from tested primitive *(spec increment — pre-authorized)* | pending |
| A/B/C + injection redesign + cleanups | deferred to follow-on plans |

## Blocked phases

*(none yet — the autonomy loop appends dated entries here when it records-and-continues a block)*
