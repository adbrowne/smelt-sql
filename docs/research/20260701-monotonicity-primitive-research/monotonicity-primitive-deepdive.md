## Part 6 — The monotonicity primitive

Three of the worked conditions converge on **one missing analysis**, and this
part is its full treatment. It is written to slot in as an expanded replacement
for the thin "Cross-cutting prerequisite" section: §2.5 (`UNION` branches),
§4.6 (subquery/CTE pushdown), and §5.4 (joins) all block on the same predicate,
and none of the analysis it needs exists in `smelt-logical` today.

The primitive answers a single question about a model's projected event-time:

> *Does this `event_time` expression trace back, monotonically, to a real source
> partition column — and if so, to **which** column, on **which** source, under
> **what** constant offset?*

The rest of this part pins that down formally (6.1), classifies what is decidable
statically (6.2), says where a static decision is impossible and a declaration is
required instead (6.3), shows how the three consumers call the one interface
(6.4), proposes its placement and shape in `smelt-logical` (6.5), enumerates the
edge cases and the conservative-fallback contract (6.6), and closes with the
open questions it raises (6.7).

### 6.1 Precise definition

Let a source `S` carry a partition column `p` (its `timeseries.partition_column`)
and let the model project an `event_time` value through some expression
`e = f(...)`. The runtime uses `event_time` in two places (§4.2): the **outer
output-clamp** filters rows on the projected `e` directly
(`inject_time_filter`, `transformer.rs:272`), and the **per-source scan filter**
filters `S` on its partition column `p`
(`inject_source_filters`, `transformer.rs:65`, using the `source_partition_col`
carried by `BoundResult::Bounded`, `source_bounds.rs:79`).

