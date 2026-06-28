---
feature: pipe_sql
status: experimental
last_reviewed: 2026-06-27
owners: [andrew]
---

# Pipe SQL

> **What this is.** A normative spec for **pipe syntax** in Data-World SQL — the BigQuery/Databricks-style `FROM t |> WHERE … |> AGGREGATE …` form, where a query is written as a linear chain of `|>` stages. Covers the pipe-query grammar, the operator set, where a pipe query may appear, its scoping semantics, and how it lowers to standard SQL. Out of scope: the **meta-language** `|>` operator (`x |> f(args)` ≡ `f(x, args)`, a compile-time first-arg pipe over meta values — see `meta_language.md` §"Pipe operator"); the `smelt.<path>` addressing scheme (`architecture.md` §"Resolution"); model frontmatter and materialization (`models.md`); the type system that infers each stage's schema (`types.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings, no inline phase labels, no plan-vocabulary status callouts in §Surface, §Semantics, §Design, or §Constraints. Implementation status goes in §Known Divergences (describe behaviour, link the plan) or §References → Plans (history). See the Timeless-oracle rule in `CLAUDE.md`.

## Surface

### Pipe query form

A **pipe query** is a query written as a table source followed by a chain of pipe stages:

```sql
FROM orders
|> WHERE status = 'paid'
|> AGGREGATE sum(amount) AS revenue GROUP BY customer_id
|> WHERE revenue > 1000
|> ORDER BY revenue DESC
|> LIMIT 10
```

Each `|> OPERATOR …` stage consumes the table produced by the prior stage and produces a new table. A pipe query is a complete query: it produces a relation exactly as a `SELECT` statement does, and is usable in every position a query is (see "Where a pipe query may appear").

### Pipe-mode trigger

A query body is parsed as a pipe query when, after any optional leading `WITH` clause, the body **begins with a bare `FROM` clause** (no leading `SELECT`) and the `FROM` is followed by at least one `|>` stage. This is the **FROM-first** form. A body that begins with `SELECT` is a standard SELECT statement and is parsed as today; appending `|>` stages onto a leading `SELECT … FROM …` query is not part of this surface (see §Known Divergences).

The `|>` token is the same `PIPE_ARROW` lexed for the meta-language pipe; which language owns it is decided by parse position (see §Semantics — "Token disambiguation").

### Pipe operators

The pipe operators below are supported. Each row gives the exact keyword form and the standard-SQL clause it corresponds to. Syntax inside a stage reuses the existing clause grammar (a `|> WHERE` stage wraps the same predicate grammar as a `WHERE` clause).

| Operator | Form | Meaning |
|---|---|---|
| *(entry)* | `FROM <table_ref>` | Table source: a table, `smelt.<path>`, a parenthesised subquery, a join chain. Same `FROM` grammar as a SELECT. |
| `WHERE` | `\|> WHERE <predicate>` | Filter rows. Before any aggregation this is a `WHERE`; after aggregation it is a `HAVING`; after a window column it is a `QUALIFY` (see lowering). |
| `SELECT` | `\|> SELECT <expr> [AS <alias>], …` | Project to exactly the listed columns. A stage, not "logically last". |
| `EXTEND` | `\|> EXTEND <expr> AS <alias>, …` | Append computed columns, keeping all input columns. |
| `SET` | `\|> SET <col> = <expr>, …` | Replace the value of existing columns in place. |
| `DROP` | `\|> DROP <col>, …` | Remove the named columns, keep the rest. |
| `RENAME` | `\|> RENAME <old> AS <new>, …` | Rename columns, keep all others. |
| `AS` | `\|> AS <alias>` | Give the whole intermediate table a range-variable alias. |
| `AGGREGATE` | `\|> AGGREGATE <agg_expr> [AS <alias>], … [GROUP BY <group_expr> [AS <alias>], …]` | Combined projection + grouping. Output column order is grouping keys, then aggregates. Without `GROUP BY`, full-table aggregation (one row). |
| `ORDER BY` | `\|> ORDER BY <expr> [ASC\|DESC] [NULLS …], …` | Order rows. |
| `LIMIT` | `\|> LIMIT <n> [OFFSET <m>]` | Row limit / offset. |
| `JOIN` | `\|> [INNER\|LEFT\|RIGHT\|FULL\|CROSS] JOIN <table_ref> [ON <cond> \| USING (<cols>)]` | Join; the pipe input is the left side, only the right table is named. |
| set ops | `\|> {UNION\|INTERSECT\|EXCEPT} {ALL\|DISTINCT} (<query>) [, (<query>)…]` | Set operation against one or more parenthesised queries. |
| `DISTINCT` | `\|> DISTINCT` | Deduplicate rows. |

