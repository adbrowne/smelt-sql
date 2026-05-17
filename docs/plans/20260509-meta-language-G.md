# Plan: Meta-Language Phase G — LSP completeness sweep, docs-site polish, `/smelt-loop` tier-3

**Date**: 2026-05-17
**Spec**: [`docs/specs/meta_language.md`](../specs/meta_language.md), [`docs/specs/meta_config_loading.md`](../specs/meta_config_loading.md), [`docs/specs/lsp.md`](../specs/lsp.md) §"Rename"
**Spec diff**: working-tree edit to `docs/specs/lsp.md` Rename table (lambda-parameter row added); no body changes to `meta_language.md` or `meta_config_loading.md` (G is the final-polish phase per meta-plan §3 "Spec increment: none new")
**Tracking PR / branch**: PR #117 (`feat: typed meta-programming`) — `research/typed-meta-programming` (overall plan: [`docs/plans/20260509-meta-language-overall.md`](20260509-meta-language-overall.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-you-optimized-stallman.md`)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/meta_language.md` (the §Surface, §LSP, §Known Divergences subsections — the spec is the correctness oracle), `docs/specs/meta_config_loading.md` (loader surface), and `docs/specs/lsp.md` §"Rename" (where the Phase G spec increment lands).
2. Confirm you are on branch `research/typed-meta-programming`. If not, ask before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent (`model: sonnet`) → reviewer subagent (`model: sonnet`) → iterate → record + commit + push.

