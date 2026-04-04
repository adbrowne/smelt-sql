# Type Semantics

This document describes smelt's type inference semantics and where they intentionally differ from backend databases (DuckDB, Spark, etc.). These are design decisions, not bugs.

## Design Principle

**Prefer strictness over permissiveness.** smelt catches errors at compile time, not runtime. When there's ambiguity, smelt rejects the construct with a clear diagnostic rather than silently coercing.

## DataType System

smelt's type system includes:

| Category | Types |
|----------|-------|
| Numeric | `Boolean`, `SmallInt`, `Integer`, `BigInt`, `Float`, `Double`, `Decimal(precision, scale)` |
| String | `Varchar(max_length?)`, `Char(length)`, `Text` |
| Binary | `Blob` |
| Temporal | `Date`, `Time`, `Timestamp(with_timezone)`, `Interval` |
| Complex | `Array(element_type)`, `Struct(fields)` |
| Special | `Null`, `Unknown` |

Type family membership (`is_numeric()`, `is_string()`, `is_temporal()`) gates promotion rules. Cross-family promotion is never attempted — e.g., `Integer + Varchar` yields `Unknown`, not an implicit cast.

---

## Integer Division is Truncating

smelt preserves integer types through division:

| Expression | smelt | DuckDB |
|-----------|-------|--------|
| `SmallInt / SmallInt` | SmallInt | Double |
| `Integer / Integer` | Integer | Double |
| `BigInt / BigInt` | BigInt | Double |
| `Decimal / Decimal` | Decimal(38,10) | Double |
| `Float / Float` | Float | Double |

**Rationale:** DuckDB v1.5+ switched to non-truncating division where all division returns Double. smelt intentionally uses truncating integer division (`5 / 2 = 2`) and preserves the numeric type family. This matches traditional SQL semantics and avoids surprising type changes when dividing integers.

Backend compilation can insert explicit casts if the target engine uses different division semantics.

## SUM of Integers Returns BigInt

| Expression | smelt | DuckDB |
|-----------|-------|--------|
| `SUM(SmallInt)` | BigInt | Decimal(38,0) |
| `SUM(Integer)` | BigInt | Decimal(38,0) |
| `SUM(BigInt)` | BigInt | Decimal(38,0) |
| `SUM(Float)` | Double | Double |
| `SUM(Double)` | Double | Double |
| `SUM(Decimal)` | Decimal (same precision/scale) | Decimal |

**Rationale:** DuckDB uses HUGEINT internally for integer sums, which maps to Decimal(38,0) through Arrow. smelt uses BigInt as the widened integer type for sums since it stays within the integer family. Spark agrees with smelt here (SUM(Integer) -> BigInt).

SUM always returns nullable (empty groups produce NULL).

## String Functions Return Text

All string manipulation functions (`UPPER`, `LOWER`, `TRIM`, `SUBSTRING`, `REPLACE`, `CONCAT`, etc.) return `Text`, not `Varchar`.

| Expression | smelt | DuckDB | Spark |
|-----------|-------|--------|-------|
| `UPPER('hello')` | Text | Varchar | String |
| `'a' \|\| 'b'` | Text | Varchar | String |
| `SUBSTRING(col, 1, 3)` | Text | Varchar | String |

**Rationale:** smelt unifies all string types internally to `Text`, avoiding length-tracking complexity. The `to_backend_sql()` conversion emits `VARCHAR` for backends that don't have a `TEXT` type.

String-returning functions: `UPPER`, `LOWER`, `TRIM`, `LTRIM`, `RTRIM`, `SUBSTRING`, `SUBSTR`, `REPLACE`, `TRANSLATE`, `REVERSE`, `REPEAT`, `LPAD`, `RPAD`, `INITCAP`, `QUOTE_IDENT`, `QUOTE_LITERAL`, `LEFT`, `RIGHT`, `SPLIT_PART`, `STRING_AGG`, `LISTAGG`, `CONCAT`, `CONCAT_WS`.

Integer-returning string functions: `LENGTH`, `CHAR_LENGTH`, `POSITION`, `STRPOS` (return BigInt).

