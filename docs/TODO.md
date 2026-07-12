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

## Refresh-as-maintenance-plan: ratification queue (2026-07-06) — CLOSED 2026-07-07

All items done: decisions 1–11 ratified 2026-07-06 (`09-spec-readiness.md` §1); F4 (`25c04a70`)
and F1/G-11 (`770c77f1`) landed; G-12 probed (double-fold pinned, `b1ead60f`); the spec-diff map's
headline rows landed (`maintenance_plan.md` new `3f65a671`; `models.md` rewrite `aa326a3f`;
`sources.md` `fb9a5977`); M0 deleted (`7d1b4f17`). A four-way spec-vs-research review (2026-07-07)
confirmed the landed specs faithful and identified the residue, now queued as autonomy-loop
sub-plans: `docs/plans/20260707-maintenance-plan-spec-alignment.md` (SA1–SA5) and
`docs/plans/20260707-maintenance-plan-impl.md` (MP1–MP16). Pre-framework registry rows
(keyed-collapse K3–K6, keyed-time-partitioned, L4 batched/versioned/mv) superseded — see the
master's 2026-07-07 note.

## 2026-07-12 — ON-join `SELECT *` schema expansion drops the right side

Found while fixing NATURAL/USING join-star dedupe (parser-gap-closure). `model_schema`'s
wildcard expansion (`row_extensions` in `crates/smelt-db/src/queries/schema.rs`) now covers
NATURAL and USING joined refs with join-shared columns deduped (DuckDB-verified [x, y, z]),
but ON-joined refs are still not expanded at all: `SELECT * FROM smelt.models.a JOIN
smelt.models.b ON a.x = b.x` infers only a's columns, where DuckDB yields all four
[x, y, x, z] with the shared name duplicated. Expanding ON joins means admitting duplicate
column names into inferred schemas (find-by-name consumers, LSP completion, input-constraint
keying all assume unique names today), so it needs its own pass. Current behavior is pinned
by `on_join_star_current_behavior_left_side_only` in
`crates/smelt-db/tests/integration/join_star_schema.rs` — update that test when fixing.
Related limitation to fix in the same pass: `SharedWithPrior` (NATURAL) dedupes against ALL
prior refs' names, but DuckDB's NATURAL binds only to the adjacent join operand — in
`FROM a, b NATURAL JOIN c`, a column shared between a and c (but not b) is wrongly dropped
where DuckDB would carry the duplicate.

## 2026-07-12 — TABLESAMPLE/PIVOT/UNPIVOT vs alias ordering (parser + printer)

Found during the parser-gap-closure review (PR #158, derived-table alias fix f68ebd86).
smelt's parser accepts only `base TABLESAMPLE(...) AS alias` and the printer emits that
same order — but real DuckDB v1.5.4 REJECTS it and requires `base AS alias TABLESAMPLE(...)`
(oracle-verified). Pre-existing parser grammar bug (`parser/select.rs` parses TABLESAMPLE
before alias); previously masked because the old printer dropped both clauses, now live:
the printer emits DuckDB-invalid SQL whenever TABLESAMPLE/PIVOT/UNPIVOT co-occurs with an
alias. Not exercised by the current corpus/seed gates. Fix: swap grammar+printer to
alias-first order and add a seed line; PIVOT/UNPIVOT ordering unprobed, verify while there.

## 2026-07-12 — External-ledger `smelt_fails_unclassified` triage (236 → root-caused)

Triaged all `smelt_fails_unclassified` entries (236 measured via re-parse against the live
ledger; the task brief that kicked this off estimated 237, likely a stale count from before
an unrelated prior commit) in
`crates/smelt-parser-compat/tests/corpus/external_ledger.toml` by re-parsing each corpus
statement and bucketing by first-error signature + syntactic pattern (script discarded,
see `docs/plans/` for methodology if resurrected). Three genuinely small parser/lexer gaps
were fixed with red-green tests (26 ledger entries closed); the remaining 209 entries were
reclassified into 56 named root-cause categories (`gaps.rs`-style vocabulary) with an
honest note per category — no fabricated root causes, each was independently confirmed by
direct parse inspection.

**Fixed this pass:**
1. Double-quoted-identifier aliases (`AS "median_delay"`) — `parse_select_item` and
   `parse_table_ref`'s explicit-`AS` branch only accepted `IDENT`, not the `STRING` token
   smelt's lexer produces for double-quoted text (`consume_string` doesn't distinguish `'`
   from `"`). Fixed in `parser/select.rs` + `parser/mod.rs` (`at_quoted_ident_alias`) +
   `ast.rs` (`alias()`/`alias_token_text()`) + `printer.rs` (re-quote on print — the
   printer must not silently drop the quotes DuckDB/PostgreSQL require for aliases that
   need them). Closed 21 entries. Surfaced a second, unrelated printer bug in the same
   pass (`Display for Subquery` drops a VALUES-clause CTE body) — reclassified as
   `roundtrip_mismatch`, not fixed (see below).
