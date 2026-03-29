# Smelt SQL Dialect: Critical Analysis

*March 2026*

This report provides a critical analysis of smelt's SQL dialect — its design, parser implementation, type system, and testing infrastructure. It identifies strengths, weaknesses, and concrete suggestions for improvement.

---

## 1. Dialect Design

### 1.1 What smelt parses

Smelt's SQL dialect is a **multi-dialect superset**: it accepts standard SQL plus extensions from PostgreSQL, DuckDB, and Spark. The dialect includes:

- **Core SQL**: SELECT, FROM, WHERE, GROUP BY, HAVING, ORDER BY, LIMIT/OFFSET, CTEs (WITH/RECURSIVE), subqueries, UNION/INTERSECT/EXCEPT
- **Joins**: INNER, LEFT/RIGHT/FULL OUTER, CROSS, LATERAL
- **Expressions**: CASE, CAST, BETWEEN, IN, EXISTS, window functions with full frame specification
- **PostgreSQL extensions**: DISTINCT ON, TABLESAMPLE, `::` cast, regex operators (`~`, `~*`, `!~`, `!~*`), JSON operators (`->`, `->>`, `#>`, `#>>`, `@>`, `<@`), FILTER clause, LATERAL
- **DuckDB/Spark extensions**: QUALIFY, PIVOT/UNPIVOT, trailing commas, lambda expressions (`x -> expr`)
- **smelt extensions**: `smelt.ref('model')`, `smelt.source('schema.table')`, named parameters (`param => value`)

### 1.2 What it does not parse

Smelt is a SELECT-only parser. It does not handle:

- DML: INSERT, UPDATE, DELETE, MERGE
- DDL: CREATE TABLE/VIEW/INDEX, ALTER, DROP
- DCL: GRANT, REVOKE
- TCL: BEGIN, COMMIT, ROLLBACK
- GROUPING SETS, ROLLUP, CUBE
- WINDOW clause (named window definitions at SELECT level)
- COLLATE
- Type constructors beyond ARRAY/STRUCT/ROW (e.g., MAP in Spark)
- Set-returning functions in SELECT (UNNEST as a select item)

### 1.3 Critique: The superset approach is risky

**Problem**: Accepting the union of multiple dialects means smelt will happily parse SQL that no single backend can execute. A query using both PostgreSQL's `DISTINCT ON` and DuckDB's `QUALIFY` is syntactically valid to smelt but cannot run anywhere.

**The `star_in_expression` gap illustrates this well**: smelt accepts `SELECT * + 1 FROM t` or `CASE WHEN * THEN ...` because `*` is parsed as a primary expression without restriction. No backend accepts this.

**Suggestion**: Introduce a dialect mode (even if just for diagnostics) that flags constructs unsupported by the target backend. This doesn't need to be enforced at parse time — a post-parse lint pass would suffice. The `smelt-parser-compat` crate already has the infrastructure for this via gap tracking; it could be promoted from test infrastructure to a user-facing feature.

### 1.4 Critique: No formal grammar specification

The dialect is defined implicitly by the parser code (3,760 lines of `parser.rs`). There is no BNF/EBNF grammar, no railroad diagrams, and no formal specification beyond the README's examples.

**Why this matters**: Without a grammar, it's impossible to reason about ambiguity, predict parse behavior for novel inputs, or generate conformant SQL from other tools. The parser *is* the specification, which makes it fragile — any bug is indistinguishable from an intentional design choice.

**Suggestion**: Extract a grammar from the parser. This can be done incrementally: start with a PEG or EBNF for the expression precedence hierarchy (which is the most complex part), then extend to full statements. The grammar becomes documentation and can be used to generate railroad diagrams for the website.

---

## 2. Parser Implementation

### 2.1 Architecture

The parser follows a clean layered design:

```
Lexer (lexer.rs, 475 lines)
  → Token stream
Parser (parser.rs, 3,760 lines)
  → Rowan GreenNode (lossless CST)
AST wrappers (ast.rs, 1,916 lines)
  → Typed accessors over CST
Printer (printer.rs)
  → CST → SQL string (round-trip)
```

