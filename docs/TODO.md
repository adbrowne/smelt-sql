# TODO

## Cross-Model Type Inference

- [x] **Salsa cycle recovery for circular refs** — `salsa::cycle` recovery attributes added to `resolved_model_schema`, `typed_model_schema`, and `type_context` queries. Recovery functions return empty/default schemas and produce diagnostics. Tests cover A→B→C→A cycles and mutual dependencies.

- [x] **Cross-model type mismatch diagnostics** — Compares `model_input_constraints` against upstream `typed_model_schema`, produces warning diagnostics for incompatible types. Tests cover VARCHAR in SUM(), compatible numeric types, multiple mismatches, and chains.

- [x] **Multi-model property tests** — `prop_multi_model_type_inference` (128 cases), `prop_three_model_type_inference` (three-model chains), and `prop_join_type_inference` all implemented with `multi_model_scenario_strategy()` generator.

## Property Test Generator Coverage Gaps

The generators in `crates/smelt-db/tests/prop_helpers/generators.rs` currently only cover a small subset of what the parser and type inference support. Each item below means: add the feature to the proptest generators so it gets randomly tested against DuckDB.

### Functions

- [x] **String functions** — Added LTRIM, RTRIM, CONCAT, CHAR_LENGTH, CHARACTER_LENGTH, REPLACE, LPAD, RPAD, REPEAT, SUBSTRING, SUBSTR, SPLIT_PART, STRPOS, LEFT, RIGHT. Omitted: INITCAP (not in DuckDB, no simple equivalent), TRANSLATE, QUOTE_IDENT, QUOTE_LITERAL, POSITION (STRPOS covers same path), TO_CHAR
- [x] **Math functions** — Added POWER, EXP, LN, LOG, LOG10, LOG2, MOD, SIGN, SIN, COS, TAN, ATAN, ATAN2, SINH, COSH, TANH, PI. Omitted: ASIN/ACOS (domain-restricted, sample values cause errors)
- [x] **Temporal functions** — Added DATE_PART, DATE_TRUNC. Omitted: EXTRACT (special syntax, DATE_PART covers same path), MAKE_DATE/MAKE_TIME/MAKE_TIMESTAMP/MAKE_TIMESTAMPTZ/AGE (complex multi-arg with specific value requirements)
- [x] **Statistical/advanced aggregates** — Added STDDEV, VARIANCE, STDDEV_POP, STDDEV_SAMP, VAR_POP, VAR_SAMP, BOOL_AND, BOOL_OR, EVERY (via dialect remapping), BIT_AND, BIT_OR, BIT_XOR. Omitted: MEDIAN/MODE/PERCENTILE_CONT/PERCENTILE_DISC (DuckDB syntax differences), APPROX_COUNT_DISTINCT/ANY_VALUE/FIRST/LAST (limited DuckDB support), CORR/COVAR_POP/COVAR_SAMP/REGR_SLOPE (need 2-column aggregates)
- [x] **Null handling functions** — Added COALESCE, NULLIF
- [x] **Comparison functions** — Added GREATEST, LEAST
- [x] **JSON functions** — Canonicalized JSON functions: JSON_OBJECT (accepts json_build_object, json_object), JSON_ARRAY (accepts json_build_array, json_array), TO_JSON (accepts to_jsonb, row_to_json), JSON_EXTRACT, JSON_EXTRACT_TEXT (accepts json_extract_string, get_json_object, json_value), JSON_ARRAY_LENGTH, JSON_OBJECT_KEYS (accepts json_keys), JSON_CONTAINS. Added TO_JSON, JSON_ARRAY, JSON_OBJECT to property test generators. Not yet generated: JSON_EXTRACT, JSON_EXTRACT_TEXT, JSON_ARRAY_LENGTH (require JSON input column which the generator framework doesn't produce yet).

### Expressions and Operators

- [x] **Window functions** — Added ROW_NUMBER, RANK, DENSE_RANK, NTILE, CUME_DIST, PERCENT_RANK, LAG, LEAD, FIRST_VALUE, LAST_VALUE, NTH_VALUE with random OVER clauses (PARTITION BY, ORDER BY). Frame specs not yet covered.
- [x] **BETWEEN / IN expressions** — Added BETWEEN and IN for numeric columns (both return Boolean). EXISTS not yet covered.
- [x] **GROUP BY + window functions combined** — `QueryShape::GroupByWindow` generates `RANK() OVER (ORDER BY SUM(x))` alongside GROUP BY. Window frame specs (ROWS/RANGE/GROUPS BETWEEN) also added to window function generators.
- [x] **Scalar subqueries** — `ExprKind::ScalarSubquery` generates `(SELECT COUNT(*) FROM data)` and `(SELECT MIN(col) FROM data)`.
- [x] **Regex operators** — Parser already lexed `~`, `~*`, `!~`, `!~*`; added to `BinaryExpr::operator()`, type inference (`→ Boolean`), and generators.
- [x] **JSON operators** — Added type inference for `->`, `->>` (Text), `#>`, `#>>` (Text), `@>`, `<@` (Boolean). Added `->` and `->>` to property test generators against DuckDB.
- [x] **Mixed-type binary operations** — `BinaryOp` generator now picks two columns of different numeric types (e.g., `int_col + bigint_col`) with correct type promotion (Double > Decimal > BigInt > Integer).
- [x] **Boolean/unary expressions** — Added `ExprKind::IsNull` (`col IS NULL`/`IS NOT NULL`), `Comparison` (`col = col`, `<`, `>`, etc.), `UnaryNot` (`NOT bool_col`), `UnaryMinus` (`-num_col`), `Exists` (`EXISTS (SELECT ...)`).
- [x] **LIKE / ILIKE operators** — Added `LIKE_KW`/`ILIKE_KW` to parser lexer, parsed as binary expressions, type inference returns Boolean, generators produce `str_col LIKE '%pattern%'`.
- [x] **Additional functions** — Added STRING_AGG, ANY_VALUE, APPROX_COUNT_DISTINCT to generators. EXTRACT and MAKE_DATE/MAKE_TIMESTAMP re-enabled in `expr_kind_strategy()` in Phase 58 (April 27, 2026): the historical `FROM`-inside-EXTRACT bug had already been fixed end-to-end (parser, AST `Expr::cast`, alias extraction, type inference); a regression test in `crates/smelt-db/tests/extract_alias_extraction.rs` now pins the behaviour, and 500 prop cases run cleanly. TO_CHAR omitted (not available in DuckDB).

### Types

- [x] **Interval type** — Added `BaseType::Interval` with `CAST('1 day' AS INTERVAL)`, Arrow mapping already handles `Duration`/`Interval`.
- [x] **Time type** — Added `BaseType::Time` with `CAST('12:00:00' AS TIME)`, Arrow mapping already handles `Time32`/`Time64`.
- [ ] **Array types** — Add ARRAY literals, ARRAY_AGG, array subscript, array slice
- [ ] **Row/Struct types** — Add ROW(...) and STRUCT(...) constructors

### Syntax Variants

- [x] **PostgreSQL `::` cast syntax** — `ExprKind::Cast` now randomly chooses between `CAST(col AS TYPE)` and `col::TYPE`.
- [x] **CAST to more types** — Added INTEGER, BIGINT, DOUBLE, VARCHAR, BOOLEAN, DATE, TIMESTAMP targets
- [x] **GROUP BY / HAVING** — `QueryShape::GroupByHaving` generates multi-column GROUP BY with HAVING predicates via `generate_having_predicate()`
- [x] **DISTINCT / DISTINCT ON** — `QueryShape::Distinct` generates `SELECT DISTINCT` queries.

### Deferred

- [ ] **SET operations (UNION/INTERSECT/EXCEPT)** — Type coercion rules across union branches, requires different query shape
- [ ] **QUALIFY clause** — Parsed and rewritten by dialect printer, needs query-shape-level testing
- [ ] **PIVOT/UNPIVOT** — Complex syntax, rare usage, needs dedicated generator shape
- [ ] **Lambda expressions** — No type inference support yet, needs inference work first
- [ ] **Ordered-set aggregates (MEDIAN/MODE/PERCENTILE_CONT/PERCENTILE_DISC)** — DuckDB syntax differences for ordered-set aggregates
- [ ] **Two-column aggregates (CORR/COVAR_POP/COVAR_SAMP/REGR_SLOPE)** — Needs multi-column aggregate generator support
- [ ] **Aggregate FILTER clause** — `COUNT(*) FILTER (WHERE cond)`, parsed but not generated
- [ ] **WITHIN GROUP (ORDER BY)** — For STRING_AGG/LISTAGG, parsed but not generated
- [x] **EXTRACT parser support** — `EXTRACT(YEAR FROM col)` and `MAKE_DATE`/`MAKE_TIMESTAMP` are now exercised end-to-end by `expr_kind_strategy()`. Phase 58 (April 27, 2026) confirmed the historical `FROM`-inside-EXTRACT bug had already been fixed across the parser, AST, alias extraction, and type inference; a dedicated regression test (`crates/smelt-db/tests/extract_alias_extraction.rs`) was added before re-enabling the generator entries.

## smelt test

- [ ] **Graph-aware selectors for `smelt test --select`** — Currently `--select` uses substring matching on test names. Should support the same graph-aware selector syntax as `smelt run` (e.g., `tag:X`, `+model_name`, `model_name+`).

### Known DuckDB Incompatibilities (discovered during generator expansion)

- **INITCAP**: Not available in DuckDB (no simple equivalent)
- **EVERY**: Not natively in DuckDB; dialect printer remaps to BOOL_AND (now tested)
- **LEFT/RIGHT**: Parser keyword conflict fixed; now tested
- **ASIN/ACOS**: Domain-restricted to [-1,1]; sample values (42, 100, etc.) cause errors
- **SIGN**: DuckDB and Spark both return TINYINT (SmallInt) regardless of input type; fixed smelt inference to match
- **`~*`, `!~`, `!~*`**: DuckDB only supports `~` (case-sensitive regex); `~*` (case-insensitive), `!~`, `!~*` are not available. Generator uses `~` only.
- **TO_CHAR**: Not available in DuckDB (PostgreSQL-only). Omitted from generators.

## smelt.as_struct follow-ups (Phase 38 deferred)

- [x] **Relocate as_struct lowering helper to smelt-planner** — Phase 42 (2026-04-25): `as_struct_to_sql` and `backend_supports_struct_literal` moved to `crates/smelt-planner/src/lowering/as_struct.rs`. The smelt-db site re-exports them so existing call sites and tests keep working; the lowering helper is now the canonical production location.
- [x] **Wire as_struct into `format_plan` / SQL emission** — Phase 55 (2026-04-27): `PrintContext` in `smelt-dialect` now carries optional `smelt_as_struct` and `smelt_fn` closure fields. `SMELT_AS_STRUCT_CALL` and `SMELT_FN_CALL` nodes are expanded during SQL printing; `SqlCompiler` in `smelt-cli` wires up both closures from the TypeContext and function body map. See commit 6d6e5b1.
- [x] **as_struct in function bodies that declare no backends** — Phase 42 (2026-04-25): `as_struct_backend_diagnostics_for_file` now consults `project_active_backends`, a new Salsa-tracked query that parses `smelt.yml`'s `targets:` map. When a function's `BackendSet` is `All` (no explicit `backends:` frontmatter), the diagnostic intersects against the workspace's active backends and fires when any of them lacks struct-literal capability.
- [x] **ABS(Decimal) returns Double** — Phase 53 (2026-04-26): divergence registered in `crates/smelt-db/tests/prop_helpers/divergences.rs` as `abs_decimal` and `abs_decimal_schema_resolved`. Property-test regressions file updated. See commit 0ee244c.

## VALUES / sources resolver — follow-ups from 2026-05-28 plans

Reviewer-raised gaps adjacent to `docs/plans/20260528-source-leaf-name-collision.md` and `docs/plans/20260528-values-derived-table-typing.md`. Both are worth probing but neither blocks the closed UNKNOWN clusters.

- [x] **JOIN-side parallel of the `smelt.sources.*` shadow.** Closed incidentally by the generator-emission Phase 2 fix (commit `37bc3845`): `resolve_table_ref_schema` was updated to route `smelt.<path>` value-form JOIN references through the same `resolved_columns_for_path` method that `process_table_ref_pure` uses. The hand-authored-first ordering in that method, combined with W3's pre-existing collision discard, makes the same leaf-collision class unreachable in JOIN context. No targeted test was added — the existing per-entity-fixture test in `crates/smelt-db/tests/source_leaf_collision.rs` exercises the FROM-clause path; a JOIN-clause analogue is a worthwhile defensive addition but not blocking.
- [ ] **VALUES-body CTE arity check.** `check_cte_alias_arity` (`crates/smelt-db/src/type_inference/values.rs`, added in commit 47d874a4) returns early when the CTE body isn't a `SELECT`, so `WITH cte(a) AS (VALUES (1, 2)) SELECT * FROM cte` is silently exempt from the `AliasColumnArityMismatch` diagnostic that the SELECT-body case enforces. Symmetric coverage is a small extension — the underlying VALUES column count is already available via `Subquery::values_clause()` (commit 86f755fe) and `infer_values_columns` (commit 01fc027f). Mirror the SELECT-body path's arity check.

## P7c (diagnostic-parity) — PAUSED for a design decision (2026-06-03)

`docs/plans/20260531-diagnostic-parity.md` P7c (config-loader build-path execution)
is **partially landed and the tree is red at pre-flight** (`example_diagnostics::
meta_config_clean_workspace` fails). Commit `58c2fcd4` shipped the P7c detector
(bare `List<…>`/`Map<…>` loaders in scalar position → `MetaListInScalarPosition`),
the **List<…>** build-path lowering, and `meta_config_e2e.rs` — but left the
`examples/meta_config` models (`cohorts.sql`, `tenants.sql`) in their now-forbidden
bare `SELECT smelt.config.load_yaml(...)` form, so analysis is red.

**List loader is finishable now:** `cohorts.sql` rewrites cleanly to a consuming
form (`reduce(map(load_yaml('configs/cohorts.yaml', List<…>), fn c => c.region),
concat_with(', '))`) — verified analysis-clean; the List form already builds +
executes (`meta_config_e2e.rs`).

**Map loader is blocked — needs a human decision.** A `Map<Text, …>` loader value
has **no parser-supported in-model consumer**: `load_yaml(...) |> m => m.keys()`
fails ("pipe RHS must be a function call" / "Expected RPAREN, found ARROW") and
`load_yaml(...).keys()` fails ("Expected RPAREN, found DOT"). Loader-result
consumption through SELECT expressions is documented as deferred wiring
(`crates/smelt-cli/tests/example_diagnostics.rs:1531-1534`). So the bare Map form
(now forbidden by the P7c detector) has no clean replacement. The Map root-shape
loader is woven through docs: `docs-site/docs/meta-language/{maps.md,config-loaders.md,
reference.md}` and the canonical `examples/meta_config/models/tenants.sql`.

Decision needed (pick one direction):
- **(A) Drop Map-in-model.** Accept that a `Map<…>` loader cannot be consumed in a
  model SELECT today; convert/remove `tenants.sql`, walk back the `maps.md` /
  `config-loaders.md` / `reference.md` worked examples, and record a Known
  Divergence in `meta_config_loading.md`. (If `tenants` is converted to a clean
  `List` loader, `meta_config` could come **off** `KNOWN_UNBUILDABLE` entirely.)
- **(B) Wire Map consumption.** Implement a parser/analyzer-supported binding form
  (e.g. `m.entries()` / `m.keys()` reachable from a SELECT expr) so a Map loader
  can be consumed, then give `tenants.sql` a clean consuming form. Larger scope.
- **(C) Exempt bare Map/List loaders from the detector** when no consumer exists —
  contradicts the just-committed P7c forbid-bare-loaders design decision
  (`3c58cd29`), so only with Andrew's sign-off.

## Refresh-as-maintenance-plan: ratification queue (2026-07-06)

The research directory `docs/research/20260705-refresh-as-maintenance-plan/` is complete and
awaits human review before the spec work starts. The queue of decisions (with recommendations
already argued) is `09-spec-readiness.md` §1 — headline items:

- [ ] Ratify the five design-fork resolutions in `03-design-forks.md` (G-11 clamp wrap, G-10
      composite keys, FIX-2 delta-channel wiring, BigInt truncation fix-first, G-06 clamp subsumed)
- [ ] Decide mode-name sugar vs removal (`04-knobs.md` K1) and the retention trust default
      (`05-source-properties.md` P5) — the two flagged-contentious surface choices
- [ ] Point the property loop at the `cumulative_aggregate`/`merge_into` MERGE path (the only
      live path where a ledger obligation can be violated today; cells written in
      `06-proof-obligations.md` §5 / `07-example-catalogue.md` closing list)
- [ ] Then `/smelt:spec` per the spec-diff map in `09-spec-readiness.md` §4

Done alongside: M0 of `08-code-placement.md` (dead CLI incremental executors deleted, `7d1b4f17`).
