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
Codes are grouped by owning feature: models & core analysis, sources, seeds, timeseries, incremental & cumulative, types, Python models, functions & expansion, meta-language, records/maps/loaders, and multi-model production.

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
