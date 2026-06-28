---
feature: testing
status: experimental
last_reviewed: 2026-06-26
owners: [andrew]
---

# Testing

> **What this is.** A normative spec for the smelt testing framework — the `smelt.test` declaration kind, mock data injection via `PASSING`, CTE isolation via the `#` operator, assertion semantics, and property-based test behavior.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.
>
> **Naming history.** Earlier cross-references (in `architecture.md` Known Divergences, `functions.md`, `seeds.md`, `sources.md`) called this file `tests.md`. The canonical name is `testing.md`; the older name is no longer current.

## Surface

### Test declaration format

A test is a `smelt.test` declaration — a peer of `smelt.define` and `smelt.extern` on the kind axis (`architecture.md` §"Kind is determined by file format and content"). It declares an assertion query, mock input data for that query's dependencies, and the expected output rows:

```sql
smelt.test daily_revenue_basic AS (
    SELECT order_date, total_revenue
    FROM smelt.daily_revenue
)
PASSING orders AS (
    {order_id: 1, amount: 100.0, order_date: '2024-01-15'},
    {order_id: 2, amount: 200.0, order_date: '2024-01-15'}
)
EXPECT (
    {order_date: '2024-01-15', total_revenue: 300.0}
)
```

Grammar:

```
[<frontmatter block>]
smelt.test <name> AS ( <select> )
  [ PASSING <dep> AS ( <rows> ) ]...
  EXPECT ( <rows> )
  [;]
```

- **`<name>`** — the test's identity. The full `smelt.<path>` address is the declaring file's directory joined with `<name>` (e.g. `tests/marts/file.sql` declaring `smelt.test customers_no_nulls` → `smelt.tests.marts.customers_no_nulls`). A test is addressable for tooling (LSP, selectors) but is **never** valid in a `TableExpr` position — it produces no database object (`architecture.md`).
- **`<select>`** — an arbitrary assertion query. It references the model(s) under test via `smelt.<path>`, and may target a model's internal CTE via the test-local `#` operator (below). There is no separate `model:` field; the model under test is whatever the query references.
- **`PASSING <dep> AS ( <rows> )`** — mock data. `<dep>` is the bare address path of a `smelt.<path>` dependency the assertion query reaches (the `smelt.<path>` minus the leading `smelt.`, e.g. `orders` or `silver.orders`). `<rows>` is a comma-separated list of record literals. Zero or more `PASSING` clauses. This reuses the function-call `PASSING` machinery (`functions.md` §"`PASSING` clauses"); a test substitutes a table dependency, where a function substitutes a fragment parameter.
- **`EXPECT ( <rows> )`** — required. The expected output rows, as a comma-separated list of record literals.

`PASSING` and `EXPECT` are **context-sensitive keywords**, recognised only at these positions in a `smelt.test` declaration (the same positional rule that makes `PASSING` a keyword only after a smelt function call — `functions.md`). Everywhere else they are ordinary identifiers.

Tests are discovered by the same project-wide scan as every other declaration (discovery is not gated by `paths:`; see `architecture.md` §"Resolution"). A `smelt.test` may be co-located in a multi-section file alongside the models it tests, or placed in a separate `tests/` directory — which needs no special registration, as every non-excluded directory is scanned.

### Frontmatter knobs

A Layer-2 YAML frontmatter block may precede the `smelt.test` declaration (the same per-declaration frontmatter mechanism that attaches to models and `smelt.define`s — `architecture.md`). Only two keys apply:

| Key | Type | Required | Default | Description |
|-----|------|----------|---------|-------------|
| `check_order` | bool | no | `false` | If `true`: compare rows positionally. If `false`: compare as sets. |
| `cases` | integer | no | `10` | Number of property-based test iterations (used when row literals omit columns). |

The model under test, the mock inputs, and the expectations all live in the grammar (`<select>`, `PASSING`, `EXPECT`) — there are no `model:`, `target_cte:`, `inputs:`, or `expect:` frontmatter keys.

### CTE references — the `#` operator

Within a `smelt.test` body, a model's internal CTE is addressable as `smelt.<model_path>#<cte_name>`:

```sql
smelt.test daily_agg_rollup AS (
    SELECT day, revenue
    FROM smelt.daily_revenue#daily_agg
)
PASSING orders AS (
    {order_id: 1, amount: 100.0, order_date: '2024-01-01'}
)
EXPECT (
    {day: '2024-01-01', revenue: 100.0}
)
```

The `#<cte>` suffix selects one CTE within the referenced model. The CTE's upstream chain — every CTE it depends on, directly and transitively — runs **as written**; only the model's external `smelt.<path>` dependencies are mockable via `PASSING`. This lets a long CTE chain be tested one transformation at a time without mocking the entire upstream graph.

