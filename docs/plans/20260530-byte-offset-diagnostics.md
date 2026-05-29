# Plan: byte-offset diagnostics with LineIndex at the boundary

**Date**: 2026-05-30
**Spec**: [`docs/specs/architecture.md`](../specs/architecture.md)
**Spec diff**: uncommitted working tree — `architecture.md` §Semantics gains "Diagnostic range encoding rule"; §Known Divergences gains the entry tracking this plan.
**Tracking PR / branch**: branch `worktree-unknown_types`.
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/architecture.md` §"Diagnostic range encoding rule" and §"Known Divergences" — these are the correctness oracle.
2. Confirm you are on branch `worktree-unknown_types`. If not, ask before continuing.
3. Find the next `pending` phase in the Progress table. If all are `done`, run Verification and stop.

**Per-phase loop (`/smelt:implement`):** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**
- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A pre-existing failure unrelated to the plan surfaces.
- The `line-index` crate API behaves differently from the investigation note (`LineIndex::new(text)`, `line_col(TextSize) -> LineCol`, `to_wide(WideEncoding, LineCol) -> WideLineCol`).

**Conventions every phase:**
- Real-fixture coverage at every phase. Phase 3 adds a non-ASCII fixture.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commit using the `Commit.` line verbatim; push after each.
- Honor `CLAUDE.md` invariants: `smelt-db` pure-function rule; Workspace-Loading-Parity rule; Project Isolation rule; Run-pipeline parity rule; the new Diagnostic-range-encoding rule.
- **Timeless-oracle rule.** Phase vocabulary lives in this plan only. Spec / docs-site edits describe the feature as if it has always existed.

---

## Context

`architecture.md` §"Diagnostic range encoding rule" specifies that diagnostics carry byte-offset `TextRange` internally, with `(line, column)` conversion happening exactly once at the LSP / CLI boundary backed by a per-file `LineIndex`. Today every diagnostic-producing helper in `smelt-db` and every translation helper (`shifted_range`, `shifted_body_text_range`, `remap_body_range`, `body_position_to_byte`) converts inline using `offset_to_position`, which counts codepoints labelled as "column" — non-ASCII text produces drift from byte and UTF-16 positions. The rule depends on the `line-index = "0.1.2"` crate (rust-lang/rust-analyzer) for the boundary converter.

## Scope

### In scope (spec coverage)
- `architecture.md` §"Diagnostic range encoding rule" — the rule's behaviour is brought to parity with its description: internal `TextRange`, boundary `LineIndex`, LSP `positionEncodingKind` negotiation, codepoint columns for terminal output.
- `architecture.md` §"Known Divergences" — the "Diagnostic range encoding rule is not yet upheld" entry is closed and deleted as the final step of Phase 4.

### Explicitly deferred
- Encoding negotiation for the runtime `RunReporter` and the UI HTTP/WebSocket adapter — Phase 3 ships LSP negotiation; UI / runtime encoding negotiation lands when concrete editor-in-browser work surfaces.
- Migration of every existing `Range`-based diagnostic test assertion to `TextRange` form — Phase 2 introduces a test-side conversion helper so existing assertions keep working through `LineIndex` at assertion time. A future refactor can simplify those tests.
- The wider question of whether `Position` should ever exist as a public type — the boundary converter currently produces LSP / CLI `Range` values directly; if a third surface emerges (e.g. JSON dump format), it negotiates its own encoding at its own boundary.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 13142fe2 | 2026-05-30 |
| 2     | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |

---

### Phase 1: Plumb `LineIndex` at the LSP / CLI boundary alongside existing code

**Goal.** Introduce `line-index = "0.1.2"` as a dependency of `smelt-lsp` and `smelt-cli`. Build a small `BoundaryConverter` (or use `LineIndex` directly) at each of the four LSP / CLI diagnostic emission points named in `architecture.md` §"Diagnostic range encoding rule" — `smelt-lsp::backend::publish_diagnostics`, `smelt-cli::commands::type_::run`, `smelt-cli::commands::run::report_diagnostic`, `smelt-runtime::reporter::emit_diagnostic`. Each converter is constructed but, for this phase, used in a no-op shape: it receives a `TextRange` derived from the existing diagnostic's `Range` round-trip, runs it through `LineIndex`, and asserts (under `debug_assertions`) that the resulting `(line, column)` matches the original `Range`. No behavioural change for ASCII workspaces. Phase 2 flips the diagnostic-shape and the converter becomes load-bearing.

**Pre-conditions.** None.

**TDD tests to write first.**
1. `crates/smelt-lsp/tests/line_index_parity.rs` (new) — for every ASCII fixture under `examples/per_cohort_union/`, `examples/staging_from_sources/`, and at least one broken fixture (e.g. `per_cohort_union_broken_emission_body_undeclared_column`): assert that the `LineIndex::new(text).line_col(diagnostic.text_range.start)` round-trips to the same `(line, column)` as the existing `offset_to_position(text, byte_offset_of(diagnostic.range.start))`. Drives the LineIndex contract directly.
2. `crates/smelt-lsp/tests/line_index_parity.rs` — a non-ASCII synthetic case (string literal embedded in the test, not a fixture yet): for `let text = "α\nβ\nγ\n";` and a known body-local `TextRange`, assert `LineIndex::new(text).line_col(...).line == 1` (i.e. line 1) and `.col == 0` for a `TextSize` pointing at the first byte of `β`. This is a focused unit gate on the crate's behaviour we depend on.
3. Regression: `cargo test -p smelt-cli --test example_diagnostics` (~85 cases) stays green; `cargo test -p smelt-lsp --test example_workspaces` (~25 cases) stays green; `cargo test -p smelt-db --quiet` stays green.

**Implementation shape.**
- `Cargo.toml` (workspace root): add `line-index = "0.1.2"` to `[workspace.dependencies]`.
- `crates/smelt-lsp/Cargo.toml`, `crates/smelt-cli/Cargo.toml`, `crates/smelt-runtime/Cargo.toml`: add `line-index.workspace = true`.
- `crates/smelt-lsp/src/diagnostics_boundary.rs` (new): a `BoundaryConverter { line_index: LineIndex, encoding: WideEncoding }` with a `convert(text_range: TextRange) -> lsp_types::Range` method. The encoding is set at construction; default to `WideEncoding::Utf16` (LSP default). Phase 3 wires negotiation.
- `crates/smelt-cli/src/diagnostics_terminal.rs` (new): a sibling converter that produces codepoint-based `(line, column)` values for terminal output. Initially returns the same shape as the LSP converter; Phase 3 swaps to a codepoint-counting path.
- The four named emission points each construct a converter from the file text and the current `Diagnostic.range` (still `Range`-shaped); under `cfg(debug_assertions)`, recompute via `LineIndex` and `assert_eq!` against the existing `Range`. No production behavioural change.
- Pure-function rule: the converters are pure; `LineIndex` is constructed at the consumer site and threaded as `&LineIndex`.

**Critical files (allowed to touch in this phase).**
- `Cargo.toml` — workspace dep declaration.
- `crates/smelt-lsp/Cargo.toml`, `crates/smelt-cli/Cargo.toml`, `crates/smelt-runtime/Cargo.toml` — per-crate dep.
- `crates/smelt-lsp/src/diagnostics_boundary.rs` (new) — `BoundaryConverter`.
- `crates/smelt-cli/src/diagnostics_terminal.rs` (new) — terminal converter.
- `crates/smelt-lsp/src/backend.rs` (or the `publish_diagnostics` site) — call the converter in shadow mode.
- `crates/smelt-cli/src/commands/type_.rs`, `crates/smelt-cli/src/commands/run.rs` — call the terminal converter in shadow mode.
- `crates/smelt-runtime/src/reporter.rs` (the `emit_diagnostic` site) — call a converter in shadow mode.
- `crates/smelt-lsp/tests/line_index_parity.rs` (new) — TDD coverage.

**Docs touched.** None — no user-visible surface change yet; the converter exists in shadow.

> *Header override*: this phase is code-only despite the plan header `code+docs`. Phase 4 carries the spec / docs-site touch (deletion of the Known Divergence entry + CLAUDE.md mirror).

**Review checklist (material findings only):**
- [ ] `LineIndex` is constructed per file, not per diagnostic.
- [ ] `BoundaryConverter` is pure; no `&dyn salsa::Database` parameter, no Salsa query calls.
- [ ] Shadow-mode `assert_eq!` under `debug_assertions` is on — confirms LineIndex output matches existing `Range` for ASCII inputs.
- [ ] Non-ASCII unit test passes; the crate's API is what the plan investigation note claimed.
- [ ] `example_diagnostics` and `example_workspaces` stay green (no behavioural drift).
- [ ] No diagnostic-shape change; `Diagnostic::range` is still `Range` after this phase.

**Commit.** `feat(lsp): plumb LineIndex at the diagnostic boundary in shadow mode`

---

### Phase 2: Flip `Diagnostic::range` to `TextRange`, refactor all helpers and tests

**Goal.** `smelt_db::Diagnostic::range` becomes `TextRange`. Every `_for_select` / `_for_ast` / `_for_syntax` diagnostic helper, every Salsa accumulator emission, every emission-body translation helper (`shifted_range`, `shifted_body_text_range`, `remap_body_range`, `body_position_to_byte`, `shift_diagnostic_ranges`) collapses to integer-arithmetic on `TextRange`. The `range_offset: usize` parameter on the `_for_select` helpers (added by `20260529-emission-body-diagnostics.md` Phase 1) becomes a `TextSize` offset added to each emitted range; the helpers no longer need a `text: &str` parameter for line/col conversion. The LSP and CLI boundary converters from Phase 1 (still in shadow mode) become the *only* `Range`-producing sites. All existing tests assert on positions via a new `diag.range_in(line_index)` test-side helper that converts `TextRange` → `(line, column)` for assertion compatibility; no `(line, column)` value is computed inside `smelt-db` or `smelt-parser` outside of the boundary.

**Pre-conditions.** Phase 1 done.

**TDD tests to write first.**
1. `crates/smelt-db/tests/diagnostic_range_is_text_range.rs` (new) — assertion that `Diagnostic::range` is `TextRange` at compile time (a type-checking smoke test): construct a diagnostic, assign `.range` from a `TextRange`, and assert `mem::size_of::<Diagnostic>() == size_of_before_minus_position_overhead`. Compile-fail variant: `let _: Range = diag.range` fails to compile.
2. `crates/smelt-db/tests/byte_offset_helpers.rs` (new) — for each shifted helper that survives (just `shift_text_range(range: TextRange, offset: TextSize) -> TextRange` — everything else collapses): unit cases asserting byte arithmetic. Cases: identity (offset = 0), non-zero offset, overflow-safe behaviour at `TextSize::MAX`.
3. `crates/smelt-db/tests/emitted_model_body_diagnostics.rs` (existing — 8 cases) — update each case's assertion to use the new test-side helper: `let pos = line_index.line_col(diag.range.start()); assert_eq!(pos.line, expected_line)`. The lift-protection regression remains the headline assertion.
4. `crates/smelt-cli/tests/example_diagnostics.rs` (existing — ~85 cases) — same conversion at assertion time. Many tests don't care about position, only diagnostic codes; those don't need helper changes.
5. `crates/smelt-lsp/tests/example_workspaces.rs` (existing — ~25 cases) — same conversion.
6. Phase 1's `crates/smelt-lsp/tests/line_index_parity.rs` — graduate: drop the `assert_eq!` against the old `Range` shape (which no longer exists on `Diagnostic`) and assert positions directly via the LineIndex contract.
7. Regression: `cargo test -p smelt-db --quiet 2>&1 | tail -40`, `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-lsp --test example_workspaces` all green.

**Implementation shape.**
- `crates/smelt-db/src/diagnostics_types.rs`: `Diagnostic::range` field type changes from `crate::Range` to `rowan::TextRange`. Same for `Position` fields on `DiagnosticData` variants (deleted; data is byte-offset or doesn't need a position at all).
- Pure helpers across `crates/smelt-db/src/queries/`: every `text: &str, range_offset: usize` pair becomes a single `range_offset: TextSize` parameter (or drops both if the helper no longer needs line/col conversion at all). Internal logic: `out.push(Diagnostic { range: ast_node.text_range() + range_offset, … })`. No `text_range_to_range` calls.
- `synthesise_emission_body_analysis` in `crates/smelt-db/src/queries/project.rs`: `generator_file_text: &str` and `body_offset: usize` parameters become a single `body_offset: TextSize`. The body-local `TextRange` from the body's parse is shifted by `body_offset` before being stored on the diagnostic. The helper `remap_body_range`, `body_position_to_byte`, `shifted_body_text_range` are deleted.
- `shifted_range` and `shift_diagnostic_ranges` in `crates/smelt-db/src/queries/check_types.rs`: deleted. Their call sites use raw `range + range_offset` arithmetic.
- The boundary converter from Phase 1 graduates out of shadow mode: it is the only path that produces a `lsp_types::Range` / terminal `(line, column)` from a `Diagnostic`. The `debug_assertions` shadow check is removed.
- Test-side helper (in `crates/smelt-db/src/test_harness.rs` or a `tests/common.rs`): `fn assert_diagnostic_at(line_index: &LineIndex, diag: &Diagnostic, line: u32, col_or_range: …)` that all existing position-checking tests use. Avoids a flag-day rewrite of every test.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/diagnostics_types.rs` — diagnostic shape.
- `crates/smelt-db/src/queries/check_types.rs`, `crates/smelt-db/src/queries/function_diagnostics.rs`, `crates/smelt-db/src/queries/loader.rs`, `crates/smelt-db/src/queries/project.rs`, and every other `queries/*.rs` file containing a `_for_file` / `_for_select` helper — signature collapses.
- `crates/smelt-db/src/lib.rs` — `check_file_diagnostics`, `file_diagnostics`, and the diagnostic-accumulator path.
- `crates/smelt-db/src/test_harness.rs` (or a new `crates/smelt-db/tests/common.rs`) — the test-side conversion helper.
- `crates/smelt-lsp/src/diagnostics_boundary.rs` — graduate to load-bearing mode.
- `crates/smelt-lsp/src/backend.rs` — use the converter as the sole `Range` producer.
- `crates/smelt-cli/src/diagnostics_terminal.rs`, `crates/smelt-cli/src/commands/type_.rs`, `crates/smelt-cli/src/commands/run.rs` — same.
- `crates/smelt-runtime/src/reporter.rs` — same.
- `crates/smelt-db/tests/emitted_model_body_diagnostics.rs`, `crates/smelt-cli/tests/example_diagnostics.rs`, `crates/smelt-lsp/tests/example_workspaces.rs`, `crates/smelt-lsp/tests/line_index_parity.rs` — assertion updates via the test-side helper.