**Phase 6 is the expert-reviewer dispatch loop** — after Phases 0–5 commit, dispatch the meta-plan §5 expert reviewers applicable to G (lsp-expert, salsa-expert, examples-curator, docs-reviewer), address material findings, and re-dispatch each expert until clean (or stop-the-line per meta-plan §7). Do NOT skip Phase 6. The autonomy loop's `<<ALL_DONE>>` sentinel may only fire once Phase 6's acceptance gate is met AND every meta-plan §10 verification criterion holds.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` first to update).
- `cargo test`, `cargo clippy --all-targets`, or `cargo test -p smelt-cli --test example_diagnostics` surfaces a pre-existing failure unrelated to the plan.
- Phase 6: an expert flags the same material finding on round 3 (per-expert bound), or two different experts flag the same systemic concern in the same round.
- The `/smelt-loop large` run cannot complete cleanly after three iterations of skill-diff application — that is a UX regression per meta-plan §7 and a stop-the-line condition.

**Conventions every phase:**

- Real-fixture coverage — every code-touching phase exercises its change in an existing example or in the `tests/agent-loop/fixtures/large/` fixture.
- Red-green TDD: failing test before any implementation in Phases 1 (LSP rename).
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope. In particular, no new `meta_language.md` Surface entries (all surface shipped through F); no record-field rename (deferred to Known Divergences per below); no new diagnostic codes; no performance refactors beyond what reviewers actually flag.
- Honor architectural invariants from `CLAUDE.md`: `crates/smelt-db/src/type_inference/` and `crates/smelt-types/src/signatures.rs` remain pure (no Salsa imports inside analysis logic); Salsa queries call pure inference functions, not the other way around. Rename logic in `crates/smelt-lsp/src/lib.rs` (or a new module) consumes Salsa-resolved spans but does not add new analysis logic.
- Timeless-oracle rule: spec and user-doc edits read as if the feature has always existed. Phase vocabulary lives in this plan only — never inside `docs/specs/` or `docs-site/docs/` body sections. The `lsp.md` Rename table row added in Phase 0 reads as a feature description, not a "Phase G adds…" note.

---

## Context

Phases A–F shipped the typed meta-programming surface: `List<T>` with literals and spread (A); HOFs `map`/`filter`/`reduce`, lambdas, pipe `|>`, contextual reducers, and `smelt.config.var` (B); narrow reflection `smelt.columns_of` + `ColumnRef` (C); wide reflection `smelt.models.*` / `smelt.sources.*` + `ModelRef`/`SourceRef` (D); records, `Map<K,V>`, and YAML/JSON loaders with schema validation (E1); multi-model production via `generates: models` and `ModelDef` (E2); and polish — multi-arg lambdas, parameterised reducers, meta-world ternary (F).

Phase G is the final-polish phase. No new feature surface lands; no new diagnostic codes; no new spec body. The deliverables are:

1. **LSP rename completeness** — the rename surface in `lsp.md` lists CTE names, `smelt.<path>` references, and column names. The meta-language constructs that *plausibly* warrant rename are lambda parameters (single-file, fully scoped — clear win) and record field names (cross-cutting and risky — explicit Known Divergence per below). This phase ships lambda-parameter rename and documents the record-field-rename gap.
2. **docs-site polish** — `docs-site/docs/meta-language/` has 14 pages shipped phase-by-phase (`index.md`, `lists.md`, `hofs.md`, `lambdas.md`, `pipes.md`, `reflection.md`, `records.md`, `maps.md`, `config-loaders.md`, `config-vars.md`, `generators.md`, `reducers.md`, `ternary.md`, `reference.md`). They were written incrementally; G is the cross-cutting polish pass — navigation order, cross-links between related constructs, alphabetical completeness of the reference page, examples-in-prose for the pages most likely to be read on their own (`index.md`, `generators.md`, `reflection.md`).
3. **`/smelt-loop` tier-3 clean run** — the `large` tier fixture (`tests/agent-loop/fixtures/large/`) already exists and is well-designed (`spec.md` exercises generator + YAML loader + downstream union). G runs the loop end-to-end at least once, addresses any TOOL_BUG / DOCS_GAP findings inline, and applies the SKILL_GAP diffs the reviewer surfaces.
4. **Smelt-app-builder skill update sweep** — a new dated reference doc at `.claude/skills/smelt-app-builder/references/20260517-meta-final.md` consolidating any non-obvious gotchas the loop run surfaces, plus the per-phase skill references stay in place. The skill body (SKILL.md) stays under 250 lines per `smelt-loop.md` constraint.
5. **Final consolidated drift report** — `/smelt:validate meta_language` and `/smelt:validate meta_config_loading` both return zero drift. The overall plan's Phase G row updates to `done`; PR #117 description gets a phase checklist showing all eight phases completed.

This is the **terminal phase**. After Phase 6's acceptance gate, the meta-plan §10 verification criteria must all hold; the autonomy loop emits `<<ALL_DONE>>` and exits.

## Scope

### In scope (spec coverage)

- `lsp.md` Rename table: new row for **lambda parameter** — within-file rename of the parameter name; scope = the lambda's body. The rename also updates downstream pipe-rewrites' lambdas if and only if the lambda's textual binder occurs only inside the same lambda body. Cross-lambda parameter shadowing is preserved (an inner `fn x =>` parameter `x` is renamed only within that inner lambda).
- `lsp.md` Known Divergences entry: **record-field rename is not supported**. Record types are structural and anonymous; renaming a field would have to propagate through every record-literal constructor and projection in scope, and across loader schema arguments. Tracked as a v2 enhancement. The current LSP gracefully reports prepare-rename as not supported when the cursor is on a record field name.
- Code shipped:
  - Lambda-parameter prepare-rename + rename request handling in `crates/smelt-lsp/src/lib.rs` (or extracted into a new `rename_lambda.rs` if reviewer pressure forces it — defer the split decision to implementer judgement at Phase 1).
  - Salsa-resolved lambda-parameter span set: the AST already exposes lambda parameter binders and their use-site occurrences (via the type-inference scope chain). Rename collects every span that resolves to the *same binder node ID* and emits one `TextEdit` per span. No new Salsa query is added; the rename handler consumes the existing parse + type-inference queries.
  - The rename handler rejects the rename if the new name is not a valid SQL identifier, collides with an enclosing-scope binder name (would shadow), or collides with a known meta-namespace keyword (`if`, `then`, `else`, `fn`, `let`).
- Docs-site cross-cutting polish:
  - `index.md` — adds a "How the pieces fit together" worked example flowing list → HOF → reducer → generator.
  - `reference.md` — full alphabetical sweep, one entry per shipped HOF / reflection function / loader / reducer; each entry has type signature + 1–3 line example.
  - Cross-link addition: every page that mentions another construct adds a single inline link the first time it appears (e.g. `lambdas.md`'s reference to `map` links to `hofs.md`; `generators.md`'s reference to `ModelDef` links to `records.md`).
  - Navigation order in `mkdocs.yml` (if used) follows: index → lists → hofs → lambdas → pipes → reducers → ternary → records → maps → reflection → config-loaders → config-vars → generators → reference.
- `/smelt-loop` integration:
  - Run `claude /smelt-loop --tier large --iterations 3` (or the manual recipe in `tests/agent-loop/harness/`) and capture `eval.json`, `retro.md`, `transcript.md` artifacts under `~/.smelt-test-runs/`.
  - Triage findings per `smelt-loop.md` §"Decide if the reviewer should run":
    - TOOL_BUG → file a separate fix-it commit on the branch if scope ≤ ~30 lines and clearly within the meta-language surface (e.g., a diagnostic message mismatch); otherwise log under "Deferred during implementation" with the run-dir path.
    - DOCS_GAP → fold into Phase 2 docs polish if the same page exists; otherwise log.
    - SKILL_GAP → apply the proposed `proposed_skill_diff.patch` after review.
  - Termination rule (from `smelt-loop.md` Stop conditions): three consecutive iterations pass cleanly = converged. If we hit converged in iteration 1, stop; the loop has nothing left to learn from this fixture.
- `smelt-app-builder` skill update:
  - `.claude/skills/smelt-app-builder/references/20260517-meta-final.md` — gotchas the tier-3 loop surfaced (concrete, scoped — only what the loop's review notes flagged). If the loop is clean and no diffs are proposed, write one short reference doc summarising the meta-language workflow ("read `smelt docs show meta-language/generators` before writing a `.gen.sql` file"; "`smelt.models.with_tag` cannot be used inside generator bodies — see `GeneratorBodyForbidsModelReflection`").

### Explicitly deferred

- **Record-field rename.** Risky and cross-cutting; record fields are anonymous-shape and would require propagating renames through every literal constructor, every projection, and every loader `schema` argument that uses a struct type. Tracked as a Known Divergence in `lsp.md`.
- **Map-key rename.** Map keys are values, not identifiers — rename is not the right primitive.
- **Generator-emitted-model rename.** Already covered by the existing `smelt.<path>` rename row. Verified at Phase 5 (loop run) — not re-shipped.
- **`zip_with` and other deferred HOFs from the meta-plan §3 ledger.** None forced by the loop's tier-3 fixture or any docs example.
- **Performance optimisation beyond what the loop or reviewers surface.** No proactive profiling pass.
- **Cross-feature spec touches not already landed.** The meta-plan §6 cross-feature table is closed by end of Phase F; G adds only the `lsp.md` Rename row.
- **`diagnostics.md` spec authoring.** `lsp.md` §"Diagnostic codes pre-`diagnostics.md`" still applies; not on the meta-plan critical path.
- **Find-references for lambda parameters.** Symmetric to rename; deferred unless trivially derivable from the same span set without new Salsa work — implementer judgement at Phase 1.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 0     | done     | 5ef1a6dd | 2026-05-17 |
| 1     | done     | c224ebde | 2026-05-17 |
| 2     | done     | 2cb3ca85 | 2026-05-17 |
| 3     | done     | 16477eb6 | 2026-05-17 |
| 4     | done     | 727b85d4 | 2026-05-17 |
| 5     | done     | a77de490 | 2026-05-17 |
| 6     | done     | 67d22a17 | 2026-05-17 |

---

## Phases

### Phase 0: Commit the `lsp.md` spec touch + this plan

**Goal.** Land the working-tree spec edit to `docs/specs/lsp.md` (new Rename row for lambda parameters; Known Divergence row for record-field rename) and this plan file as a single atomic commit that opens Phase G.

**Pre-conditions.** Working tree contains the `lsp.md` Rename-table edit (one new row) and Known Divergences edit (one new bullet). This plan file at `docs/plans/20260509-meta-language-G.md` is staged. No code changes in `crates/`. The overall plan's Phase G row still reads `pending`. `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all pass on `HEAD`.