2. `FIRST(x)`/`LAST(x)` as aggregate function names — `FIRST_KW`/`LAST_KW` weren't in
   `at_keyword_as_function_name`'s allowlist (unlike `LEFT`/`RIGHT`). One-line addition.
   Closed 3 entries (a 4th, `FIRST(i ORDER BY i)`, still needs `aggregate_call_order_by_clause`).
3. Leading-dot decimal literals (`.5`, `.000_005`) — the lexer's digit dispatch never
   tried `consume_number` when the current char was `.`; added a guarded match arm before
   the `...`/`..` spread-operator arms. Closed 3 entries.

**Not fixed — explicitly out of scope this pass** (real, would need `smelt-db` nullability/
join-topology work, not just parser grammar): `FROM t "quoted"`-style comma-joins
(`implicit_cross_join_comma_syntax`) would need a "no explicit modifier ⇒ inner join"
default in `JoinClause::join_type()` to special-case comma-joins as cross joins, which is
inference-semantics territory, not a grammar-only change.

**Top of the follow-up list** (full 56-category table below; effort is a rough guess, not
sized): `NOT IN` / `NOT ILIKE` (`not_prefixed_binary_operator`, 6 entries) is surprisingly
broken — `SELECT 2 NOT IN (2, 3)` alone fails — and is likely a very small fix (binary
operator precedence/lookahead in `expr.rs`), probably the single highest-leverage item here.
`quoted_table_name_in_from` (`FROM "flights"`) is the same root cause as fix #1 above but in
`parse_table_ref`'s primary-identifier path rather than the alias path — also likely small,
and the resulting scope (any double-quoted table/schema name) is probably underrepresented
in this bucket's raw count (1) because most instances get preempted by an earlier error in
the same statement.

Counts below are the entries *this triage pass* moved into each category — categories that
already had entries before the pass (e.g. `file_glob_or_path_literal_from`,
`sqllogictest_template_placeholder`) have higher ledger totals; the ledger itself is the
authoritative count.

| Category | Count | Effort guess | DuckDB-relevant? |
|---|---|---|---|
| `implicit_cross_join_comma_syntax` | 25 | Medium (join-topology semantics, see above) | Yes |
| `sqllogictest_template_placeholder` | 23 | N/A (not real SQL, test-harness artifact) | No |
| `postgres_typed_literal_prefix` | 14 | Medium (extend typed-literal-prefix keyword set) | No |
| `postgres_geometric_operators` | 11 | Medium (new operator tokens + geometry types) | No |
| `file_glob_or_path_literal_from` | 10 | Medium (string-literal-as-table-source grammar) | Yes |
| `aggregate_call_order_by_clause` | 8 | Medium (WITHIN GROUP / agg ORDER BY grammar) | Yes |
| `at_time_zone_or_time_tz_type` | 7 | Small–Medium | Partial |
| `postfix_dot_field_access_on_parenthesized_expr` | 7 | Medium | Partial |
| `not_prefixed_binary_operator` | 6 | **Small** (see above — top pick) | Yes |
| `jsonb_operators` | 6 | Medium | No |
| `sql_json_constructor_functions` | 6 | Medium | Partial |
| `postgres_interval_range_qualifier` | 5 | Small–Medium | No |
| `range_keyword_as_identifier_or_function` | 5 | Small (same shape as the FIRST/LAST fix) | Yes |
| `row_value_comparison` | 5 | Medium (row-constructor comparison grammar) | Yes |
| `postgres_table_inheritance_wildcard` | 4 | Small (grammar) but pg-only, low value | No |
| `star_exclude_or_rename_clause` | 4 | Medium (nested contexts + trailing comma) | Yes |
| `select_into_clause` | 3 | Medium | Partial |
| `double_equals_operator` | 3 | **Small** (lexer: `==` as EQ alias) | Yes |
| `group_by_tuple_expression` | 3 | Medium | Yes |
| `numeric_literal_followed_by_ident_no_space` | 3 | N/A (intentional fail-loud, see lexer.rs) | No |
| `malformed_corpus_statement` | 3 | N/A (extraction artifact, not real SQL) | No |
| `quoted_table_name_in_from` | 1 (undercounted, see above) | **Small** (same fix shape as #1) | Yes |
| everything else (33 categories, ≤2 entries each) | 41 | Mixed | Mixed |

Ledger delta this pass: 236 `smelt_fails_unclassified` → 0 (26 entries closed outright — the
27th fix, a leading-dot-decimal CTE, surfaced the unrelated printer bug above and was
reclassified rather than closed; 209 entries redistributed across the other 55 named
categories in the table above, which together with `roundtrip_mismatch` account for all
236). `cargo test -p smelt-parser-compat --test external_corpus` stays green throughout
(`ledger_has_no_stale_entries` re-validated after every batch of edits).

## 2026-07-12 — Residue from walrus named-arg work (PR #158, 47e74c1c)

- `NULL::VARCHAR` (top-level cast of NULL, and casts inside named-arg values) fails to
  parse — likely a parse_expression precedence gap around `::` on NULL/named-arg value
  positions; fails standalone too, pre-existing. 1 ledger entry recategorized under
  `smelt_fails_unclassified` carries the actual error (`Expected expression, found DOUBLE_COLON`).
