# Plan: Collation Axis — Portable Binary-Only Contract

**Date**: 2026-06-13
**Spec**: [`docs/specs/types.md`](../specs/types.md) §17 Collation (+ §Design collation paragraph, `NonPortableCollation` diagnostic)
**Spec diff**: `53f3329a` (§17 added)
**Tracking PR / branch**: `main` (worktree `type_system`)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/types.md` §17 + the §Design collation paragraph + the `NonPortableCollation` diagnostic entry — that is the correctness oracle. Do not re-open settled spec decisions (binary-only portable surface; value-domain placement; field-deferral).
2. Confirm you are on branch `main` in the `type_system` worktree. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking branch.
- Don't widen scope: a phase may not reach into a later phase's scope, and **no phase adds a collation field to `DataType`** (deferred — see Scope).
- Honor architectural invariants from `CLAUDE.md` (`type_inference.rs` purity; fail-loud discipline; `MetadataError` exhaustiveness).
- **Timeless-oracle rule.** Phase vocabulary lives in this plan file only. Spec/user-doc edits describe the feature as if it has always existed.

---

## Context

§17 makes binary the only portable collation and rejects non-binary collation in portable code. This plan implements that enforcement — the portable surface — and defers the value-domain representation and engine-bound behaviour, exactly as the decimal axis landed arithmetic but deferred engine-bound division (§15). The motivating hazard (Postgres's locale default) is latent until a Postgres runtime backend exists, so its emission-pin fix rides with that backend rather than here.

## Scope

### In scope (spec coverage)
- §17 "Non-binary collation is not in the portable surface" — `COLLATE` expression parsing and the `NonPortableCollation` diagnostic; binary collation names accepted as a no-op.
- §17 "Binary is the only portable collation" — regression coverage that binary string comparison/grouping is stable and divergence-free on the live DuckDB oracle.
- §17 Surface diagnostic catalogue — `NonPortableCollation` reaches `DiagnosticCode` and the diagnostic mapping.

### Explicitly deferred
- **The `Collation` field on the string `DataType` variants** (value-domain representation). Portable code is uniformly binary and non-binary is rejected at entry, so nothing portable stores a collation; the field's only consumer is engine-bound models. Lands with the engine-declaration feature, same gating as engine-bound decimal division (§15).
- **Engine-bound native collation** and the collation-mismatch `TypeMismatch` rule (§17) — no portable trigger without the field; gated on engine declarations.
- **Pin `COLLATE "C"` on PostgreSQL emission** (§17). No Postgres *runtime* backend exists (ROADMAP item 8), so the unsoundness is latent and the pin is not executably verifiable; correct pin placement needs the backend's emission context. Lands with the Postgres backend.
- **Folding collation into the output fingerprint** (§17, `output_fingerprint.md`) — gated on the fingerprint-runtime wiring (ROADMAP item 3).
- **Source-YAML-declared collation**, ordering-only carve-outs, and `LIKE`/pattern-matching collation rules — deferred per §17 Known Divergences.

## Progress tracking

| Phase | Status   | Commit | Date       |
|-------|----------|--------|------------|
| 1     | done     |        | 2026-06-13 |
| 2     | done     |        | 2026-06-13 |

---

### Phase 1: `COLLATE` parsing + `NonPortableCollation` diagnostic

**Goal.** Lex and parse a postfix `expr COLLATE <name>` collation clause; accept binary collation names as a no-op passthrough; emit `NonPortableCollation` for any non-binary collation in portable code.

**Pre-conditions.** None — `COLLATE` is unhandled today (the lexer has no `COLLATE` keyword; the diagnostic does not exist).

**TDD tests to write first.** Listed verbatim — write these as failing tests before any implementation:
- `crates/smelt-parser/src/...::collate_clause_parses` — `SELECT name COLLATE NOCASE FROM t` parses to a `COLLATE` expression node over the `name` operand (a parse tree, not an error node); `name COLLATE "C"` parses equivalently.
- `crates/smelt-db/src/type_inference.rs::tests::binary_collation_passes_through` — `name COLLATE "C"` (and `BINARY`, `UTF8_BINARY`, case-insensitive) infers the operand's type unchanged and emits no diagnostic.
- `crates/smelt-db/src/type_inference.rs::tests::non_binary_collation_diagnoses` — `name COLLATE NOCASE` emits a `NonPortableCollation` Error anchored at the `COLLATE` clause span; the expression type degrades to `Unknown` (reason `Unresolved`).
- `crates/smelt-cli/tests/example_diagnostics.rs` (or the broken-fixtures harness) — a new `examples/broken/` model using `COLLATE NOCASE` surfaces exactly one `NonPortableCollation` diagnostic; a sibling model using `COLLATE "C"` surfaces none.

**Implementation shape.**
- `smelt-parser`: add a `COLLATE` keyword to the lexer; parse a postfix collation clause in the expression grammar (`Collate(operand, collation_name)` AST node, binding tighter than comparison so `a COLLATE c = b` groups as `(a COLLATE c) = b`). Error-recovery: a missing collation name yields the operand with a parser error, not a panic.
- `smelt-db`: add `DiagnosticCode::NonPortableCollation`; extend the diagnostic mapping. In `type_inference.rs`, handle the collate node — normalise the collation name and check membership in the binary set `{C, POSIX, BINARY, UTF8_BINARY}` (case-insensitive). Binary → return operand type unchanged. Non-binary → push a `NonPortableCollation` diagnostic at the clause span and return `Unknown(Unresolved)`. Keep inference pure (CLAUDE.md purity rule).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-parser/src/{lexer,parser,ast}.rs` — `COLLATE` token, postfix parse, AST node.
- `crates/smelt-db/src/lib.rs` — `DiagnosticCode::NonPortableCollation` + mapping.
- `crates/smelt-db/src/type_inference.rs` — collate-node inference + diagnostic.
- `examples/broken/` — non-binary fixture (and a binary-passes fixture under an existing example).