**TDD tests to write first.** None — Phase 0 is the spec + plan commit. Code TDD starts in Phase 1.

**Implementation shape.** Verify the spec diff via `git diff docs/specs/lsp.md`; cross-check that the new Rename row reads as a timeless feature description (no `Phase G` vocabulary); cross-check that the Known Divergences bullet describes the gap in behavioural terms (`record-field rename is not supported because…`) and not in plan terms.

**Critical files (allowed to touch in this phase).**
- `docs/specs/lsp.md` — Phase G spec increment (Rename row + Known Divergence row).
- `docs/plans/20260509-meta-language-G.md` — this plan.

**Docs touched.** *Spec-only — the user-facing rename docs (`docs-site/docs/guide/editor-features.md`) are touched in Phase 1 once the implementation lands.*

- `docs/specs/lsp.md` — new Rename row (lambda parameter) + new Known Divergence bullet (record-field rename).

**Review checklist** (material findings only):
- [ ] `lsp.md` Rename row added; reads as feature description (no plan vocabulary)
- [ ] `lsp.md` Known Divergences row added; describes behaviour, not plan phase
- [ ] No body edits to `meta_language.md` or `meta_config_loading.md`
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` all pass
- [ ] Plan file structure matches plan template

**Commit.** `feat(meta-language-G): scaffold Phase G — lsp.md spec touch + plan`

---

### Phase 1: LSP lambda-parameter rename (prepare + rename)

**Goal.** Implement prepare-rename and rename for lambda-parameter binders in the LSP. The handler resolves every span that references the same lambda parameter binder (the binder itself + every use inside the lambda body), validates the new name, and returns a single-file `WorkspaceEdit`.

**Pre-conditions.** Phase 0 commit landed. `lsp.md` Rename table includes the lambda-parameter row.

**TDD tests to write first.** *Listed verbatim — write these as failing tests before any implementation:*
- `crates/smelt-lsp/tests/rename_lambda.rs::rename_lambda_param_single_use` — open a workspace with a model using `xs |> map(fn x => x + 1)`, prepare-rename on the `x` binder, rename to `n`, assert the `WorkspaceEdit` contains exactly two `TextEdit`s for the same URI covering the binder and the use site; assert the resulting file text is `xs |> map(fn n => n + 1)`.
- `crates/smelt-lsp/tests/rename_lambda.rs::rename_lambda_param_multiple_uses` — model with `xs |> map(fn x => x * x + 1)`, rename `x` → `y`; assert all three spans are renamed; resulting file has `fn y => y * y + 1`.
- `crates/smelt-lsp/tests/rename_lambda.rs::rename_lambda_param_inner_shadowing_preserved` — model with `xs |> map(fn x => smelt.models.with_tag('cohort') |> map(fn x => x.path))`; renaming the outer `x` only renames the outer binder (the inner `x` parameter and its body usage stay `x`). Assert exactly one edit for the outer binder + zero edits for the inner lambda body.
- `crates/smelt-lsp/tests/rename_lambda.rs::rename_lambda_param_rejects_invalid_identifier` — rename `x` → `1abc`; assert the response is an error with message indicating invalid identifier.
- `crates/smelt-lsp/tests/rename_lambda.rs::rename_lambda_param_rejects_keyword_collision` — rename `x` → `if`; assert the response is an error with message indicating a reserved keyword.
- `crates/smelt-lsp/tests/rename_lambda.rs::rename_lambda_param_rejects_shadowing_outer_binder` — model with nested lambdas where renaming the inner parameter would shadow an outer one already referenced in the inner body; assert error.
- `crates/smelt-lsp/tests/rename_lambda.rs::rename_lambda_param_multi_arg` — model with `fn (a, b) => a + b`, rename `a` → `x`; assert only the `a` spans (binder + body use) are renamed and `b` is untouched.
- `crates/smelt-lsp/tests/rename_lambda.rs::prepare_rename_lambda_param_returns_range` — prepare-rename on a lambda parameter binder; assert it returns the binder's text range and the current placeholder name.
- `crates/smelt-lsp/tests/rename_lambda.rs::prepare_rename_on_non_lambda_target_does_not_match` — prepare-rename on a non-binder token in the lambda body (e.g. an operator) returns `None`; existing rename targets (CTE, `smelt.<path>`, column) continue to work via their existing handlers.

**Implementation shape.** Extend the existing rename handler in `crates/smelt-lsp/src/lib.rs` to dispatch on a new `RenameTarget::LambdaParam` variant. The dispatcher uses the AST + type-inference scope chain (which already knows every binder and every binder-resolving use site) to gather spans. The new-name validator is a small pure helper (reuse the existing identifier-validity check; add the meta-keyword set check). If the rename target is a record field, the handler returns the not-supported response described in the Known Divergence.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-lsp/src/lib.rs` (or `crates/smelt-lsp/src/rename_lambda.rs` if the implementer splits) — rename handler dispatch + lambda-param rename impl.
- `crates/smelt-lsp/tests/rename_lambda.rs` — the new test file.
- `docs-site/docs/guide/editor-features.md` — one-paragraph update under "Rename" listing lambda parameters.