**Docs touched.** None — this is the heavy refactor phase. Spec / docs-site touches happen in Phase 4 once the rule is uphold-able and the divergence can be deleted.

**Review checklist (material findings only):**
- [ ] `Diagnostic::range` is `TextRange`; the type-shape smoke test compiles green.
- [ ] No `text_range_to_range` / `offset_to_position` calls inside `smelt-db` or `smelt-parser` (except inside boundary converters or test-side helpers).
- [ ] The deleted helpers (`shifted_range`, `shifted_body_text_range`, `remap_body_range`, `body_position_to_byte`, `shift_diagnostic_ranges`) are *actually deleted* — `rg "shifted_body_text_range|remap_body_range|body_position_to_byte"` returns zero matches.
- [ ] Every existing position-checking test passes via the test-side conversion helper.
- [ ] `example_diagnostics` and `example_workspaces` stay green (ASCII regression baseline).
- [ ] Pure-function rule preserved.
- [ ] Spec text on the LSP boundary's positionEncoding negotiation is still aspirational (Phase 3 wires it); the boundary converter ships with `WideEncoding::Utf16` hardcoded.

**Commit.** `refactor(db): carry TextRange through diagnostics and collapse body-offset helpers`

---

### Phase 3: LSP `positionEncodingKind` negotiation + non-ASCII fixture