### Deferred operators

The following BigQuery pipe operators are **not** supported. Using one in a pipe query is a hard error (`PipeOperatorUnsupported`), not a silent no-op:

| Operator | Reason |
|---|---|
| `PIVOT` / `UNPIVOT` | Output columns depend on data values and cannot be determined at compile time; smelt rejects standard `PIVOT`/`UNPIVOT` for the same reason (`check_types.rs::check_unsupported_constructs`). |
| `WINDOW` (as a pipe operator) | Window functions are supported inside `SELECT`/`EXTEND` expressions; the `\|> WINDOW name AS (…)` operator form is not. |
| `CALL` (table-valued function application) | Table-valued-function piping has no end-to-end smelt support. |
| `TABLESAMPLE` | Sampling is available on a `FROM` table reference only, not as a stage. |
| `ASSERT` | Row-level runtime assertions have no smelt equivalent construct. |

### Where a pipe query may appear

A pipe query may appear:

1. As a **model body** — a `.sql` model (single- or multi-model section) whose body is a FROM-first pipe query. All model frontmatter (`materialization`, `refresh`, `incremental`, `tags`, …) applies unchanged (`models.md`).
2. As a **subquery or CTE body** — anywhere a parenthesised query or a `WITH` CTE body is legal, e.g. `FROM (FROM t |> WHERE p)` or `WITH recent AS (FROM events |> WHERE ts > …) …`.

A pipe query may itself begin with a `WITH` clause; the CTEs are in scope for the FROM-first body.

### Diagnostics

| Code | Trigger | Message |
|---|---|---|
| `PipeUnknownOperator` | `\|>` followed by a token that is not a recognised pipe operator keyword | `unknown pipe operator '<kw>'` |
| `PipeOperatorUnsupported` | a recognised-but-deferred operator (`PIVOT`/`UNPIVOT`/`WINDOW`/`CALL`/`TABLESAMPLE`/`ASSERT`) | `pipe operator '<kw>' is not supported — <reason>` |
| `PipeStageMalformed` | a stage whose body does not parse against the operator's clause grammar | `malformed '<kw>' pipe stage` |

Name resolution, type, and aggregate-shape errors inside a stage reuse the existing Data-World diagnostics (the same a non-pipe query would raise), anchored at the offending stage span.

## Semantics

### Stage model and scope

1. A pipe query is the **left-to-right composition** of its stages. The input to the first stage is the `FROM` table; the input to each later stage is the output table of the prior stage.
2. **Column scope flows stage-to-stage.** At any stage the visible columns are exactly the output columns of the previous stage. A column introduced by `|> EXTEND e AS y`, `|> SET`, or `|> AGGREGATE … AS y` is immediately referenceable by the next stage's `|> WHERE y > 0`, `|> ORDER BY y`, etc. After `|> AGGREGATE`, only the grouping keys and aggregate outputs are in scope; pre-aggregation columns are no longer visible.
3. Each stage's output schema is inferred by the type system using the same per-clause logic that infers a SELECT's schema, applied as a per-stage transform over the running scope (`types.md`; the analyzer already threads a "columns visible here" context through `FROM → WHERE → GROUP BY → HAVING → SELECT`). `EXTEND` adds a column to the scope, `AGGREGATE` replaces the scope with keys+aggregates, `DROP`/`RENAME` edit it, `WHERE`/`ORDER BY`/`LIMIT`/`DISTINCT` pass it through, `JOIN` extends it with the right side's columns.