**Docs touched.**
- `docs-site/docs/guide/editor-features.md` — describes lambda-parameter rename as a feature ("renaming a lambda parameter updates the binder and every use within the lambda's body").

**Review checklist** (material findings only):
- [ ] All nine TDD tests listed exist and assert what's specified
- [ ] `lsp.md` Rename row's behavioural rules (shadowing, keyword collision, invalid identifier) are honoured
- [ ] No new Salsa query was added (spans gathered via existing AST + scope chain)
- [ ] Pure-function-rule respected — no Salsa imports added to `type_inference.rs`
- [ ] Record-field rename returns the not-supported response per Known Divergence; no silent partial implementation
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green
- [ ] User-doc edit is timeless — no phase labels

**Commit.** `feat(meta-language-G): LSP rename for lambda parameters`

---

### Phase 2: Docs-site polish — cross-cutting sweep

**Goal.** Polish `docs-site/docs/meta-language/` end-to-end: navigation order, cross-links, alphabetical reference page, examples-in-prose on the high-traffic pages (`index.md`, `generators.md`, `reflection.md`).

**Pre-conditions.** Phase 1 landed. Docs-site builds (`mkdocs build` or whichever generator is configured) without warnings on `HEAD`.

**TDD tests to write first.** Docs-only — no test code. Coverage is verified by `/smelt:validate meta_language` returning zero drift on the Surface ↔ docs-site axis at Phase 5. Sanity-check via:
- `cd docs-site && mkdocs build --strict` (or equivalent) reports no warnings.
- Every shipped HOF, reflection function, loader, reducer, and ternary keyword appears in `reference.md` exactly once.

**Implementation shape.**

- **`index.md`** — add a "How the pieces fit together" section at the bottom: a single worked example that reads `cohorts.yaml`, maps to `ModelDef`s, and references the result via `smelt.models.with_tag` in a downstream union. ~30 lines of prose + code. Links each construct to its dedicated page.
- **`reference.md`** — full alphabetical sweep. Inventory each shipped construct from the spec's Surface and confirm the reference page has a one-entry-per-construct row with signature + 1–3-line example. Add missing entries; fix non-alphabetical ordering; remove duplicates.
- **Cross-link sweep** — read each of the 14 pages and add inline links the first time another construct is mentioned. Discipline: at most one link per construct per page (avoid link soup).
- **Navigation** — update `mkdocs.yml` (or whatever drives the sidebar) so the meta-language section reads in the order listed under §Scope above. If `mkdocs.yml` already lists the section, just reorder. Do not introduce new files.
- **`generators.md`** and **`reflection.md`** — these are the pages most likely to be read on their own. Ensure each opens with a 3–5-sentence framing of the *use case* (not just the surface), and that the first code block in each is a complete, runnable example.

**Critical files (allowed to touch in this phase).**
- `docs-site/docs/meta-language/index.md`
- `docs-site/docs/meta-language/reference.md`
- `docs-site/docs/meta-language/generators.md`
- `docs-site/docs/meta-language/reflection.md`
- `docs-site/docs/meta-language/*.md` (one-link-per-construct cross-link sweep across the remaining 10 pages)
- `docs-site/mkdocs.yml` (or equivalent) — navigation order
- *Not* touched: `docs/specs/meta_language.md`, `docs/specs/meta_config_loading.md`, `docs/specs/lsp.md` body sections (spec ↔ docs alignment is verified at Phase 5, not changed here).

**Docs touched.** This phase *is* the docs touch.

**Review checklist** (material findings only):
- [ ] `index.md` has a "How the pieces fit together" worked example linking each major construct
- [ ] `reference.md` is alphabetical, complete (one entry per shipped construct), and every entry has a type signature
- [ ] Cross-links added on first mention only; no link soup
- [ ] Navigation order in `mkdocs.yml` matches the Scope listing
- [ ] `generators.md` and `reflection.md` open with use-case framing and a runnable example
- [ ] No phase labels or plan vocabulary anywhere in docs-site
- [ ] `mkdocs build --strict` (or equivalent) passes

**Commit.** `docs(meta-language-G): docs-site polish — nav, cross-links, reference completeness`

---

