# Diagnostics reference

smelt emits named diagnostic codes through its compiler, LSP server, and CLI. Every diagnostic carries a severity level (`Error`, `Warning`, `Info`, `Hint`), a human-readable message, and a stable code you can use to look up remediation guidance.

## Severity levels

| Severity | Meaning |
|----------|---------|
| `Error`  | The model or function declaration is invalid; smelt will not compile the project until the error is resolved. |
| `Warning` | Likely incorrect or sub-optimal; the project compiles but may behave unexpectedly. |
| `Info` / `Hint` | Informational; no action required. |

## Code catalogue

The complete `DiagnosticCode` catalogue — every variant, its severity, and the user-input condition that triggers it — is maintained in
[`docs/specs/diagnostics.md`](https://github.com/brownie/smelt/blob/main/docs/specs/diagnostics.md).
Codes are grouped by owning feature: models & core analysis, sources, seeds, timeseries, incremental & cumulative, types, Python models, functions & expansion, meta-language, records/maps/loaders, multi-model production, and property diff (`PropertyDowngrade`, `PropertyDiffBaselineUnavailable` — see [`smelt explain --diff`](smelt-explain.md)).

### Example: `UnknownStructFieldType`

**Severity**: Error

Emitted when a `smelt.define` or `smelt.extern` parameter or return-type annotation contains a `Struct<{…}>` shape whose field type text cannot be resolved to a recognised `DataType`. The diagnostic is anchored at the individual field's type-reference span, not the whole annotation, so the exact bad token is highlighted.

**Example** — the following emits `UnknownStructFieldType` on the `Bogus` span:

```sql
smelt.define my_fn(t: Expr<Struct<{a: Integer, b: Bogus}>>) -> Expr<Integer> AS (
  t.a
)
```

**Fix**: Replace the unrecognised type name with a concrete smelt `DataType` such as `Integer`, `Text`, `Float`, `Boolean`, `Timestamp`, or a nested `Struct<{…}>`.

### Example: `UnsupportedOnBackend`

**Severity**: Error

A built-in's backend support can differ by *where* it's called — as a scalar expression, an
aggregate, a whole-partition window (`OVER (PARTITION BY …)` with no `ORDER BY` or frame), or a
running window (any narrower frame, including the common `OVER (PARTITION BY … ORDER BY …)`).
smelt transparently restructures a whole-partition window over an aggregate-only built-in — or an
aggregate over a window-only built-in — around a synthesised CTE. That restructure has no correct
form for a **running** window, so a running window over a built-in the target backend offers only
as an aggregate is refused at compile time. See [Position-dependent aggregate
support](../guide/targets.md#position-dependent-aggregate-support) for the full picture and the
rewrite to apply by hand.

**Example** — targeting DuckDB, a running `PERCENTILE_CONT` window emits `UnsupportedOnBackend`:

```sql
SELECT
    id,
    g,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g ORDER BY t) AS running_med
FROM tbl
```

**Fix**: Rewrite the query so the affected aggregate is computed once per partition — for example,
by grouping into a CTE keyed on the partition column and joining the constant value back onto each
row of the partition, rather than asking the target backend for a running form it does not have.

The refusal names the construct, the reason it doesn't fit the backend, and the backend itself.
For the query above, targeting DuckDB, the message reads:

<!-- unsupported-on-backend-refusal-text -->
```text
UnsupportedOnBackend: this model uses 2 constructs the DuckDB backend cannot express:
  `PERCENTILE_CONT` — DuckDB has the ordered-set aggregate but no running-window form of it; only a window covering the whole partition can be restructured around a grouped CTE
  `PERCENTILE_CONT` — DuckDB has the ordered-set aggregate but no running-window form of it; only a window covering the whole partition can be restructured around a grouped CTE
```

#### A template's spelling cannot carry a modifier

A built-in whose target spelling is a fixed template over its own positional arguments — DuckDB's
`DATE_SUB(d, i)`, spelled `d - i` — has no place in that spelling for a call modifier: `DISTINCT`,
`FILTER (WHERE …)`, `WITHIN GROUP (ORDER BY …)`, an `ORDER BY` inside the argument list, `IGNORE
NULLS`/`RESPECT NULLS`, a named (`=>`) argument, or a `*` argument. Dropping the modifier would
change the answer — a dropped `DISTINCT` counts duplicates the author excluded — so the call is
refused at compile time rather than silently stripped.

**Example** — targeting DuckDB, `DATE_SUB` carrying `DISTINCT`:

```sql
SELECT DATE_SUB(DISTINCT d, INTERVAL 1 DAY) AS x FROM events
```

<!-- unsupported-on-backend-template-modifier-refusal-text -->
```text
UnsupportedOnBackend: this model uses 1 construct the DuckDB backend cannot express:
  `DATE_SUB` — this built-in's target spelling is a fixed template over positional arguments; DISTINCT cannot be expressed by a template (a dropped DISTINCT would count duplicates the author excluded) and is refused rather than silently dropped
```

**Fix**: Rewrite the query without the modifier — for example, deduplicate the input rows in a CTE
before calling the function — or pick a target backend whose registry entry for that built-in is
not a template.

#### A verdict that depends on operand type

Some built-ins lower differently depending on the type of the values passed to them, not only on
where they're called. `//` is DuckDB's native floor/true division operator, but Spark and BigQuery
have no infix `//`: on Spark, `a // b` lowers to `a DIV b` when both operands are integral and to
plain `a / b` when both are floating-point or decimal. When an operand's type cannot be resolved at
compile time, smelt refuses rather than guess — guessing wrong here would silently compute a
*different number*, not fail loudly.

**Example** — targeting Spark, `//` over a column whose type cannot be resolved:

```sql
SELECT a // b AS x FROM t
```

<!-- unsupported-on-backend-operand-class-refusal-text -->
```text
UnsupportedOnBackend: this model uses 1 construct the Spark SQL backend cannot express:
  `//` — Spark SQL has no infix `//`; use a typed FLOOR(a / b) or DIV(a, b)
```

**Fix**: Give the operands a resolvable type — for example, by declaring the upstream column's type
or wrapping the operand in an explicit `CAST` — or rewrite the expression as a typed `FLOOR(a / b)`
or `DIV(a, b)` call.