**Goal.** `smelt-lsp::Backend::initialize` advertises `position_encodings: [Utf16, Utf8]` in server capabilities, reads the client's `general.positionEncodings` capability, picks the first mutually-supported encoding, and stores it on `Backend`. The boundary converter takes the negotiated encoding and uses `LineIndex::to_wide(WideEncoding::Utf16, …)` for UTF-16 columns or returns the raw `LineCol` for UTF-8 byte columns. A non-ASCII fixture confirms positions are correct under both encodings. The standing CI gate `cargo test -p smelt-lsp --test position_encoding` is the spec-named gate.

**Pre-conditions.** Phase 2 done.

**TDD tests to write first.**
1. `crates/smelt-lsp/tests/position_encoding.rs` (new) — the spec-named CI gate. Cases:
   - Client advertises `position_encodings: [Utf8]` → server negotiates Utf8 → an UndeclaredColumn diagnostic on a column with a 2-byte UTF-8 character preceding it reports `character` as the byte column.
   - Client advertises `position_encodings: [Utf16]` (or no advertisement) → server negotiates Utf16 → the same diagnostic reports `character` as the UTF-16 code unit column.
   - Client advertises an unsupported encoding (e.g. `Utf32`) → server falls back to Utf16, no error.
   - ASCII baseline: existing `examples/per_cohort_union/` is exercised under both Utf8 and Utf16 negotiation and produces byte-identical `lsp_types::Range` values (because columns are ASCII-only).