### Phase 3: `/smelt-loop` tier-3 fixture verification + medium-tier expansion

**Goal.** Verify the existing `tests/agent-loop/fixtures/large/` fixture is sound for a clean tier-3 run; add the medium-tier expansion fixtures referenced in meta-plan §4 (Phase A–B sized asks) that have not yet landed.

**Pre-conditions.** Phases 0–2 landed. `tests/agent-loop/harness/setup_run.sh` and `tests/agent-loop/harness/eval.sh` operational on `HEAD`. `tests/agent-loop/fixtures/medium/spec.md` and `validate.py` exist.

**TDD tests to write first.** None — this phase is fixture verification + addition. The acceptance gate is "harness setup + smoke checks pass for tier large" and "the medium-tier additions parse and validate" at Phase 4.

**Implementation shape.**

1. **Tier-3 fixture audit.** Read `tests/agent-loop/fixtures/large/{spec.md,validate.py}` end-to-end. Confirm:
   - `spec.md` is consistent with shipped meta-language surface (generators, `smelt.config.load_yaml`, `ModelDef`, `smelt.models.with_tag` optional bonus path).
   - `validate.py` checks all the acceptance gates listed in `.claude/commands/smelt-loop.md` §"Tier overview" (9-row union, per-country counts, generator-file frontmatter exists, `configs/` exists).
   - Seed CSV `seeds/raw_orders.csv` has the documented 12 rows.
   - Any inconsistency → fix in-place; no new files.
2. **Medium-tier expansion.** Read `tests/agent-loop/fixtures/medium/spec.md`. If the spec does not already include a Phase A–B asks subsection (e.g., "use `[a, b, c]` for a dimension list"; "express this CASE chain via `fn` and `reduce(or_any)`"), append one. Update `validate.py` if and only if the new asks change the acceptance gate; otherwise leave validation alone. Discipline: do not add asks that the existing `validate.py` cannot acceptance-gate.
3. **Smoke test the harness.** Run `bash tests/agent-loop/harness/setup_run.sh large local /tmp/smelt-loop-smoke-G`. Confirm setup succeeds; do *not* run the build subagent in this phase.

**Critical files (allowed to touch in this phase).**
- `tests/agent-loop/fixtures/large/spec.md` — small consistency edits only.
- `tests/agent-loop/fixtures/large/validate.py` — only if the audit surfaces a gate mismatch.
- `tests/agent-loop/fixtures/medium/spec.md` — Phase A–B asks subsection if not already present.
- `tests/agent-loop/fixtures/medium/validate.py` — only if the new asks change the gate.
- *Not* touched: anything in `crates/`, `docs/specs/`, `docs-site/`.

**Docs touched.** None — fixture surface is not user-facing.

**Review checklist** (material findings only):
- [ ] `large/spec.md` is consistent with shipped surface (generators, loaders, `ModelDef`, optional `with_tag` path)
- [ ] `large/validate.py` acceptance gates match `smelt-loop.md` §"Tier overview"
- [ ] If `medium/` got new asks, they have a corresponding gate in `validate.py` (no untestable asks)
- [ ] Harness smoke test (`setup_run.sh large local`) succeeds
- [ ] No code/spec changes leaked into this phase

**Commit.** `chore(meta-language-G): /smelt-loop tier-3 fixture audit + medium-tier expansion`

---

### Phase 4: `/smelt-loop` tier-3 clean run + skill diffs

**Goal.** Run the tier-3 loop end-to-end at least once, address any TOOL_BUG / DOCS_GAP findings inline (within the bounds in §Scope), and apply SKILL_GAP diffs the reviewer surfaces.

**Pre-conditions.** Phases 0–3 landed. Harness smoke test green. `~/.smelt-test-runs/` writable.

**TDD tests to write first.** None — this phase is a meta-test (we are running the loop, not writing one). The acceptance gate is "tier-3 loop terminates per `smelt-loop.md` Stop conditions, all TOOL_BUG / DOCS_GAP findings logged or fixed, all applied SKILL_GAP diffs reviewed and committed".

**Implementation shape.**