`#` is **test-local**: a `smelt.<model>#<cte>` reference is valid **only** inside a `smelt.test` body. The same reference in any other position (a model body, a `smelt.define` body) is a hard error (`CteRefOutsideTest`). A model's CTEs remain an internal implementation detail everywhere except the tests that assert on them; this keeps CTE names out of the public `smelt.<path>` surface and lets a model's internals be refactored without breaking other models.

### Record-literal value → SQL type coercion

Row literals are meta-language record literals; each scalar value coerces to a SQL type for the mock fixture:

| Literal value | SQL type |
|---------------|----------|
| Integer (`42`) | `INTEGER` |
| Float (`3.14`) | `DOUBLE` |
| Decimal string (`'300.00'`, a numeric string with a decimal point and no exponent) | `DECIMAL` |
| String `'YYYY-MM-DD'` pattern | `DATE` |
| String `'YYYY-MM-DD HH:MM:SS'` or `'YYYY-MM-DDTHH:MM:SS'` pattern | `TIMESTAMP` |
| Other string | `VARCHAR` |
| Boolean (`true` / `false`) | `BOOLEAN` |
| Null (`null`) | `NULL` |

Strings that match the `YYYY-MM-DD` pattern are automatically cast to `DATE`; strings whose first 19 characters match `YYYY-MM-DD HH:MM:SS` (with a space or `T` separator, and an optional fractional-seconds suffix) are cast to `TIMESTAMP`. If you need a string that looks like a date or timestamp, this is a known limitation — no escape mechanism exists today.

### Comparison behavior

- **Columns**: Only columns listed in `EXPECT` rows are compared. Extra columns in the actual output are ignored.
- **Floating point**: When the actual column is `FLOAT`/`DOUBLE`, values within `1e-6` of each other are treated as equal.
- **Decimal**: When the actual column is `DECIMAL`, values are compared **exactly by numeric value** — no `1e-6` tolerance. The `1e-6` tolerance applies only to `FLOAT`/`DOUBLE` actuals. This keeps money columns (`SUM(amount)` over a `DECIMAL` source, which yields `DECIMAL`) from passing on a sub-cent discrepancy. An expected value written as a float literal (e.g. `300.0`) is compared against a `DECIMAL` actual by its exact numeric value, so `300.0` equals `300.00` but `300.001` does not.
- **Row order**: When `check_order: false` (default), row order does not matter; both actual and expected are compared as multisets. When `check_order: true`, rows are compared positionally.

### Selector behaviour

`smelt test --select <expr>` matches `<expr>` as a **plain substring** against test names. The full `tag:` / `path:` / `+upstream` / `downstream+` selector grammar in `model_selection.md` does **not** apply to `smelt test`; it applies to `smelt run`, `smelt build`, and `smelt explain`. Substring match is asymmetric with the rest of the CLI and is tracked as a divergence in `model_selection.md` Known Divergences (and in this spec's Known Divergences below); aligning the two is open work.

### Diagnostic codes (owned by this spec)

| Code | Severity | Trigger |
|---|---|---|
| `UnknownTestInput` | Error | A `PASSING` clause names a `<dep>` that is not a compiled dependency of the assertion query (catches a typo that would otherwise be silently replaced with an empty CTE → a false-green test). Anchored at the offending name. |
| `UnknownTestCte` | Error | A `smelt.<model>#<cte>` reference names a `<cte>` that does not exist in the referenced model's `WITH` clause. Anchored at the `#<cte>` suffix. |
| `CteRefOutsideTest` | Error | A `smelt.<model>#<cte>` reference appears outside a `smelt.test` body. Anchored at the `#` operator. |

## Semantics

### Execution model

`smelt test` executes each test's assertion query against a **fresh in-memory DuckDB instance**. No connection to the project's configured target database is made. The lifecycle per test:

1. Resolve the assertion query `<select>`, inlining the body of every model it references (and, for a `#<cte>` reference, the target CTE and its internal CTE chain).
2. Substitute each dependency named in a `PASSING` clause with a CTE containing the mock rows.
3. Replace any external dependency **not** named in a `PASSING` clause with an empty CTE (zero rows).
4. Execute the rewritten query in in-memory DuckDB.
5. Compare actual output rows against the `EXPECT` rows per the comparison rules above.
6. Report PASS or FAIL.

### Full-query tests

When the assertion query references models with no `#` operator, each referenced model's SQL is inlined and mock data is substituted for every external `smelt.<path>` dependency named in a `PASSING` clause. Dependencies not named in any `PASSING` clause are replaced with empty CTEs (zero rows).

**Unmatched `PASSING` names are diagnosed.** Every `PASSING <dep>` must name a compiled dependency the assertion query actually reaches. A `PASSING` clause that matches no dependency is reported via `UnknownTestInput` and fails the test loudly, rather than silently creating an unused mock CTE. This catches the typo class (`order` vs `orders`) that would otherwise leave the real dependency mocked as an empty CTE — a false green in a testing tool.

If an inlined model body already begins with its own `WITH` clause (after any leading line comments), the mock CTEs are injected **inside** that existing `WITH` rather than prepended as a second one — `WITH <mock_ctes>, <model's existing ctes> ...` — so the compiled test SQL remains a single, well-formed query. Bodies without a leading `WITH` get a fresh `WITH <mock_ctes>` prefix.

### CTE-level tests

A `smelt.<model>#<cte>` reference in the assertion query targets a CTE within the model, but the mock boundary is still the **model's external dependencies** — the `smelt.<path>` inputs feeding the CTE chain — not the model's internal CTEs. smelt:

1. Extracts the target CTE and the chain of model-internal CTEs it depends on (directly and transitively) from the model's `WITH` clause.
2. Substitutes the model's external dependencies (the `smelt.<path>` refs reachable from the target CTE's dependency chain) with mock data from the `PASSING` clauses.
3. Executes the assertion query with the target CTE's internal chain running **as written**.
4. Compares the output against `EXPECT`.

