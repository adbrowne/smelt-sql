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