1. **Run the loop.** Invoke `claude /smelt-loop --tier large --iterations 3` (or follow the per-iteration recipe in `smelt-loop.md` step-by-step). Capture each iteration's `run_dir` under `~/.smelt-test-runs/`.
2. **Triage per iteration.**
   - For each iteration's `retro.md`, `eval.json`, and reviewer-emitted `review_notes.md`:
     - **TOOL_BUG**: if scope ≤ ~30 lines and clearly within meta-language surface (e.g., a diagnostic-message typo, an off-by-one frame-stack range), file a separate fix-it commit on the branch — see "Fix-it commits" below. Otherwise log under "Deferred during implementation".
     - **DOCS_GAP**: if the gap is on a `docs-site/docs/meta-language/` page already polished in Phase 2, fold the fix into a new docs commit (do NOT amend Phase 2's commit). Otherwise log.
     - **SKILL_GAP**: review `proposed_skill_diff.patch`; apply via `git apply` if the diff is sound; reject and log if not. The reviewer's notes explain *why* each diff hunk is justified.
3. **Skill update.** After all iterations complete, write `.claude/skills/smelt-app-builder/references/20260517-meta-final.md`:
   - If the loop ran clean and no SKILL_GAP diffs were proposed: write a short reference doc summarising meta-language workflow gotchas (`smelt docs show meta-language/generators` first; `smelt.models.with_tag` cannot be used inside generator bodies; etc.).
   - If diffs were applied: the reference doc summarises *what changed and why* (concrete; not a marketing summary).
4. **Termination check.** Per `smelt-loop.md` Stop conditions, the loop converges when three consecutive iterations pass cleanly OR three skill diffs are rejected in a row. If neither holds after 3 iterations, log the situation under "Deferred during implementation" and *do not loop further in this plan execution* — the goal is a clean run, not exhaustive coverage.
5. **Fix-it commits.** Any TOOL_BUG fix is its own commit (e.g., `fix(meta-language-G): {bug summary}`); skill-diff applications are committed as `chore(meta-language-G): apply /smelt-loop skill diff iter-{N}` per iteration's diff. The summary at this phase's end (the Phase 4 commit listed below) is the **final** commit that lands the new reference doc and consolidates the loop's outcome.

**Critical files (allowed to touch in this phase).**
- `.claude/skills/smelt-app-builder/SKILL.md` — only if a SKILL_GAP diff was reviewer-approved and applied.
- `.claude/skills/smelt-app-builder/references/20260517-meta-final.md` — new reference doc summarising the loop outcome.
- `docs-site/docs/meta-language/*.md` — only if a DOCS_GAP fix is in-scope.
- `crates/**` — only for an in-scope TOOL_BUG fix (rare; commit separately).
- `tests/agent-loop/fixtures/large/*` — generally untouched; if the loop surfaces a fixture issue, log it.

**Docs touched.** Only if a DOCS_GAP fix is in-scope.

**Review checklist** (material findings only):
- [ ] `/smelt-loop large` ran ≥ 1 iteration; transcript + retro + eval artifacts present under `~/.smelt-test-runs/`
- [ ] Every TOOL_BUG / DOCS_GAP / SKILL_GAP finding is either fixed (with a justified commit) or logged under "Deferred during implementation"
- [ ] `.claude/skills/smelt-app-builder/references/20260517-meta-final.md` exists and reflects the loop outcome
- [ ] `SKILL.md` is still under 250 lines (per `smelt-loop.md` constraint)
- [ ] No silent code edits outside the documented critical files

**Commit.** `chore(meta-language-G): /smelt-loop tier-3 run + skill diffs`

(Any in-scope TOOL_BUG fixes are separate `fix(meta-language-G): {summary}` commits *before* this consolidating one.)

---

### Phase 5: Final validation + overall plan close-out

**Goal.** Run `/smelt:validate meta_language` and `/smelt:validate meta_config_loading` to confirm zero drift; update `docs/plans/20260509-meta-language-overall.md`'s Phase G row to `done`; ensure PR #117's description has the phase checklist updated.

**Pre-conditions.** Phases 0–4 landed. `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all green.

**TDD tests to write first.** None — validation is the test.

**Implementation shape.**

1. **`/smelt:validate meta_language`** — run and capture output. Drift = anything the spec asserts that the implementation/docs/examples do not reflect, or vice versa. Zero drift required. If the report surfaces actionable items:
   - In-scope spec/doc fix (≤ ~30 lines) → fix in-place; commit `chore(meta-language-G): /smelt:validate drift fix — {summary}`; re-run validate.
   - Larger fix → log under "Deferred during implementation" and emit `<<PAUSE_FOR_HUMAN>>` per stop-the-line.
2. **`/smelt:validate meta_config_loading`** — same protocol.
3. **Overall plan row update.** Set Phase G to `done` in `docs/plans/20260509-meta-language-overall.md`, fill `Date` and `Commit` (the commit hash of this phase's commit will go in at push time). Same edit applied to the meta-plan §3 phase table if it diverged.
4. **PR description update.** Run `gh pr edit 117 --body "$(...)"` (or equivalent) to update the description's phase checklist: all eight phases A–G show as completed. Mention the killer demo (`examples/per_cohort_union/`) and the `/smelt-loop` tier-3 clean run.

**Critical files (allowed to touch in this phase).**
- `docs/plans/20260509-meta-language-overall.md` — Phase G row to `done` + commit hash.
- Any spec/doc that `/smelt:validate` surfaces drift on, scoped to ≤ ~30 lines per surface.
- PR description (via `gh pr edit` — no in-repo file edit).

**Docs touched.** Only if validate surfaces drift.

**Review checklist** (material findings only):
- [ ] `/smelt:validate meta_language` reports zero drift
- [ ] `/smelt:validate meta_config_loading` reports zero drift
- [ ] Overall plan's Phase G row updated to `done` with date + commit
- [ ] PR #117 description has phase checklist showing all eight phases done
- [ ] No new pending drift logged in "Deferred during implementation" without an explicit reason

**Commit.** `chore(meta-language-G): close Phase G — validate zero drift + plan row done`

---

### Phase 6: Expert reviewer dispatch loop

**Goal.** Run each Phase G applicable expert reviewer from meta-plan §5 over the Phase G diff, address material findings, and re-dispatch each expert until it reports clean — or escalate via stop-the-line per the bounds below.

**Pre-conditions.** Phases 0–5 complete and committed. Working tree clean. `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `cargo test -p smelt-cli --test example_diagnostics` all pass.

**Experts to dispatch (Phase G subset of meta-plan §5).**

| Expert | Model | Scope (file allowlist) | What to verify |
|---|---|---|---|
| **lsp-expert** | Sonnet | `crates/smelt-lsp/src/lib.rs` + new LSP code paths (e.g., `crates/smelt-lsp/src/rename_lambda.rs` if extracted) + `crates/smelt-lsp/tests/rename_lambda.rs` | Lambda-parameter rename correctness: spans gathered are complete and tight (no over-broad edits); prepare-rename returns the right range; rejection rules (invalid identifier, keyword collision, shadowing) honour `lsp.md`; record-field rename gracefully reports not-supported; no regressions in existing rename targets (CTE, `smelt.<path>`, column). |
| **salsa-expert** | Sonnet | `crates/smelt-db/src/lib.rs` (Salsa queries touched indirectly by the rename handler) + `crates/smelt-lsp/src/lib.rs` rename dispatch | Confirm no new Salsa query was added; confirm rename handler does not bypass Salsa caching (re-uses existing parse + type-inference queries); confirm no accidental O(workspace) scan on a single-file rename. |
| **examples-curator** | Haiku | `tests/agent-loop/fixtures/large/`, `tests/agent-loop/fixtures/medium/`, `examples/per_cohort_union/` (verify still passing) | Tier-3 fixture is minimal-but-realistic; happy path + at least one edge case exercised; the killer demo (`examples/per_cohort_union/`) still passes `example_diagnostics`. |
| **docs-reviewer** | Haiku | `docs-site/docs/meta-language/` deltas; `docs-site/docs/guide/editor-features.md` rename paragraph | Cross-link sweep clean (one link per construct per page); `reference.md` alphabetical + complete; `index.md` "How the pieces fit together" worked example sound; no phase labels anywhere; `editor-features.md` lambda-rename paragraph matches `lsp.md` Rename row. |

**Loop discipline.**

1. **Round 1.** Dispatch all four experts in parallel — single message, multiple Agent tool calls. Each prompt MUST include:
   - The phase plan path (`docs/plans/20260509-meta-language-G.md`) and the spec sections that are the oracle (`docs/specs/meta_language.md`, `docs/specs/meta_config_loading.md`, `docs/specs/lsp.md` §Rename).
   - The exact file scope from the table above.
   - The diff range to review (`git log --oneline <phase-G-base>..HEAD` — phase-G-base is the commit immediately before `feat(meta-language-G): scaffold Phase G`).
   - Explicit instruction: report only **material** findings (correctness, spec drift, architectural-invariant breaks). Skip nits and stylistic preferences.
   - Output format: numbered list of findings with file:line refs, or "no material findings".
2. **Address findings.** For each expert that returns material findings:
   - Mechanical fix (≤ ~30 lines, single concern) → edit directly.
   - Non-trivial fix → dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist, with the expert's findings as input. Do NOT widen scope into earlier phases or post-plan items.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` after each fix batch.
   - Commit per expert: `review(meta-language-G): address {expert-name} feedback`. Push immediately.
3. **Re-dispatch** only the expert whose findings were addressed, providing the round-1 prompt plus a diff of what changed. "No material findings" → that expert is **clean** and exits the loop.
4. **Repeat** step 2 → step 3 until every expert is clean.
5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason) and stop the autonomy loop if any of:
   - Same expert flags a material finding on round 3 (per-expert bound).
   - Two **different** experts flag the same systemic concern in the same round (per meta-plan §7).
   - An expert's findings would force a non-trivial spec change — pause for the user.
   - A fix surfaces a pre-existing failure unrelated to this phase.