## CEIL/FLOOR of Double Returns Double

| Expression | smelt | DuckDB | Spark |
|-----------|-------|--------|-------|
| `CEIL(double_col)` | Double | Double | BigInt |
| `FLOOR(double_col)` | Double | Double | BigInt |
| `CEIL(decimal_col)` | Decimal(p, 0) | Decimal | Decimal |

**Rationale:** CEIL/FLOOR of a Double is mathematically an integer but may exceed integer range. Returning Double preserves the value space. Decimal inputs preserve precision with scale set to 0. Spark converts to BigInt, which can overflow for large doubles.

Nullability is inherited from the input expression.

## SIGN Returns SmallInt

| Expression | smelt | DuckDB | Spark |
|-----------|-------|--------|-------|
| `SIGN(integer_col)` | SmallInt | Integer | Integer |
| `SIGN(double_col)` | SmallInt | Double | Double |
| `SIGN(decimal_col)` | SmallInt | Decimal | Decimal |

**Rationale:** SIGN always returns -1, 0, or 1. SmallInt is the smallest type that represents these values. Backends that return the input type are over-widening.

## UNION Type Promotion Rules

When combining columns across `UNION`, `INTERSECT`, or `EXCEPT`, smelt promotes types using these rules:

### Null and Unknown

- `Unknown + T` -> `T` (Unknown is dominated by any concrete type)
- `Null + T` -> `T` (nullable) (Null adds nullability)

### Numeric Hierarchy

Promotion follows: `SmallInt < Integer < BigInt < Decimal < Float < Double`

The wider type wins:

| Left | Right | Result |
|------|-------|--------|
| SmallInt | Integer | Integer |
| Integer | BigInt | BigInt |
| BigInt | Decimal | Decimal |
| Integer | Float | Float |
| Float | Double | Double |
| Decimal(10,2) | Decimal(15,3) | Decimal(15,3) |

For Decimal + Decimal, precision and scale are both widened: `max(p1, p2)` and `max(s1, s2)`.

### String Promotion

- `Varchar + Text` -> `Text`
- `Varchar + Varchar` -> `Text` (length constraints are discarded)
- `Char + Text` -> `Text`

### Temporal Promotion

- `Date + Timestamp` -> `Timestamp`
- `Time + Timestamp` -> `Timestamp`
- `Date + Time` -> `Timestamp(without timezone)`
- `Timestamp(tz1) + Timestamp(tz2)` -> `Timestamp(tz1 || tz2)`

### Cross-Family

Incompatible type families (e.g., Integer + Text, Boolean + Date) produce `Unknown`. smelt does not implicitly cast across type families. DuckDB is more permissive here (e.g., Boolean + Integer -> Integer).

## Temporal Arithmetic Rules

### Addition

| Expression | Result Type |
|-----------|-------------|
| `DATE + INTERVAL` | Timestamp (no timezone) |
| `TIMESTAMP + INTERVAL` | Timestamp (preserves timezone) |
| `TIME + INTERVAL` | Time |
| `INTERVAL + INTERVAL` | Interval |
| `INTERVAL * numeric` | Interval |

Addition is commutative — `INTERVAL + DATE` produces the same type as `DATE + INTERVAL`.

### Subtraction

| Expression | Result Type |
|-----------|-------------|
| `DATE - DATE` | Interval |
| `TIMESTAMP - TIMESTAMP` | Interval |
| `TIME - TIME` | Interval |
| `DATE - INTERVAL` | Timestamp (no timezone) |
| `TIMESTAMP - INTERVAL` | Timestamp (preserves timezone) |
| `TIME - INTERVAL` | Time |
| `INTERVAL - INTERVAL` | Interval |

### Multiplication and Division

| Expression | Result Type |
|-----------|-------------|
| `INTERVAL * numeric` | Interval |
| `numeric * INTERVAL` | Interval |
| `INTERVAL / numeric` | Interval |

All temporal arithmetic results are nullable.

## Array Type Rules

### Array Literals

