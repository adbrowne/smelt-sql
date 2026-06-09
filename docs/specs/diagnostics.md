---
feature: diagnostics
status: experimental
last_reviewed: 2026-06-10
owners: [andrew]
---

# Diagnostics

This spec documents the `DiagnosticCode` catalogue for smelt — the set of
named error/warning/hint codes the compiler emits, their semantics, and where
they are anchored in the source.

## Surface

Diagnostics are surfaced through two paths:
- **LSP**: the language server reports them in real time as the user edits.
- **CLI**: `smelt build` / `smelt run` / `smelt type` print them and set a
  non-zero exit code when any Error-severity diagnostic is present.

Every diagnostic carries:
- **Severity** — `Error`, `Warning`, `Info`, or `Hint`.
- **Code** — a `DiagnosticCode` variant (enables code-action lookups and stable
  cross-references).
- **Range** — a `rowan::TextRange` (byte offsets into the source file).

### Fail-loud invariants

The diagnostic system enforces a *fail-loud* discipline:

1. Every path that can encounter an unrecognisable user input **must** emit a
   diagnostic rather than silently falling back to an inferred or unknown value.
2. Specifically, every `DataType::Unknown` site in production code is either
   classified as *legitimate* (a deliberate meta-language placeholder) or
   covered by a diagnostic (the guard test
   `crates/smelt-types/tests/unknown_census.rs` enforces this).

The full catalogue of `DiagnosticCode` variants lives in
`crates/smelt-db/src/diagnostics_types.rs`.

## Codes introduced by the silent-failures-hardening plan

The following codes were introduced as part of the fail-loud hardening work
documented in `docs/plans/20260608-silent-failures-hardening.md`.

### `UnknownStructFieldType`

**Severity**: Error  
**Anchor**: the individual field's `TYPE_REF` span (inside the struct
annotation, not the whole parameter span)

Emitted when a `smelt.define` or `smelt.extern` parameter or return-type
annotation has a `Struct<{…}>` shape whose field type text cannot be parsed
as a recognised concrete `DataType`.

Example:
```sql
-- Error on the `Bogus` span:
smelt.define my_fn(t: Expr<Struct<{a: Integer, b: Bogus}>>) -> Expr<Integer> AS (
  t.a
)
```

The struct value is still constructed (with `DataType::Unknown` as the field
type) for downstream use, but this diagnostic ensures the author is told
exactly which field name is unrecognised rather than receiving a later,
context-free `Unknown`-propagation error.

## Known divergences

- **Full back-catalogue** (BUG-052): the ~70 existing `DiagnosticCode`
  variants predating this spec are not yet documented here. Tracked as
  deferred work in `docs/plans/20260608-silent-failures-hardening.md`.

## Open questions

None currently open.

## References

- **Code**: `crates/smelt-db/src/diagnostics_types.rs` — full `DiagnosticCode` enum
- **Code**: `crates/smelt-types/src/signatures.rs` — `struct_field_unknown_ranges` pure helper
- **Code**: `crates/smelt-db/src/queries/function_diagnostics.rs` — `struct_field_type_unknown_diagnostics_for_file`
- **Tests**: `crates/smelt-db/tests/struct_field_type.rs`
- **Tests**: `crates/smelt-types/tests/unknown_census.rs` — guards every `DataType::Unknown` site
- **Plans (history)**: `docs/plans/20260608-silent-failures-hardening.md`
- **Related specs**: `docs/specs/architecture.md` §"Fail-loud invariants"