**Docs touched.**
- `docs/specs/types.md` — none required (§17 + the `NonPortableCollation` Surface entry already describe this behaviour; verify they match the landed diagnostic span/recovery and adjust wording only if drift appears).
- `docs-site/docs/...` — add a short "string collation is binary in portable code" note to the types/portability page, with the two remedies (compare byte-wise; declare an engine).

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] §17 "non-binary collation is not in the portable surface" satisfied; binary names accepted as no-op
- [ ] `type_inference.rs` stays pure; diagnostic flows through the standard mapping
- [ ] No collation field added to `DataType`; no scope creep into Phase 2
- [ ] User docs updated to match Surface
- [ ] Spec + docs-site edits are timeless — no phase vocabulary in body

**Commit.** `feat(types): COLLATE parsing + NonPortableCollation diagnostic (binary-only portable surface, §17)`

---

### Phase 2: Binary string-comparison coverage + spec reconciliation

**Goal.** Lock in that binary string comparison/grouping is stable and divergence-free on the live oracle, and reconcile the spec's Known Divergence to the post-implementation state.

**Pre-conditions.** Phase 1 done — the diagnostic guarantees only binary collation reaches inference, so the oracle's string columns are uniformly binary.

**TDD tests to write first.**
- `crates/smelt-db/tests/type_property_tests.rs` (or a focused sibling) — a case asserting string `=`, `<`, `GROUP BY`, `DISTINCT`, and `ORDER BY` over a `Text` column infer correctly and that DuckDB execution of the same is deterministic/byte-wise; no new entry is required in `divergences.rs` (binary agreement holds), and the test fails if a non-binary collation is silently introduced.
- `crates/smelt-cli/tests/example_diagnostics.rs` — a real `examples/` model exercising binary string `GROUP BY`/`DISTINCT`/`ORDER BY` compiles with zero diagnostics (the positive companion to Phase 1's broken fixture).

**Implementation shape.** Primarily test + docs; no new inference behaviour beyond Phase 1. If the oracle surfaces any string-comparison divergence (DuckDB vs the Spark registry), register it `ByDesign` in `divergences.rs` with a one-line rationale rather than masking it.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/tests/type_property_tests.rs` (+ `prop_helpers/` if a generator tweak is needed).
- `crates/smelt-db/tests/prop_helpers/divergences.rs` — only if a real divergence is found.
- `examples/` — positive binary-string fixture.

**Docs touched.**
- `docs/specs/types.md` — the Known Divergences entry was already reconciled when Phase 1 landed (so no commit ever contradicts the code); Phase 2 only flips the §Constraints "Standing collation gate" from "after the collation plan lands" to active, naming the test.
- `docs-site/docs/...` — extend the Phase 1 note with the binary-comparison guarantee (portable string ops produce identical results on every engine).

**Review checklist** (material findings only):
- [ ] TDD tests exist and exercise binary string comparison on a real fixture + the live oracle
- [ ] §17 "binary is the only portable collation" guarantee is verified, not just asserted
- [ ] Spec Known Divergence + Constraints gate reconciled to the true post-plan state (deferrals named honestly)
- [ ] No scope creep (no `DataType` field, no Postgres pin)
- [ ] Spec + docs-site edits are timeless — deferrals described behaviourally with this plan linked

**Commit.** `test(types): binary string-comparison coverage + reconcile §17 collation divergence`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `cargo test -p smelt-parser` — `COLLATE` parses.
- `cargo test -p smelt-db --test type_property_tests` — binary string comparison oracle green.
- `cargo test -p smelt-cli --test example_diagnostics` — non-binary fixture raises `NonPortableCollation`; binary fixtures clean.
- `/smelt:validate types` reports the collation portion as enforced, with only the named deferrals (DataType field, engine-bound collation, Postgres pin, fingerprint fold) outstanding.