The internal CTEs — both the target CTE and every CTE it depends on, direct and transitive — execute exactly as written; only the model's external inputs are mockable. `PASSING` names in a CTE-level test are therefore the same bare address paths as in a full-query test (the model's `smelt.<path>` dependencies), never internal CTE names. A `#<cte>` naming a CTE absent from the model is reported via `UnknownTestCte`.

### Property-based tests

A test is treated as property-based when one or more columns of an input row are **omitted from its record literal**. For each of the `cases` iterations:

1. smelt infers the type of each omitted column from the model's type checker.
2. Generates a random value of the appropriate type.
3. Executes the test with the augmented input data.
4. Checks that specified output columns in `EXPECT` match (unspecified output columns are ignored).
5. Verifies the query does not crash.

Each iteration uses a different random seed derived from the test's global seed. If any iteration fails, the failure report includes the random seed that caused it for reproduction.

### Tests always use DuckDB

`smelt test` always runs against in-memory DuckDB, regardless of the project's configured targets. Tests on Spark-only projects are not validated against Spark semantics. This is a known design gap.

## Design

**Test is a declaration kind, not a materialization.** A `smelt.test` declaration is signalled by a `smelt.<noun>` keyword, exactly like `smelt.define` and `smelt.extern` — kind lives on the kind axis (`architecture.md`), not smuggled in through a `materialization:` frontmatter flag. A test produces no output and nothing in the DAG depends on it; it is not a persistence strategy, so it has no business being a `materialization` value. *Modelling a test as a `materialization` value* (a `materialization: test` flag on a bare SELECT) was rejected because it makes a test the only kind signalled by a frontmatter flag rather than by file format or a `smelt.<noun>` keyword the way every other kind is — an asymmetry the keyword form removes. The parser, type checker, and LSP still handle the assertion query with the same machinery they use for any model SELECT; only the kind signal and the input/expectation surface live in the grammar.

**`expect`/`inputs` in grammar, not YAML.** Expectations and mocks live in `EXPECT (...)` and `PASSING (...)` clauses rather than YAML keys. This reuses the function-call `PASSING` machinery (a test mocks a table dependency the way a function call binds a fragment parameter) and keeps the whole test — query, mocks, expectations — in one SQL-native form the type checker reads end-to-end. YAML `inputs:`/`expect:` maps were rejected because they put the test data in a second grammar the type checker had to re-derive types for, divorced from the query they parameterise.

**Mock by dependency name.** A `PASSING` clause is keyed by the dependency's bare address path (the `smelt.<path>` minus the `smelt.` prefix), not by some other handle. This is the same key shape for full-query and CTE-level tests: a CTE-level test targets a CTE but mocks the model's external dependencies, so the names mirror the addresses that appear in the SQL body, and tests read naturally without an extra lookup table.

