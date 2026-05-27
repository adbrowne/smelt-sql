# Plan: Recursive type-annotation validation (nested unknown type names)

**Date**: 2026-05-28
**Spec**: [`docs/specs/functions.md`](../specs/functions.md)
**Spec diff**: §Semantics rule 8 extended (validation recurses into composite type positions); diagnostic-table `InvalidFunctionTypeRef` description extended; Known Divergence added recording the current gap.
**Tracking PR / branch**: PR #124 — branch `worktree-unknown_types`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/functions.md` §Semantics 8 and §"Diagnostic codes" (`InvalidFunctionTypeRef`) — the correctness oracle. Also skim `docs/specs/function_schema_inference.md` §Known Divergences (the `ColumnTypeUnresolved` reservation that this plan's declaration-side approach replaces).
2. Confirm you are on branch `worktree-unknown_types`. If not, ask before continuing.
3. Find the next `pending` phase in the Progress table. If all are `done`, run Verification and stop.

**Per-phase loop (`/smelt:implement`):** implementer subagent → reviewer subagent → iterate → record + commit + push to PR #124.

**When to pause and ask the user:**
- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule (update the spec via `/smelt:spec` first).
- A pre-existing failure unrelated to this plan surfaces.

**Conventions every phase:**
- Red-green TDD; the diagnostic test drives the *real* diagnostic query (`file_diagnostics` / `check_type_diagnostics`), not a pure sub-helper, so the wiring is exercised.
- Real-fixture coverage: a broken-function example under `examples/` validated through `example_diagnostics` (Salsa-direct) **and** `example_workspaces` (real LSP Backend).
- Atomic per-phase commit using the `Commit.` line verbatim; push after each.
- Honor `CLAUDE.md` invariants: `smelt-db` pure-function rule; the Workspace Loading Parity Rule.
- **Timeless-oracle rule.** Phase vocabulary lives in this plan only. Spec / `docs-site` / code-comment edits describe the feature as if it has always existed — no `Phase N` headings/labels/callouts. As the gap closes, delete its §Known Divergences entry from `functions.md` in the same commit.

---

## Context

`InvalidFunctionTypeRef` (`functions.md` §Semantics 8) fires only when a whole `smelt.define` / `smelt.extern` type annotation fails to parse. An unrecognized type name nested inside a composite type — most commonly a struct field type in a `-> Expr<Struct<{…}>>` return — parses to a structurally-valid annotation with that field absorbed as `Unknown`, so the declaration is not flagged. Such a function then contributes a present, `Unknown`-typed column to every caller that projects it via `.*`, with no diagnostic anywhere. This is the one remaining function-side source of a silent `Unknown` after the function-schema inference fixes (`function_schema_inference.md`). Reporting it at the declaration (the origin) is the spec-faithful fix: the caller's column becomes `Propagated` and correctly stays silent, and no call-site `ColumnTypeUnresolved` is required.

## Scope

### In scope (spec coverage)
- `functions.md` §Semantics 8 — type-annotation validation recurses into composite positions (struct field types, array element types, map key/value types); an unrecognized nested type name fires `InvalidFunctionTypeRef` at the annotation span.

### Explicitly deferred
- Call-site `ColumnTypeUnresolved` (`function_schema_inference.md`) — reserved for a genuinely caller-specific unresolvable case (a well-formed signature a particular call still cannot type); none exists today. This plan deliberately handles the malformed-signature case at the declaration instead.
- Single-field `.field` projection and row-tail (`..r`) struct spread — separately deferred in `function_schema_inference.md`.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | pending  |        |      |

---

### Phase 1: Recurse type validation into composite positions → `InvalidFunctionTypeRef`

**Goal.** A `smelt.define` / `smelt.extern` signature whose annotation contains an unrecognized type name nested in a composite type (struct field, array element, map key/value) emits `InvalidFunctionTypeRef` at the offending annotation, instead of silently absorbing the nested name as `Unknown`.

**Pre-conditions.** None beyond the current `function_diagnostics` validation pass.

