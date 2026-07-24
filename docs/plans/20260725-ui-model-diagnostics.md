# Plan: Model Diagnostics (UI)

**Date**: 2026-07-25
**Spec**: [`docs/specs/ui_model_diagnostics.md`](../specs/ui_model_diagnostics.md)
**Spec diff**: new spec
**Tracking PR / branch**: `spec-redraft-incremental-models` (current branch — this UI work is the practical companion to that spec-clarity effort, giving a hands-on way to explore the incremental-models maintenance plan while it's being redrafted)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/ui_model_diagnostics.md` — it is the correctness oracle. Do not re-open settled spec decisions (in particular: the technique-preview set is additive display-only and must never widen real execution admission in `smelt-logical::maintenance::choice`; the CLI/UI must stay thin consumers of the one `smelt-runtime` builder).
2. Confirm you are on branch `spec-redraft-incremental-models`. If not, switch to it before continuing; ask the user if unsure.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.
- Phase 2 (or 3) implementation reveals that `smelt-logical::maintenance::emit`'s emitters cannot be called against a cell's contract data without also being the admitted technique (i.e. they assume `cell.technique` context beyond a swappable parameter) — this would mean the technique-preview design needs a small emitter-signature change, which is in scope, but the reviewer must confirm it doesn't touch `choice.rs`'s real-execution semantics.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/` (prefer `examples/timeseries/` for maintenance-plan/technique-preview coverage, since it already has non-trivial multi-cell models).
- Red-green TDD: failing test before any implementation.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call: fmt + clippy + tests + example_diagnostics, failures-only output) — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md`, in particular: Salsa purity, maintenance-plan purity (`smelt-logical` derives, backends/consumers never re-derive), fail-loud discipline (a `NotApplicable` preview must always carry its reason — never a silent omission).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/ui_model_diagnostics.md` and `docs-site/docs/...` describe the feature as if it has always existed — no `### Phase A — …` headings, no `(Phase B)` inline labels, no `[deferred to Phase E1]` callouts in spec/user-doc body. The plan's Progress tracking table is where "what landed when" lives.

---

## Context

Today `smelt explain <model> --show-sql` is the only place any of this data — relation contracts, per-cell maintenance-plan statements — is assembled, and it only ever shows the SQL for the technique a run would actually execute (`ChosenTechnique`), not the alternatives. No UI surface exists for any of it, and no endpoint anywhere serializes a model's derived-property set (`docs/specs/model_properties.md` §Surface catalogue). This plan builds the shared `smelt-runtime::diagnostics` builder, the two thin consumers on top of it (`smelt explain`, a new UI REST endpoint), and the UI page, per `docs/specs/ui_model_diagnostics.md` §Surface.

## Scope

### In scope (spec coverage)
- §Surface "smelt-runtime builder" — `ModelDiagnostics`: property set, relation contract, per-cell technique-preview sets.
- §Semantics "Technique preview set" / "Admissibility verdict" — the three-state verdict computed via `smelt-logical::maintenance::choice`'s existing admission logic, read-only, never widening real execution.
- §Semantics "Comment stripping" — `strip_sql_comments`.
- §Surface "CLI" — `--technique` flag, `--json` gaining full preview array + properties.
- §Surface "UI REST API" — `GET /api/models/:name/diagnostics`.
- §Surface "UI page" — full-screen diagnostics page, technique picker, SqlViewer, page-wide remove-comments toggle.

### Explicitly deferred
- Editable SQL buffer / ad hoc-SQL analysis endpoint / in-editor LSP — §Future Extensions, not decided.
- Widening `smelt bakeoff` to read from this builder — §Known Divergences open question, not decided.
- Any technique registry expansion beyond what `smelt-logical`'s emitters already implement — §Limitations, out of scope by spec.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | cb878c9a | 2026-07-25 |
| 2a    | pending  |        |      |
| 2b    | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |

---

### Phase 1: `strip_sql_comments`

**Goal.** A pure, tested `smelt-parser` function that strips `COMMENT` trivia tokens from SQL text while preserving everything else byte-for-byte, per §Semantics "Comment stripping" and §Constraints.

**Pre-conditions.** None — self-contained in `smelt-parser`.

**TDD tests to write first.**
- `crates/smelt-parser/tests/strip_comments.rs::strips_line_comment_preserves_layout` — a `-- comment` on its own line and trailing a statement; asserts surrounding whitespace/newlines unchanged.
- `crates/smelt-parser/tests/strip_comments.rs::strips_nested_block_comment` — `/* outer /* inner */ still outer */`, matching the lexer's existing nesting-aware `consume_block_comment`.
- `crates/smelt-parser/tests/strip_comments.rs::preserves_comment_like_text_inside_string_literal` — a string literal containing `-- not a comment` or `/* not a comment */` must survive untouched (the lexer already tokenizes this correctly; the test guards the stripping function doesn't regress it).
- `crates/smelt-parser/tests/strip_comments.rs::idempotent` — running twice equals running once.
- `crates/smelt-parser/tests/strip_comments.rs::real_fixture_examples_timeseries` — run against at least one commented model file under `examples/timeseries/` (grep first for one with comments; add a comment to a copy if none exist) and snapshot the stripped output.

**Implementation shape.** New `pub fn strip_sql_comments(sql: &str) -> String` in `crates/smelt-parser/src/lib.rs` (or a new `src/strip_comments.rs` module re-exported from `lib.rs`, matching the existing `strip_frontmatter` placement style). Implementation: lex the input, reassemble the original source by concatenating every token's original text span except `SyntaxKind::COMMENT` tokens — operate on raw text spans and the token stream, not the printer's AST reconstruction (§Design "Why comment stripping is a new token-level utility, not the printer").

**Critical files (allowed to touch in this phase).**
- `crates/smelt-parser/src/lib.rs` (or new `src/strip_comments.rs`) — new function
- `crates/smelt-parser/tests/strip_comments.rs` — new test file

**Docs touched.**
- `docs/specs/ui_model_diagnostics.md` — none needed (Surface/Semantics already describe this function; no drift to fix).

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Idempotence and whitespace-preservation hold, including the nested-block-comment and string-literal cases
- [ ] No architectural invariant violated (pure function, no `unwrap`/`expect` beyond what's already budgeted)
- [ ] No scope creep into Phase 2's builder
- [ ] Spec/docs-site edits (if any) are timeless

**Commit.** `feat(smelt-parser): add strip_sql_comments token-level comment stripper`

---

### Phase 2a: `smelt-runtime::diagnostics` — properties + relation contract

**Goal.** Stand up the `smelt-runtime::diagnostics` module and its `ModelDiagnostics` type with the property-set and relation-contract fields populated (no technique previews yet — that's Phase 2b), per §Surface "smelt-runtime builder" (first two bullets).

**Pre-conditions.** Phase 1 done (not a hard dependency, but keeps the crate's new-module pattern consistent).

**TDD tests to write first.**
- `crates/smelt-runtime/tests/diagnostics.rs::properties_cover_full_catalogue` — for a real fixture model in `examples/timeseries/` with a known grain/determinism/monotonicity shape, assert every property named in `docs/specs/model_properties.md` §Surface catalogue appears in the returned `ModelDiagnostics` (model-scoped and column-scoped where applicable) — this is the "exhaustive serialization" guard the spec's Surface bullet requires.
- `crates/smelt-runtime/tests/diagnostics.rs::relation_contract_matches_existing_explain_output` — build diagnostics for a model with upstream edges, compare `ModelDiagnostics.contract`/`inbound_edges` against the current `smelt-cli::explain::build_relation_contract` output for the same model (golden comparison — protects the "moved, not reimplemented" requirement from the earlier planning conversation).
- `crates/smelt-runtime/tests/diagnostics.rs::no_live_backend_required` — call the builder with no target/backend configured, assert it succeeds (§Constraints "must not require a live backend connection").

**Implementation shape.** New `crates/smelt-runtime/src/diagnostics.rs`: `pub fn build_model_diagnostics(db: &dyn ..., model: ModelId) -> Result<ModelDiagnostics, DiagnosticsError>`. `ModelDiagnostics` struct (all `Serialize`) with `properties: PropertySet` (new adapter type wrapping/serializing the existing `smelt-logical::analysis::walk` `PropertyVector` output — add `Serialize` derives to the underlying verdict types where missing, don't restructure the walk itself, per the earlier plan's note), `contract: RelationContractView`, `inbound_edges: Vec<InboundEdgeContract>`, and a `cells` field left as a placeholder (`Vec<PlanCellDiagnostics>`, populated in Phase 2b — define the type now if convenient, or defer to 2b, implementer's call). Move `build_relation_contract` out of `crates/smelt-cli/src/explain.rs` into this module; **do not** yet change `explain.rs`'s call sites (that's Phase 3) — instead, this phase makes `smelt-cli` depend on `smelt-runtime::diagnostics::build_relation_contract` for the same result, keeping `explain.rs`'s own function as a thin re-export or leaving both temporarily present if that's cleaner; the implementer should pick whichever keeps this phase's diff smallest, and Phase 3 is where `explain.rs` fully switches over.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/diagnostics.rs` — new
- `crates/smelt-runtime/src/lib.rs` — register the new module
- `crates/smelt-logical/src/analysis/*` — add `Serialize` derives to verdict types only if not already present (no logic changes)
- `crates/smelt-runtime/tests/diagnostics.rs` — new

**Docs touched.**
- `docs/specs/ui_model_diagnostics.md` — none needed yet (Surface already describes the target shape); if the implementer discovers the property catalogue has grown/shrunk since the spec was written, update `model_properties.md` §Surface to match reality first, then note it here.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Property serialization is exhaustive against `model_properties.md` §Surface, not a subset
- [ ] Relation contract output is byte/value-identical to current CLI behavior (golden test passes)
- [ ] Salsa purity respected — `build_model_diagnostics` is a thin wrapper if it touches Salsa queries directly, or takes already-resolved query results (match the existing pattern `smelt-cli::explain` uses)
- [ ] No scope creep into Phase 2b's technique-preview logic
- [ ] Spec/docs-site edits (if any) are timeless

**Commit.** `feat(smelt-runtime): add diagnostics builder for model properties and relation contract`

---

### Phase 2b: `smelt-runtime::diagnostics` — technique previews + admissibility

**Goal.** Add the per-cell technique-preview set with the three-state admissibility verdict, the highest-risk logic in this plan, per §Surface "smelt-runtime builder" (technique-preview bullet) and §Semantics "Technique preview set" / "Admissibility verdict".

**Pre-conditions.** Phase 2a done (`ModelDiagnostics` scaffold exists).

**TDD tests to write first.**
- `crates/smelt-runtime/tests/diagnostics.rs::admitted_preview_matches_live_run_statements` — for a cell whose admitted technique is known, assert the `Admitted` preview's statements are identical to what `smelt-cli::explain::build_cell_statement_group` (pre-refactor, or the moved equivalent) emits today — protects §Constraints' "must be identical to what a live run executes" invariant.
- `crates/smelt-runtime/tests/diagnostics.rs::recompute_is_always_interchangeable_when_not_admitted` — for any cell where recompute isn't the admitted technique, assert its preview verdict is `InterchangeableAlternative` (§Semantics, region recompute is always contract-agnostic-sound).
- `crates/smelt-runtime/tests/diagnostics.rs::not_applicable_carries_reason` — construct/find a cell with `RowIdentity::WholeRow` (no key), assert the keyed-fold preview is `NotApplicable{reason}` with a non-empty reason, and that its illustrative SQL is still present (§Semantics: emitter still called for illustration).
- `crates/smelt-runtime/tests/diagnostics.rs::exactly_one_admitted_per_cell` — property-style assertion across several fixture models' cells.
- `crates/smelt-runtime/tests/diagnostics.rs::every_known_technique_has_an_entry` — for a cell, assert the preview set has one entry per technique in the registry the emitters implement (§Semantics: "never partial by omission").
- `crates/smelt-runtime/tests/diagnostics.rs::choice_rs_execution_semantics_unchanged` — a regression guard: run `resolve_cell_choice` directly (unaffected by this phase) on a fixture cell and assert its result is unchanged from before this phase (protects §Design's "additive, doesn't widen `choice.rs`" claim — if this test doesn't already exist for `choice.rs`, add a minimal one here rather than skipping the guard).

**Implementation shape.** Extend `crates/smelt-runtime/src/diagnostics.rs`: `PlanCellDiagnostics { cell: PlanCell-derived fields, technique_previews: Vec<TechniquePreview> }`, `TechniquePreview { technique: Technique, statements: Vec<Statement>, admissibility: Admissibility }`, `enum Admissibility { Admitted, InterchangeableAlternative, NotApplicable { reason: String } }`. Move `build_cell_statement_group` from `crates/smelt-cli/src/explain.rs` into this module, parameterized by `technique: Technique` instead of reading `cell.technique` directly (per the earlier planning conversation's design). For each cell, iterate the full technique registry (`DeleteInsert`, `KeyedFold`, `ColumnScopedMerge`, `InPlaceUpdate`, region recompute): call the corresponding `smelt_logical::maintenance::emit::emit_*` function against the cell's own already-derived contract/row-identity/column data to get statements; separately compute admissibility by calling `smelt_logical::maintenance::choice::resolve_cell_choice`/`admits()` for that technique against this cell — where `admits()` doesn't cleanly answer "is this technique's preconditions met" for techniques outside its current binary set, add the missing structural precondition check (e.g. row-identity-requires-key for keyed-fold) as new pure classification logic in this module (or in `smelt-logical` if it's a natural extension of `choice.rs`'s existing precondition checks — implementer's call, but it must not change `resolve_cell_choice`'s actual resolved output for real execution, per the regression test above).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/diagnostics.rs` — extend
- `crates/smelt-logical/src/maintenance/choice.rs` — only if a structural-precondition helper needs to live there (read-only addition, no change to `resolve_cell_choice`'s resolved output)
- `crates/smelt-runtime/tests/diagnostics.rs` — extend

**Docs touched.**
- `docs/specs/ui_model_diagnostics.md` — none needed (already describes this fully); if implementation reveals the technique registry has members with no emitter yet, add/confirm the §Limitations bullet already covering this.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] `Admitted` preview is byte-identical to live-run statements
- [ ] Exactly one `Admitted` entry per cell, always
- [ ] `NotApplicable` always carries a reason (fail-loud discipline)
- [ ] `resolve_cell_choice`'s real-execution output is provably unchanged (regression test passes)
- [ ] No new technique registry members invented beyond what emitters already implement (§Limitations)
- [ ] Spec/docs-site edits (if any) are timeless

**Commit.** `feat(smelt-runtime): add per-cell technique previews with admissibility verdicts`

---

### Phase 3: `smelt-cli` — thin `explain.rs` + `--technique` flag

**Goal.** `smelt explain` becomes a thin renderer of `smelt-runtime::diagnostics::build_model_diagnostics`, and gains `--technique <name>`, per §Surface "CLI".

**Pre-conditions.** Phase 2a and 2b done.

**TDD tests to write first.**
- `crates/smelt-cli/tests/explain.rs::show_sql_output_unchanged` — golden/snapshot comparison of `smelt explain <model> --show-sql` output before vs. after the refactor, on an `examples/timeseries/` model — must be byte-identical (protects the "no behavior change to existing default" requirement).
- `crates/smelt-cli/tests/explain.rs::technique_flag_renders_named_technique` — `smelt explain <model> --show-sql --technique keyed_fold` renders that technique's preview statements for every cell that has one.
- `crates/smelt-cli/tests/explain.rs::technique_flag_reports_not_applicable_per_cell` — for a cell where the requested technique is `NotApplicable`, assert the CLI reports the reason per-cell rather than silently omitting it (§Surface: "the CLI reports that per-cell rather than silently omitting it").
- `crates/smelt-cli/tests/explain.rs::json_includes_full_preview_array_and_properties` — `--json` output includes all technique previews per cell (not just admitted) and the property set.

**Implementation shape.** `explain.rs`'s report builders call `smelt_runtime::diagnostics::build_model_diagnostics` once and render from the returned `ModelDiagnostics`; delete the now-duplicate `build_relation_contract`/`build_cell_statement_group` definitions in `smelt-cli` in favor of the `smelt-runtime` versions from Phase 2a/2b. Add `--technique <name>` to `crates/smelt-cli/src/commands/explain.rs`'s arg parsing, mapping the accepted names (`delete_insert`, `keyed_fold`, `column_scoped_merge`, `in_place_update`, `recompute`) to `Technique`/recompute variants.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/explain.rs` — refactor to consume shared builder
- `crates/smelt-cli/src/commands/explain.rs` — new flag
- `crates/smelt-cli/tests/explain.rs` — new/extended tests

**Docs touched.**
- `docs/specs/ui_model_diagnostics.md` — none needed (already describes `--technique`).
- `docs-site/docs/` — update the `smelt explain` CLI reference page with the new flag and an example.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Existing `--show-sql`/`--json` output is unchanged (golden test passes) — this is the thin-consumer boundary's main risk
- [ ] `explain.rs` no longer derives contract/statement data itself (§Semantics "Thin-consumer boundary")
- [ ] `--technique` failure/not-applicable path is fail-loud, not silent
- [ ] docs-site CLI reference updated
- [ ] Spec/docs-site edits are timeless

**Commit.** `refactor(smelt-cli): explain consumes shared diagnostics builder, add --technique flag`

---

### Phase 4: `smelt-ui` — REST endpoint

**Goal.** `GET /api/models/:name/diagnostics` returns `ModelDiagnostics` as JSON, per §Surface "UI REST API".

**Pre-conditions.** Phase 2a and 2b done (Phase 3 not required, but doing it first reduces risk of finding builder bugs simultaneously in two consumers).

**TDD tests to write first.**
- `crates/smelt-ui/tests/api.rs::diagnostics_endpoint_returns_full_payload` — hit the route for a fixture model, assert the response deserializes into the expected shape and matches `smelt explain --json`'s equivalent fields for the same model (cross-consumer parity check).
- `crates/smelt-ui/tests/api.rs::diagnostics_endpoint_404_for_unknown_model` — matches existing `/api/models/:name` 404 convention.

**Implementation shape.** `crates/smelt-ui/src/build.rs`: `build_model_diagnostics_response(model) -> ModelDiagnosticsResponse`, thin adapter over `smelt_runtime::diagnostics::build_model_diagnostics`. Check `crates/smelt-ui/Cargo.toml` for an existing `smelt-runtime` dependency; if present, reuse `ModelDiagnostics`'s `Serialize` impl directly rather than hand-mirroring a parallel Rust struct (per the earlier planning conversation's preference to avoid drift) — if absent, add the dependency (already implied by `smelt-ui`'s existing use of `execute_project` per `architecture.md`'s Run pipeline parity rule, so this should not be a new architectural edge). Route registered in `crates/smelt-ui/src/{api.rs,server.rs}` following the existing `/api/models/:name` handler pattern. `ui/src/types.ts` gets the hand-mirrored TypeScript type (existing convention) and `ui/src/api.ts` gets `fetchModelDiagnostics(name)`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-ui/src/{types.rs,build.rs,api.rs,server.rs}`
- `ui/src/types.ts`, `ui/src/api.ts`
- `crates/smelt-ui/tests/api.rs`

**Docs touched.**
- `docs/specs/ui_model_diagnostics.md` — none needed.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Endpoint is a thin adapter — no derivation logic in `smelt-ui` itself (§Semantics "Thin-consumer boundary")
- [ ] 404 behavior matches existing convention
- [ ] `ui/src/types.ts` mirrors the Rust response shape accurately
- [ ] Spec/docs-site edits (if any) are timeless

**Commit.** `feat(smelt-ui): add GET /api/models/:name/diagnostics endpoint`

---

### Phase 5: UI — SqlViewer + ModelDiagnostics page + navigation

**Goal.** The full-screen diagnostics page exists, is reachable from the graph view, and matches §Surface "UI page" exactly: overview, full property set, maintenance plan with technique picker + admissibility badges, read-only syntax-highlighted SQL viewers, page-wide remove-comments toggle.

**Pre-conditions.** Phase 4 done.

**TDD tests to write first.** (React/TS — use whatever test runner `ui/` already has; if none exists yet, component tests are the implementer's call on tooling, but the manual walkthrough below is mandatory regardless.)
- `ui/src/components/SqlViewer.test.tsx::renders_readonly_syntax_highlighted_sql` — smoke test the CodeMirror wrapper renders given SQL and cannot be edited (no onChange fired on keypress).
- `ui/src/pages/ModelDiagnostics.test.tsx::technique_picker_swaps_sql_and_badge` — selecting a different technique in a cell's picker updates the displayed `SqlViewer` content and the admissibility badge to match the fixture response's corresponding preview entry.
- `ui/src/pages/ModelDiagnostics.test.tsx::remove_comments_toggle_applies_to_every_viewer` — toggling the page-level checkbox strips comments from both the model's own SQL viewer and every technique-preview viewer simultaneously.

**Implementation shape.** Add `@uiw/react-codemirror` + `@codemirror/lang-sql` to `ui/package.json`. New `ui/src/components/SqlViewer.tsx` (read-only CodeMirror wrapper, `props: { sql: string, removeComments: boolean }` — comment-stripped variant can come from the API response per §Surface, or be computed client-side against the raw SQL; follow whichever the Phase 2/4 response shape actually provides). New `ui/src/pages/ModelDiagnostics.tsx` following the `RunPlanner.tsx`/`RunHistory.tsx` full-screen-page pattern (own `useQuery` on `fetchModelDiagnostics`, own scroll container). Wire navigation: extend `App.tsx`'s state to track an open diagnostics model (not a `Page` union entry, since this is per-model, not a standalone nav tab — add e.g. `diagnosticsModel: string | null` alongside `selectedModel`), add an "Open Diagnostics" action to `ModelDetail.tsx`'s side panel and/or a node context action in `Graph.tsx`/`ModelNode.tsx`, and render `<ModelDiagnostics>` full-screen in place of the graph+panel row when open, with a close action returning to the graph.

**Critical files (allowed to touch in this phase).**
- `ui/package.json` — new CodeMirror deps
- `ui/src/components/SqlViewer.tsx` — new
- `ui/src/pages/ModelDiagnostics.tsx` — new
- `ui/src/App.tsx` — navigation state
- `ui/src/components/ModelDetail.tsx` — "Open Diagnostics" entry point
- `ui/src/components/Graph.tsx` / `ModelNode.tsx` — optional second entry point, implementer's call

**Docs touched.**
- `docs/specs/ui_model_diagnostics.md` — none needed.
- `docs-site/docs/` — deferred to Phase 6 (dedicated docs phase), do not write UI user docs here.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Page is a superset of `ModelDetail.tsx`'s existing fields (§Surface "UI page" — nothing regresses relative to the side panel)
- [ ] SqlViewer is genuinely read-only (no accidental editability)
- [ ] Admissibility badge reflects the API response verbatim — no client-side re-derivation (§Semantics "Thin-consumer boundary" applies to the frontend too)
- [ ] Remove-comments toggle is page-wide, applies to every viewer
- [ ] `cd ui && npm run build` succeeds (required before `smelt-ui` compiles)
- [ ] Manual walkthrough performed against `examples/timeseries/` (open a model with a multi-cell plan, confirm properties/technique-picker/badges/toggle/close all work) — record the result in the phase's review notes
- [ ] Spec/docs-site edits (if any) are timeless

**Commit.** `feat(ui): add model diagnostics page with technique picker and SQL viewer`

---

### Phase 6: User docs

**Goal.** `docs-site/docs/` documents the diagnostics UI page and the `--technique` CLI flag as if they've always existed, per the repo's Timeless-oracle rule.

**Pre-conditions.** Phases 1–5 done (the feature must exist before documenting it).

**TDD tests to write first.** N/A (docs-only phase) — verify instead with `/smelt:validate ui_model_diagnostics` after writing, to confirm no drift between spec/code/docs.

**Implementation shape.** Add a docs-site page under the UI documentation section covering: opening the diagnostics page, reading the property panel, using the technique picker and interpreting admissibility badges, the remove-comments toggle. Update the existing `smelt explain` CLI reference page with `--technique`.

**Critical files (allowed to touch in this phase).**
- `docs-site/docs/...` — new UI diagnostics page, updated CLI reference page (exact paths: find the existing UI/CLI doc tree structure first, match its conventions)

**Docs touched.**
- `docs/specs/ui_model_diagnostics.md` — update §Known Divergences to remove the "entire surface is unimplemented" entry now that it's landed; add the real plan path to §References → Plans (history); fill in §References → Code/Tests/User docs with the actual final paths (may differ slightly from the placeholders written when the spec was drafted).

**Review checklist** (material findings only):
- [ ] docs-site pages read as timeless feature descriptions, no phase vocabulary
- [ ] Spec's Known Divergences and References sections updated to reflect landed reality
- [ ] `/smelt:validate ui_model_diagnostics` reports zero drift

**Commit.** `docs: add user documentation for model diagnostics UI and --technique flag`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

- `bash .claude/scripts/verify-phase.sh` passes after every phase and at the end.
- `cargo test -p smelt-runtime --test diagnostics` — all technique-preview and property-serialization tests green.
- `cargo test -p smelt-cli --test explain` — golden `--show-sql`/`--json` comparisons green, `--technique` tests green.
- `cargo test -p smelt-ui --test api` — endpoint tests green.
- `cd ui && npm run build` succeeds; manual walkthrough per Phase 5's checklist performed against `examples/timeseries/`.
- `/smelt:validate ui_model_diagnostics` reports zero drift.