**Critical files (allowed to touch in this phase).** Anything within an expert's scope per the table above, plus the phase plan file (to record round counts).

**Review checklist** (material findings only — applied to the expert-dispatch *process*):

- [ ] All four applicable experts were dispatched at least once.
- [ ] Every material finding was either fixed or escalated; none silently dropped.
- [ ] Round count per expert recorded under "Deferred during implementation".
- [ ] No fix touched files outside the dispatching expert's scope.
- [ ] No expert ran more than 3 rounds; if any did, autonomy loop emitted `<<PAUSE_FOR_HUMAN>>`.
- [ ] All cargo checks green at end of phase.
- [ ] `/smelt:validate meta_language` and `/smelt:validate meta_config_loading` still report zero drift.

**Acceptance gate.** Append a one-line summary to "Deferred during implementation":

> Phase 6 expert review: lsp-expert clean (R{n}), salsa-expert clean (R{n}), examples-curator clean (R{n}), docs-reviewer clean (R{n}). No stop-the-line fired.

**Commit(s).** Per round, per expert with findings: `review(meta-language-G): address {expert-name} feedback`. If round 1 came back clean, no commit for that expert. The acceptance-gate summary lands in the next commit (or in `chore(meta-language-G): record Phase 6 review summary` if no other phase-6 commits were made).

After Phase 6's acceptance gate is met, all eight phases A–G are `done`. The meta-plan §10 verification criteria must hold; the autonomy loop emits `<<ALL_DONE>>` and exits.

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **Phase 3 audit outcome (2026-05-17):** No fixture edits required. `large/spec.md`, `large/validate.py`, and `large/seeds/raw_orders.csv` (13 lines = header + 12 rows, 3 shipped + 1 cancelled per country) are consistent with both the shipped meta-language surface and `.claude/commands/smelt-loop.md` §"Tier overview" (union table, per-country counts, generator frontmatter, configs/ YAML). `medium/spec.md` already includes Phase A meta-list lift, Phase B HOF + `smelt.config.var`, Phase C column reflection, Phase D wide reflection, and records/maps/loader extensions at lines 105–168; all diagnostic codes referenced match the spec's diagnostic tables. Harness smoke test `setup_run.sh large local /tmp/smelt-loop-smoke-G-<ts>` succeeded (wheel built, smelt 0.3.2 installed, `smelt --help` and `smelt docs list` smoke checks both passed). All four cargo CI gates green.

