# smelt Development Roadmap

This document tracks the implementation status of smelt, aligned with the spec in [DESIGN.md](DESIGN.md).

## Current Status

**JSON Function Canonicalization (March 21, 2026)**: Redesigned JSON function support to accept all dialect variants and map to canonical internal functions:
- **Canonical functions**: JsonObject, JsonArray, ToJson, JsonExtract, JsonExtractText, JsonArrayLength, JsonObjectKeys, JsonContains
- **Dialect aliases**: `json_build_object` (PG) and `json_object` (DuckDB) both resolve to `JsonObject`; `get_json_object` (Spark), `json_extract_string` (DuckDB), and `json_extract_path_text` (PG) all resolve to `JsonExtractText`
- **JSON operator type inference**: `->`, `->>`, `#>`, `#>>` return Text; `@>`, `<@` return Boolean
- **Property test generators**: TO_JSON, JSON_ARRAY, JSON_OBJECT functions and `->` / `->>` operators tested against DuckDB
- JSON represented as Text internally (no DataType::Json); DuckDB JSON maps to Varchar via Arrow

**Spark Type Oracle for Property Tests (March 21, 2026)**: Extended type property tests to verify smelt's inference against Spark SQL (in addition to DuckDB):
- **SparkOracle**: New `TypeOracle` implementation that runs `DESCRIBE QUERY` via `docker exec` against a Spark container
- **Multi-backend divergences**: `find_divergence()` now accepts a `backend` parameter; DuckDB and Spark divergences are registered independently
- **Unified test flow**: Each proptest case checks DuckDB (always) + Spark (when `SPARK_CONTAINER_ID` env var is set), gracefully skipping Spark-invalid queries
- **CI integration**: New `type-property-spark` job in `compat.yml` (nightly / `run-docker-tests` label) runs 500 proptest cases against both backends
- 27 tests: 256-case proptest + 5 smoke tests + unit tests across modules (up from 21)

**Property-Based Type Inference Tests (March 21, 2026)**: Integration test suite in `smelt-db/tests/` that uses proptest + DuckDB to verify smelt's type inference against real database behavior:
- **Property tests**: Generate random typed CTE queries, run against DuckDB, compare inferred vs actual column types
- **Type oracle**: Trait-based design (`TypeOracle`) for future PostgreSQL/Spark backends
- **Divergence registry**: Known mismatches (e.g., `SUM(DOUBLE)` → Decimal vs Double, `CEIL(INTEGER)` → Integer vs Double) registered with status (KnownBug/ByDesign/BackendSpecific)
- **Compatible type handling**: Text/Varchar, Decimal precision, integer width differences marked Compatible rather than failures

**Optimizer: Cube Split + Incremental Materialization (March 14, 2026)**: New `smelt-optimizer` crate implementing the first two optimization rules:
- **Cube split**: `-- smelt:cube_split` annotation on GROUP BY splits queries with multiple COUNT(DISTINCT) into parallel sub-queries joined on GROUP BY keys (NULL-safe via IS NOT DISTINCT FROM). Non-distinct aggregates (COUNT(*), SUM, etc.) are included in the first sub-query.
- **Incremental materialization**: YAML frontmatter `incremental: { partition_column: ... }` detected by optimizer, validated against SELECT/GROUP BY, source time column extracted from expressions like `date_trunc('day', event_time)`.
- **Composition**: Both optimizations can apply to the same model — cube split steps get time-filtered, execution uses DELETE+INSERT pattern.
- **Mandatory time range**: CLI errors when incremental models are selected but `--event-time-start`/`--event-time-end` are missing.
- **CLI integration**: Optimizer runs after model discovery, transformations applied via `execute_plan()` / `execute_plan_incremental()`.
- 29 unit tests + 5 integration tests verifying correctness via EXCEPT-based comparison against naive queries.

**SQL Parser Gap Fill (March 13, 2026)**: Implemented 12 features across 6 categories to close remaining SQL parser gaps:
- **Set operations**: INTERSECT [ALL], EXCEPT [ALL] alongside existing UNION [ALL]
- **Block comments**: `/* ... */` with nested comment support
- **ARRAY literals**: `ARRAY[1, 2, 3]` syntax
- **VALUES clause**: Standalone and in CTEs: `VALUES (1, 'a'), (2, 'b')`
- **JSON operators**: `->`, `->>`, `#>`, `#>>`, `@>`, `<@`
- **Regex operators**: `~`, `~*`, `!~`, `!~*`
- **ROW constructor**: `ROW(1, 2, 3)`
- **ANY/ALL/SOME**: Array comparisons: `= ANY(...)`, `> ALL(...)`
- **WITHIN GROUP**: Ordered-set aggregates: `PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY val)`
- **Window EXCLUDE**: `EXCLUDE CURRENT ROW / GROUP / TIES / NO OTHERS`
- **FETCH FIRST**: `FETCH FIRST N ROWS ONLY` with OFFSET support
- **STRUCT literals**: `STRUCT(expr AS name, ...)`
Removed 7 known parser gaps from smelt-parser-compat. Lambda expressions updated to work with new `->` tokenization.

**Multi-Dialect Compatibility Testing (March 13, 2026)**: Extended `smelt-parser-compat` with three-layer verification: pg_query (existing), sqlparser-rs DatabricksDialect (new, pure Rust), and sqlglot subprocess (new, Python, optional). Added Spark-specific SQL generators, dialect-aware gap tracking, Docker-based Spark integration tests, and unified `compare_all_parse_results()`. CI workflow renamed from `pg-compat.yml` to `compat.yml` with sqlglot and Spark Docker jobs.

**Smelt SQL Multi-Dialect Superset (March 12, 2026)**: Defining smelt SQL as a logical SQL superset — PostgreSQL base with cherry-picked features from DuckDB and Spark. Phase 4a-4e: QUALIFY clause, lambda expressions/array functions, PIVOT/UNPIVOT, array subscript notation, DATE literal normalization. Parser-level features with backend-specific rewrite rules.

**Python Model Support - Phase 3 LSP + Performance (March 9, 2026)**: Content-hash caching for Python model subprocess results (persisted to `.smelt/python_cache.json`), `.py` file watching with dynamic LSP watcher registration, Python execution error diagnostics surfaced in the editor, and background subprocess execution with last-known-good fallback on failure.

**Python Model Support - Phase 1 (March 8, 2026)**: Python models as an escape hatch for programmatic model generation. Python functions decorated with `@model` return SQL strings that get parsed by the existing smelt parser. Includes: Python SDK (`python/smelt/`), subprocess protocol with JSON I/O, `@model` decorator scanning, iterative discovery with fixed-point validation, full ref extraction from generated SQL, mixed SQL/Python dependency graphs, LSP awareness (Python models as valid ref targets), and comprehensive tests. Python config via `SMELT_PYTHON` env var or `python` field in `smelt.yml`.

**Row Polymorphism (March 7, 2026)**: Models with `SELECT *` now properly propagate types and schemas through `RowExtension` entries instead of placeholder wildcard columns. New `resolved_model_schema()` Salsa query recursively expands wildcards through model chains. Input constraint extraction identifies what columns each model requires from its refs.

**Comprehensive Type Inference (January 18, 2026)**: Added recursive type inference for MIN/MAX, COALESCE, NULLIF, CASE expressions, scalar subqueries, and window functions (LAG, LEAD, etc.) to preserve argument types. Implemented binary operator type inference for arithmetic, comparisons, and string concatenation. The `smelt table` command now correctly shows TIMESTAMP for MIN/MAX of timestamp columns instead of UNKNOWN.

**Parser Gap Fixes (January 18, 2026)**: Fixed 5 high/medium severity parser gaps discovered by PostgreSQL compatibility testing: `<>` operator, `||` string concatenation, UNION ALL printing, NULLS FIRST/LAST printing, and expressions in function arguments.

**PostgreSQL Compatibility Testing (January 17, 2026)**: Property-based testing infrastructure added to verify parser compatibility with PostgreSQL. New `smelt-parser-compat` crate with pg_query integration, gap tracking, and CI workflows.

**Table Alias Autocomplete (January 17, 2026)**: LSP now provides column completions when typing `t.` where `t` is a table alias (e.g., `FROM smelt.source('raw.users') t`).

**Type System Complete (January 16, 2026)**: Full type system implementation with smelt-types crate, TypeChecking Salsa queries, and LSP integration showing types in hover and completion.

**Source Support Complete (January 3, 2026)**: Full `smelt.source()` support for external source tables defined in sources.yml, with LSP diagnostics, hover, and completion.

**YAML Frontmatter Metadata Support Complete (December 31, 2025)**: Models can now specify configuration inline using YAML frontmatter, with SQL metadata taking precedence over smelt.yml.

**Multi-Backend Architecture with Basic Incremental Materialization Complete (December 27, 2024)**: Parser, LSP, multi-backend CLI with DuckDB and Spark (stub) implementations, and basic incremental materialization support.

```sql
-- ✅ Supported syntax (parser & LSP)
SELECT * FROM smelt.ref('model_name')
SELECT * FROM smelt.ref('events', filter => date > '2024-01-01')
SELECT * FROM smelt.ref('orders', filter => status = 'active', limit => 100)
SELECT * FROM smelt.source('raw.users')  -- External source tables
```

```bash
# ✅ Supported CLI commands
smelt run                           # Execute all models
smelt run --show-results            # Preview query results
smelt run --verbose                 # Show compiled SQL
smelt run --dry-run                 # Validate without executing
smelt run --target prod             # Execute against Spark target
```

```yaml
# ✅ Supported configuration
targets:
  dev:
    type: duckdb
    database: dev.duckdb
    schema: main
  prod:
    type: spark
    connect_url: sc://localhost:15002
    catalog: spark_catalog
    schema: production
```

---

## ✅ Phase 16: YAML Frontmatter Metadata Support (COMPLETED)

**Completed**: December 31, 2025

### What Was Implemented

- **YAML frontmatter metadata extraction** in SQL files
  - `crates/smelt-cli/src/metadata.rs` - Complete frontmatter parsing (~370 lines)
  - `extract_file_metadata()` - Parse YAML frontmatter from SQL files
  - `ModelMetadata` struct - All metadata fields (materialization, incremental, tags, owner, description)
  - Single-model files: `--- ... ---` wrapping YAML
  - Multi-model files: `--- name: model_name ---` section delimiters
  - Backward compatible - files without frontmatter still work

- **Configuration precedence system**
  - **SQL file metadata > smelt.yml > defaults**
  - `Config::get_materialization_with_metadata()` - Check SQL first, fall back to YAML
  - `Config::get_incremental_with_metadata()` - SQL wins for incremental config
  - `ModelFile.metadata` field - Stores extracted metadata during discovery

- **End-to-end integration**
  - `discovery.rs` - Extract metadata when discovering models
  - `compiler.rs` - Use metadata when compiling models
  - `main.rs` - Check metadata for incremental execution
  - Example: `examples/models/user_summary.sql` with frontmatter

- **Comprehensive test coverage**
  - 13 new unit tests in `metadata.rs`
  - Tests for single-model, multi-model, error recovery
  - Backward compatibility tests
  - All 168 tests passing across workspace

### Implementation Details

**Metadata Format (Single-Model):**
```sql
---
name: daily_revenue
materialization: table
incremental:
  enabled: true
  event_time_column: transaction_timestamp
  partition_column: revenue_date
tags: [revenue, core]
owner: analytics-team
description: Daily revenue aggregation
---

SELECT DATE(transaction_timestamp) as revenue_date, ...
```

**Metadata Format (Multi-Model):**
```sql
--- name: model1 ---
materialization: table
---
SELECT ...

--- name: model2 ---
materialization: view
---
SELECT ...
```

**Supported Metadata Fields:**
- `name` (string) - Model name (optional in single-model, required in multi-model)
- `materialization` (`table` | `view`) - How to materialize
- `incremental` (object) - Incremental config (enabled, event_time_column, partition_column)
- `tags` (array) - Organization tags
- `owner` (string) - Team/person responsible
- `description` (string) - Model documentation
- `backend_hints` (object) - Backend-specific settings (forward compatibility)
- `custom` (object) - Custom fields (forward compatibility)

**Error Handling:**
- Malformed YAML → diagnostic, fall back to treating file as SQL
- Missing required fields → validation errors
- Invalid section delimiters → error recovery at sync points

### Design Decisions