- `[1, 2, 3]` -> `Array(Integer)`, non-nullable
- `[1.0, 2]` -> `Array(Double)`, via numeric promotion
- `[NULL, 'hello']` -> `Array(Text)`, NULL is compatible with any element type
- `[]` -> `Array(Unknown)`, empty array has unknown element type

**Mixed-type rejection:** `[1, 'hello']` produces a type error. smelt does not coerce across type families within array literals. This is stricter than DuckDB (which would cast to VARCHAR).

### Array Subscript

- `arr[i]` -> element type of `arr`, always nullable (out-of-bounds returns NULL)

### Array Slice

- `arr[start:end]` -> same `Array` type as `arr`, nullable

## Struct Type Rules

### Struct Literals

Named fields:
```sql
STRUCT(1 AS a, 'hello' AS b)  -- Struct([("a", Integer), ("b", Text)])
```

Positional fields (ROW constructor):
```sql
ROW(1, 'hello', TRUE)  -- Struct([("v1", Integer), ("v2", Text), ("v3", Boolean)])
```

Struct literals themselves are non-nullable.

### Field Access

Struct fields are accessed via dot notation:

```sql
SELECT s.a FROM (SELECT STRUCT(1 AS a) AS s) t
```

Field lookup is case-insensitive. When `s.a` is encountered:
1. First, try to resolve as `table.column` (normal qualified reference)
2. If that fails, check if `s` is a struct-typed column and look up field `a`

Field access results are always nullable (conservative — may be refined later).

## Nullability Rules

### Non-nullable expressions

- `COALESCE(a, b)` is non-nullable when at least one argument is non-nullable or a non-null literal
- `CASE WHEN ... THEN ... ELSE ...` is non-nullable when ELSE is present AND all branches are non-nullable
- `CASE WHEN ... THEN ...` (no ELSE) is always nullable (implicit NULL default)
- `CAST(expr AS type)` preserves the input expression's nullability
- `IFNULL(a, b)` / `NVL(a, b)` follows the same rules as COALESCE
- Array and struct literals are non-nullable (the container itself)
- `EXISTS(subquery)` is non-nullable (always TRUE or FALSE)
- `COUNT(*)` and `COUNT(expr)` are non-nullable

### Always nullable

- SUM, AVG, MIN, MAX (empty groups produce NULL)
- Scalar subqueries (may return no rows)
- `IN (subquery)` (three-valued logic with NULL)
- Array subscript (out-of-bounds)
- Struct field access (conservative)

## Complete Divergence Registry

All intentional differences from backends are tracked in `crates/smelt-db/tests/prop_helpers/divergences.rs`:

### By Design (intentional smelt choices)

| Name | smelt | Backend | Reason |
|------|-------|---------|--------|
| `integer_division` | SmallInt | Double (DuckDB) | Truncating division |
| `smallint_division` | SmallInt | Double (DuckDB) | Truncating division |
| `bigint_division` | BigInt | Double (DuckDB) | Truncating division |
| `decimal_division` | Decimal(38,10) | Double (DuckDB) | Preserve decimal type |
| `float_division` | Float | Double (DuckDB) | Preserve float type |
| `string_concat` | Text | Varchar (DuckDB) | Unified string type |
| `string_functions` | Text | Varchar (DuckDB) | Unified string type |
| `cross_family_rejection` | Unknown | Varies (DuckDB) | Strict type families |

### Backend-Specific (different backends disagree)

| Name | smelt | Backend | Notes |
|------|-------|---------|-------|
| `sum_integer` | BigInt | Decimal(38,0) (DuckDB) | DuckDB uses HUGEINT |
| `avg_decimal` | Double | Decimal (Spark) | Spark preserves Decimal |
| `ceil_floor_double` | Double | BigInt (Spark) | Spark truncates to int |
| `sign_double` | SmallInt | Double (Spark) | smelt minimizes width |
| `sign_integer` | SmallInt | Integer (Spark) | smelt minimizes width |
| `sign_bigint` | SmallInt | BigInt (Spark) | smelt minimizes width |
| `sign_decimal` | SmallInt | Decimal (Spark) | smelt minimizes width |