**TDD tests to write first.**
- `crates/smelt-db/...` unit test driving the real diagnostic query — a `smelt.define` declaring `-> Expr<Struct<{a: Integer, b: Bogus}>>` emits exactly one `InvalidFunctionTypeRef` at the return-type span; a fully-valid struct return emits none. Cover one nested array/map case if cheap.
- `crates/smelt-db/...` unit test — a closed, valid struct return (e.g. `Expr<Struct<{a: Integer, b: Text}>>`) and a row-tail (`..r`) return do **not** emit `InvalidFunctionTypeRef` (the tail marker is not an unknown type name).
- New broken-function example `examples/functions_broken_struct_field_type/` with a `smelt.define` declaring an unknown struct field type; `crates/smelt-cli/tests/example_diagnostics.rs` (broken-workspace path, one-code-per-fixture pattern) asserts it emits exactly `InvalidFunctionTypeRef`.
- Regression: `example_diagnostics` (currently 75) and `example_workspaces` (21) stay green — no existing example newly flags.

**Implementation shape.** In the signature type-validation that produces `InvalidFunctionTypeRef` (`crates/smelt-db/src/queries/function_diagnostics.rs`, the pass over `sig.params` / `sig.return_type` that today only checks `Some(Err(_))`), additionally walk *resolved* composite types (`SmeltType::Struct` fields, `Array` element, `Map` key/value, and `Expr<…>` payloads thereof) for any field whose resolved `DataType`/`SmeltType` is `Unknown` arising from an unrecognized type-name token (as opposed to a deferred sort or a legitimate type variable). Emit `InvalidFunctionTypeRef` at the annotation range for each. Reuse the existing `is_deferred_phase13_sort` exclusion so `TableExpr`/`AggExpr`/`WindowExpr`/`SelectItems` heads are not falsely flagged, and do not flag row-tail markers (`..` / `..r`). Keep the analysis pure (no Salsa inside the check); the Salsa query stays a thin wrapper. Determine the precise distinction between "unrecognized type name → Unknown" and "legitimately Unknown" during investigation; if the parser does not retain enough information to tell them apart at the field level, extend the type-ref parse to carry that signal rather than guessing — report if this widens beyond `smelt-db`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/queries/function_diagnostics.rs` — the validation pass.
- `crates/smelt-types/src/signatures.rs` — only if the resolved struct/array/map field needs an "unrecognized-name" signal surfaced (prefer reading existing data).
- `crates/smelt-parser/src/parser/types.rs` — only if the parse must retain the unknown-name signal; report before relying on this.
- `crates/smelt-db/src/tests.rs` and/or `crates/smelt-db/tests/*.rs` — unit tests.
- `crates/smelt-cli/tests/example_diagnostics.rs` — broken-fixture test.
- `examples/functions_broken_struct_field_type/` — the fixture.

**Docs touched (timeless phrasing — no plan/phase vocabulary in body).**
- `docs/specs/functions.md` — remove the §Known Divergence beginning "**Nested unrecognized type names are absorbed as `Unknown` rather than flagged.**".
- `docs-site/docs/reference/language.md` — note that an unrecognized type name in a function's struct/array/map type annotation is an `InvalidFunctionTypeRef` error at the declaration.

**Review checklist (material findings only):**
- [ ] TDD tests exist, drive the real diagnostic query, and assert the struct-field case fires + valid/row-tail cases do not.
- [ ] Spec `functions.md` §Semantics 8 satisfied; diagnostic fires at the declaration (origin), not the call site.
- [ ] `smelt-db` pure-function rule preserved; if parser/signatures had to change, the change is reported and justified.
- [ ] `example_diagnostics` + `example_workspaces` stay green; the new broken fixture emits exactly `InvalidFunctionTypeRef`.
- [ ] No scope creep into `ColumnTypeUnresolved` or call-site emission.
- [ ] Spec + docs-site edits are timeless (no `Phase X`).

**Commit.** `feat(functions): flag unrecognized nested type names in signatures (InvalidFunctionTypeRef)`

---

## Deferred during implementation

(Append-only.)

## Verification

- A `smelt.define -> Expr<Struct<{a: Integer, b: Bogus}>>` fixture emits exactly `InvalidFunctionTypeRef` at the declaration.
- `cargo test -p smelt-cli --test example_diagnostics` — green (incl. the new broken fixture).
- `cargo test -p smelt-lsp --test example_workspaces` — green.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` — green.
- `/smelt:validate functions` — no drift on the §Semantics 8 / `InvalidFunctionTypeRef` entries.