**Post-parse extraction** (not in lexer/parser):
- Keeps parser focused on SQL syntax only
- Better error recovery (YAML errors don't break SQL parsing)
- No circular dependencies between crates
- Metadata extraction is CLI/LSP concern, not parser concern

**Boxed metadata** in ModelFile:
- Large enum variant warning fixed by boxing `ModelMetadata`
- Reduces stack allocation overhead

**SQL-first precedence**:
- Inline configuration closer to code
- Easier to understand model behavior
- Gradual migration path from smelt.yml

### Updated Files

1. **crates/smelt-cli/src/metadata.rs** (NEW, 470 lines)
   - Core extraction logic with comprehensive tests

2. **crates/smelt-cli/src/config.rs** (MODIFIED)
   - Added `get_materialization_with_metadata()`
   - Added `get_incremental_with_metadata()`
   - Added `PartialEq` to `IncrementalConfig`

3. **crates/smelt-cli/src/discovery.rs** (MODIFIED)
   - Added `metadata` field to `ModelFile`
   - Extract metadata during model discovery
   - Handle multi-model files

4. **crates/smelt-cli/src/compiler.rs** (MODIFIED)
   - Use metadata when determining materialization
   - Both `compile()` and `compile_with_sql()` updated

5. **crates/smelt-cli/src/main.rs** (MODIFIED)
   - Use metadata for incremental config lookup

6. **crates/smelt-cli/src/lib.rs** (MODIFIED)
   - Export metadata types

7. **README.md** (MODIFIED)
   - New "Model Configuration" section with frontmatter examples
   - Updated Quick Example with frontmatter
   - Configuration precedence documentation

8. **examples/models/user_summary.sql** (MODIFIED)
   - Added frontmatter example

### Test Results

All 168 tests passing:
- 37 smelt-cli tests (including 13 new metadata tests)
- 88 smelt-parser tests
- 10 smelt-db tests
- 5 smelt-backend-duckdb tests
- 2 smelt-backend-spark tests
- 20 property-based tests
- 6 integration tests

Cargo clippy passes with no warnings.

### Future Work (Not Implemented)

**Comment Annotations** (Phase 17):
- Extract `-- @key: value` annotations from SQL comments
- Attach to models and columns
- LSP hover/diagnostics for annotations

**LSP Metadata Validation** (Phase 17):
- Real-time diagnostics for invalid YAML
- Autocomplete for metadata keys
- Quick fixes for common mistakes

**Multi-Model Discovery** (Future):
- Currently only single-model per file fully supported
- Multi-model parsing works but discovery needs enhancement

---

## ✅ Phase 17: Source Support (COMPLETED)

**Completed**: January 3, 2026

### What Was Implemented

- **Parser support for `smelt.source()` function calls**
  - `smelt.source('source.table')` syntax for external source references
  - `SourceCall` AST wrapper in `smelt-parser/src/ast.rs`
  - Extracts source name and table name from dotted format
  - Position tracking for accurate diagnostics

- **Database queries for source resolution**
  - `sources_yaml` input query for raw YAML content
  - `model_sources()` query extracts all source calls from a file
  - `sources_config()` parses YAML into structured config
  - `resolve_source()` resolves source.table to table definition
  - All query results cached via Salsa for incremental compilation

- **YAML configuration format**
  ```yaml
  sources:
    - name: raw
      database: analytics
      schema: raw
      tables:
        - name: users
          columns:
            - name: id
              type: INTEGER
            - name: email
              type: VARCHAR
  ```

- **LSP integration**
  - Diagnostics for undefined sources with accurate positions
  - Hover shows source schema with column definitions
  - Autocomplete for source.table names inside `source('')`
  - Sources.yml loaded on workspace initialization

### Implementation Details

**Parser** (`crates/smelt-parser/src/ast.rs`):
- `SourceCall` struct wrapping `FunctionCall`
- `from_function_call()` validates `smelt.source()` namespace
- `qualified_name()` extracts full `source.table` string
- `source_name()` / `table_name()` extract individual parts
- `name_range()` for precise diagnostic positioning

**Database** (`crates/smelt-db/src/lib.rs`):
- `SourceLocation` struct with source_name, table_name, qualified_name, range
- `SourcesConfig` / `SourceDef` / `SourceTableDef` / `SourceColumnDef` types
- Serde deserialization for sources.yml format
- `file_diagnostics()` updated to check undefined sources

**LSP** (`crates/smelt-lsp/src/main.rs`):
- `initialize()` loads sources.yml from workspace root
- `hover()` shows source schema when hovering over source() calls
- `completion()` provides source.table completions inside source('')
- Completion context detection for `source('|')` pattern

### Design Decisions

**Dotted syntax** (`source.table`):
- Single argument format like dbt's source()
- Easier to type than two separate arguments
- Clear visual distinction from model references

**Column validation out of scope**:
- Schema tracking infrastructure exists but validation not implemented
- Applies to both refs AND sources
- Would require database introspection or YAML column definitions
- Marked as future work in "Column Schema Tracking" section

**Sources.yml at workspace root**:
- Consistent with smelt.yml location
- Single source of truth for external tables
- LSP loads on initialization for fast completions

### Test Results

All 172 tests passing:
- 40 smelt-cli tests
- 115 smelt-parser tests
- 10 smelt-db tests
- 5 smelt-backend-duckdb tests
- 2 smelt-backend-spark tests

Cargo clippy passes with no warnings.

### Future Work

**Source freshness tracking** (Future):
- Track when sources were last updated
- Warn on stale source data
- Integration with source monitoring

**Source documentation** (Future):
- Rich descriptions in sources.yml
- LSP hover shows documentation
- Markdown support for column descriptions

---

## ✅ Phase 18: Type System (COMPLETED)

**Started**: January 16, 2026

### Goal

Add a type system to smelt that models tables and columns with SQL types, enabling:
- Output schema inference for CREATE TABLE statements
- Type-aware LSP feedback (diagnostics, hover, completion with types)
- Source types from `sources.yml` treated as authoritative

### Design Decisions (Confirmed)

1. **Nullability tracking**: Yes - track nullable vs non-nullable for DDL generation
2. **Type inference depth**: Basic - source types, column refs, CAST, literals, common aggregates
3. **Crate structure**: New `smelt-types` crate for clean separation

### Implementation Progress

#### ✅ Phase 1: Type Infrastructure (COMPLETED)

**New crate**: `crates/smelt-types/`

- **DataType enum** - SQL types with full precision:
  - Numeric: Boolean, SmallInt, Integer, BigInt, Float, Double, Decimal(precision, scale)
  - String: Varchar(max_length), Char(length), Text
  - Binary: Blob
  - Date/Time: Date, Time, Timestamp(with_timezone), Interval
  - Complex: Array(inner_type)
  - Special: Null, Unknown

- **TypedColumn struct** - Column with type and nullability

- **Type parsing** - Parses SQL type strings:
  - Parameterized types: `VARCHAR(255)`, `DECIMAL(10,2)`
  - Type aliases: `INT` → Integer, `FLOAT8` → Double
  - Case-insensitive, whitespace-tolerant
  - Comprehensive test coverage

**Files**:
- `crates/smelt-types/Cargo.toml`
- `crates/smelt-types/src/lib.rs` - DataType enum, TypedColumn
- `crates/smelt-types/src/parse.rs` - Type string parsing

#### ✅ Phase 2: Database Integration (COMPLETED)

**Updated**: `crates/smelt-db/`

- `SourceColumnDef.data_type` now uses `Option<DataType>` instead of `Option<String>`
- Custom deserializer parses type strings from sources.yml into structured types
- `Column` struct gains `data_type: Option<TypedColumn>` field
- Re-exports `DataType` and `TypedColumn` from smelt-types

**Files**:
- `crates/smelt-db/Cargo.toml` - Added smelt-types dependency
- `crates/smelt-db/src/lib.rs` - Updated SourceColumnDef, imports
- `crates/smelt-db/src/schema.rs` - Added data_type to Column

#### ✅ Phase 3: CLI Unification (COMPLETED)

**Updated**: `crates/smelt-cli/`

- `SourceColumn` retains string storage for serialization compatibility
- New methods: `column_type_str()` for raw string, `data_type()` for parsed type
- Constructor: `SourceColumn::new(name, column_type, description)`
- CLI can access both raw string (for YAML output) and parsed type (for validation)

**Files**:
- `crates/smelt-cli/Cargo.toml` - Added smelt-types dependency
- `crates/smelt-cli/src/config.rs` - Updated SourceColumn with methods

#### ✅ Phase 4: Type Inference Queries (COMPLETED)

**Completed**: January 16, 2026

**New module**: `crates/smelt-db/src/type_inference.rs`

- **TypeContext** - Context for type inference with source and model column types
  - `add_source_column()` - Register source columns from sources.yml
  - `add_model_column()` - Register upstream model columns
  - `add_alias()` - Track table aliases for qualified lookups
  - `lookup_column()` - Look up column type by name with qualifier resolution

- **Expression type inference**:
  - Literal types: integers (SmallInt/Integer/BigInt based on value), decimals, strings, booleans, NULL
  - CAST expressions: Parses target type from TypeSpec AST
  - Function calls: Known return types for common aggregates

- **Aggregate function types**:
  - `COUNT(*)` → BigInt (non-nullable)
  - `SUM(numeric)` → Decimal(38, 10) (nullable)
  - `AVG(numeric)` → Double (nullable)
  - `MIN/MAX(any)` → Preserves argument type (e.g., MIN(timestamp) → Timestamp)
  - `COALESCE(args)` → Type of first argument
  - `NULLIF(a, b)` → Type of first argument (always nullable)
  - Date functions: NOW → Timestamp, CURRENT_DATE → Date
  - String functions: CONCAT, UPPER, etc. → Text
  - Window ranking: ROW_NUMBER, RANK, DENSE_RANK → BigInt
  - Window navigation: LAG, LEAD, FIRST_VALUE, LAST_VALUE → Preserves argument type

- **TypeChecking Salsa query group**:
  - `type_context(path)` - Builds TypeContext with source and upstream model types
  - `typed_model_schema(path)` - Returns ModelSchema with inferred column types

**Files**:
- `crates/smelt-db/src/type_inference.rs` - New module (~300 lines)
- `crates/smelt-db/src/lib.rs` - TypeChecking query group and implementations

#### ✅ Phase 5: LSP Integration (COMPLETED)

**Completed**: January 16, 2026

**What was implemented**:
- **Hover for ref() calls** - Shows typed schema with columns in a table format:
  - Column names with types (e.g., `user_id: INTEGER`, `count: BIGINT`)
  - Nullability indicator (`?` suffix for nullable columns)
  - Source information (from model, computed, etc.)
- **Completion with types** - Column completions include type information:
  - Detail shows `column_name: type_str`
  - Documentation shows expression and source lineage
- **Helper function** - `format_type()` for consistent type display with nullability

**Files modified**:
- `crates/smelt-lsp/Cargo.toml` - Added smelt-types dependency
- `crates/smelt-lsp/src/main.rs` - Integrated TypeChecking queries, updated hover and completion

### Test Results

- `smelt-types`: 13 tests passing
- `smelt-db`: 16 tests passing (including 3 type inference tests, 3 typed schema tests)
- `smelt-cli`: 37 tests passing
- `smelt-parser`: 115 tests passing
- `smelt-backend-duckdb`: 5 tests passing
- `smelt-backend-spark`: 2 tests passing
- Property-based tests: 20 tests passing
- Total workspace: 216+ tests passing
- Cargo clippy passes with no warnings

### Future Work (Deferred)

- Type coercion warnings
- Backend-specific types (HUGEINT for DuckDB, Spark types)
- LSP quick-fixes for type errors

### Type Checker Feature Gaps

The following SQL features are supported by the parser but not yet handled by the type checker:

**High Priority:**
- ✅ **BETWEEN/IN/EXISTS type inference** - Returns Boolean (January 18, 2026)
- ✅ **Unary operators** - NOT (→ Boolean), negation (→ preserve numeric type) (January 18, 2026)
- ✅ **UNION type inference** - Combined result type from multiple SELECT statements (January 18, 2026)

**Medium Priority:**
- ✅ **JOIN column tracking** - Columns from joined tables fully available (January 18, 2026)
- ✅ **LATERAL correlation** - Correlated column references in lateral subqueries (January 18, 2026)
- ✅ **FILTER clause awareness** - Aggregate types preserved with FILTER clauses (January 18, 2026)

**CTE Type Inference Status:**
- ✅ Nested CTEs (WITH inside CTEs) - CTE columns in nested scopes fully resolved
- ✅ Recursive CTEs without explicit column lists - types inferred from anchor term
- ⏸️ UNION type reconciliation in recursive CTEs - uses anchor term types only (acceptable for MVP)

### ✅ Phase 18j: Extended Function Type Inference (January 18, 2026)

Added type inference for 50+ additional SQL functions across math, date/time, string, and aggregate categories.

**Math functions** (`crates/smelt-db/src/type_inference.rs`):
- Preserve argument type: `ABS`, `SIGN`, `ROUND`, `TRUNC`, `CEIL`, `FLOOR`, `MOD`
- Return Double: `POWER`, `SQRT`, `EXP`, `LN`, `LOG`, `LOG10`, `LOG2`
- Trigonometric: `SIN`, `COS`, `TAN`, `ASIN`, `ACOS`, `ATAN`, `ATAN2`, `SINH`, `COSH`, `TANH`
- Constants: `PI`, `RANDOM` (non-nullable)

**Date/time functions**:
- `EXTRACT`, `DATE_PART` → Double
- `MAKE_DATE` → Date, `MAKE_TIME` → Time, `MAKE_TIMESTAMP` → Timestamp
- `AGE` → Interval

**String functions**:
- Text output: `REPLACE`, `TRANSLATE`, `REVERSE`, `REPEAT`, `LPAD`, `RPAD`, `LEFT`, `RIGHT`, `SPLIT_PART`, `INITCAP`, `QUOTE_IDENT`, `QUOTE_LITERAL`
- Integer output: `POSITION`, `STRPOS`

**Aggregate/special functions**:
- `GREATEST`, `LEAST` → preserve first argument type
- `ARRAY_AGG` → Array of argument type
- `STRING_AGG`, `LISTAGG` → Text
- JSON functions (basic): `JSON_BUILD_OBJECT`, `TO_JSON`, etc. → Text

### ✅ Phase 18i: FILTER Clause Awareness (January 18, 2026)

Aggregate functions with FILTER clauses now have proper AST support and type inference works correctly.

**Parser changes** (`crates/smelt-parser/src/ast.rs`):
- `FilterClause` AST wrapper with `expression()` method to get the filter condition
- `FunctionCall::filter_clause()` method to access the optional FILTER clause

**Type inference behavior**:
- FILTER clause does not change the return type of aggregates
- `COUNT(*) FILTER (WHERE ...)` → BigInt (same as COUNT without FILTER)
- `SUM(x) FILTER (WHERE ...)` → Decimal (same as SUM without FILTER)
- `AVG(x) FILTER (WHERE ...)` → Double (same as AVG without FILTER)

**Example:**
```sql
SELECT
    COUNT(*) as total,
    COUNT(*) FILTER (WHERE status = 'completed') as completed,
    SUM(amount) FILTER (WHERE status = 'pending') as pending_sum
FROM smelt.source('raw.orders')
-- total → BIGINT, completed → BIGINT, pending_sum → DECIMAL(38,10)
```

### ✅ Phase 18h: LATERAL Correlation (January 18, 2026)

LATERAL subqueries can now access columns from preceding table references in the FROM clause. Subquery columns are properly registered in the type context.

**Parser changes** (`crates/smelt-parser/src/ast.rs`):
- `TableRef::is_lateral()` - Check if table reference has LATERAL keyword
- `TableRef::subquery()` - Get the subquery if this table reference contains one
- Updated `TableRef::alias()` to correctly detect aliases after subquery closing parenthesis

**Type inference changes** (`crates/smelt-db/src/lib.rs`):
- `process_table_ref()` now handles subqueries (including LATERAL subqueries)
- Infers column types from subquery's SELECT list
- Registers subquery columns under the alias name in the type context
- Column names derived from: alias, column reference name, or generated name

**Example:**
```sql
SELECT u.id, recent.total_amount
FROM smelt.source('raw.users') u
LEFT JOIN LATERAL (
    SELECT SUM(o.amount) as total_amount
    FROM smelt.source('raw.orders') o
    WHERE o.user_id = u.id
) recent ON true
-- recent.total_amount → DECIMAL(38,10) (inferred from SUM aggregate)
```

**Limitations:**
- LATERAL correlation itself (u.id reference inside subquery) is parsed but not yet validated at the type level
- Subqueries without aliases are not tracked (alias is required for column registration)

### ✅ Phase 18g: JOIN Column Tracking (January 18, 2026)

Columns from joined tables are now properly tracked in the type context, enabling type inference for queries with JOINs.

**Changes** (`crates/smelt-db/src/lib.rs`):
- Extracted `process_table_ref()` helper function for processing table references
- `type_context()` now iterates over `from_clause.joins()` in addition to `table_refs()`
- All join types supported: INNER, LEFT, RIGHT, FULL, CROSS

**Example:**
```sql
SELECT o.id, o.amount, u.name
FROM smelt.source('raw.orders') o
INNER JOIN smelt.source('raw.users') u ON o.user_id = u.id
-- All columns (id, amount, name) have proper types from their respective sources
```

### ✅ Phase 18f: UNION Type Inference (January 18, 2026)

Added type inference for UNION and UNION ALL queries. Types from all branches are combined using type promotion.

**Parser changes** (`crates/smelt-parser/src/ast.rs`):
- `SelectStmt::has_union()` - Check if SELECT has a UNION clause
- `SelectStmt::is_union_all()` - Check if UNION is UNION ALL
- `SelectStmt::union_select()` - Get the SELECT statement after UNION

**Type inference** (`crates/smelt-db/src/type_inference.rs`):
- `promote_types()` - Combine two types to the widest compatible type
  - Numeric promotion: SmallInt < Integer < BigInt < Decimal < Double
  - String types unify to Text
  - Timestamp types prefer timezone-aware if either has it
  - Unknown is dominated by any known type
- `infer_select_column_types()` - Infer types for SELECT, recursively handling UNION
  - Combines types from all UNION branches using promotion
  - Supports chained UNIONs (A UNION B UNION C)

**Example:**
```sql
SELECT CAST(1 AS INTEGER) as n
UNION
SELECT CAST(2 AS BIGINT) as n
-- n → BIGINT (promoted from INTEGER + BIGINT)
```

**Limitations:**
- Column resolution in UNION branches with different sources can be ambiguous if unqualified column names conflict

### ✅ Phase 18e: Unary Operator Type Inference (January 18, 2026)

Added type inference for unary operators (NOT and negation).

**Parser changes** (`crates/smelt-parser/src/ast.rs`):
- `BinaryExpr::is_unary()` - Check if expression is unary (no right operand)
- `BinaryExpr::unary_operand_column()` - Extract column reference from unary operand
  - Handles bare identifier tokens not wrapped in expression nodes
  - Supports qualified names (table.column)

**Type inference** (`crates/smelt-db/src/type_inference.rs`):
- **NOT operator** → Boolean (nullable, NOT NULL = NULL)
- **Unary negation** (`-expr`) → Preserves numeric type of operand
  - Works with column references, expressions, and nested unary ops

**Example:**
```sql
SELECT -amount as negative_amount, NOT is_active as inactive
FROM smelt.source('raw.orders')
-- negative_amount → INTEGER (preserves source column type)
-- inactive → Boolean
```

### ✅ Phase 18d: BETWEEN/IN/EXISTS Type Inference (January 18, 2026)

Added type inference for BETWEEN, IN, and EXISTS expressions. All three return Boolean type.

**Changes** (`crates/smelt-db/src/type_inference.rs`):
- BETWEEN expressions → Boolean (nullable, could be NULL if any operand is NULL)
- IN expressions → Boolean (nullable, could be NULL if expr or values contain NULL)
- EXISTS expressions → Boolean (non-nullable, always returns TRUE or FALSE)

### ✅ Phase 18c: CTE Type Inference (January 18, 2026)

Added type inference for Common Table Expressions (CTEs). CTE columns are now registered in `TypeContext` and can be resolved in the main query.

**TypeContext changes** (`crates/smelt-db/src/type_inference.rs`):
- `cte_columns: HashMap<String, TypedColumn>` - CTE column storage
- `cte_names: HashSet<String>` - Known CTE names for shadowing checks
- `add_cte_column()` - Register CTE columns with their inferred types
- `is_cte()` - Check if a name is a known CTE
- Updated `lookup_column()` to check CTEs first (CTEs shadow outer scope)

**CTE column inference** (`crates/smelt-db/src/type_inference.rs`):
- `infer_cte_columns()` - Infer column names and types from CTE query
- Handles explicit column lists (e.g., `WITH cte(a, b) AS ...`)
- Falls back to alias names, then column references, then generated names

**Integration** (`crates/smelt-db/src/lib.rs`):
- Process WITH clause before FROM clause in `type_context()`
- CTEs processed in order for forward references between CTEs
- Bootstrap recursive CTEs with Unknown types before inference
- CTE aliases registered for qualified lookups (e.g., `cte_name.column`)

**Example that now works:**
```sql
WITH daily_totals AS (
    SELECT DATE(created_at) as day, SUM(amount) as total
    FROM smelt.source('raw.orders')
    GROUP BY DATE(created_at)
)
SELECT day, total FROM daily_totals WHERE total > 1000
-- day → DATE, total → DECIMAL(38,10)
```

### ✅ Phase 18b: Comprehensive Type Inference (January 18, 2026)

Added recursive type inference for functions that preserve argument types:

**Parser changes** (`crates/smelt-parser/src/ast.rs`):
- `FunctionCall::arguments()` method to extract argument expressions from function calls
- `BinaryExpr` AST type with `left()`, `right()`, and `operator()` methods
- `Expr::as_binary()` method for binary expression detection

**Type inference improvements** (`crates/smelt-db/src/type_inference.rs`):
- **CASE expressions**: Infers type from first THEN expression (falls back to ELSE)
- **Scalar subqueries**: Infers type from first column in SELECT list (always nullable)
- **MIN/MAX**: Now recursively infers argument type instead of returning Unknown
- **COALESCE**: Infers type from first argument
- **NULLIF**: Infers type from first argument (always nullable)
- **Window ranking functions**: ROW_NUMBER, RANK, DENSE_RANK, NTILE → BigInt
- **Window distribution functions**: CUME_DIST, PERCENT_RANK → Double
- **Window navigation functions**: LAG, LEAD, FIRST_VALUE, LAST_VALUE, NTH_VALUE → Preserves argument type
- **Binary operators**:
  - Logical (AND, OR) → Boolean
  - Comparison (=, <>, <, >, etc.) → Boolean
  - String concatenation (||) → Text
  - Arithmetic (+, -, *, /) → Promotes to widest numeric type

---

## ✅ Phase 19: Table Alias Autocomplete (COMPLETED)

**Completed**: January 17, 2026

### Goal

Enable column autocompletion when typing `alias.` after a table reference alias, improving the developer experience when writing SQL with aliased tables.

### What Was Implemented

- **Parser: `alias()` method for `TableRef`**
  - Extracts explicit AS aliases: `FROM smelt.source('raw.users') AS u` → "u"
  - Extracts implicit aliases: `FROM smelt.source('raw.users') u` → "u"
  - Returns `None` when no alias present
  - Works for both main table refs and JOINed tables

- **LSP: Dot trigger character**
  - Added `.` to completion trigger characters (alongside `'` and `(`)
  - Enables autocompletion immediately after typing `t.`

- **LSP: `QualifiedColumn` completion context**
  - New `CompletionContext::QualifiedColumn(alias)` variant
  - Detects when cursor is after `identifier.` pattern
  - Filters out `smelt.source()` and `smelt.ref()` namespace patterns

- **LSP: Alias-to-source/model resolution**
  - `extract_from_aliases()` parses FROM clause to map aliases to targets
  - Supports both `smelt.source()` (columns from sources.yml) and `smelt.ref()` (columns from model schema)
  - Handles JOINed tables in addition to main table ref

- **Type context: Explicit alias registration**
  - Type context now registers explicit AS aliases in addition to implicit table name aliases
  - Enables `t.column` lookups when `t` is an alias for a source/model

### Implementation Details

**Parser** (`crates/smelt-parser/src/ast.rs`):
- `TableRef::alias()` method walks CST tokens looking for:
  - AS keyword followed by IDENT → explicit alias
  - IDENT after function call but before any other keyword → implicit alias
- Added `FunctionCall::syntax()` method for range comparison

**LSP** (`crates/smelt-lsp/src/main.rs`):
- `extract_alias_before_dot()` helper for detecting `alias.` pattern
- `AliasTarget` enum: `Source { source_name, table_name }` or `Model { model_name }`
- `extract_from_aliases()` parses SELECT statement's FROM clause
- Completion handler returns columns with type information for matched alias

**Database** (`crates/smelt-db/src/lib.rs`):
- `type_context()` now calls `table_ref.alias()` and registers explicit aliases
- Both explicit alias and table name work for column lookups

### Test Coverage

**Parser tests** (`crates/smelt-parser/src/parser.rs`):
- `test_table_ref_explicit_as_alias` - explicit `AS u` alias
- `test_table_ref_implicit_alias` - implicit `u` alias
- `test_table_ref_no_alias` - no alias present
- `test_table_ref_alias_with_ref_call` - alias with `smelt.ref()`
- `test_join_table_ref_alias` - aliases on both main and joined tables

**LSP integration tests** (`crates/smelt-lsp/tests/integration.rs`):
- `test_type_context_registers_explicit_source_alias`
- `test_type_context_registers_implicit_source_alias`
- `test_type_context_registers_table_name_as_fallback_alias`
- `test_source_columns_available_for_completion`
- `test_model_columns_available_for_ref_alias`
- `test_join_aliases_both_registered`

### Test Results

All 261 tests passing:
- 120 smelt-parser tests (including 5 new alias tests)
- 29 smelt-lsp integration tests (including 6 new completion tests)
- 16 smelt-db tests
- 37 smelt-cli tests
- 20 property-based tests
- Others (backend, datagen, types)

Cargo clippy passes with no warnings.

---

## ✅ Phase 20: PostgreSQL Compatibility Testing (COMPLETED)

**Completed**: January 17, 2026

### Goal

Add property-based testing infrastructure to ensure smelt's SQL dialect can handle PostgreSQL SELECT queries correctly, with:
- Parse tree matching: Generate SQL, parse with both smelt and pg_query, verify equivalence
- Type checking: Verify smelt's (future) type inference matches PostgreSQL's actual types
- Known gap tracking: Document and track intentional divergences from PostgreSQL

### What Was Implemented

- **New crate**: `crates/smelt-parser-compat/`
  - `pg_query` integration for PostgreSQL parser comparison (vendored libpg_query)
  - Fingerprint-based semantic equivalence checking
  - Gap tracking infrastructure with regex patterns

- **PostgreSQL grammar generators** (`src/pg_generators.rs`)
  - 30+ proptest strategies for generating valid PostgreSQL SELECT queries
  - Generators for simple SELECT, JOIN, GROUP BY, CTE, window functions, etc.
  - Gap-triggering generators for known unsupported syntax

- **Gap tracking system** (`src/gaps.rs`)
  - `KnownGap` struct with id, description, category, patterns, severity, planned_fix
  - 20+ documented gaps organized by category:
    - `smelt_fails`: PostgreSQL features not yet in smelt (array subscripts, JSON operators, LIKE, etc.)
    - `pg_fails`: smelt extensions not in PostgreSQL (smelt.ref, smelt.source, named parameters)
    - `fingerprint_mismatch`: Semantic differences (intentional divergences)
  - Pattern matching with compiled regex for efficient gap detection
  - `GapSummary` for high-level statistics

- **Parse equivalence tests** (`tests/parse_equivalence.rs`)
  - Property tests with 500+ cases by default
  - Properties: smelt_valid_implies_pg_valid, pg_valid_implies_smelt_valid, semantic_equivalence
  - Tests for all query types: simple SELECT, JOIN, CTE, window functions, CASE, CAST, etc.
  - Explicit unit tests for common SQL patterns

- **Type checking tests** (`tests/type_checking.rs`)
  - testcontainers-based PostgreSQL integration
  - Tests for aggregate types, CAST types, expression types, window function types
  - Placeholder for future smelt type inference comparison
  - Tests are `#[ignore]` by default (require Docker)

- **Known gaps tests** (`tests/known_gaps.rs`)
  - Explicit tests for each documented gap category
  - Tests verifying gap pattern detection
  - Tests confirming which syntax works in both parsers

- **CI integration**
  - Updated `test.yml` to include pg-compat tests (100 cases for quick feedback)
  - New `pg-compat.yml` workflow:
    - Parse equivalence tests (500 cases on every push/PR)
    - Extended tests (2000 cases nightly)
    - Type checking tests (when labeled `run-docker-tests`)
    - Gap report generation on main branch pushes

### Known Parser Gaps (Documented)

**High severity (all fixed in Phase 21):**
- ~~String concatenation operator (`||`)~~ ✅ Fixed January 18, 2026
- ~~`<>` not-equal operator~~ ✅ Fixed January 18, 2026
- ~~Expressions in function arguments~~ ✅ Fixed January 18, 2026
- LIKE/ILIKE/SIMILAR TO operators (still pending)

**Medium severity (planned fix):**
- ~~UNION ALL printing~~ ✅ Fixed January 18, 2026
- ~~NULLS FIRST/LAST printing~~ ✅ Fixed January 18, 2026
- Array subscripts (`arr[1]`)
- JSON operators (`->`, `->>`, etc.)
- Array literals (`ARRAY[1, 2, 3]`)
- INTERSECT/EXCEPT set operations
- Interval literals (`INTERVAL '1 day'`)
- Type-qualified literals (`DATE '2024-01-01'`)
- VALUES clause
- ANY/ALL/SOME array comparisons

**Low severity:**
- GROUPING SETS, CUBE, ROLLUP
- Pattern matching operators (`~`, `~*`, etc.)
- ROW constructor
- NATURAL JOIN
- AT TIME ZONE
- FETCH FIRST/NEXT
- FOR UPDATE/SHARE

**Intentional divergences (smelt extensions):**
- `smelt.ref()` function
- `smelt.source()` function
- Named parameters with `=>` syntax
- Trailing commas (DuckDB-friendly)

### Files Created

- `crates/smelt-parser-compat/Cargo.toml`
- `crates/smelt-parser-compat/src/lib.rs`
- `crates/smelt-parser-compat/src/gaps.rs`
- `crates/smelt-parser-compat/src/normalize.rs`
- `crates/smelt-parser-compat/src/pg_generators.rs`
- `crates/smelt-parser-compat/tests/parse_equivalence.rs`
- `crates/smelt-parser-compat/tests/type_checking.rs`
- `crates/smelt-parser-compat/tests/known_gaps.rs`
- `.github/workflows/pg-compat.yml`

### Files Modified

- `.github/workflows/test.yml` - Added pg-compat test step

### Test Results

The test suite establishes baseline compatibility and documents known gaps. Property tests run 500 cases by default, with 2000 cases in nightly extended runs.

### Phase 20b: Multi-Dialect Compatibility Testing (March 13, 2026)

Extended the compatibility testing infrastructure from PostgreSQL-only to a three-layer multi-dialect verification system:

**Layer 1: pg_query** (existing, unchanged)
- PostgreSQL's vendored C parser with fingerprint-based semantic equivalence

**Layer 2: sqlparser-rs with DatabricksDialect** (new)
- Pure Rust SQL parser, closest available dialect to Spark SQL
- Added `sqlparser = "0.61"` dependency
- `SparkSqlparserResult` type for parsing and comparison
- Validates smelt's normalized output re-parses successfully

**Layer 3: sqlglot via subprocess** (new, optional)
- Python SQL parser/transpiler with explicit `dialect="spark"` support
- Gated behind `SQLGLOT_AVAILABLE` env var
- `SqlglotResult` type with cached availability check

**Layer 4: Spark SQL via Docker** (new, integration)
- `apache/spark` Docker image with `spark-sql` CLI
- Validates SQL with `EXPLAIN <sql>` (parse + plan without executing)
- All tests `#[ignore]` — run on schedule or labeled PRs

**New files:**
- `crates/smelt-parser-compat/src/spark_generators.rs` — proptest generators for QUALIFY, TRANSFORM/AGGREGATE lambdas, PIVOT/UNPIVOT, array subscript/slice
- `crates/smelt-parser-compat/tests/spark_integration.rs` — Docker-based Spark SQL integration tests
- `.github/workflows/compat.yml` — replaces `pg-compat.yml`, adds sqlglot and Spark Docker jobs

**Modified files:**
- `src/lib.rs` — `SparkSqlparserResult`, `SqlglotResult`, `compare_spark_parse_results()`, `compare_sqlglot_parse_results()`, `compare_all_parse_results()`
- `src/gaps.rs` — `dialect` field on `KnownGap`, spark-specific gaps (`spark_trailing_comma`, `spark_named_params`)
- `src/normalize.rs` — Spark keywords (QUALIFY, PIVOT, UNPIVOT, TRANSFORM, AGGREGATE)
- `tests/parse_equivalence.rs` — Spark property tests and unit tests
- `tests/known_gaps.rs` — Spark gap helpers and tests

**Key discovery:** DatabricksDialect supports `::` PostgreSQL-style casts, so `spark_pg_cast` is not actually a gap.

---

## ✅ Phase 21: Parser Gap Fixes (COMPLETED)

**Completed**: January 18, 2026

### Goal

Fix the parser gaps discovered by Phase 20's PostgreSQL compatibility testing infrastructure. These gaps represented syntax that PostgreSQL accepts but smelt-parser rejected or printed incorrectly.

### What Was Fixed

#### 1. `<>` Not-Equal Operator (High Severity)
- **Problem**: smelt-parser didn't recognize `<>` as a not-equal operator
- **Solution**: Added `<>` recognition in lexer using match guards, reusing existing `NE` token
- **Test**: `SELECT * FROM t WHERE a <> b` now parses successfully

#### 2. `||` String Concatenation Operator (High Severity)
- **Problem**: smelt-parser didn't support `||` for string concatenation
- **Solution**:
  - Added `CONCAT` token to syntax_kind.rs
  - Added `||` recognition in lexer
  - Added `parse_concat_expr()` in parser's expression precedence chain
- **Test**: `SELECT 'a' || 'b'` and `SELECT first_name || ' ' || last_name FROM users` now parse

#### 3. UNION ALL Printing (Medium Severity)
- **Problem**: Printer dropped `ALL` from `UNION ALL`, outputting just `UNION`
- **Solution**: Fixed `has_union_all()` function to skip whitespace tokens between `UNION` and `ALL` keywords
- **Test**: `SELECT id FROM a UNION ALL SELECT id FROM b` round-trips correctly

#### 4. NULLS FIRST/LAST Printing (Medium Severity)
- **Problem**: Printer dropped `NULLS FIRST` and `NULLS LAST` from ORDER BY clauses
- **Solution**: Fixed `null_ordering()` method in ast.rs to skip whitespace tokens when looking for FIRST/LAST keywords
- **Test**: `SELECT * FROM t ORDER BY name NULLS FIRST` round-trips correctly

#### 5. Expressions in Function Arguments (High Severity)
- **Problem**: Expressions like `func(a + b)` failed to parse because the parser consumed the identifier before checking for named parameter syntax
- **Solution**:
  - Added `is_named_parameter()` lookahead helper that checks for `IDENT =>` pattern without consuming tokens
  - Refactored `parse_argument()` to use lookahead instead of consuming IDENT first
  - Added `skip_trivia()` calls in `parse_additive_expr()` and `parse_multiplicative_expr()` to handle whitespace before operators
- **Test**: `SELECT func(a + b)`, `SELECT COUNT(id * 2)`, `SELECT COALESCE(a, b + c)` now parse

### Additional Improvements

#### SQL Generator Fixes
During CI testing, property tests discovered that the SQL generators could produce invalid SQL:
- **Star in expressions**: Removed `*` from `simple_expr()` generator (only valid in SELECT list or COUNT(*))
- **Reserved keywords as identifiers**: Added `do`, `to`, `if`, `no`, `of` to keyword blocklist

#### New Gap Patterns
Added gap patterns for remaining syntax differences:
- `star_in_expression`: Detects SQL with `*` used in invalid expression contexts
- `reserved_keyword_as_identifier`: Detects PostgreSQL reserved words used as identifiers

### Files Modified

**smelt-parser crate:**
- `crates/smelt-parser/src/lexer.rs` - Added `<>` and `||` recognition
- `crates/smelt-parser/src/syntax_kind.rs` - Added `CONCAT` token
- `crates/smelt-parser/src/parser.rs` - Added `parse_concat_expr()`, `is_named_parameter()`, whitespace handling
- `crates/smelt-parser/src/printer.rs` - Fixed `has_union_all()` whitespace handling
- `crates/smelt-parser/src/ast.rs` - Fixed `null_ordering()` whitespace handling

**smelt-parser-compat crate:**
- `crates/smelt-parser-compat/src/gaps.rs` - Removed fixed gaps, added new patterns, updated tests
- `crates/smelt-parser-compat/src/pg_generators.rs` - Fixed SQL generators

### Test Results

All tests passing:
- Property tests: 500 cases per run with no failures
- Unit tests: All existing tests pass plus new tests for fixed features
- CI: All checks pass including pg-compat workflow

### Impact

After these fixes:
- **3 high-severity gaps fixed**: `<>`, `||`, expressions in function arguments
- **2 medium-severity gaps fixed**: UNION ALL printing, NULLS FIRST/LAST printing
- **0 high-severity gaps remain**
- Property tests now run cleanly without known false positives

---

## ✅ Phase 1: Async Backend Architecture (COMPLETED)

**Completed**: December 27, 2025

### What Was Implemented

- **smelt-backend** crate with async Backend trait
  - All operations async (execute_sql, create_table_as, create_view_as, etc.)
  - Arrow RecordBatch for data interchange
  - BackendCapabilities for feature detection
  - SqlDialect enum (DuckDB, SparkSQL, PostgreSQL)
  - ExecutionResult and Materialization types

- **Fully async CLI**
  - Converted main() to async with tokio runtime
  - All executor operations async
  - Clean async/await throughout

### Key Changes

- **New crate**: `crates/smelt-backend/` - Backend trait definition
- **Updated**: `crates/smelt-cli/src/main.rs` - Async main function
- **Updated**: `crates/smelt-cli/Cargo.toml` - Added tokio dependency

### Test Results

All 31 existing tests passing after async conversion.

---

## ✅ Phase 2: DuckDB Backend Implementation (COMPLETED)

**Completed**: December 27, 2025

### What Was Implemented

- **smelt-backend-duckdb** crate
  - Full Backend trait implementation for DuckDB
  - Arc<Mutex<Connection>> for thread-safe async access
  - All operations wrapped in tokio::spawn_blocking
  - Comprehensive test suite (5 tests)

- **CLI refactored to use Backend trait**
  - executor.rs converted to backend-agnostic async functions
  - execute_model() and validate_sources() accept any Backend
  - Removed direct DuckDB dependency from CLI

### Implementation Details

**New crate**: `crates/smelt-backend-duckdb/`
- `src/lib.rs` - DuckDbBackend implementation
  - execute_sql: Prepares statement and queries Arrow RecordBatch
  - create_table_as/create_view_as: DDL operations
  - drop_table/view_if_exists: Safe cleanup
  - get_row_count: Efficient counting
  - get_preview: Limited result sets
  - table_exists: Information schema queries
  - ensure_schema: CREATE SCHEMA IF NOT EXISTS
  - dialect(): Returns SqlDialect::DuckDB
  - capabilities(): DuckDB features (QUALIFY, MERGE, CREATE OR REPLACE)

**Updated files**:
- `crates/smelt-cli/src/executor.rs` - Backend-agnostic functions
- `crates/smelt-cli/src/lib.rs` - Updated exports
- `crates/smelt-cli/Cargo.toml` - Added smelt-backend-duckdb dependency

### Test Results

All 36 tests passing (5 new DuckDB backend tests + 31 existing).

---

## ✅ Phase 3: Spark Backend Support (COMPLETED)

**Completed**: December 27, 2025

### What Was Implemented

- **smelt-backend-spark** crate (stub implementation)
  - Defines interface for Spark Connect integration
  - Documents requirements (protoc, spark-connect crate)
  - Working stub that returns appropriate errors
  - 2 tests for creation and stub behavior
  - Ready for real Spark Connect implementation

- **Multi-backend configuration**
  - Target struct supports both DuckDB and Spark
  - Optional fields: database (DuckDB), connect_url/catalog (Spark)
  - BackendType enum for backend selection
  - backend_type() method determines backend from config

- **Feature-flagged compilation**
  - default = ["duckdb"]
  - spark = ["smelt-backend-spark"]
  - Spark backend optional to reduce binary size
  - Clear error if Spark target used without --features spark

- **Runtime backend selection**
  - Box<dyn Backend> for polymorphism
  - Backend created based on target configuration
  - Prints backend type and connection details at startup

### Configuration Format

```yaml
# DuckDB target
targets:
  dev:
    type: duckdb
    database: dev.duckdb
    schema: main

# Spark target
targets:
  prod:
    type: spark
    connect_url: sc://localhost:15002
    catalog: spark_catalog
    schema: production
```

### Implementation Details

**New crate**: `crates/smelt-backend-spark/`
- Stub implementation of Backend trait
- Documents Spark Connect requirements
- Qualified table names: catalog.schema.table
- Future work: Real Spark Connect integration

**Updated files**:
- `crates/smelt-cli/src/config.rs` - Multi-backend Target struct
- `crates/smelt-cli/src/main.rs` - Backend selection logic
- `crates/smelt-cli/Cargo.toml` - Feature flags

### Benefits

- **Clean separation**: Each backend is its own crate
- **Optional dependencies**: Spark only when needed
- **Extensible**: Easy to add new backends
- **Backward compatible**: Existing configs still work
- **Validated architecture**: Multi-backend pattern proven with stub

### Test Results

All 38 tests passing (5 DuckDB + 2 Spark + 18 CLI + 10 db + 3 parser).

---

## ✅ Phase 4: Smelt SQL — Multi-Dialect Superset (COMPLETED March 12, 2026)

**Status**: Parser extensions complete for all 5 sub-phases. Backend rewrite rules are deferred to future work.

### Philosophy

Smelt SQL is a **logical SQL superset** built on a PostgreSQL-compatible base, cherry-picking the best syntax from multiple dialects. Users write clean, expressive SQL; smelt rewrites it to the target backend's dialect automatically. This aligns with smelt's core design: users specify *what* to compute, the framework handles *how*.

- PostgreSQL remains the baseline (validated by `smelt-parser-compat` / `pg_query`)
- Extensions are added when they provide a clear DX improvement
- Trailing commas (DuckDB-style) are already accepted as the first such extension
- Each new feature needs: parser support, CST printing, pg_query gap tracking (if non-PG), and backend rewrite rules where applicable

### Sub-phases

#### ✅ Phase 4a: QUALIFY clause (COMPLETED March 12, 2026)

**What**: `QUALIFY` clause after `HAVING` for filtering on window function results. New `QUALIFY_KW` keyword, `QUALIFY_CLAUSE` node, `QualifyClause` AST wrapper, printer support, pg_query gap entry. Parser tests + round-trip tests.

---

#### ✅ Phase 4b: Lambda expressions and array functions (COMPLETED March 12, 2026)

**What**: Lambda arrow syntax (`->`) for function arguments. New `THIN_ARROW` token, `LAMBDA_EXPR`/`LAMBDA_PARAM_LIST` nodes, `LambdaExpr` AST wrapper. Supports single-param (`x -> x + 1`) and multi-param (`(acc, x) -> acc + x`). Disambiguates `(a, b) -> expr` from parenthesized expression via lookahead. Keywords like `FILTER` can now be used as function names when followed by `(`. pg_query gap entry added.

---

#### ✅ Phase 4c: PIVOT / UNPIVOT (COMPLETED March 12, 2026)

**What**: PIVOT/UNPIVOT clauses on table references. New `PIVOT_KW`/`UNPIVOT_KW` keywords, `PIVOT_CLAUSE`/`UNPIVOT_CLAUSE`/`PIVOT_IN_LIST` nodes, `PivotClause`/`UnpivotClause` AST wrappers. Parsed after table ref (alongside TABLESAMPLE). Supports aggregate expressions, FOR column, and IN value lists. pg_query gap entry added.

---

#### ✅ Phase 4d: Array subscript notation (COMPLETED March 12, 2026)

**What**: Postfix `[expr]` subscript and `[expr:expr]` slice syntax. New `LBRACKET`/`RBRACKET`/`COLON` tokens, `ARRAY_SUBSCRIPT`/`ARRAY_SLICE` nodes, `ArraySubscript`/`ArraySlice` AST wrappers. Supports chaining (`matrix[1][2]`) and on function results (`ARRAY(1,2,3)[1]`). Closed `array_subscript` gap in `gaps.rs`.

---

#### ✅ Phase 4e: DATE literal normalization (COMPLETED March 12, 2026)

**What**: Type keywords (DATE, TIME, TIMESTAMP, INTERVAL) followed by string literals are now recognized as typed literals. Both `DATE '2024-01-01'` and `DATE('2024-01-01')` parse correctly. The former is a typed literal, the latter a function call — both are valid in the AST.

---

#### ✅ Phase 4f: Trailing comma removal + EXPLODE/UNNEST renaming (COMPLETED March 15, 2026)

**What**: Two dialect rewrites in `smelt-dialect`. (1) Trailing commas in SELECT and GROUP BY lists are stripped for Spark and PostgreSQL via `supports_trailing_commas` capability flag. (2) EXPLODE↔UNNEST function renaming based on dialect: DuckDB/PostgreSQL normalize to UNNEST, Spark normalizes to EXPLODE.

---

### Dialect Rewrite Summary

| Feature | Smelt SQL | DuckDB | Spark | PostgreSQL |
|---------|-----------|--------|-------|------------|
| QUALIFY | ✅ Native | ✅ Pass-through | 🔄 Subquery wrap | 🔄 Subquery wrap |
| Lambda `->` | ✅ Native | ✅ Pass-through | ✅ Pass-through | ❌ Error (no support) |
| EXPLODE/UNNEST | ✅ Both accepted | ✅ → UNNEST | ✅ → EXPLODE | ✅ → UNNEST |
| PIVOT | ✅ Native | ✅ Pass-through | ✅ Pass-through | ❌ Error or crosstab |
| Array subscript | ✅ Native | ✅ Pass-through | ✅ Pass-through | ✅ Pass-through |
| DATE literal | ✅ Both forms | 🔄 → `DATE '...'` | 🔄 → `DATE('...')` | 🔄 → `DATE '...'` |
| `::` cast | ✅ Native | ✅ Pass-through | 🔄 → `CAST()` | ✅ Pass-through |
| Trailing commas | ✅ Accepted | ✅ Pass-through | ✅ Stripped | ✅ Stripped |

---

## 🔮 Future Phases (Not Started)

### Phase 5: Named Parameter Compilation

**Value**: Make named parameters functional in CLI execution

**Work**:
- Parse `filter =>` parameter expressions
- Inject as WHERE clause in compiled SQL
- Validate parameter types and compatibility

**Effort**: Medium

---

### Phase 6: Incremental Materialization *(Partially Complete)*

**Basic incremental materialization completed in Phase 9** (December 27, 2024): DELETE+INSERT strategy, partition management, DuckDB backend support, CLI with `--event-time-start`/`--event-time-end`. Further enhanced with optimizer integration (March 14, 2026): safety checks, YAML frontmatter detection, cube split composition.

**Remaining work** (advanced incremental): strategy expansion (MERGE, APPEND, INSERT_OVERWRITE), temporal dependency inference, data latency, backfill intelligence, schema evolution. See [docs/plans/20260322-incremental-model-support.md](plans/20260322-incremental-model-support.md) for the comprehensive plan.

**Design**: See [DESIGN.md](DESIGN.md#incremental-table-builds) for full specification. Note: `lookback_days` and `-- @materialize` annotation syntax described there are superseded by the current approach (YAML frontmatter config, AST-inferred temporal dependencies).

**Effort**: Medium-High (for remaining work)

---

### Phase 7: Additional Backends

**Candidates**:
- PostgreSQL (via tokio-postgres)
- BigQuery (via google-cloud-bigquery)
- Snowflake (via snowflake-connector-rs)
- Databricks SQL (via REST API)

**Pattern**: Each backend is a new crate implementing Backend trait

**Effort**: Low-Medium per backend (architecture is proven)

---

## ✅ Phase 8: JOIN Syntax Support (COMPLETED)

**Completed**: December 27, 2024

### What Was Implemented

- **Full JOIN syntax support** in parser
  - All JOIN types: INNER, LEFT, RIGHT, FULL OUTER, CROSS
  - ON conditions with expressions
  - USING conditions with column lists
  - Proper error recovery for incomplete JOINs

- **Lexer updates**
  - 9 new keywords: JOIN, INNER, LEFT, RIGHT, FULL, OUTER, CROSS, ON, USING
  - All keywords recognized case-insensitively

- **Parser enhancements**
  - parse_join_clause() with complete JOIN type handling
  - parse_join_condition() for ON and USING clauses
  - Updated parse_from_clause() to parse JOINs instead of comma-separated tables
  - LSP-friendly error recovery maintains usable CST even with partial syntax

- **AST wrappers**
  - JoinClause type with join_type(), table_ref(), and condition() methods
  - JoinType enum (Inner, Left, Right, Full, Cross)
  - JoinCondition type with is_on(), is_using(), on_expression(), using_columns()
  - FromClause::joins() iterator

- **Examples updated**
  - example2_naive.rs and example2_optimized.rs now use explicit CROSS JOIN
  - Comma-separated FROM syntax no longer supported (breaking change)

### Test Results

All 12 parser tests passing, including:
- INNER, LEFT, RIGHT, FULL, CROSS JOIN variants
- ON and USING conditions
- Multiple JOINs in sequence
- Error recovery for missing table refs and conditions

### Breaking Changes

**Removed comma-separated FROM syntax:**
- Old: `FROM users, orders`
- New: `FROM users CROSS JOIN orders`
- Justification: Aligns with design doc requirement for explicit JOIN syntax

---

## ✅ Phase 9: Basic Incremental Materialization (COMPLETED)

**Completed**: December 27, 2024

### What Was Implemented

- **Backend trait enhancements** for incremental updates
  - `MaterializationStrategy` enum (FullRefresh | Incremental)
  - `PartitionSpec` type (column + values for DELETE clause)
  - `execute_model_incremental()` method with strategy parameter
  - `delete_partitions()` and `insert_into_from_query()` primitives

- **DuckDB backend** incremental support
  - DELETE by partition using IN clause with SQL escaping
  - INSERT INTO ... SELECT pattern
  - Auto-creates table on first run (graceful degradation)
  - Spark backend updated with stub implementations

- **SQL model examples** demonstrating materialization strategies
  - `examples/models/transactions.sql` - Source model with timestamped events
  - `examples/models/daily_revenue.sql` - Daily aggregation using incremental materialization
  - Configuration in `examples/smelt.yml` with incremental settings
  - Source data setup with 30 days of transaction data (setup_sources.sql)
  - sources.yml updated with transactions table schema

- **Removed** `smelt-examples` Rust crate
  - Not the right pattern for this project
  - Replaced with SQL model examples in examples/ workspace

- **CLI integration** for incremental execution
  - CLI flags: `--event-time-start` and `--event-time-end` for time range specification
  - Time range parsing and validation (ISO 8601 YYYY-MM-DD format)
  - SQL transformation via `inject_time_filter()` to add WHERE clause filtering
  - Partition date generation from time ranges
  - End-to-end orchestration in `main.rs` (incremental vs full refresh path)

### Implementation Details

**New types** (`crates/smelt-backend/src/types.rs`):
- `PartitionSpec { column: String, values: Vec<String> }` - Specifies which partitions to update
- `MaterializationStrategy::FullRefresh` - DROP + CREATE (existing behavior)
- `MaterializationStrategy::Incremental { partition }` - DELETE + INSERT by partition

**Backend trait** (`crates/smelt-backend/src/lib.rs`):
- `execute_model_incremental()` - Strategy-aware model execution with default implementation
- `delete_partitions()` - DELETE WHERE column IN (values) - trait method, backends implement
- `insert_into_from_query()` - INSERT INTO ... SELECT - trait method, backends implement

**DuckDB backend** (`crates/smelt-backend-duckdb/src/lib.rs`):
- Implements delete_partitions using IN clause with SQL escaping (single quote escape)
- Implements insert_into_from_query using standard SQL
- Auto-creates table on first run if it doesn't exist

**SQL Examples** (`examples/`):
- `models/daily_revenue.sql` - Aggregates transactions by date and user
- `smelt.yml` - Configures incremental: { enabled: true, partition_column: revenue_date }
- `sources.yml` - Defines transactions table schema
- `setup_sources.sql` - Populates 30 days of sample transaction data

**CLI Integration** (`crates/smelt-cli/src/`):
- `main.rs` - Orchestrates incremental vs full refresh execution
  - Parses `--event-time-start` and `--event-time-end` CLI arguments
  - Loads incremental config from `smelt.yml` per model
  - Determines execution strategy (incremental if both config + time range present)
  - Calls `inject_time_filter()` to transform SQL with WHERE clause
  - Generates partition dates using `generate_partition_dates()`
  - Invokes `executor::execute_model_incremental()` with partition spec
- `transformer.rs` - AST-based SQL transformation
  - `inject_time_filter()` adds time range WHERE clause to source queries
  - Uses Rowan parser for precise text replacement
  - Preserves existing WHERE clauses (appends with AND)
- `config.rs` - Incremental configuration types
  - `IncrementalConfig` with `event_time_column` and `partition_column`
  - `Config::get_incremental()` method for per-model settings

### Design Decisions

**DELETE+INSERT vs MERGE**:
- Chose DELETE+INSERT for universal backend support
- MERGE support varies (DuckDB: yes, Spark: Delta only, PostgreSQL: 15+ only)
- DELETE+INSERT works everywhere and is easy to reason about

**Partition specification**:
- Simple string-based partition values (not typed)
- Supports multiple partitions in one operation (IN clause)
- Future: Could add partition expressions, range specifications

**First run handling**:
- Auto-creates table if it doesn't exist (check with table_exists)
- Avoids separate schema management
- Graceful degradation to full refresh on first run

**Configuration in YAML, not SQL comments**:
- Incremental settings in smelt.yml, not annotation parsing
- Avoids need to implement annotation parser (Phase deferred indefinitely)
- Still demonstrates the intent and validates the backend API

### Future Work (Phase 10+)

Phase 9 includes complete end-to-end incremental materialization with CLI integration. Future enhancements could include:

- **Watermark tracking** - Automatically track last processed timestamp and resume from watermark, eliminating need to manually specify time ranges each run
- **Non-date partition support** - Support hourly timestamps, string categories, integer ranges (currently limited to daily date partitions)
- **Auto-detection** - Infer when incremental is safe from SQL semantics
- **Partition inference** - Extract partition column from WHERE clauses automatically
- **Multi-column partitions** - Support composite partition keys (e.g., date + region)
- **MERGE support** - Use MERGE/UPSERT for backends that support it (instead of DELETE+INSERT)

### Test Results

- `cargo clippy --all-targets` passes with no warnings
- Backend trait compiles successfully
- DuckDB backend implements all new methods
- Spark backend updated with stub implementations
- SQL models parse correctly

---

## ✅ Phase 10: Expression Enhancements (COMPLETED)

**Completed**: December 29, 2024

### What Was Implemented

- **CASE expressions** - Both searched and simple forms
  - `CASE WHEN condition THEN result ... ELSE default END` (searched)
  - `CASE expr WHEN value THEN result ... ELSE default END` (simple)
  - Multiple WHEN clauses supported
  - Optional ELSE clause

- **CAST expressions** - Standard SQL and PostgreSQL syntax
  - `CAST(expr AS type)` - Standard SQL syntax
  - `expr::type` - PostgreSQL double-colon operator
  - Type specifications with parameters: `VARCHAR(255)`, `DECIMAL(10,2)`

- **Subqueries** - In SELECT list and FROM clause
  - Scalar subqueries in SELECT: `(SELECT COUNT(*) FROM orders)`
  - Derived tables in FROM: `FROM (SELECT ...) AS alias`
  - Proper SELECT statement parsing within parentheses

- **BETWEEN expressions**
  - `expr BETWEEN low AND high` syntax
  - Expression-based bounds (not just literals)

- **IN expressions** - Both value lists and subqueries
  - Value lists: `status IN ('active', 'pending')`
  - Subqueries: `id IN (SELECT user_id FROM orders)`

- **EXISTS expressions**
  - `EXISTS (SELECT ... FROM ...)` syntax
  - Subquery validation

- **Unary operators** - Negative numbers and NOT
  - Unary minus: `-1`, `-amount`
  - Recursive unary chaining: `--x`
  - NOT operator for boolean negation

### Implementation Details

**Lexer updates** (`crates/smelt-parser/src/lexer.rs`):
- Added 11 new keywords: CASE, WHEN, THEN, ELSE, END, CAST, BETWEEN, IN, EXISTS, ANY, SOME
- Added DOUBLE_COLON (`::`) operator for PostgreSQL casts
- Added MINUS operator (previously missing, causing `-1` to fail)

**Parser enhancements** (`crates/smelt-parser/src/parser.rs`):
- `parse_case_expr()` - Handles both simple and searched CASE forms
- `parse_when_clause()` - Parses WHEN...THEN clauses
- `parse_cast_expr()` - Standard CAST(... AS ...) syntax
- `parse_type_spec()` - Type names with optional parameters
- `parse_subquery()` - SELECT statements in parentheses
- `parse_exists_expr()` - EXISTS (subquery) syntax
- `parse_between_expr()` - BETWEEN low AND high
- `parse_in_expr()` - IN (values/subquery) with discrimination
- `parse_unary_expr()` - Unary minus and NOT operators
- Updated `parse_primary_expr()` to detect CASE, CAST, EXISTS, subqueries, and `::` casts
- Updated `parse_comparison_expr()` to handle BETWEEN and IN
- Updated `parse_table_ref()` to support subqueries in FROM clause
- Updated `at_expression_start()` to include new expression keywords

**AST wrappers** (`crates/smelt-parser/src/ast.rs`):
- `CaseExpr` - with `case_value()`, `when_clauses()`, `else_expr()` methods
- `WhenClause` - with `condition()`, `result()` methods
- `CastExpr` - with `expression()`, `type_spec()`, `is_double_colon_cast()` methods
- `TypeSpec` - with `type_name()`, `full_text()` methods
- `Subquery` - with `select_stmt()` method
- `BetweenExpr` - with `lower_bound()`, `upper_bound()` methods
- `InExpr` - with `is_subquery()`, `subquery()`, `values()` methods
- `ExistsExpr` - with `subquery()` method
- Updated `Expr` with `as_case()`, `as_cast()`, `as_subquery()`, `as_between()`, `as_in()`, `as_exists()` methods

### Test Results

All 29 parser tests passing, including 15 new tests for Phase 10:
- `test_case_searched` - Searched CASE with multiple WHENs
- `test_case_simple` - Simple CASE matching values
- `test_case_no_else` - CASE without ELSE clause
- `test_cast_standard` - CAST(price AS INTEGER)
- `test_cast_postgres_double_colon` - price::INTEGER
- `test_cast_with_params` - CAST(name AS VARCHAR(255))
- `test_cast_decimal` - CAST(amount AS DECIMAL(10, 2))
- `test_subquery_in_select` - Scalar subquery in SELECT list
- `test_subquery_in_from` - Derived table in FROM clause
- `test_between` - price BETWEEN 10 AND 100
- `test_between_with_expressions` - BETWEEN with column references
- `test_in_values` - IN with string literals
- `test_in_numbers` - IN with numeric literals
- `test_in_subquery` - IN with subquery
- `test_exists` - EXISTS with correlated subquery
- `test_complex_nested_expressions` - Combined CASE, cast, IN
- `test_unary_minus` - Negative number literals

### Bug Fixes

- **Fixed missing MINUS operator** - The lexer was not handling `-` as a standalone token, causing it to fall through to ERROR. This made unary minus and negative numbers fail to parse.
- **Fixed expression precedence** - Used `parse_comparison_expr()` in WHEN/THEN clauses instead of `parse_expression()` to avoid consuming keywords like WHEN, ELSE, END.

---

## ✅ Phase 11: Core SQL Clauses (COMPLETED)

**Completed**: December 29, 2024

### What Was Implemented

- **ORDER BY clause** - Comprehensive sorting support
  - Multiple sort expressions: `ORDER BY col1 DESC, col2 ASC`
  - Sort direction: `ASC` / `DESC` (optional, defaults to ASC)
  - Null ordering: `NULLS FIRST` / `NULLS LAST`
  - Expression-based ordering (not just column references)

- **LIMIT clause** - Result set size control
  - Numeric limits: `LIMIT 10`
  - `LIMIT ALL` for explicit unlimited results
  - `OFFSET n` for pagination: `LIMIT 10 OFFSET 20`

- **HAVING clause** - Post-aggregation filtering
  - `HAVING COUNT(*) > 5` after GROUP BY
  - Full expression support (same as WHERE)
  - Proper ordering requirement (must follow GROUP BY)

- **DISTINCT keyword** - Duplicate elimination
  - `SELECT DISTINCT city FROM users`
  - `SELECT ALL` also supported (explicit default)
  - Parsed after SELECT, before column list

- **SELECT without FROM** - Constant expressions
  - `SELECT 1 + 1 AS result`
  - FROM clause now optional in parser
  - Enables calculations and function testing

### Implementation Details

**Lexer updates** (`crates/smelt-parser/src/lexer.rs`):
- Added 11 new keywords: ORDER, LIMIT, OFFSET, HAVING, DISTINCT, ALL, ASC, DESC, NULLS, FIRST, LAST
- All keywords recognized case-insensitively

**Parser enhancements** (`crates/smelt-parser/src/parser.rs`):
- `parse_having_clause()` - HAVING expression parsing
- `parse_order_by_clause()` - Comma-separated ORDER BY items
- `parse_order_by_item()` - Single sort specification with direction and null ordering
- `parse_limit_clause()` - LIMIT value (number/ALL) with optional OFFSET
- Updated `parse_select_stmt()` to handle DISTINCT/ALL and all new clauses
- Updated `at_keyword_that_ends_table_ref()` to include new keywords
- Made FROM clause optional (SELECT without FROM now valid)
- Proper clause ordering enforced: SELECT [DISTINCT] ... [FROM] ... [WHERE] ... [GROUP BY] ... [HAVING] ... [ORDER BY] ... [LIMIT]

**AST wrappers** (`crates/smelt-parser/src/ast.rs`):
- `HavingClause` - with `expression()` method
- `OrderByClause` - with `items()` iterator
- `OrderByItem` - with `expression()`, `direction()`, `null_ordering()` methods
- `SortDirection` enum (Asc, Desc)
- `NullOrdering` enum (First, Last)
- `LimitClause` - with `limit_value()`, `offset_value()` methods
- `LimitValue` enum (Number, All)
- Updated `SelectStmt` with:
  - `having_clause()` method
  - `order_by_clause()` method
  - `limit_clause()` method
  - `is_distinct()` method

**SyntaxKind updates** (`crates/smelt-parser/src/syntax_kind.rs`):
- Added 11 new keyword tokens
- Added 4 new composite node types: HAVING_CLAUSE, ORDER_BY_CLAUSE, ORDER_BY_ITEM, LIMIT_CLAUSE
- Updated `is_keyword()` to include all new keywords

### Test Results

All 43 parser tests passing, including 14 new tests for Phase 11:
- `test_order_by_basic` - Simple ascending sort
- `test_order_by_multiple` - Multiple sort columns
- `test_order_by_nulls` - DESC NULLS LAST
- `test_order_by_nulls_first` - ASC NULLS FIRST
- `test_order_by_expression` - Complex expression ordering (CASE)
- `test_limit_offset` - LIMIT 10 OFFSET 20
- `test_limit_only` - LIMIT without OFFSET
- `test_limit_all` - LIMIT ALL
- `test_having_clause` - Simple HAVING with COUNT
- `test_having_complex_expression` - HAVING with AND
- `test_distinct` - SELECT DISTINCT
- `test_select_all` - SELECT ALL
- `test_complete_query` - All clauses combined
- `test_select_without_from` - SELECT 1 + 1

Cargo clippy passes with no warnings.

### Design Decisions

**FROM clause made optional**:
- Aligns with PostgreSQL and DuckDB behavior
- Enables `SELECT 1 + 1` for testing expressions
- Useful for constant value generation

**HAVING requires GROUP BY semantically but not syntactically**:
- Parser accepts HAVING without GROUP BY (for error recovery)
- Semantic validation should flag this as an error (future work)
- Matches SQL standard error handling approach

**LIMIT ALL vs no LIMIT**:
- Both are valid and equivalent
- LIMIT ALL is explicit about intent
- Useful when overriding default limits

**Expression-based ORDER BY**:
- Supports arbitrary expressions, not just column references
- Enables sorting by CASE expressions, computations, etc.
- Consistent with WHERE and HAVING expression support

---

## ✅ Phase 12: Window Functions (COMPLETED)

**Completed**: December 29, 2024

### What Was Implemented

- **Window function syntax** - Full OVER clause support
  - `OVER (ORDER BY col)` - Simple window ordering
  - `OVER (PARTITION BY col ORDER BY col)` - Partitioned windows
  - `OVER window_name` - Named window references (parsed, not yet implemented in executor)

- **PARTITION BY clause** - Window partitioning
  - Single column: `PARTITION BY user_id`
  - Multiple columns: `PARTITION BY user_id, category`
  - Full expression support (same as GROUP BY)

- **Window frames** - Frame specification for aggregates
  - Frame units: `ROWS`, `RANGE`, `GROUPS`
  - Frame bounds:
    - `UNBOUNDED PRECEDING` / `UNBOUNDED FOLLOWING`
    - `CURRENT ROW`
    - `N PRECEDING` / `N FOLLOWING` (numeric offsets)
  - Frame extents:
    - Single bound: `ROWS UNBOUNDED PRECEDING`
    - Between bounds: `ROWS BETWEEN 3 PRECEDING AND CURRENT ROW`

- **Common window functions** - All standard SQL window functions
  - Row numbering: `ROW_NUMBER()`, `RANK()`, `DENSE_RANK()`
  - Offset functions: `LAG()`, `LEAD()`
  - Aggregates: `SUM()`, `AVG()`, `COUNT()`, etc. with OVER clause
  - All functions work with PARTITION BY, ORDER BY, and frame specifications

### Implementation Details

**Lexer updates** (`crates/smelt-parser/src/lexer.rs`):
- Added 11 new keywords: OVER, PARTITION, WINDOW, ROWS, RANGE, GROUPS, UNBOUNDED, PRECEDING, FOLLOWING, CURRENT, ROW
- All keywords recognized case-insensitively

**Parser enhancements** (`crates/smelt-parser/src/parser.rs`):
- `parse_window_spec()` - OVER clause with inline or named window
- `parse_partition_by()` - Comma-separated partition expressions
- `parse_window_frame()` - Frame unit and extent specification
- `parse_frame_bound()` - Individual frame boundary parsing
- Updated `parse_primary_expr()` to detect OVER after function calls
- Window specs attached to function calls in both simple and namespaced forms

**AST wrappers** (`crates/smelt-parser/src/ast.rs`):
- `WindowSpec` - with `partition_by()`, `order_by()`, `frame()`, `window_name()` methods
- `PartitionByClause` - with `expressions()` iterator
- `WindowFrame` - with `unit()`, `bounds()` methods
- `FrameUnit` enum (Rows, Range, Groups)
- `FrameBound` - with `text()` method for bound representation
- Updated `Expr` with `window_spec()` method

**SyntaxKind updates** (`crates/smelt-parser/src/syntax_kind.rs`):
- Added 11 new keyword tokens
- Added 4 new composite node types: WINDOW_SPEC, PARTITION_BY_CLAUSE, WINDOW_FRAME, FRAME_BOUND
- Updated `is_keyword()` to include all new keywords

### Test Results

All 58 parser tests passing, including 15 new tests for Phase 12:
- `test_window_function_basic` - ROW_NUMBER() with ORDER BY
- `test_window_function_partition` - SUM with PARTITION BY and ORDER BY
- `test_window_frame_rows` - ROWS BETWEEN 3 PRECEDING AND CURRENT ROW
- `test_window_frame_unbounded` - ROWS UNBOUNDED PRECEDING
- `test_window_frame_range` - RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
- `test_window_frame_groups` - GROUPS BETWEEN 1 PRECEDING AND 1 FOLLOWING
- `test_multiple_window_functions` - Multiple window functions in same query
- `test_window_function_with_frame_offset` - Numeric offset bounds
- `test_window_function_partition_multiple_columns` - Multi-column partitioning
- `test_window_function_range_unbounded_following` - UNBOUNDED FOLLOWING
- `test_window_function_with_aggregate` - AVG as window function
- `test_window_function_rank` - RANK() function
- `test_window_function_dense_rank` - DENSE_RANK() function
- `test_window_function_lag` - LAG() offset function
- `test_window_function_lead` - LEAD() offset function

Cargo clippy passes with no warnings.

### Design Decisions

**Window specs as separate nodes**:
- WINDOW_SPEC is a child of the expression containing the function call
- This allows easy detection of window functions via `expr.window_spec()`
- Preserves accurate position tracking for LSP features

**Frame specification flexibility**:
- Parser accepts all three frame units (ROWS, RANGE, GROUPS)
- Single bound form defaults to `CURRENT ROW` as upper bound
- Semantic validation of frame specs deferred to future work

**Named window references**:
- Parser accepts `OVER window_name` syntax
- Named windows can reference a WINDOW clause (not yet implemented)
- Foundation for future WINDOW clause support

**Reuse of ORDER BY parsing**:
- Window ORDER BY uses same `parse_order_by_clause()` as statement-level
- Enables consistent handling of ASC/DESC and NULLS FIRST/LAST
- Code reuse reduces parser complexity

### Future Work

**Semantic validation** (not implemented):
- Validate frame bounds make sense (start before end)
- Check that RANGE/GROUPS have ORDER BY
- Verify window function usage (not in WHERE, etc.)

**WINDOW clause** (not implemented):
- Statement-level `WINDOW name AS (...)` definitions
- Window reference resolution in OVER clauses
- Window inheritance and extension

**Execution support** (not implemented):
- Actual window function execution in backends
- Frame calculation algorithms
- Optimization of multiple windows over same partition

---

## ✅ Phase 13: Common Table Expressions (CTEs) (COMPLETED)

**Completed**: December 29, 2024

### What Was Implemented

- **WITH clause** - Full CTE support
  - `WITH cte_name AS (SELECT ...) SELECT ... FROM cte_name`
  - Multiple CTEs: `WITH cte1 AS (...), cte2 AS (...) SELECT ...`
  - Optional column list: `WITH summary(dept, total) AS (...)`
  - Nested CTEs: CTEs can have their own WITH clauses

- **RECURSIVE CTEs** - Recursive query support
  - `WITH RECURSIVE tree AS (SELECT ... UNION ALL SELECT ...) SELECT ...`
  - Base case + recursive case pattern
  - Proper UNION support for recursive queries

- **UNION clause** - Set operations
  - `SELECT ... UNION SELECT ...` - Remove duplicates
  - `SELECT ... UNION ALL SELECT ...` - Keep all rows
  - Required for recursive CTEs

### Implementation Details

**Lexer updates** (`crates/smelt-parser/src/lexer.rs`):
- Added 3 new keywords: WITH, RECURSIVE, UNION
- All keywords recognized case-insensitively

**Parser enhancements** (`crates/smelt-parser/src/parser.rs`):
- `parse_with_clause()` - WITH keyword, optional RECURSIVE, comma-separated CTEs
- `parse_cte()` - CTE name, optional column list, AS (query)
- Updated `parse_file()` to accept WITH as start of SELECT statement
- Updated `parse_select_stmt()` to parse WITH clause before SELECT keyword
- Added UNION support after LIMIT clause with optional ALL keyword
- Column list parsing with lookahead to distinguish from query parentheses
- Recursive calls support nested CTEs (WITH inside WITH)

**AST wrappers** (`crates/smelt-parser/src/ast.rs`):
- `WithClause` - with `is_recursive()`, `ctes()` methods
- `Cte` - with `name()`, `query()`, `column_names()` methods
- Updated `SelectStmt` with `with_clause()` method
- Column list extraction from CTE definition

**SyntaxKind updates** (`crates/smelt-parser/src/syntax_kind.rs`):
- Added 3 new keyword tokens: WITH_KW, RECURSIVE_KW, UNION_KW
- Added 2 new composite node types: WITH_CLAUSE, CTE
- Updated `is_keyword()` to include all new keywords

### Test Results

All 66 parser tests passing, including 8 new tests for Phase 13:
- `test_cte_basic` - Simple CTE with single query
- `test_cte_multiple` - Multiple CTEs separated by commas
- `test_cte_recursive` - Recursive CTE with UNION ALL
- `test_cte_nested` - CTE containing another WITH clause
- `test_cte_with_window_function` - CTE using window functions
- `test_cte_with_column_list` - CTE with explicit column names
- `test_union_basic` - Simple UNION query
- `test_union_all` - UNION ALL preserving duplicates

Cargo clippy passes with no warnings.

### Design Decisions

**WITH clause before SELECT**:
- Follows SQL standard ordering
- WITH must come first, before SELECT keyword
- Enables clean separation of CTEs from main query

**Column list as optional**:
- Parser accepts both `WITH name AS (...)` and `WITH name(col1, col2) AS (...)`
- Column list parsing uses lookahead to avoid ambiguity with query parentheses
- Column names extracted via `column_names()` method for future validation

**UNION support added**:
- Required for recursive CTEs (base case UNION ALL recursive case)
- Supports both UNION (deduplicate) and UNION ALL (keep duplicates)
- Positioned after LIMIT clause, allowing multiple SELECT statements to be combined
- Recursive parsing allows chaining: `SELECT ... UNION SELECT ... UNION SELECT ...`

**Nested CTEs allowed**:
- CTEs can contain WITH clauses themselves
- Recursive `parse_select_stmt()` call handles nesting naturally
- Enables complex query organization and modularity

**Subquery reuse**:
- CTE query uses existing SUBQUERY node type
- Consistent with subqueries in FROM and expressions
- Simplifies AST structure and parsing logic

### Future Work

**Semantic validation** (not implemented):
- Validate CTE references exist and are in scope
- Check recursive CTE structure (base case + recursive case)
- Verify column list matches query column count
- Detect circular references in non-recursive CTEs

**LSP features** (not implemented):
- Go-to-definition for CTE references
- Autocomplete CTE names in FROM clauses
- Highlight CTE definitions and usages
- Diagnostics for undefined CTE references

**Execution support** (not implemented):
- CTE materialization in backends (inline vs materialized)
- Recursive CTE execution (iteration until fixpoint)
- Optimization opportunities (CTE inlining, hoisting)

---

## ✅ Testing Infrastructure (Phases 1-3, COMPLETED)

**Completed**: December 29, 2024

### What Was Implemented

Comprehensive testing infrastructure to ensure parser correctness and robustness:

#### Phase 1: SQL Printer (~570 lines)
**File**: `crates/smelt-parser/src/printer.rs`

- Implemented Display trait for 20+ AST node types
- Enables round-trip testing: parse → print → parse
- Two format modes: Compact (single-line) and Pretty (multi-line)
- Formatting rules:
  - Keywords: UPPERCASE (SELECT, WHERE, etc.)
  - Identifiers: preserve case
  - Proper expression precedence and parenthesization
  - Line breaks at major clauses

**Tests**: 10 printer tests verifying round-trip preservation

#### Phase 2: Property-Based Testing (~470 lines)
**Files**: `tests/proptest_generators.rs`, `tests/proptest_round_trip.rs`

- Grammar-based SQL generators using proptest
- 30+ generators for all SQL constructs:
  - Simple SELECT, WHERE, JOIN, GROUP BY, ORDER BY, LIMIT
  - DISTINCT, CTEs, window functions
  - Expression combinations (CASE, CAST, BETWEEN, IN, etc.)
- 20 property tests verifying:
  - Round-trip preservation (parse → print → parse)
  - Parser never panics on any input
  - Position tracking correctness
  - Error recovery produces usable CSTs
- **2810+ test cases** run automatically (100 per property by default)

**Dependency**: Added proptest 1.4 to dev-dependencies

#### Phase 3: Fuzzing with cargo-fuzz
**Files**: `fuzz/fuzz_targets/*.rs`, `fuzz/Cargo.toml`

- Two fuzz targets:
  - `parse_never_panics`: Verifies parser never panics (110,993 executions, zero crashes)
  - `round_trip`: Verifies round-trip preservation (discovered edge case)
- Corpus seeded with 9 SQL test cases from parser test suite
- Coverage-guided mutation testing with libFuzzer
- Found edge case: Printer normalizes keyword case (`WHERe` → `WHERE`), affecting error-recovery behavior in invalid SQL

**Known Issue**: Round-trip test found that printer changes mixed-case keywords to uppercase, which can affect parse errors in malformed SQL. This is acceptable since the printer is designed for valid SQL only.

### Test Coverage

**Total Tests**:
- 78 unit tests (inline in `src/parser.rs`)
- 10 printer tests (inline in `src/printer.rs`)
- 2810+ property-based tests (in `tests/`)
- 2 fuzz targets (in `fuzz/`)

**Coverage**: >90% of parser.rs code paths

**All SQL Features Tested**:
- ✅ Keywords, identifiers, literals, operators
- ✅ SELECT, FROM, WHERE, GROUP BY, HAVING, ORDER BY, LIMIT
- ✅ JOIN types (INNER, LEFT, RIGHT, FULL, CROSS) with ON/USING
- ✅ Expressions (binary, unary, CASE, CAST, subqueries, BETWEEN, IN, EXISTS)
- ✅ Window functions (OVER, PARTITION BY, frames)
- ✅ CTEs (WITH, RECURSIVE, UNION)
- ✅ smelt extensions (smelt.ref, smelt.metric, => operator)
- ✅ Error recovery (partial CSTs with errors)

### Documentation

- `fuzz/README.md` - Fuzzing guide with examples
- `tests/README.md` - Testing strategy and philosophy
- Updated `crates/smelt-parser/Cargo.toml` - Added proptest dependency
- Updated `/Cargo.toml` - Excluded fuzz directory from workspace

### Key Design Decisions

**SQL Printer as Foundation**:
- Printer enables round-trip testing without external dependencies
- Opinionated formatting (uppercase keywords) simplifies implementation
- Display trait provides ergonomic API for AST → SQL conversion

**Grammar-Based Property Testing**:
- Generate valid SQL by construction (avoids bias toward simple cases)
- Compositional generators combine small pieces into complex queries
- Default 100 cases for fast PR checks, 1000 for thorough CI validation

**Fuzzing Finds Real Bugs**:
- Coverage-guided fuzzing discovered edge case with keyword case normalization
- Minimization reduced 57-byte failing input to 31 bytes
- Demonstrates value of continuous fuzzing for quality improvement

**Testing Philosophy**:
1. Fast feedback loop (unit tests inline, property tests default to 100 cases)
2. Grammar-based generation (realistic test cases)
3. Error recovery testing (parser never panics)
4. Round-trip preservation (valid SQL survives parse → print → parse)

#### Phase 4: CI Integration (~200 lines)
**Files**: `.github/workflows/test.yml`, `.github/workflows/fuzz.yml`, `.github/CI.md`

**Completed**: December 29, 2024

- **Main test workflow** (`test.yml`):
  - Runs on push to main/feature branches and PRs
  - Formatting check with `cargo fmt`
  - Linting with `cargo clippy -D warnings`
  - Build all targets
  - Unit tests (`cargo test --lib`)
  - Property-based tests (quick mode: 100 cases)
  - Fuzz build verification
  - Aggressive caching for faster CI (~3-5 minutes total)

- **Fuzzing workflow** (`fuzz.yml`):
  - Pull requests: Quick 60s per target
  - Nightly: Thorough 600s per target at 2 AM UTC
  - Manual dispatch with custom duration
  - Matrix strategy (parallel execution)
  - Automatic crash artifact upload
  - Fails CI if crashes found

- **Documentation** (`.github/CI.md`):
  - Workflow descriptions with triggers and duration
  - Local reproduction commands
  - Debugging guides for CI failures
  - Best practices for contributors

**Caching Strategy**:
- Cargo registry, git index, build artifacts
- Invalidated when Cargo.lock changes
- Reduces CI time from ~10min to ~3min

### Future Enhancements (Not Implemented)

**Coverage reporting**:
- Integrate codecov or similar
- Track coverage trends over time
- Fail CI if coverage drops below threshold

**Performance benchmarks**:
- Criterion benchmarks for parser
- Detect performance regressions in CI
- Track parser speed over time

---

## ✅ PostgreSQL-Specific Features (Phases 14-15, COMPLETED)

**Completed**: December 29, 2024

Implemented PostgreSQL-specific SQL syntax extensions to expand parser capabilities:

### Phase 14: PostgreSQL Extended Syntax (~150 lines)

**Features implemented**:

1. **DISTINCT ON (expr, expr)** - Select first row per group
   ```sql
   SELECT DISTINCT ON (user_id) * FROM events ORDER BY user_id, created_at DESC
   ```
   - Alternative to window functions for deduplication
   - Common in PostgreSQL for "top N per group" queries
   - Requires ORDER BY to be deterministic

2. **LATERAL joins** - Correlated subqueries in FROM clause
   ```sql
   FROM users u, LATERAL (SELECT * FROM orders WHERE user_id = u.id) o
   ```
   - Enables subquery to reference columns from preceding table references
   - Powerful for complex join logic
   - Parsed as modifier on table references

3. **TABLESAMPLE** - Table sampling syntax
   ```sql
   FROM events TABLESAMPLE BERNOULLI (10) REPEATABLE (seed)
   ```
   - Sampling methods: BERNOULLI (row-level) or SYSTEM (block-level)
   - Optional REPEATABLE for deterministic sampling
   - Useful for testing on large datasets

**Implementation**:
- 5 new keywords: LATERAL, TABLESAMPLE, BERNOULLI, SYSTEM, REPEATABLE
- 2 new composite nodes: DISTINCT_ON_CLAUSE, TABLESAMPLE_CLAUSE
- Updated SELECT parsing for DISTINCT ON
- Updated JOIN parsing for LATERAL keyword
- Updated table reference parsing for TABLESAMPLE
- 7 new parser tests

### Phase 15: Aggregate Function Enhancements (~80 lines)

**Features implemented**:

1. **FILTER (WHERE condition)** - Conditional aggregation
   ```sql
   SELECT
     COUNT(*) as total,
     COUNT(*) FILTER (WHERE status = 'completed') as completed
   FROM orders
   ```
   - Alternative to CASE-based conditional aggregation
   - Cleaner syntax than `SUM(CASE WHEN ... THEN 1 ELSE 0 END)`
   - Applies to any aggregate function

**Implementation**:
- 1 new keyword: FILTER
- 1 new composite node: FILTER_CLAUSE
- Updated function call parsing to detect FILTER after arguments
- Critical fix: Allow keywords as named parameter names (e.g., `filter => value`)
- 3 new parser tests

**Key Design Decision**:
- Modified `parse_argument()` to allow keywords as parameter identifiers
- Prevents `FILTER` keyword from breaking `filter => value` in smelt.ref() calls
- Uses lookahead for ARROW (`=>`) to distinguish keyword usage from named parameters

**Test Coverage**:
- 10 new unit tests for PostgreSQL features
- All 88 existing tests still passing
- Property tests updated to include new syntax

---

### Column Schema Tracking (Future)

**Goal**: Enable smarter LSP features through column-level validation

**Value**:
- Autocomplete for column names in SELECT, WHERE, GROUP BY
- Real-time validation of column references
- Type inference from SELECT expressions
- Better error messages for typos and invalid references

**Work**:
- Track column schemas in smelt-db queries
- Infer output columns from SELECT clause
- Validate column references against available schemas
- LSP autocomplete integration for column names
- Handle star (*) expansion and aliases

**Effort**: Medium-High (3-5 days)

**Dependencies**: Requires schema metadata from database or model definitions

---

## Deferred Indefinitely

These features require significant architectural work and are not prioritized:

### Metrics DSL (Spec lines 132-153)
- YAML/declarative metric definitions
- Metric registry and resolution
- Temporal semantics (trailing windows, decomposability)
- Parameter validation

### Type System
🔄 **IN PROGRESS** - See Phase 18 below for current implementation status

### Configuration Annotations (Spec lines 437-464)
- Parse `@materialize`, `@partition_by` annotations
- Store config metadata in AST/database
- Validate configuration options

### Rewrite Rules Framework (Spec lines 284-346)
- Rule framework (Egg or similar)
- Engine-specific translations
- Cost-based optimization

### Learning/Optimization (Spec Phase 6)
- Historical run data
- Optimization suggestions
- Cost modeling

---

## Parser & LSP Status

### ✅ Implemented (Phases 1-13, December 2024)

**Core Features:**
- `smelt.ref()` parsing and validation
- Named parameters (`filter => expr`, `limit => 100`)
- LSP diagnostics for undefined refs
- Go-to-definition for model references
- Incremental compilation via Salsa
- Error recovery in parser

**SQL Syntax (Phases 8, 10-15):**
- All JOIN types (INNER, LEFT, RIGHT, FULL, CROSS)
- ON and USING conditions
- CASE expressions (both searched and simple forms)
- CAST expressions (standard and PostgreSQL `::` syntax)
- Subqueries (in SELECT and FROM clauses)
- BETWEEN, IN, EXISTS expressions
- Unary operators (-, NOT)
- ORDER BY clause with ASC/DESC and NULLS FIRST/LAST
- LIMIT and OFFSET clauses
- HAVING clause for post-aggregation filtering
- DISTINCT and ALL keywords
- SELECT without FROM (constant expressions)
- Window functions (OVER clause with PARTITION BY, ORDER BY, frame specs)
- Common Table Expressions (WITH clause, RECURSIVE)
- UNION and UNION ALL set operations
- **PostgreSQL-specific features (Phases 14-15)**:
  - DISTINCT ON (expr, ...) - Select first row per group
  - LATERAL joins - Correlated subqueries in FROM clause
  - TABLESAMPLE - Table sampling with BERNOULLI/SYSTEM and REPEATABLE
  - FILTER (WHERE condition) - Conditional aggregation

## ✅ Row Polymorphism (COMPLETED)

**Completed**: March 7, 2026

### What Was Implemented

- **Row extensions replace wildcard columns**: `SELECT *` no longer creates placeholder `Column { name: "*" }` entries. Instead, models produce `RowExtension` entries that reference the upstream model, enabling proper expansion.

- **`resolved_model_schema()` Salsa query**: Recursively resolves row extensions by expanding upstream model schemas. Supports chains (A -> B -> C where each uses `SELECT *`). Salsa memoization prevents redundant computation.

- **Type propagation through wildcards**: Types from upstream models now flow through `SELECT *` chains. `resolved_model_schema()` returns fully typed columns even when intermediate models use wildcards.

- **Input constraint extraction**: New `model_input_constraints()` Salsa query analyzes SQL to determine what columns each model requires from its refs. Handles qualified (`t.col`) and unqualified column references, function arguments, WHERE clauses, and JOIN ON conditions.

- **Parser fix for `SELECT *, col`**: Fixed parser to handle `SELECT *, additional_columns` syntax (previously only parsed `SELECT *` alone).

- **LSP hover improvements**: Hover on `smelt.ref()` now shows resolved schemas with expanded wildcards, unresolved row extensions, and input constraints.

### Files Modified

- `crates/smelt-parser/src/parser.rs` - Fixed `SELECT *, col` parsing
- `crates/smelt-parser/src/ast.rs` - Added `SelectItem::is_wildcard()`, `Expr::text_range()`
- `crates/smelt-db/src/schema.rs` - Added `RowExtension`, `InputConstraint`, `ColumnConstraint`, `ResolvedSchema` types; extended `ModelSchema`
- `crates/smelt-db/src/lib.rs` - New `resolved_model_schema()`, `model_input_constraints()` queries; updated `model_schema()`, `process_table_ref()`, `typed_model_schema()`, `available_columns()`
- `crates/smelt-db/src/type_inference.rs` - Added `TypeContext::resolve_alias()`
- `crates/smelt-lsp/src/main.rs` - Updated hover to use resolved schemas and show constraints

### ⏸️ Deferred

- `smelt.metric()` support (awaiting metrics design)
- Configuration annotations (`@materialize`, etc.)
- Additional SQL syntax (INTERSECT/EXCEPT, INSERT/UPDATE/DELETE, CREATE TABLE/VIEW)

---

## ✅ Python Model Support - Phase 3: LSP + Performance (COMPLETED)

**Completed**: March 9, 2026

### What Was Implemented

- **Content-hash caching**: Python model subprocess results are cached by SHA-256 hash of `.py` file content. Cache persists to `.smelt/python_cache.json` so it survives LSP restarts. Unchanged files skip subprocess execution entirely.

- **`.py` file watching**: LSP dynamically registers file watchers for `**/models/**/*.py` via `workspace/didChangeWatchedFiles`. Works with any LSP client, not just VSCode. VSCode extension also updated to watch `.py` files.

- **Python error diagnostics**: Subprocess failures (runtime errors, invalid JSON, missing interpreter) are surfaced as LSP diagnostics on the `.py` file. Line numbers extracted from Python tracebacks when possible. Uses separate `"smelt-python"` diagnostic source.

- **Background execution with last-known-good fallback**: Python model re-execution on file change runs in a background `tokio::spawn` task using `spawn_blocking` for the subprocess. On failure, keeps previous SQL in Salsa (last-known-good) and publishes error diagnostics. On success, updates Salsa and refreshes all diagnostics.

- **Single-file re-execution**: New `execute_single_python_file()` avoids re-scanning all Python files when only one changes.

### Files Modified

- `crates/smelt-lsp/Cargo.toml` - Added `sha2` dependency
- `crates/smelt-lsp/src/python_scan.rs` - Added `PythonModelCache`, `PythonScanResult`, `PythonModelError`, content hashing, `execute_single_python_file()`, traceback line extraction
- `crates/smelt-lsp/src/main.rs` - Added `python_cache`, `python_diagnostics`, `project_roots` fields to `Backend`; `did_change_watched_files` handler; `handle_python_file_change()` with background execution; dynamic watcher registration in `initialized()`; Python diagnostic publishing
- `editors/vscode/src/extension.ts` - Added `.py` file watcher alongside `.sql` watcher

### ⏸️ Deferred

- **Python import dependency tracking**: Cache only hashes the model file itself, not its imports. If a Python model imports from a sibling module, changes to that module won't invalidate the cache. Acceptable for now.
- **Phase 2 (PyO3 AST bindings)**: Still deferred — Phase 1 SQL strings remain sufficient.

---

## Release & Distribution

smelt uses a **Python-wrapping-Rust** distribution model (like ruff, uv, polars): Rust binaries are compiled and bundled into Python wheels via [maturin](https://github.com/PyO3/maturin). Users install with `pip install smelt-sql` and get native binaries without needing a Rust toolchain. Standalone binaries are also published for non-Python users.

### ✅ Phase R1: Maturin Build Setup (March 15, 2026)

Set up the local build pipeline for producing Python wheels that bundle Rust binaries.

- Root `pyproject.toml` with `[build-system] requires = ["maturin>=1.7,<2"]` build backend
- Package name `smelt-sql` with `bindings = "bin"` to bundle `smelt` CLI and `smelt-lsp` binaries
- Python helper module `smelt_sql/` with `lsp_binary_path()` and `cli_binary_path()` for editor extensions
- Renamed `python/pyproject.toml` package to `smelt-runner` to distinguish from the distribution package
- `scripts/prepare-release.sh` helper that prints the release checklist

### ✅ Phase R2: Cross-Platform CI Builds (March 15, 2026)

GitHub Actions workflow to build wheels and standalone binaries for all major platforms.

- `.github/workflows/release.yml` triggered on `v*` tags and `workflow_dispatch`
- Build matrix:
  - Linux x86_64 (`ubuntu-latest`) and aarch64 (`ubuntu-24.04-arm`)
  - macOS x86_64 (`macos-13`) and aarch64 (`macos-latest`)
  - Windows x86_64 (`windows-latest`)
- `PyO3/maturin-action@v1` for Python wheels, `cargo build --release` for standalone binaries
- Standalone archives: `.tar.gz` (Unix) and `.zip` (Windows) with LICENSE and README

### ✅ Phase R3: GitHub Releases (March 15, 2026)

Automate GitHub Release creation with attached artifacts.

- `softprops/action-gh-release@v2` creates a release from the `v*` tag
- Attach standalone binaries, Python wheels, and `SHA256SUMS.txt` checksums
- Version sync check across `Cargo.toml`, `pyproject.toml`, and `editors/vscode/package.json`
- Release notes auto-generated from git log (previous tag to HEAD)
- Pre-release detection for tags containing `-rc`, `-beta`, or `-alpha`

### ✅ Phase R4: PyPI Publishing (March 15, 2026)

Publish Python wheels to PyPI so users can `pip install smelt-sql`.

- `pypa/gh-action-pypi-publish@release/v1` with OIDC trusted publishing (no API tokens)
- `id-token: write` permission added to release workflow
- Stable releases (`v*` without pre-release suffix) publish to PyPI with `environment: pypi`
- Pre-release tags (`-rc`, `-beta`, `-alpha`) publish to TestPyPI with `environment: testpypi`
- One-time OIDC setup documented in `scripts/prepare-release.sh`

### ✅ Phase R5: VSCode Extension Publishing (March 15, 2026)

Publish the VSCode extension to the Marketplace with runtime LSP discovery.

- Refactored `extension.ts` with `findLspCommand()` discovery chain:
  1. User config (`smelt.serverPath` setting)
  2. Python environment (`pip install smelt-sql` → `smelt_sql.lsp_binary_path()`)
  3. `$PATH` lookup (`which smelt-lsp` / `where.exe smelt-lsp`)
  4. Cargo fallback (development mode, only if Cargo.toml found)
- Discovery method logged to output channel for debugging
- `vscode-publish` CI job publishes to VS Code Marketplace via `vsce publish`
- Open VSX publishing via `ovsx` (continue-on-error for optional registry)
- `ovsx` added as devDependency in `editors/vscode/package.json`

### ✅ Phase R6: Documentation Site (March 15, 2026)

Public-facing documentation site built with MkDocs Material.

- `docs-site/mkdocs.yml` with Material theme, search, code copy, dark/light mode toggle
- Initial pages: home, installation, quickstart, SQL models guide, editor setup, language reference
- Content reorganized from `README.md` and `docs/` into user-facing structure
- `.github/workflows/docs.yml` deploys to GitHub Pages on push to `main` (paths: `docs-site/**`, `README.md`, `docs/**`)
- Uses `actions/deploy-pages@v4` with `actions/upload-pages-artifact@v3`

### ✅ Phase R7: Crate Publishing (March 15, 2026)

Publish reusable Rust crates to crates.io for Rust ecosystem consumers.

- Publishable crates (`smelt-parser`, `smelt-types`, `smelt-dialect`) have `description` and `repository` metadata
- `smelt-dialect` path dependency on `smelt-parser` includes `version = "0.1.0"` for crates.io compatibility
- 12 internal crates marked `publish = false`: smelt-backend, smelt-backend-duckdb, smelt-backend-spark, smelt-bench, smelt-cli, smelt-core, smelt-datagen, smelt-db, smelt-lsp, smelt-optimizer, smelt-parser-compat, smelt-ui
- `crates-publish` CI job publishes in dependency order (smelt-types → smelt-parser → smelt-dialect) with index waits
- Uses `CARGO_REGISTRY_TOKEN` secret, stable releases only

### ✅ Phase R8: Continuous Dev Releases (March 15, 2026)

Automated dev releases from every merge to `main`, so users can install the latest changes without waiting for a tagged release.

- `.github/workflows/dev-release.yml` triggered on push to `main`
- Version computed as `X.Y.Z-dev.YYYYMMDDHHMM` from `Cargo.toml` base version + timestamp
- Maturin converts Cargo semver to PEP 440: `0.1.0-dev.202603151430` → `0.1.0.dev202603151430`
- Git tags use `dev-YYYYMMDD-SHORTSHA` format (avoids triggering `release.yml`)
- GitHub Releases created as pre-release with `make_latest: false`
- Dev wheels published to real PyPI (`.devN` versions require `pip install --pre`)
- `pyproject.toml` switched to `dynamic = ["version"]` — maturin reads version from `Cargo.toml`
- Stable releases now only bump 2 files: `Cargo.toml` and `editors/vscode/package.json`
- VSCode extension and crates.io skipped for dev releases (pre-release not well supported)
- One-time OIDC setup needed: add `dev-release.yml` as trusted publisher on pypi.org

### Dependency Diagram

```
R6 (Docs)  ─── ✅

R1 (Maturin) → R2 (CI Builds) → R3 (GitHub Releases)  ← all ✅
                                       │
                                       ├──→ R4 (PyPI)         ✅
                                       └──→ R5 (VSCode Ext)   ✅

R7 (Crates.io) ─── ✅

R8 (Dev Releases) ─── ✅  (pushes to main → PyPI dev builds)
```

---

## Contributing

When working on the next phase:

1. **Before starting**: Review the spec in [DESIGN.md](DESIGN.md) for requirements
2. **During development**: Update this roadmap with progress
3. **After completion**: Mark phase as complete with date
4. **Add tests**: Ensure new features have test coverage
5. **Update docs**: Keep CLAUDE.md and comments up to date

See [CLAUDE.md](../CLAUDE.md) for development workflow and architecture notes.