Incrementalisation is correct only when these two filters select **the same
rows** — i.e. when filtering the output on `e` is *equivalent* to filtering the
source on `p`. That is a statement about the function `f` relating `p` (or the
source's own event clock) to `e`.

**The exact property needed** is not order-preservation in the strict sense, nor
value-injectivity. It is:

> **Interval-preimage-is-an-interval.** For every window `[lo, hi)` on `e`, the
> set of source rows with `f(p) ∈ [lo, hi)` is exactly the set with
> `p ∈ [a, b)` for some thresholds `a, b` (i.e. the preimage of a half-line is a
> half-line). Equivalently, `f` is **monotone non-decreasing**.

Two clarifications this framing forces, both of which matter to the whitelist in
6.2:

- **Non-decreasing suffices; strict monotonicity is not required.** `DATE_TRUNC`
  and `CAST(ts AS DATE)` are *many-to-one* (a whole day of timestamps maps to one
  date) yet still push cleanly, because the model's window boundaries are
  themselves granularity-aligned: `partition_column` **is** `DATE_TRUNC('day', e)`
  in the canonical model (`incremental.rs:172`–`176` reads the partition column's
  expression `text`). A plateau of `f` never straddles a window boundary, so the
  half-line preimage is exact. Requiring strict monotonicity would needlessly
  reject the single most common shape.
- **We need window-preserving, not value-preserving.** The output-clamp already
  filters on `e` verbatim, so it is trivially correct whenever `e` is projected
  (that is all E2's `is_column_projected_in_sql` check, `rule_diagnostics.rs:236`,
  verifies today). Monotonicity is the *extra* fact required to **relocate** that
  filter onto `p` at the source — i.e. it licenses the Part 4 pushdown, not the
  bare injection. This is why the primitive is a prerequisite for the *pushdown*
  half of every relaxation, and why §4.6 phrases its conservative fallback as
  "stay at the outer clamp" — the outer clamp needs no monotonicity, only the
  push does.

There is a second, weaker use where monotonicity still matters even **without**
pushdown: the §2.5 *independent-partitionability* / NULL hazard. A `UNION`
branch that stamps `event_time` with a constant or `NULL` is a *static seed*, not
a monotone image of any clock — it lands in one partition forever (constant) or
never passes `e >= start` at all (`NULL`), silently breaking incremental ≡ full
(property **P3**, §2.3, 1 violating row). So the predicate has to reject
constant/`NULL`/plateau-collapsing expressions too; "monotone image of a real
source clock" is precisely the condition that excludes them. The two uses share
one predicate: *e is a monotone non-decreasing, total, source-traceable image of
S's clock.*

### 6.2 What is decidable statically from the SELECT expression

`smelt-parser` already exposes a rich typed expression tree — `Expr` offers
`as_column_ref`, `as_function_call`, `as_cast`, `as_extract`, `as_case`,
`as_binary` (`ast.rs:1860`–`1968`), `FunctionCall::name`/`arguments`
(`ast.rs:2240`,`:2316`), `BinaryExpr::left`/`right`/`operator`
(`ast.rs:2103`–`2113`), `CastExpr::expression` and its target type
(`ast.rs:2725`). So a real structural classifier is feasible; it does **not**
need to be a substring heuristic like the A5 test
(`stripped_sql.contains(event_time_column)`, `incremental.rs:196`). The one
plumbing wrinkle: `analyze_select` currently keeps only the *raw text* of each
select item (`SelectItemKind::{GroupByKey,…}.text`, `analysis/mod.rs:9`–`16`)
and discards the `Expr` node, so the primitive must either re-parse the
event-time expression text or `analyze_select` must be extended to retain the
node (see 6.5).

Classify the event-time expression `e` by walking it from the projected column
toward the leaves. The proposed **monotone whitelist** — each form provably
non-decreasing across DuckDB/Spark/Postgres:

| Form | Example | Monotone? | Traces to |
|---|---|---|---|
| transparent alias / bare column | `created_at AS event_time` | identity | the column, offset 0 |
| qualified column | `f.event_ts AS event_time` | identity | column on the qualified input |
| `DATE_TRUNC(unit, col)` | `DATE_TRUNC('day', event_ts)` | non-decreasing (step) | `col` |
| `CAST(col AS DATE/TIMESTAMP)` | `CAST(event_ts AS DATE)` | non-decreasing (truncation) | `col` |
| `date_bin` / `time_bucket` / `FLOOR(col to grid)` | `time_bucket('1 hour', ts)` | non-decreasing | `col` |
| `col ± INTERVAL '<const>'` | `event_ts + INTERVAL '1 day'` | strictly increasing shift | `col`, offset folds into the bound |
| `col AT TIME ZONE '<const>'` | `ts AT TIME ZONE 'UTC'` | non-decreasing (shift; DST plateaus but never decreases) | `col` |

The **non-monotone / order-breaking** forms, which must yield *not-traceable*:

| Form | Why it breaks | Example |
|---|---|---|
| arithmetic on **two** columns | not monotone in either alone; also multi-source (6.6) | `end_ts - start_ts` |
| `MOD` / `EXTRACT(HOUR/DOW/…)` | periodic — preimage of an interval is a union of intervals | `EXTRACT(HOUR FROM ts)` |
| `CASE WHEN …` | piecewise; generally neither monotone nor total | `CASE WHEN … THEN a ELSE b END` |
| `COALESCE(col, <const>)` | injects a constant for `NULL` rows — the §2.5 seed hazard in function form | `COALESCE(event_ts, '1970-01-01')` |
| `GREATEST/LEAST(col, <const>)` | clamps to a plateau that *can* straddle a window boundary | `GREATEST(ts, '2020-01-01')` |
| unknown scalar UDF | monotonicity unknowable from the call site | `my_udf(ts)` |
| constant / `NULL` literal | static seed, not a stream (§2.5 case 2) | `TIMESTAMP '2020-01-01'`, `NULL` |
| run-nondeterministic clock | `NOW()`/`CURRENT_DATE` shift each run; not source-traceable | `NOW()` (also B5, `incremental.rs:288`) |

**Where engine semantics matter.** The whitelist is deliberately the intersection
of what is monotone on *every* target backend, because smelt is multi-backend
(§4.4) and a per-engine monotonicity table would make eligibility a function of
the backend rather than of the plan. Two watch-points: (a) `CAST` is only
whitelisted for date/timestamp target types — `CAST(ts AS VARCHAR)` is monotone
*only* for ISO-8601 lexical form and not in general, so it is excluded; (b)
month/year `INTERVAL` arithmetic has a non-uniform step but is still monotone
non-decreasing, so it is admitted even though the offset cannot be folded to a
fixed `Seconds` (it stays a symbolic offset — cf. `source_bounds` approximating
`MONTH ≈ 30 days`, `source_bounds.rs:506`).

Composition is closed under the whitelist: a composition of monotone
non-decreasing functions is monotone non-decreasing, so `DATE_TRUNC('day',
CAST(event_ts AS TIMESTAMP) + INTERVAL '2 hours')` traces through all three
layers to `event_ts` with a `+2h` offset. The classifier recurses on the single
column-bearing argument at each layer and fails closed the moment a layer has two
column-bearing arguments or an unrecognised head.

### 6.3 Where a static decision is impossible — the declared guarantee

Static classification runs out in three situations: (a) an opaque scalar UDF
whose body smelt cannot see; (b) a smelt function (`smelt.functions.*`) whose
expanded body is monotone but too large to re-derive cheaply; (c) a genuinely
data-dependent monotonicity (e.g. a column the modeller *knows* is
append-only-monotone but which the SQL does not prove). For these, the safe
default is *not-traceable* (6.6) — but the modeller may supply the guarantee.

The natural annotation home already exists and already has this exact "trust the
declaration" shape:

- **`FunctionProperties`** (`logical.rs`, near `:74`) already carries
  `deterministic`, `idempotent`, `append_only` — declared, unverified booleans on
  a smelt function. A `monotone_event_time` (or per-argument `monotone`) property
  slots in beside them and lets a function-wrapped event-time expression be
  admitted by declaration when its body is not statically classifiable.
- **`timeseries:` frontmatter** (`config.rs:477`) is where a per-model override
  would live if the guarantee is about a specific model's projection rather than
  a reusable function — e.g. asserting that a named event-time expression is
  monotone in a named source column.

The precedent — and the caution — is the declared `joins:` **cardinality**
(`JoinSpec`, `logical.rs:103`; `Cardinality::{OneToOne,OneToMany}`,
`logical.rs:135`). The planner already trusts that declaration *for optimisation*
(join elimination, the §20E soundness caveat). A monotonicity declaration would
be trusted **for correctness** — the stakes are strictly higher, exactly as §5.4
and the last open question of Part 5 flag. The design rule that falls out: a
declaration may *widen* eligibility, but the conservative static default when no
declaration is present must be *reject-the-push*, never *assume-monotone*.

### 6.4 The three consumers call one interface

All three worked conditions reduce to one call with the same signature. The input
is a SELECT (or a `UNION` branch), the projected `event_time` expression, and the
set of source refs with their declared partition columns (the `BoundContext`
already built for bound derivation, `source_bounds.rs:131`; assembled from the
graph in `incremental.rs:559`–`568`). The output is not a bare boolean — per
Part 4 it must name the **deepest source column** the filter can be pushed to, so
it doubles as the injection-point resolver.

- **§2.5 `UNION` branches — "independently partitionable".** For each branch,
  call the primitive on that branch's `event_time` projection against that
  branch's own sources. A branch that returns *traceable* is a partitionable
  stream (Strategy A / B is safe on it); a branch that returns *static-seed* is
  the P3 `NULL`/constant hazard and must be named and rejected, not silently
  dropped.
- **§4.6 subquery/CTE conservatism.** Before pushing the proven-safe filter below
  a derived-table or CTE boundary, call the primitive on the outer `event_time`
  resolved through the body. *Traceable → source-column* licenses the push (Part 4
  "eligibility = maximal pushdown depth"); *not-traceable →* stay at the outer
  clamp (today's behaviour) — never push a filter the primitive did not license.
- **§5.4 joins — "exactly one input carries a monotone event_time".** Call the
  primitive on the model's `event_time` against every join input. Incrementalisable
  iff it returns *traceable to exactly one input* (the driving fact); that input's
  scan is windowed and every other input is full-scanned. Two traceable inputs is
  the multi-clock hazard (J4); zero is a dim-side or ambiguous clock (reject). This
  replaces the A5 substring test (`incremental.rs:195`) with a resolution that
  names *which* input carries the clock.

The shared **output** therefore wants to be, per the Part 4 framing, a *trace*
rather than a predicate — the source, the traced source column, and any constant
offset — so that `inject_source_filters` can write the filter at that exact column
and the offset can be merged into the derived `BoundResult` (whose
`source_partition_col`, `source_bounds.rs:79`, is precisely the "deepest source
column" the primitive computes). One analysis; three consumers; one injection
point.

### 6.5 Proposed placement and shape in `smelt-logical`

**Placement.** A new pure module `crates/smelt-logical/src/analysis/monotonicity.rs`,
sibling to `source_bounds.rs` and `temporal.rs` under `analysis/`. This respects
the **Layered single-ownership** invariant (analysis lives in `smelt-logical`,
above `smelt-parser`, below `smelt-db`/`smelt-planner`) and the **Salsa purity**
rule (a pure function over parser AST + declared context; any Salsa query in
`smelt-db` is a thin wrapper that assembles the inputs and calls it). It has no
new dependency — it consumes `smelt-parser`'s `Expr` tree and the existing
`BoundContext`.

**Shape.** A trace enum plus one entry point (illustrative, not final):

```rust
/// Constant temporal shift folded out of a monotone chain (col ± INTERVAL const).
pub enum Offset { Seconds(Seconds), Symbolic(String) /* e.g. months/years */ }

pub enum EventTimeTrace {
    /// `event_time` is a monotone non-decreasing image of `source_column`
    /// on `source`, shifted by `offset`. The licence to push the filter to
    /// `source.source_column` (Part 4), and to fold `offset` into the bound.
    Traceable { source: String, source_column: String, offset: Offset },
    /// Constant or NULL-injecting — a static seed, not a partitionable stream
    /// (§2.5 case 2 / P3). Names the offending sub-expression.
    StaticSeed { reason: String },
    /// Cannot prove monotone traceability: non-monotone fn, CASE, multi-source
    /// arithmetic, unknown UDF, run-nondeterministic clock. Conservative — the
    /// consumer must not push (§4.6).
    NotTraceable { reason: String },
}

pub fn trace_event_time(
    event_time_expr: &smelt_parser::Expr,
    ctx: &crate::analysis::source_bounds::BoundContext,
) -> EventTimeTrace;
```

**Why this is the natural first implementation phase** (as the doc claims):

1. **It is the shared blocker.** §2.5, §4.6 and §5.4 cannot ship without it, and
   they cannot each grow a private, divergent copy without re-introducing exactly
   the syntax-vs-semantics inconsistency §3.3 exposed. One analysis keeps the three
   relaxations honest with each other.
2. **It is pure and independently testable.** No injection changes, no runtime
   changes — a function from `(Expr, BoundContext)` to `EventTimeTrace`. It can be
   red-green unit-tested on the whitelist/blacklist of 6.2 and property-tested
   against DuckDB (the §2.3/§3.5/§5.5 harness already reproduces the hazards it must
   catch: P3, Q5, J3–J5) *before* any consumer is wired up.
3. **Its output type is designed for the consumers, not retrofitted.** Returning a
   trace (source + column + offset) rather than a boolean means the same result
   feeds the eligibility verdict *and* the Part 4 pushdown-depth walk *and* the
   `BoundResult` the runtime already threads — so wiring each consumer is a small
   follow-on, not a re-analysis.

### 6.6 Edge cases and the conservative-fallback contract

- **NULL `event_time` (the §2.5 hazard, P3).** Any expression that can evaluate to
  `NULL` for some rows silently drops those rows from *every* incremental window
  while a full refresh keeps them. Statically this is decidable for the syntactic
  cases — a `NULL` literal, or `COALESCE(col, <const>)` — which the classifier
  routes to `StaticSeed`. It is **not** decidable at this layer for a merely
  *nullable column* (column nullability is inferred above `smelt-logical`, in
  `smelt-db`); that gap is called out as an open question (6.7). The conservative
  stance: a syntactically NULL-injecting form is a seed; a plain column is treated
  as traceable (matching today's behaviour, which already lets nullable event-times
  through the outer clamp) and the residual nullability risk is the modeller's,
  unless we choose to thread nullability in.
- **Constant / static-seed event_time.** A literal timestamp → `StaticSeed` (§2.5
  case 2). Distinct from a real low-volume stream (§2.5 case 1), which still traces
  to a genuine clock and is safe.
- **Run-nondeterministic functions.** `NOW()`/`CURRENT_DATE`/`CURRENT_TIMESTAMP` are
  constant-per-run but *shift between runs*, so they are not source-traceable →
  `NotTraceable`. This dovetails with the B5 "split the bucket" stub (Part 1
  closing list): the monotonicity primitive is exactly the analysis that
  distinguishes a run-deterministic clock (admissible as an outer clamp, never as a
  pushed source filter) from a row-nondeterministic one.
- **Multi-source expression.** An `event_time` built from columns of two different
  sources (e.g. `f.ts` and `d.ts`) has no single source to push to → `NotTraceable`.
  This *is* the join multi-clock case (§5.4 / J4): the primitive returning
  "traceable to more than one input" is the same fact as "there is a second clock".
- **The conservative-fallback contract (the load-bearing invariant).** The
  primitive must be **sound in one direction**: it may return `NotTraceable` for a
  form that is in fact safe (a false negative — merely a missed optimisation, the
  consumer stays at the outer clamp), but it must **never** return `Traceable` for
  a form that is not monotone-source-traceable (a false positive — an unsound
  pushed filter, the §4.6 danger). Every unrecognised head, every two-column
  argument, every unknown UDF fails **closed** to `NotTraceable`. This is the same
  fail-loud / fail-safe discipline the codebase already enforces elsewhere
  (`cardinality_from_str` maps any unknown string to the conservative
  `OneToMany`, `logical.rs:~146`), and it is what the empirical harness (P3, Q5,
  J3–J5) exists to keep honest.

### 6.7 Open questions this raises

- **Column nullability at this layer.** The syntactic NULL forms are catchable, but
  a nullable source column that produces `NULL` `event_time` rows is not visible in
  `smelt-logical` (nullability is inferred in `smelt-db`). Do we thread a
  nullability signal down into the primitive (widening what it can prove), accept
  the residual risk as the modeller's, or reject any event-time whose leaf column
  is not provably non-null?
- **Offset folding vs. symbolic offsets.** `col + INTERVAL '1 day'` folds cleanly
  into a `Seconds` offset that merges with `source_bounds` Form B
  (`source_bounds.rs:359`). Month/year intervals are monotone but non-uniform —
  carry them as a `Symbolic` offset the runtime rewrites per-engine, or refuse to
  push them (outer-clamp only)?
- **Static vs. declared boundary.** How much of the whitelist (6.2) do we ship as
  static classification before leaning on a declared `monotone` property on
  `FunctionProperties` / `timeseries:` (6.3)? Given the §20E precedent, does
  trusting a declaration *for correctness* (not just optimisation) warrant a
  stricter opt-in (e.g. an `unstable_`-style workspace flag, as `provenance:`
  already requires per `logical.rs:70`–`73`)?
- **Reusing the trace as the Part 4 injection point.** The trace's
  `(source, source_column, offset)` is designed to be the "deepest safe injection
  point." Can `inject_source_filters` / bound derivation consume it directly, or
  does the operator-by-operator pushdown walk (Part 4 open questions) still need a
  separate pass for the intervening operators the primitive skipped over?
- **`analyze_select` retaining the `Expr` tree.** The primitive needs the parsed
  event-time expression, but `SelectAnalysis` currently keeps only raw `text`
  (`analysis/mod.rs:9`). Extend `analyze_select` to retain the node (one change,
  many future analyses benefit), or have the primitive re-parse the expression text
  in isolation (cheaper to land, but re-parses)?
