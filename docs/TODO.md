# TODO

## Cross-Model Type Inference

- [ ] **Salsa cycle recovery for circular refs** — Currently, if model A refs model B which refs model A, Salsa will panic with a cycle error. Add `salsa::cycle` recovery attributes to `resolved_model_schema`, `typed_model_schema`, and `type_context` queries to return empty/default schemas gracefully and produce a diagnostic.

- [ ] **Cross-model type mismatch diagnostics** — When a downstream model uses a column in a type-incompatible way (e.g., `SUM(col)` where upstream infers `col` as VARCHAR), produce a warning diagnostic in the LSP. Compare `model_input_constraints` against actual upstream `typed_model_schema`.

- [ ] **Multi-model property tests** — Extend `type_property_tests.rs` to generate two-model chains (model_A with typed CTE columns, model_B refs model_A) and verify types match DuckDB output. Requires setting up Salsa Database with multiple models in the property test.

## Property Test Generator Coverage Gaps

The generators in `crates/smelt-db/tests/prop_helpers/generators.rs` currently only cover a small subset of what the parser and type inference support. Each item below means: add the feature to the proptest generators so it gets randomly tested against DuckDB.

### Functions

- [x] **String functions** — Added LTRIM, RTRIM, CONCAT, CHAR_LENGTH, CHARACTER_LENGTH, REPLACE, LPAD, RPAD, REPEAT, SUBSTRING, SUBSTR, SPLIT_PART, STRPOS. Omitted: INITCAP (not in DuckDB), LEFT/RIGHT (SQL keyword conflicts), TRANSLATE, QUOTE_IDENT, QUOTE_LITERAL, POSITION (STRPOS covers same path), TO_CHAR
- [x] **Math functions** — Added POWER, EXP, LN, LOG, LOG10, LOG2, MOD, SIGN, SIN, COS, TAN, ATAN, ATAN2, SINH, COSH, TANH, PI. Omitted: ASIN/ACOS (domain-restricted, sample values cause errors)
- [x] **Temporal functions** — Added DATE_PART, DATE_TRUNC. Omitted: EXTRACT (special syntax, DATE_PART covers same path), MAKE_DATE/MAKE_TIME/MAKE_TIMESTAMP/MAKE_TIMESTAMPTZ/AGE (complex multi-arg with specific value requirements)
- [x] **Statistical/advanced aggregates** — Added STDDEV, VARIANCE, STDDEV_POP, STDDEV_SAMP, VAR_POP, VAR_SAMP, BOOL_AND, BOOL_OR, BIT_AND, BIT_OR, BIT_XOR. Omitted: EVERY (not in DuckDB), MEDIAN/MODE/PERCENTILE_CONT/PERCENTILE_DISC (DuckDB syntax differences), APPROX_COUNT_DISTINCT/ANY_VALUE/FIRST/LAST (limited DuckDB support), CORR/COVAR_POP/COVAR_SAMP/REGR_SLOPE (need 2-column aggregates)
- [x] **Null handling functions** — Added COALESCE, NULLIF
- [x] **Comparison functions** — Added GREATEST, LEAST
- [ ] **JSON functions** — Add JSON_BUILD_OBJECT, JSON_BUILD_ARRAY, TO_JSON, TO_JSONB, ROW_TO_JSON (none covered)

### Expressions and Operators

- [ ] **Window functions** — Add ROW_NUMBER, RANK, DENSE_RANK, NTILE, CUME_DIST, PERCENT_RANK, LAG, LEAD, FIRST_VALUE, LAST_VALUE, NTH_VALUE with OVER clauses (none covered)
- [x] **BETWEEN / IN expressions** — Added BETWEEN and IN for numeric columns (both return Boolean). EXISTS not yet covered.
- [ ] **Scalar subqueries** — Generate `(SELECT ...)` in expression position (none covered)
- [ ] **Regex operators** — Add `~`, `~*`, `!~`, `!~*` PostgreSQL regex operators (none covered, type inference also missing)
- [ ] **JSON operators** — Add `->`, `->>`, `#>`, `#>>`, `@>`, `<@` (none covered, type inference also missing)
- [ ] **Mixed-type binary operations** — Generate cross-type arithmetic (INTEGER + BIGINT, DECIMAL + DOUBLE) to test type promotion rules

### Types

- [ ] **Interval type** — Add as a base type for temporal arithmetic testing
- [ ] **Time type** — Add as a base type for MAKE_TIME and time functions
- [ ] **Array types** — Add ARRAY literals, ARRAY_AGG, array subscript, array slice
- [ ] **Row/Struct types** — Add ROW(...) and STRUCT(...) constructors

### Syntax Variants

- [ ] **PostgreSQL `::` cast syntax** — Generate `expr::type` in addition to `CAST(expr AS type)`
- [x] **CAST to more types** — Added INTEGER, BIGINT, DOUBLE, VARCHAR, BOOLEAN, DATE, TIMESTAMP targets
- [ ] **GROUP BY / HAVING** — Generate multi-column GROUP BY with HAVING predicates
- [ ] **DISTINCT / DISTINCT ON** — Generate DISTINCT expressions

### Known DuckDB Incompatibilities (discovered during generator expansion)

- **INITCAP**: Not available in DuckDB
- **EVERY**: Not available in DuckDB (BOOL_AND covers same semantics)
- **LEFT/RIGHT**: SQL keywords conflict with function parsing in smelt's parser
- **ASIN/ACOS**: Domain-restricted to [-1,1]; sample values (42, 100, etc.) cause errors
- **SIGN**: DuckDB returns TINYINT (SmallInt) regardless of input type; fixed smelt inference to match