### Token disambiguation

`|>` is one lexed token (`PIPE_ARROW`) shared with the meta-language pipe. Ownership is by **parse position**, and the two never legally co-occur:

1. Inside a pipe query, a `|>` that appears at a **stage boundary** (after a complete clause/stage) is the SQL pipe and introduces the next stage.
2. A `|>` that appears **inside a Data-World expression** (e.g. within a `WHERE` predicate or a `SELECT` item) is not a stage boundary. It remains the meta-only operator and is rejected in that position with `PipeInDataPosition` (`meta_language.md` §"Pipe operator"). The meta-language pipe continues to operate only in meta-expression contexts (`smelt.define` bodies, generator bodies).

There is no lexer change and no new glyph: the SQL pipe and the meta pipe share `|>`, disambiguated structurally.

### Lowering to standard SQL

A pipe query is **lowered to standard SQL during code generation** (the `smelt-dialect` printer stage). Lowering is a semantics-preserving rewrite; the emitted SQL computes the same relation as the pipe query.

1. Contiguous stages that fit one query level collect into a single `SELECT`: `FROM`, pre-aggregation `|> WHERE` → `WHERE`, `|> JOIN` → joins, a trailing `|> SELECT` → the select list, `|> ORDER BY`/`|> LIMIT` → trailing clauses.
2. `|> AGGREGATE agg … GROUP BY k …` → `SELECT k…, agg … GROUP BY k…`, with output column order keys-then-aggregates.
3. A `|> WHERE` that **follows an aggregation** lowers to `HAVING` (or a wrapping subquery when `HAVING` cannot express it). A `|> WHERE` that **follows a window column** lowers to `QUALIFY` or a wrapping subquery.
4. `|> EXTEND`, `|> SET`, `|> DROP`, `|> RENAME`, `|> DISTINCT` lower to a (re-)projection. When such a stage follows a stage that already fixed the projection, the prior query is wrapped as a subquery: `SELECT <new projection> FROM (<prior query>)`.
5. **Multiple `|> AGGREGATE` stages nest.** Standard SQL cannot aggregate twice at one query level, so each aggregation wraps the previous as a subquery (or CTE). A pipe query with N aggregation stages lowers to N nested query levels.
6. `|> UNION/INTERSECT/EXCEPT (q1), (q2), …` lowers to a left-folded chain of the binary set operations smelt already supports.

The lowered standard SQL is a generated artifact and carries no byte-identity obligation; pipe-query lowering is one of the rewrites the printer applies "modulo" its identity target, alongside `smelt.<path>` resolution and `QUALIFY` → subquery (`architecture.md` §"Identity properties").

### Native passthrough

The dialect layer carries a `supports_pipe_syntax` capability per backend (`BackendCapabilities`, `architecture.md` §"Identity properties"). When a backend advertises native pipe support, the printer may emit pipe syntax directly instead of the lowered form; otherwise it emits the lowered standard SQL. Every backend reports `supports_pipe_syntax = false`, so the only emitted form is lowered standard SQL — the capability exists so native emission is a flag flip, not a re-architecture. Backends whose native pipe dialect omits an operator (e.g. Spark omits `RENAME`) may pass through the supported operators and lower only the unsupported ones.

### PostgreSQL parser oracle

Pipe syntax is a deliberate extension beyond PostgreSQL grammar; the `pg_query`-based fingerprint-equivalence oracle (`architecture.md` §"Identity properties", `crates/smelt-parser-compat/`) cannot validate a pipe query, because PostgreSQL has no pipe syntax. A pipe query is registered as a known parser-compat extension gap (alongside `pivot_unpivot`), so the oracle skips it rather than reporting a spurious divergence. The Spark `sqlparser-rs`/`sqlglot` cross-check applies only on the (future) native-Spark path.

## Design

