# TODO

## Cross-Model Type Inference

- [ ] **Salsa cycle recovery for circular refs** — Currently, if model A refs model B which refs model A, Salsa will panic with a cycle error. Add `salsa::cycle` recovery attributes to `resolved_model_schema`, `typed_model_schema`, and `type_context` queries to return empty/default schemas gracefully and produce a diagnostic.

- [ ] **Cross-model type mismatch diagnostics** — When a downstream model uses a column in a type-incompatible way (e.g., `SUM(col)` where upstream infers `col` as VARCHAR), produce a warning diagnostic in the LSP. Compare `model_input_constraints` against actual upstream `typed_model_schema`.

- [ ] **Multi-model property tests** — Extend `type_property_tests.rs` to generate two-model chains (model_A with typed CTE columns, model_B refs model_A) and verify types match DuckDB output. Requires setting up Salsa Database with multiple models in the property test.

## Property Test Generator Coverage Gaps

The generators in `crates/smelt-db/tests/prop_helpers/generators.rs` currently only cover a small subset of what the parser and type inference support. Each item below means: add the feature to the proptest generators so it gets randomly tested against DuckDB.

### Functions

- [ ] **String functions** — Add SUBSTRING, REPLACE, LPAD, RPAD, SPLIT_PART, CONCAT, LTRIM, RTRIM, TRANSLATE, REPEAT, INITCAP, QUOTE_IDENT, QUOTE_LITERAL, LEFT, RIGHT, POSITION, STRPOS, SUBSTR, TO_CHAR, CHAR_LENGTH, CHARACTER_LENGTH (21 missing, only 4 of 25 covered)
- [ ] **Math functions** — Add POWER/POW, EXP, LN, LOG, LOG10, LOG2, MOD, SIGN, SIN, COS, TAN, ASIN, ACOS, ATAN, ATAN2, SINH, COSH, TANH, PI (18 missing, only 5 of 23 covered)
- [ ] **Temporal functions** — Add EXTRACT, DATE_PART, DATE_TRUNC, MAKE_DATE, MAKE_TIME, MAKE_TIMESTAMP, MAKE_TIMESTAMPTZ, AGE (none covered)
- [ ] **Statistical/advanced aggregates** — Add STDDEV, VARIANCE, STDDEV_POP, STDDEV_SAMP, VAR_POP, VAR_SAMP, MEDIAN, MODE, PERCENTILE_CONT, PERCENTILE_DISC, APPROX_COUNT_DISTINCT, ANY_VALUE, FIRST, LAST, BOOL_AND, BOOL_OR, BIT_AND, BIT_OR, BIT_XOR, CORR, COVAR_POP, COVAR_SAMP, REGR_SLOPE, EVERY (24 untested aggregates)
- [ ] **Null handling functions** — Add COALESCE, NULLIF (none covered)
- [ ] **Comparison functions** — Add GREATEST, LEAST (none covered)
- [ ] **JSON functions** — Add JSON_BUILD_OBJECT, JSON_BUILD_ARRAY, TO_JSON, TO_JSONB, ROW_TO_JSON (none covered)

### Expressions and Operators

- [ ] **Window functions** — Add ROW_NUMBER, RANK, DENSE_RANK, NTILE, CUME_DIST, PERCENT_RANK, LAG, LEAD, FIRST_VALUE, LAST_VALUE, NTH_VALUE with OVER clauses (none covered)
- [ ] **BETWEEN / IN / EXISTS expressions** — Generate these comparison expressions (none covered, all return Boolean)
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
- [ ] **CAST to more types** — Currently only DOUBLE and STRING targets; add Date, Timestamp, Boolean, Decimal(p,s)
- [ ] **GROUP BY / HAVING** — Generate multi-column GROUP BY with HAVING predicates
- [ ] **DISTINCT / DISTINCT ON** — Generate DISTINCT expressions