2. `examples/non_ascii_columns/` (new fixture) — a model `models/greek.sql` with column aliases `α`, `β`, `γ` (or similar UTF-8 multi-byte identifiers) and at least one diagnostic-producing condition (an UndeclaredColumn referring to a non-existent `δ`). Assert through `example_diagnostics` and `example_workspaces` that the diagnostic anchors correctly.
3. Regression: existing `example_diagnostics` (~85) and `example_workspaces` (~25) stay green; `emitted_model_body_diagnostics` (8) stays green.

**Implementation shape.**
- `crates/smelt-lsp/src/backend.rs::initialize`: parse `InitializeParams.capabilities.general.positionEncodings`. Default order of preference: Utf16, Utf8 (LSP default first). Pick the first mutually-supported. Store on `Backend.negotiated_encoding`. Advertise it in `InitializeResult.capabilities.position_encoding`.
- `crates/smelt-lsp/src/diagnostics_boundary.rs::BoundaryConverter`: take a `negotiated_encoding: PositionEncodingKind` parameter at construction. The `convert(text_range)` method:
  - For Utf16: `let line_col = line_index.line_col(text_range.start()); let wide = line_index.to_wide(WideEncoding::Utf16, line_col).unwrap_or(WideLineCol { line: line_col.line, col: line_col.col }); …`
  - For Utf8 (bytes): `let line_col = line_index.line_col(text_range.start()); Position { line: line_col.line, character: line_col.col }` — where `line_col.col` is already a byte offset on the line.