**Rowan** is an excellent choice for a language server parser. It provides:
- Lossless concrete syntax trees (preserves whitespace, comments)
- Cheap cloning via green node interning
- Incremental reparsing (though smelt doesn't use this yet)

### 2.2 Strengths

**Error recovery is practical.** The `sync_to()` mechanism wraps unexpected tokens in ERROR nodes and advances to the next keyword boundary. This is simple but effective for IDE use — partial parses produce usable ASTs for diagnostics and navigation.

**Expression precedence is correct and well-structured.** The precedence climbing chain (OR → AND → comparison → concatenation/JSON → additive → multiplicative → unary → primary) matches SQL semantics and handles edge cases like unary minus, IS [NOT] NULL, and BETWEEN.

**The CST/AST separation is well-executed.** AST wrappers provide typed access without losing CST fidelity. Methods like `Expr::as_function_call()`, `TableRef::is_lateral()`, and `RefCall::model_name()` are clean abstractions over the underlying tree structure.

### 2.3 Weaknesses

**The parser is a monolithic function.** All 3,760 lines live in a single `impl Parser` block. There's no separation between statement-level, clause-level, and expression-level parsing — it's all methods on one struct. This makes the parser harder to test in isolation and harder to extend.

**Suggestion**: Split the parser into modules: `parse_statement.rs`, `parse_expression.rs`, `parse_clause.rs`. The `Parser` struct can be passed as `&mut self` across module boundaries. This is a mechanical refactoring that doesn't change behavior.

**Lookahead is ad-hoc.** Lambda detection (`is_lambda_single_param()`, `is_lambda_multi_param()`) and named parameter detection (`is_named_parameter()`) use manual token scanning that duplicates parsing logic. This is fragile — if the lexer changes how tokens are emitted, these lookahead functions break silently.

**Suggestion**: Consider using Rowan's checkpoint mechanism more consistently for speculative parsing. Instead of lookahead functions that scan tokens, try parsing the construct and backtrack if it fails. This is what `parse_comparison_expr` already does for BETWEEN/IN — extend the pattern.

**LIMIT only accepts NUMBER or ALL tokens.** The parser rejects `LIMIT $1` (parameterized), `LIMIT (SELECT COUNT(*) FROM t)` (subquery), and `LIMIT col_name` (identifier). This is a needless restriction — LIMIT should accept any expression.

```rust
// Current (parser.rs:905)
if self.at(NUMBER) || self.at(ALL_KW) {
    self.advance();
} else {
    self.error("Expected number or ALL after LIMIT".to_string());
}
```

**Suggestion**: Replace with `self.parse_expression()` or at minimum accept IDENT and LPAREN (for subqueries/parameters).

**No operator precedence for set operations.** UNION, INTERSECT, and EXCEPT are parsed left-to-right with equal precedence. Per the SQL standard, INTERSECT binds tighter than UNION/EXCEPT. `SELECT 1 UNION SELECT 2 INTERSECT SELECT 3` should parse as `SELECT 1 UNION (SELECT 2 INTERSECT SELECT 3)` but smelt parses it as `(SELECT 1 UNION SELECT 2) INTERSECT SELECT 3`.

**Suggestion**: Add a precedence level for set operations, with INTERSECT > UNION = EXCEPT.

### 2.4 The printer

The printer (`printer.rs`) reconstructs SQL from the AST. It supports compact and pretty modes, though pretty mode is incomplete (`#[allow(dead_code)]` on FormatContext fields). The printer is used for round-trip testing.

**Concern**: The printer re-implements SQL serialization from scratch rather than walking the CST's preserved tokens. Since Rowan preserves all original text including whitespace, a simpler approach for identity printing would be to concatenate the CST's tokens. The current printer is needed for normalization (uppercasing keywords, etc.) but the two uses should be distinguished.

---

## 3. Type System

### 3.1 Supported types

Smelt's type system (`smelt-types`, 24 variants) covers:

| Category | Types |
|----------|-------|
| Numeric | Boolean, SmallInt, Integer, BigInt, Float, Double, Decimal{p,s} |
| String | Varchar{max_len}, Char{len}, Text |
| Binary | Blob |
| Temporal | Date, Time, Timestamp{tz}, Interval |
| Complex | Array(T) |
| Special | Null, Unknown |

### 3.2 Notable gaps

**No JSON type.** JSON values are represented as `Text`, which means `data->'key'` and `data->>'key'` both infer as Text. This loses the semantic distinction between JSON navigation (returns JSON) and text extraction (returns text). When smelt adds JSON validation or JSON-aware optimization, this will need to change.

**No MAP or STRUCT types.** STRUCT literals are parsed but there's no `DataType::Struct` variant. The parser handles `STRUCT(a, b, c)` but the type system can't represent it. This limits type inference for DuckDB's native struct operations.

**No parameterized Timestamp precision.** `TIMESTAMP(6)` is parsed to `Timestamp { with_timezone: bool }` but the precision (microseconds vs nanoseconds) is lost. This matters for Spark where `TIMESTAMP_NTZ` vs `TIMESTAMP_LTZ` have different semantics.

### 3.3 Type inference: strengths

**The inference engine is substantial (1,854 lines) and covers 60+ SQL functions across all major categories** — aggregates, window functions, string/math/date functions, JSON operations, and more. For a project at this stage, this is impressive coverage.

**The TypeContext design is sound.** Separating source columns, model columns, and CTE columns with clear shadowing rules (CTE > model > source) correctly models SQL scoping. The `missed_lookups` tracking is a clever mechanism for property tests to detect columns that fell through.

**CTE type inference with recursive bootstrapping** (using Unknown types initially, then refining) is the right approach for recursive CTEs, matching how production databases handle it.

### 3.4 Type inference: weaknesses

**Comparison operators claim `nullable: false`, which is wrong.** At `type_inference.rs:934`:

```rust
"=" | "<>" | "!=" | "<" | ">" | "<=" | ">=" | "IS" => Some(TypedColumn {
    data_type: DataType::Boolean,
    nullable: false, // Comparisons always return true/false
})
```

In SQL, `NULL = 1` evaluates to NULL, not FALSE. Only `IS NULL`/`IS NOT NULL` and `IS DISTINCT FROM` are guaranteed non-null. The comment "Comparisons always return true/false" is factually incorrect for standard SQL three-valued logic. While the `IS` operator is correctly non-null, grouping it with `=`/`<>`/etc. masks the bug.

**Suggestion**: Set `nullable: true` for all comparison operators except `IS`. This affects downstream nullable analysis.

**Decimal arithmetic uses fixed precision.** At `type_inference.rs:962`:

```rust
(Some(DataType::Decimal { .. }), _) | (_, Some(DataType::Decimal { .. })) => {
    Some(TypedColumn {
        data_type: DataType::Decimal { precision: 38, scale: 10 },
        nullable: true,
    })
}
```

`DECIMAL(10,2) + DECIMAL(10,2)` should yield `DECIMAL(11,2)` (or similar per SQL standard rules), not `DECIMAL(38,10)`. The current approach over-allocates precision and changes the scale, which can cause surprising behavior when smelt wraps outputs with type casts.

**Suggestion**: Implement proper decimal arithmetic rules. The SQL standard defines: for addition/subtraction, `precision = max(s1,s2) + max(p1-s1, p2-s2) + 1, scale = max(s1,s2)`. For multiplication, `precision = p1+p2, scale = s1+s2`. This is well-specified and worth getting right.

**COALESCE only checks the first concrete argument.** The comment at line 390 explains this is intentional to avoid incorrectness with Unknown args, but it means `COALESCE(NULL, 42)` infers as `Null` rather than `Integer`. The first argument is `NULL` (Null type), and the function stops looking. In practice the property tests likely mask this because CTEs provide typed columns, not literal NULLs.

**Suggestion**: Skip Null *and* Unknown when searching for the concrete type, but still check all arguments. The current code does skip Unknown but not Null.

**SUM always promotes to BigInt.** `SUM(SMALLINT)` should arguably promote to at least Integer (or BigInt), but `SUM(DECIMAL(10,2))` promotes to BigInt, losing the decimal. DuckDB returns `DECIMAL(38,0)` for integer sums and `DECIMAL(38, original_scale)` for decimal sums. The current inference of BigInt for decimal inputs is incorrect.

---

## 4. Testing

### 4.1 Architecture

The testing infrastructure is impressive for the project's maturity:

| Layer | Strategy | Cases |
|-------|----------|-------|
| Parser round-trip | Property-based (proptest) | ~100-1000 per suite |
| Parser compatibility | Cross-validation with pg_query + Spark | ~500 per suite |
| Type inference | Oracle-based against DuckDB (+ optional Spark) | 256 base + multi-model |
| Type conformance | Zero-divergence after cast wrapping | 256 |
| Dialect compilation | Snapshot tests (insta) | ~30 |
| LSP integration | Unit tests with TestWorkspace helper | ~15 |
| CLI end-to-end | DuckDB execution, incremental materialization | ~40 |

### 4.2 Strengths

**Oracle-based type testing is the standout feature.** Rather than asserting expected types manually, smelt generates random SQL, executes it against DuckDB, inspects the Arrow schema, and compares. This finds real divergences that manual tests would miss. The divergence registry (`divergences.rs`) with `ByDesign`/`BackendSpecific`/`KnownBug` status tracking is well-engineered.

**The multi-model property tests** (2-model chains, 3-model chains, JOIN models) verify that types propagate correctly through `smelt.ref()` boundaries. This tests the whole pipeline, not just expression-level inference.

**Parser robustness testing** (1000 random strings, assertion: parser never panics) is simple but valuable. Combined with round-trip tests (parse → print → parse), this gives good confidence in parser stability.

### 4.3 Weaknesses

**The expression generator (`proptest_generators.rs`) is too shallow.** Generated expressions have at most 2 levels of nesting (a simple expression, or `simple op simple`). There are no:
- Nested function calls: `UPPER(CONCAT(a, b))`
- CASE inside CASE
- Subqueries in expressions
- Window functions with complex frames
- Expressions using CTEs

This means the round-trip tests primarily validate flat queries. Deeply nested or complex queries — the kind that break parsers — are underrepresented.

**Suggestion**: Add recursive generators with depth limits. proptest supports this via `prop_recursive()`. Even depth 3-4 would dramatically improve coverage.

**No negative testing for type inference.** The property tests generate valid, well-typed SQL. There are no tests for:
- Type errors that smelt should detect (e.g., `SUM('hello')`)
- Expressions that should produce Unknown
- Diagnostics quality (error messages, positions)
- Recovery from type inference failures

**Suggestion**: Add a separate property test suite that generates intentionally ill-typed SQL and verifies smelt produces appropriate diagnostics.

**Parser compatibility tests require Docker.** The PostgreSQL and Spark cross-validation tests are gated behind external services and are ignored by default. This means they likely don't run in CI consistently.

**Suggestion**: Consider embedding a subset of pg_query test vectors as static fixtures. The `pg_query` crate wraps libpg_query which can run without a PostgreSQL server — verify this is being used directly (it appears to be, via `PgParseResult::parse`). If so, these tests should be runnable without Docker.

**No fuzzing infrastructure.** Property testing generates structured inputs, but a fuzzer (cargo-fuzz, afl) would generate adversarial inputs that stress error recovery paths, unicode handling, deeply nested parentheses, and pathological token sequences. For a parser that will process untrusted user input in an IDE, this is important.

**Suggestion**: Add a `fuzz/` directory with at least one target: `parse(arbitrary_bytes)` should never panic. This is low-effort, high-value.

### 4.4 The divergence registry as documentation

The divergence registry (`divergences.rs`) serves double duty as test infrastructure and as documentation of semantic differences between smelt and backends. This is an excellent pattern. However:

- The 9 registered divergences are all `BackendSpecific` or `ByDesign`. There are zero `KnownBug` entries. Either smelt's type inference is perfect (unlikely) or bugs are being masked by the `type_comparison.rs` compatibility layer which treats Text/Varchar and integer width differences as "compatible" rather than divergent.
- The compatibility layer (`TypeComparison::Compatible`) is doing a lot of heavy lifting. It considers SmallInt/Integer/BigInt all compatible with each other, and Text/Varchar/Char all compatible. This is arguably too permissive — it means smelt could infer SmallInt where the database returns BigInt and the test would pass.

**Suggestion**: Track `Compatible` matches separately from `Exact` matches in test output. Report the ratio. A high Compatible-to-Exact ratio would indicate systematic inference imprecision.

---

## 5. Architectural Observations

### 5.1 The pure function rule is well-enforced

The CLAUDE.md documents an invariant: analysis logic must be pure functions, Salsa queries are thin wrappers. Inspecting `type_inference.rs` confirms this — zero Salsa imports, pure functions taking AST nodes and `TypeContext`. This will make the planned `smelt-check` extraction straightforward.

### 5.2 The crate structure supports future evolution

```
smelt-parser (standalone)
smelt-types (standalone)
smelt-db (Salsa integration)
smelt-dialect (backend compilation)
smelt-lsp (language server)
smelt-cli (orchestration)
smelt-parser-compat (test-only)
```

This is clean. Each crate has a clear responsibility. The dependency direction is correct (parser has no downstream dependencies).

### 5.3 Concern: AST wrapper brittleness

The AST wrappers (`ast.rs`, 1,916 lines) extensively use positional child-node indexing and string matching:

```rust
// From ast.rs - relies on child ordering
fn left(&self) -> Option<Expr> {
    self.0.children().filter_map(Expr::cast).next()
}
fn right(&self) -> Option<Expr> {
    self.0.children().filter_map(Expr::cast).nth(1)
}
```

If the parser changes how it nests nodes (e.g., wrapping an operand in an extra EXPRESSION node), these accessors silently return the wrong child. The round-trip tests catch some of these issues, but not all — they test parse-print equivalence, not AST accessor correctness.

**Suggestion**: Add targeted unit tests for AST accessors on known inputs. For every AST method, there should be at least one test that parses a specific SQL string and asserts the accessor returns the expected value.

---

## 6. Prioritized Recommendations

### High priority (correctness)

1. **Fix comparison operator nullability.** `=`, `<>`, `<`, etc. should be `nullable: true`. This is a semantic bug.
2. **Fix SUM type inference for Decimal inputs.** `SUM(DECIMAL(10,2))` should not return BigInt.
3. **Implement proper decimal arithmetic precision rules.** The fixed `DECIMAL(38,10)` is a significant oversimplification.

### Medium priority (robustness)

4. **Add cargo-fuzz target.** Low effort, high value for parser safety.
5. **Deepen property test generators** with recursive expression generation.
6. **Allow expressions in LIMIT clause**, not just NUMBER/ALL.
7. **Fix INTERSECT precedence** over UNION/EXCEPT.

### Lower priority (quality)

8. **Extract a formal grammar** from the parser, even if just as documentation.
9. **Split parser.rs** into statement/expression/clause modules.
10. **Add a dialect-aware lint pass** that warns about cross-dialect constructs.
11. **Track Compatible vs Exact type matches** in property test reporting.
12. **Add JSON and STRUCT types** to the type system.

---

## 7. Conclusion

Smelt's SQL dialect implementation is strong for its stage of development. The Rowan-based parser with error recovery, the oracle-based property testing, and the pure-function type inference architecture are all solid engineering choices. The codebase is well-organized and the separation of concerns supports the project's multi-backend ambitions.

The main risks are: (1) semantic correctness gaps in type inference (nullable comparisons, decimal arithmetic) that could compound as the type system is used for optimization decisions, (2) the superset dialect approach that accepts SQL no backend can run, and (3) test generators that don't exercise deep nesting or adversarial inputs. These are all addressable without architectural changes — the foundations are sound.