**`#` for CTE isolation, test-local, distinct from `.`.** Reaching a model's internal CTE needs a syntax, and reusing the address dot (`smelt.daily_revenue.daily_agg`) would collide with directory-path addressing — that address already means "model `daily_agg` in directory `daily_revenue/`". A distinct `#` separator keeps the two meanings unambiguous without overloading the resolver. Scoping `#` to `smelt.test` bodies keeps a model's CTEs an internal detail everywhere else: a model cannot depend on another model's CTE, so internals stay refactorable. Making CTE addressing a general, project-wide capability (so any model could `FROM smelt.other#cte`) is a larger feature with its own encapsulation trade-offs and is out of scope here. Pulling CTEs into bare scope (`FROM daily_agg`) was rejected because a bare name does not say which model's CTE it is — it would need a separate model-under-test anchor and could collide with a mocked dependency or a real table.

**Set comparison by default.** `check_order: false` is the safe default. Most models do not produce ordered output, and ordering in SQL is non-deterministic unless an `ORDER BY` is present. Requiring `check_order: true` explicitly for ordered output avoids brittle tests that depend on DuckDB's internal sort order.

## Constraints & Invariants

1. **Tests run in-memory on DuckDB.** No connection to the project's configured target is made during `smelt test`.
2. **`smelt.test` declarations produce no database object.** They are never materialized by `smelt run` or `smelt build`, are excluded from execution runs, and are not valid in `TableExpr` positions.
3. **`EXPECT` is required.** A test with no `EXPECT` rows is invalid.
4. **`PASSING` names are bare dependency address paths.** For both full-query and CTE-level tests, each name is the bare address path of a dependency the assertion query reaches — the `smelt.<path>` minus the leading `smelt.` (e.g. `orders` or `silver.orders`). A CTE-level test still mocks the model's external dependencies, not its internal CTEs. A name that matches no compiled dependency is reported via `UnknownTestInput`.
5. **`#` is test-local.** A `smelt.<model>#<cte>` reference is legal only inside a `smelt.test` body; elsewhere it is `CteRefOutsideTest`. A `#<cte>` naming a non-existent CTE is `UnknownTestCte`.
6. **Column comparison uses only `EXPECT` columns.** Extra actual columns are never treated as failure.

## Known Divergences / Open Questions

- **Unlisted input dependencies replaced with empty CTEs.** Dependencies not named in a `PASSING` clause receive zero rows. This is intentional but easy to miss — a query that JOINs an unmocked dependency will silently get no rows from it.
- **Date string auto-cast is opt-out impossible.** Strings matching `YYYY-MM-DD` are always cast to `DATE`. There is no way to pass a date-shaped string as `VARCHAR` in a row literal.
- **Property-based test column discovery.** For CTE-targeted tests (a `#<cte>` reference), omitted columns are detected by walking the CTE body against the user-provided `PASSING` rows; columns referenced in the CTE but absent from the mocks trigger the property loop. Type inference for the missing columns falls back to `Text` when no other evidence is available. Behavior when type inference is unavailable is undefined.
- **Full-query property-test column detection.** For a full-query test (no `#<cte>` anchor), there is no CTE anchor for omitted-column detection. Full-query tests always run one-shot regardless of `cases`; the `cases` field has no effect. Tracked in `docs/plans/20260605-property-test-dispatch-and-week-start.md`.
- **`cases: 0` behavior.** Setting `cases: 0` when row literals have omitted columns may result in no iterations. Whether this is PASS or an error is undefined.
- **Spark test gap.** Tests always run on DuckDB. Spark-specific function behavior (MERGE semantics, Parquet type handling) cannot be tested with `smelt test`.
- **Project-wide CTE addressing.** `#` is test-local. Whether a model's CTEs should be addressable project-wide (so any model could read another's intermediate CTE — interesting for smelt's cross-model optimization story) is open, and would need its own spec covering the encapsulation and address-collision trade-offs.

## References

- **Code**:
  - `crates/smelt-parser/src/` — `smelt.test` declaration grammar, `PASSING`/`EXPECT` clauses, the `#` CTE-reference operator
  - `crates/smelt-core/src/metadata.rs` — `TestConfig` (`check_order`, `cases`), `ColumnTest`
  - `crates/smelt-core/src/resolver.rs` — `EntityKind::Test`
  - `crates/smelt-cli/src/commands/` — `smelt test` command implementation
- **User docs**:
  - `docs-site/docs/guide/testing.md`
- **Related specs**:
  - `models.md` — materialization (storage) and refresh axes; the kind axis that `smelt.test` joins
  - `functions.md` — `smelt.define`, `smelt.extern`, and the `PASSING` clause machinery a test reuses
  - `architecture.md` — the kind axis and `smelt.<path>` addressing
  - `seeds.md` — seeds as mock data sources in tests
  - `cli.md` — `smelt test` command behavior, exit codes