- New fixture `examples/non_ascii_columns/` follows the existing fixture layout (`smelt.yml`, `models/greek.sql`, plus an UndeclaredColumn-triggering second model that references a missing non-ASCII column).
- The `position_encoding` test file builds an LSP client mock with each encoding capability and asserts the negotiated value flows through correctly.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-lsp/src/backend.rs` — encoding negotiation in `initialize`.
- `crates/smelt-lsp/src/diagnostics_boundary.rs` — encoding-aware conversion.
- `crates/smelt-lsp/tests/position_encoding.rs` (new) — the standing CI gate.
- `examples/non_ascii_columns/` (new fixture) — non-ASCII coverage.
- `crates/smelt-cli/tests/example_diagnostics.rs` — case for the new fixture.
- `crates/smelt-lsp/tests/example_workspaces.rs` — case for the new fixture.

**Docs touched.** None — the spec text on encoding negotiation is already aspirational from Phase 2; this phase makes it real. The spec edit (deletion of the Known Divergence entry) is Phase 4.

**Review checklist (material findings only):**
- [ ] `InitializeResult.capabilities.position_encoding` is populated with the negotiated kind.
- [ ] Non-ASCII fixture passes under both encodings (different `character` values for the same `TextRange`).
- [ ] ASCII fixtures produce byte-identical `lsp_types::Range` under both encodings.
- [ ] `WideEncoding::Utf16` is the default when client advertises no preference (LSP compliance).
- [ ] Unknown / unsupported encoding from the client → fall back to Utf16; no error returned to client.
- [ ] No regression on `example_diagnostics` or `example_workspaces`.

**Commit.** `feat(lsp): negotiate positionEncodingKind and emit non-ASCII-correct diagnostics`

---

### Phase 4: Cleanup, CLAUDE.md mirror, close the divergence

**Goal.** Delete vestigial helpers (`offset_to_position`, `text_range_to_range`, and any remaining `Range`/`Position` types in `smelt-parser`'s `ast.rs` that are not used by the boundary converters or test-side helpers). Mirror the new architectural invariant in `CLAUDE.md` next to the existing Pure-function / Workspace-loading-parity / Project-isolation / Run-pipeline-parity rules. Delete the §Known Divergences entry from `docs/specs/architecture.md`.

**Pre-conditions.** Phases 1, 2, 3 done.

**TDD tests to write first.**
1. `rg 'offset_to_position|text_range_to_range' crates/smelt-db/src/ crates/smelt-parser/src/` should return zero matches outside of the boundary converters (`crates/smelt-lsp/src/diagnostics_boundary.rs`, `crates/smelt-cli/src/diagnostics_terminal.rs`) and the test-side helper. Bake this into a small `xtask` or just an explicit assertion in the Phase 4 commit description.
2. Regression: all existing tests stay green.

**Implementation shape.**
- `crates/smelt-parser/src/ast.rs`: `offset_to_position` and `text_range_to_range` are deleted (or moved into a `pub(crate)`-scoped helper module only used by the boundary converters via re-export). The `Position` and `Range` types may stay if `lsp-types` doesn't already provide a suitable shape — but they should be re-exported only from the boundary-converter crates, not from `smelt-parser`.
- `CLAUDE.md`: add a short summary of the Diagnostic-range-encoding rule below the existing Run-pipeline-parity rule section. Match the tone and length of the existing summaries (~2 paragraphs).
- `docs/specs/architecture.md` §Known Divergences: delete the entry "Diagnostic range encoding rule is not yet upheld".

**Critical files (allowed to touch in this phase).**
- `crates/smelt-parser/src/ast.rs` — vestigial-helper deletion.
- `CLAUDE.md` — invariant mirror.
- `docs/specs/architecture.md` — divergence-entry deletion.

**Docs touched (timeless phrasing).**
- `docs/specs/architecture.md` — delete the §Known Divergences entry. No other spec edit; the §"Diagnostic range encoding rule" itself stays as the authoritative description.
- `CLAUDE.md` — add the new invariant summary below the Run-pipeline-parity rule.
- No `docs-site/` edit. The user-facing surface is "LSP diagnostics anchor at correct positions, including for non-ASCII text" — already implicit in the LSP feature description.

**Review checklist (material findings only):**
- [ ] `offset_to_position` / `text_range_to_range` are not callable from `smelt-db`, `smelt-runtime`, `smelt-cli` (except via boundary converters), or `smelt-planner`.
- [ ] `CLAUDE.md` mirror is timeless (describes the rule, not its history). Length matches the existing parity-rule summaries.
- [ ] §Known Divergences entry is gone; `/smelt:validate architecture` reports no drift on it.
- [ ] All gates remain green.

**Commit.** `chore(arch): land Diagnostic range encoding rule and close divergence`

---

## Deferred during implementation

(Append-only.)

- Runtime `RunReporter` and UI HTTP/WebSocket adapter encoding negotiation: Phase 3 ships LSP negotiation; other surfaces use the default (terminal: codepoints; runtime: bytes) until a concrete editor-in-browser use case surfaces.

## Verification

- `cargo test -p smelt-lsp --test position_encoding` — green (the standing CI gate named in the spec).
- `cargo test -p smelt-cli --test example_diagnostics` — green (regression on ASCII fixtures; new non-ASCII fixture diagnostic codes).
- `cargo test -p smelt-lsp --test example_workspaces` — green.
- `cargo test -p smelt-db --test emitted_model_body_diagnostics` — green (8/8 with `TextRange`-form assertions).
- `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` — green.
- `target/debug/smelt type --project-dir examples/per_cohort_union` — no UNKNOWN regression (ASCII headline test from prior plans).
- `target/debug/smelt type --project-dir examples/non_ascii_columns` — correct diagnostic positions on non-ASCII content.
- `rg 'offset_to_position|text_range_to_range' crates/smelt-db/src/ crates/smelt-parser/src/` — zero matches outside boundary converters / test-side helpers.
- `/smelt:validate architecture` — no drift on the deleted divergence; no new divergence introduced.