- **Phase 4 outcome (2026-05-17):** Iteration 1 of `/smelt-loop` tier-3 ran clean (7/7 acceptance checks passed first try). Run dir: `~/.smelt-test-runs/loop-1-20260517-132128/`. Build agent surfaced substantive retro signal → reviewer ran, classified 3 SKILL_GAP / 2 DOCS_GAP / 2 TOOL_BUG findings. Skill diff (+16 lines, bringing SKILL.md to exactly 250 lines — the hard cap) applied via `git apply`. Per `smelt-loop.md` stop conditions and this plan's §Scope ("the goal is a clean run, not exhaustive coverage"), one clean iteration is sufficient; no further iterations run in this plan execution. Per-finding follow-up work for the user (TOOL_BUG and DOCS_GAP items the reviewer surfaced, see `~/.smelt-test-runs/loop-1-20260517-132128/review_notes.md`):
  - **TOOL_BUG:** `smelt test` silently ignores `materialization: test` files with a boolean-SELECT body (no "discovered but skipped" diagnostic). Reproducer in review_notes.md §1.
  - **TOOL_BUG:** `smelt build --show-plan <generator>.sql` is opaque — prints only the top-level `load_yaml` AST node, does not list emitted ModelDef paths. Reproducer in review_notes.md §2.
  - **DOCS_GAP:** `docs-site/docs/guide/testing.md` does not document the `materialization: test` boolean-assertion form (the spec fixture's shape); either document it or steer users toward the `test:`/`expect:` block. Folded into the same draft docs PR a note that `.sql` discovery under `paths:` is subdirectory-agnostic.
  - Reference doc landed: `.claude/skills/smelt-app-builder/references/20260517-meta-final.md` summarising tier-3 workflow gotchas (generator emitted-model naming `cohorts.us` → `cohorts_us`; two coexisting test layouts with silent-skip; `tests/` as a scanned directory). Cross-links to the per-phase reference docs.

- **Phase 5 outcome (2026-05-17):** `/smelt:validate meta_language` and `/smelt:validate meta_config_loading` both returned zero drift. Surface drift: all diagnostic codes present in code; user docs at `docs-site/docs/meta-language/` cover every spec-listed construct with type signatures and examples. Semantics drift: all normative rules covered by referenced tests. Invariant drift: all verifiable invariants upheld. Timeless-oracle drift: zero — no phase vocabulary in spec body or user docs. Freshness: `meta_language.md` last_reviewed 2026-05-16; most recent relevant code change 2026-05-17 (lambda-rename in LSP, scope covered by `lsp.md` not `meta_language.md` — not stale). `meta_config_loading.md` last_reviewed 2026-05-14; most recent loader code change 2026-05-16 (E2 generator pipeline, only touched loader.rs to remove a phase-vocabulary comment and make one function `pub` — not a behavioral change). No fixes applied; no drift escalated. Overall plan's Phase G row updated to `done`.

- **Phase 6 expert review (2026-05-17):** lsp-expert clean (R2), salsa-expert clean (R1), examples-curator clean (R1), docs-reviewer clean (R2). No stop-the-line fired. Round 1 lsp-expert findings (4): sibling-parameter collision in multi-arg lambdas, `?` propagating from in-loop `lambda_param_binder_range` call, `prepareRename` returning binder span instead of cursor-token span, and missing body-use cursor tests — all addressed in `a6d9fd29 review(meta-language-G): address lsp-expert feedback` (+49/-15 in `crates/smelt-lsp/src/rename_lambda.rs`, +85 in `crates/smelt-lsp/tests/rename_lambda.rs`). Round 1 docs-reviewer finding (1): missing "must not shadow an outer binder" constraint in `editor-features.md` lambda-rename paragraph — addressed in `40091ba8 review(meta-language-G): address docs-reviewer feedback` (one-line spec-alignment edit). Round 2 verified all fixes resolved; no new findings from any expert.

## Verification

How to confirm the spec is satisfied at the end (meta-plan §10):

- All phases A–G in `docs/plans/20260509-meta-language-overall.md` show `done`.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all pass.
- `/smelt:validate meta_language` reports zero drift.
- `/smelt:validate meta_config_loading` reports zero drift.
- `examples/per_cohort_union/` builds and passes its acceptance tests.
- `examples/staging_from_sources/` builds and passes.
- Every Phase A–F example passes `example_diagnostics`.
- `/smelt-loop` `large` tier completed a clean run (or three consecutive iterations pass per its termination rule).
- User docs at `docs-site/docs/meta-language/` cover every shipped HOF, every reflection function, every loader, with type signatures and small examples.
- `smelt-app-builder` skill includes the meta-language reference docs from each phase (`20260509-meta-lists.md` … `20260517-meta-final.md`) and the body survives the `/smelt-loop` test.
- LSP features for the meta-language: hover, goto-def into lambda binders + generators, completion in lambda body, diagnostics with frame stacks, rename for lambda parameters (record-field rename gracefully reported as not-supported).
- PR #117 description has phase checklist showing all eight phases A–G done; PR title is `feat: typed meta-programming`.