**Reuse `|>`, disambiguate by position.** The pipe glyph is fixed by ecosystem compatibility — BigQuery, Spark/Databricks, and the DuckDB `psql` extension all spell it `|>`. A distinct glyph for SQL pipes was rejected because it would defeat the compatibility goal (copy-pasted BigQuery pipe queries would not parse). The meta-language already owns `|>`, but the two never legally co-occur: meta pipes live in meta-expression contexts (`smelt.define`/generator bodies) and SQL pipes live at Data-World stage boundaries. `meta_language.md` §"Pipe — design rationale" anticipated this split explicitly ("pipe-SQL extension … is a separate paper that extends the SQL grammar"). The existing `PipeInDataPosition` rule is narrowed, not removed: a `|>` inside a Data-World *expression* is still meta-only and still rejected; only a `|>` at a stage boundary in a FROM-first body is the SQL pipe.

**FROM-first as the trigger.** Making "a body beginning with a bare `FROM`" the pipe-mode trigger gives an unambiguous, single-token signal that costs nothing to detect and matches the canonical BigQuery/Spark FROM-first form. The alternative — also accepting `|>` appended after a leading `SELECT … FROM …` query (BigQuery's other form) — is deferred because it requires the SELECT parser to speculatively continue into a pipe chain, a larger and more ambiguous grammar change for a form that is rarely the authoring style.

**First-class pipe nodes, lowered at the dialect printer (not desugared at parse time).** Two lowering points were considered: (L1) rewrite pipes into standard `SELECT` subtrees at parse time, before analysis; (L2) keep first-class pipe CST nodes through analysis and lower during code generation. L2 was chosen. It puts the native-vs-lowered decision at the `smelt-dialect` printer — the seam that already chooses native-vs-rewritten per backend (it rewrites `QUALIFY` → subquery for backends that lack it). That makes "collapse to standard SQL now, emit native pipes on capable backends later" a single capability flag rather than a future re-architecture, and it keeps the user's pipe structure available for diagnostics. L1 has a smaller immediate blast radius but discards pipe structure before analysis and makes native passthrough awkward. The cost of L2 — analysis and the printer must understand pipe stages — is small because the analyzer is already a stage pipeline threading a visible-column context clause-to-clause; pipe stages are the same transforms in an explicit order.

**Operator set bounded by smelt's existing SQL surface.** A pipe operator exists only where the underlying relational operation is supported end-to-end in smelt. `PIVOT`/`UNPIVOT` are deferred not as an oversight but on the same principle smelt already applies to standard `PIVOT`/`UNPIVOT`: the output schema is not determinable at compile time, which smelt's type system requires. `WINDOW`/`CALL`/`TABLESAMPLE`/`ASSERT` are deferred because they have no end-to-end equivalent yet. Deferred operators are a hard error rather than a silent drop, per the fail-loud discipline (`architecture.md` §"Fail-loud discipline").

**Scope-flows-forward is the whole point.** Pipe SQL's value over standard SQL is that evaluation order equals lexical order, so a stage sees exactly the previous stage's output and alias visibility stops being surprising. The spec preserves this precisely: scope is the running output schema, `AGGREGATE` collapses it, and a post-aggregate `|> WHERE` references aggregate aliases directly (lowered to `HAVING`).

## Constraints & Invariants

1. **A pipe query is semantically identical to its lowered standard SQL.** Lowering must preserve the computed relation; it is a rewrite, not a reinterpretation. This is property-testable against an execution oracle (a pipe query and its hand-written standard-SQL equivalent must produce equal results).
2. **No `|>` token reaches a backend that reports `supports_pipe_syntax = false`.** The lowered emission contains no pipe operators for such backends. (Mirrors the meta-language invariant that no meta `|>` reaches the database.)
3. **Every stage has a compile-time-determinable output schema.** Any construct whose output columns depend on data values (the `PIVOT`/`UNPIVOT` family) is rejected, not lowered.
4. **Deferred operators fail loud.** An unsupported or unknown pipe operator emits a diagnostic; it is never silently ignored.
5. **The meta-language `|>` is unaffected in meta contexts.** Narrowing `PipeInDataPosition` to Data-World expression positions must not change meta-expression pipe behaviour (`meta_language.md`).

## Known Divergences / Open Questions

- **Native passthrough is not emitted.** Every backend reports `supports_pipe_syntax = false`; only lowered standard SQL is generated. Native pipe emission on BigQuery/Databricks/DuckDB-via-extension is reserved by the capability flag but not implemented. Tracked in `docs/plans/` (forthcoming).
- **Pipes appended to a leading `SELECT`.** BigQuery also allows `SELECT … FROM … |> WHERE …`. smelt accepts only the FROM-first form; the SELECT-then-pipe form is unspecified surface.
- **Per-operator native capability matrix.** The `supports_pipe_syntax` flag is currently whole-feature. Backends whose native dialect supports only a subset (Spark omits `RENAME`/`CALL`/`WINDOW`/`DISTINCT`/`ASSERT`) will need a per-operator capability set before native emission can mix passthrough and lowering. Undecided until the native path is built.
- **`JOIN` lowering scope is limited to smelt model references and bare table names.** On the DuckDB backend, `|> JOIN <right_table> ON …` is lowered correctly when the right table is a `smelt.<path>` model reference or a bare identifier. Cross-engine and ephemeral-model references in JOIN right-hand positions are not yet scope-threaded. Tracked in `docs/plans/20260627-pipe_sql.md`.
- **SET/DROP/RENAME lowering is DuckDB-specific.** On the DuckDB backend, `|> SET col = expr` lowers to `SELECT * REPLACE (expr AS col) FROM (prior)`, `|> DROP col` lowers to `SELECT * EXCLUDE (col) FROM (prior)`, and `|> RENAME old AS new` lowers to `SELECT * RENAME (old AS new) FROM (prior)`. These use DuckDB column-selection extensions and are not portable to other backends; non-DuckDB backends treat SET/DROP/RENAME as unhandled and fall back to emitting verbatim pipe syntax until schema-aware lowering is implemented. Tracked in `docs/plans/20260627-pipe_sql.md`.

## References

- **Code**:
  - `crates/smelt-parser/src/parser/select.rs`, `crates/smelt-parser/src/parser/smelt_ext.rs` — query/model-body parsing entry points
  - `crates/smelt-parser/src/syntax_kind.rs`, `crates/smelt-parser/src/ast.rs` — CST node kinds and AST wrappers
  - `crates/smelt-db/src/type_inference/` — stage-by-stage schema inference (`binary.rs::walk_select_columns`, `type_context.rs`)
  - `crates/smelt-dialect/src/printer.rs`, `crates/smelt-dialect/src/dialect.rs` — lowering seam and `BackendCapabilities`
  - `crates/smelt-parser-compat/src/gaps.rs` — known parser-compat extension gaps
- **Tests**:
  - `crates/smelt-parser/tests/pipe_query.rs` — CST shape, FROM-first trigger, per-operator stage markers, error diagnostics (unknown, unsupported, malformed)
  - `crates/smelt-dialect/tests/pipe_lowering.rs` — passthrough collapse, DISTINCT lowering, EXTEND subquery wrap
  - `crates/smelt-db/tests/pipe_equivalence.rs` — DuckDB oracle equivalence for passthrough and column-editing queries
  - `crates/smelt-db/tests/pipe_scope.rs` — stage-to-stage scope threading (EXTEND/SET/DROP/RENAME undeclared-column diagnostics)
  - `crates/smelt-db/tests/pipe_diagnostics.rs` — deferred-operator hard errors (`PipeOperatorUnsupported`), stage-boundary disambiguation, and meta-pipe non-interference
  - `examples/test_workspace/models/pipe_orders.sql` — live fixture used by `example_diagnostics`
- **User docs**: *(forthcoming — `docs-site/docs/guide/` pipe-syntax page)*
- **Plans (history)**: `docs/plans/20260627-pipe_sql.md`
- **Related specs**:
  - `meta_language.md` — the meta-world `|>` operator and the `PipeInDataPosition` rule this spec narrows
  - `models.md` — model body surface (a model body may be a pipe query)
  - `architecture.md` — compilation pipeline, the dialect-printer lowering seam, `BackendCapabilities`, identity properties, the pg_query oracle
  - `types.md` — the type inference that infers each stage's schema
