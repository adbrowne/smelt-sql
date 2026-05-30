# Plan: directory-level CLAUDE.md — rules at root, gotchas per crate

**Date**: 2026-05-30
**Spec**: [`docs/specs/architecture.md`](../specs/architecture.md)
**Spec diff**: working tree edits to architecture.md happen *as part of* Phase 1 (consolidating each rule's authoritative text into the spec where it isn't already). No new normative behaviour — the rules' semantics are unchanged.
**Tracking PR / branch**: branch `worktree-unknown_types`.
**Docs**: docs-only — no code changes. (Acceptable to keep the standard `code+docs` header since spec edits are docs.)

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/architecture.md` §Semantics (lines 261-339 currently — the parity rules and the Diagnostic range encoding rule). Read the current `CLAUDE.md` sections on the same five rules (lines 190-276 currently). These are the inputs.
2. Confirm you are on branch `worktree-unknown_types`. If not, ask before continuing.
3. Find the next `pending` phase in the Progress table. If all are `done`, run Verification and stop.

**Per-phase loop (`/smelt:implement`):** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**
- The CLAUDE.md and architecture.md versions of a rule diverge in load-bearing ways (a fact / DON'T / CI gate in one but not the other) that Phase 1 cannot mechanically reconcile.
- A per-crate CLAUDE.md (Phase 3) ends up empty or near-empty — that's a signal NOT to add one; flag and skip rather than write a placeholder.
- Anyone questions whether a parity rule should be moved fully to a per-crate CLAUDE.md (it shouldn't — see "Why root, not per-crate, for rules" in Context below).

**Conventions every phase:**
- This plan is docs-only — no code edits anywhere. Build / test gates exist as regression baselines, not as work-in-progress signals.
- Atomic per-phase commit using the `Commit.` line verbatim; push after each.
- Honor `CLAUDE.md` invariants — note especially the Timeless-oracle rule.

---

## Context

The five architectural invariants (Salsa purity, Workspace loading parity, Project isolation, Run pipeline parity, Diagnostic range encoding) currently live in two places: their authoritative spec form in `docs/specs/architecture.md` §Semantics, and a hand-maintained mirror in `CLAUDE.md` (about 80 lines, between the §Architecture intro and §Key Dependencies). The mirror is load-bearing because Claude Code reads `CLAUDE.md` automatically on every session and does not read `docs/specs/` by default.

Two failure modes today:
- **Drift**: the mirror and the spec must be kept in sync by hand. Past plans have updated one without updating the other. The two-source-of-truth structure is the underlying problem.
- **Scope mismatch**: most tasks touch one crate; the rules apply across crates. The bulk of `CLAUDE.md` rule content is read on every prompt regardless of whether the task could possibly trigger any of the rules.

Claude Code's directory-level `CLAUDE.md` loading mitigates the second problem for *crate-specific* content (build steps, test idioms, internal file layout): a `crates/smelt-db/CLAUDE.md` is loaded only when Claude is working in that subtree. The mechanism is already in use at `docs/CLAUDE.md` to scope the docs directory to documentation-only tasks.

**Why root, not per-crate, for rules.** Each parity rule *deliberately spans crates*. Workspace Loading Parity binds `smelt-core`, `smelt-db`, `smelt-cli`, `smelt-lsp`. Run Pipeline Parity binds `smelt-runtime`, `smelt-cli`, `smelt-ui`. Diagnostic Range Encoding touches six crates. A rule housed in `crates/smelt-db/CLAUDE.md` does not load when Claude is editing `smelt-lsp/Backend::initialize` — but that edit is exactly the kind of change the rule is meant to prevent. The cross-crate refactors that have happened on this branch (the byte-offset plan's Phase 2 touched 44 files across 6 crates) confirm the work shape. Localising parity rules would defeat them.

Crate-specific *non-rule* content (build invocations, test idioms, file-layout guidance, gotchas like "smelt-db: Salsa macro expansions are large; prefer `rg` over full-file reads") localises cleanly and benefits from on-demand loading.

## Scope

### In scope
- Consolidate every rule's authoritative text into `docs/specs/architecture.md` §Semantics. Any wording present in `CLAUDE.md` and missing from the spec gets ported up.
- Replace the five §"… Rule" sections in root `CLAUDE.md` with a compact §"Architectural invariants" section: one-line summary per rule + link to the spec section.
- Add per-crate `CLAUDE.md` files **only where there is real gotcha content** — do not write placeholder files for every crate.

### Explicitly out of scope
- The rules' *semantics*. No DO / DON'T / CI gate is added, dropped, or weakened.
- Moving the rules to a different position within `CLAUDE.md` (the earlier "further down vs further up" question is resolved by this restructure — the §Architectural invariants section is compact enough that position becomes less important).
- Per-crate `CLAUDE.md` files for crates that don't have crate-specific gotchas worth writing. Phase 3 explicitly allows skipping crates.
- Any code change. This is docs-only.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | pending  |        |      |
| 2     | pending  |        |      |
| 3     | pending  |        |      |

---

### Phase 1: Audit and consolidate rule text into `architecture.md`

**Goal.** For each of the five rules (Salsa Purity / Workspace Loading Parity / Project Isolation / Run Pipeline Parity / Diagnostic Range Encoding), compare the wording in root `CLAUDE.md` against the corresponding §Semantics section in `docs/specs/architecture.md`. Any factual content present in `CLAUDE.md` but missing from the spec is ported into the spec (in timeless terms). After this phase, the spec is the *single authoritative source* for every rule's normative text; the `CLAUDE.md` mirror is a strict subset of the spec.

**Pre-conditions.** None.

**TDD tests to write first.** This phase is docs-only; the "tests" are inspection gates:
1. For each rule, side-by-side diff between the two locations. Produce a brief audit note in the plan's "Deferred during implementation" section listing what was ported. (If something is in the spec but NOT in `CLAUDE.md`, that's fine — the spec is allowed to be richer than the mirror.)
2. Regression: `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test --quiet 2>&1 | grep -E "test result|failed" | tail -50` all green (no code touched).

**Implementation shape.**
- For each rule: copy the `CLAUDE.md` section's text into a temp scratch; copy the matching spec section's text; diff manually; identify the union; rewrite the spec section to include the union in timeless phrasing.
- The `CLAUDE.md` versions today include things like "DO" / "DON'T" practice lists, examples, and pointers to CI gates that may or may not appear in the spec. Port everything load-bearing up; drop nothing.
- Verify each rule's spec section ends with the standing CI gate (`cargo test -p ...`) and the cross-references it had in `CLAUDE.md`.
- Do not touch `CLAUDE.md` in this phase — Phase 2 handles the replacement.

**Critical files (allowed to touch in this phase).**
- `docs/specs/architecture.md` — port wording up.

**Docs touched.** `docs/specs/architecture.md` only.

**Review checklist (material findings only):**
- [ ] Every fact in `CLAUDE.md`'s rule sections is now in the spec (port-up complete).
- [ ] Spec edits are timeless (no `Phase X`, no "added in 2026-05-30", no plan history).
- [ ] No rule's semantics changed; the audit ported wording but didn't add new DO / DON'T entries.
- [ ] Regression gates green.

**Commit.** `docs(arch): consolidate parity-rule wording in architecture.md ahead of CLAUDE.md restructure`

---

### Phase 2: Replace `CLAUDE.md` rule sections with one-line pointers

**Goal.** The five §"… Rule" sections inside `CLAUDE.md` §Architecture are deleted. A new compact §"Architectural invariants" section is added (positioned to be discoverable — see Implementation shape) containing one bullet per rule: a one-line summary + a link to the rule's section in `docs/specs/architecture.md`. The §Architecture section becomes a clean descriptive overview (High-Level Design + Parser Architecture + Crate Structure + Key Dependencies + Examples + User documentation) with no rule bodies.

**Pre-conditions.** Phase 1 done. (Otherwise the link-target sections in `architecture.md` may be missing content the `CLAUDE.md` mirror had.)

**TDD tests to write first.** Docs-only; inspection gates:
1. After the edit: `grep -nE "Pure Function Rule|Workspace Loading Parity|Project Isolation|Run Pipeline|Diagnostic Range Encoding" CLAUDE.md` — should show only the one-bullet pointers in the new §Architectural invariants section, not the full rule bodies.
2. `/smelt:validate architecture` reports zero drift (the rules are now sourced from the spec, validation walks the spec).
3. Regression: cargo gates green.

**Implementation shape.**
- New §"Architectural invariants" section, positioned immediately after §"Project Overview" (NOT inside §Architecture). The new section is short — maybe 15 lines total — and is structurally similar to a "must-read before changing X" sidebar.
- Each bullet looks like:
  ```
  - **{Rule name}** — {one-sentence summary of the constraint}. Authoritative spec: [`docs/specs/architecture.md` §"{Rule heading}"](docs/specs/architecture.md#{slug}).
  ```
- Inside §Architecture: delete the five rule sub-sections (`### Pure Function Rule`, etc.). §Architecture now flows as `High-Level Design → Parser Architecture → Crate Structure → Key Dependencies → Examples → User documentation` — descriptive content only.
- Total `CLAUDE.md` line count should drop by ~70-80 lines.

**Critical files (allowed to touch in this phase).**
- `CLAUDE.md` — restructure.

**Docs touched.** `CLAUDE.md` only.

**Review checklist (material findings only):**
- [ ] The five rule bodies are deleted from `CLAUDE.md`; nothing in `CLAUDE.md` duplicates the spec's normative text.
- [ ] The new §Architectural invariants section is compact, scannable, and links each rule to the spec.
- [ ] §Architecture flows cleanly as descriptive content with no rule sections interleaved.
- [ ] No content lost — every fact that was in the deleted sections either survives in the spec (after Phase 1) or is captured in the one-line summary.
- [ ] `/smelt:validate architecture` reports zero drift.
- [ ] Regression gates green.

**Commit.** `docs(claude): replace rule mirrors with compact pointers to architecture.md`

---

### Phase 3: Per-crate `CLAUDE.md` for crate-specific gotchas

**Goal.** Add `CLAUDE.md` files to crates where there is real crate-specific content worth on-demand loading. Do NOT write placeholder files for every crate; skip a crate if you can't fill its `CLAUDE.md` with at least a paragraph of non-rule content that a future Claude session would benefit from. Per-crate files contain build/test idioms, file-layout pointers, and gotchas — **never** parity rules (those live at root + spec).

**Pre-conditions.** Phases 1 and 2 done.

**TDD tests to write first.** Docs-only; inspection gates:
1. For each crate where a `CLAUDE.md` is added: confirm it contains zero rule bodies (`rg "Pure Function Rule|Workspace Loading Parity|Project Isolation|Run Pipeline|Diagnostic Range Encoding" crates/*/CLAUDE.md` should return no matches inside the new files).
2. Regression: cargo gates green.

**Implementation shape.**
- Crates to investigate (decide per-crate whether to add a file or skip):
  - `crates/smelt-db/CLAUDE.md` — likely YES. Salsa macro expansions are large; preferred reading patterns; where pure analysis lives (`queries/`, `type_inference/`); pointer back to root §Architectural invariants noting Salsa Purity + Project Isolation are load-bearing here.
  - `crates/smelt-lsp/CLAUDE.md` — likely YES. Backend struct surface area; how to construct a test client; the `position_encoding` and `example_workspaces` gates and what they cover; pointer back to Workspace Loading Parity + Diagnostic Range Encoding.
  - `crates/smelt-runtime/CLAUDE.md` — likely YES. Compile + execute pipeline shape; `RunReporter` trait surface; the `execute_parity` standing gate; pointer back to Run Pipeline Parity.
  - `crates/smelt-cli/CLAUDE.md` — likely YES. Reporter adapter pattern; how `example_diagnostics` and `commands/run.rs` interact with `smelt-runtime`; pointer back to Run Pipeline Parity + Workspace Loading Parity.
  - `crates/smelt-ui/CLAUDE.md` — likely YES if there's anything specific (Node deps for the frontend, dual-build idioms, JSON-serialization layer using `LineIndex`). Investigate.
  - `crates/smelt-parser/CLAUDE.md` — probably YES (small). Rowan / TextRange conventions; the no-`offset_to_position`-here invariant (which is part of Diagnostic Range Encoding); how to add a new SyntaxKind.
  - `crates/smelt-core/CLAUDE.md` — probably YES (small). `Config` ownership; `load_workspace` is the centralised eager-discovery entry point.
  - `crates/smelt-planner/CLAUDE.md` — investigate. May be small enough to skip.
  - `crates/smelt-types/CLAUDE.md` — probably SKIP. The crate is a thin vocabulary library; no gotchas worth writing.
  - `crates/smelt-dialect/CLAUDE.md` — probably SKIP for the same reason.
  - `crates/smelt-state/CLAUDE.md` — investigate. May have gotchas around `RunManifest` and `.smelt/` layout.
  - `crates/smelt-backend/` and `smelt-backend-duckdb/`, `smelt-backend-spark/` — investigate. Backend trait conventions live in `smelt-backend`; the two concrete crates may or may not need files.
  - `crates/smelt-bench/CLAUDE.md` — investigate. Custom profile flags etc.
  - `crates/smelt-datagen/CLAUDE.md` — investigate.
  - `crates/smelt-parser-compat/CLAUDE.md` — investigate (small crate).
  - `editors/vscode/CLAUDE.md` — probably YES. Node toolchain, F5 to launch Extension Host, `npm run` invocations.
- For each file: 1-3 short paragraphs. Structure:
  ```
  # crates/<name>/CLAUDE.md

  One sentence: what this crate owns.

  ## How to test
  Specific commands (if different from `cargo test -p <name>`).

  ## Gotchas
  - {something specific to working here}
  - {pointer to which architectural invariants in root CLAUDE.md apply: "Salsa Purity, Project Isolation"}

  ## Where things live
  Brief file-layout pointers if the crate is large enough to need them.
  ```
- Empty CLAUDE.md is worse than no CLAUDE.md — skip if you can't justify the content.

**Critical files (allowed to touch in this phase).**
- New `crates/*/CLAUDE.md` files for the crates you decide to populate.
- `editors/vscode/CLAUDE.md` if added.

**Docs touched.** Per-crate `CLAUDE.md` files; no other docs.

**Review checklist (material findings only):**
- [ ] No rule body appears in any per-crate `CLAUDE.md` (rules stay at root + spec).
- [ ] No placeholder files — each added file justifies its existence with ≥ 1 paragraph of crate-specific content.
- [ ] Each file mentions which architectural invariants from root `CLAUDE.md` are load-bearing for work in this crate (so a Claude session that ONLY loaded the per-crate file gets a pointer back to the rules).
- [ ] Tone matches the existing `docs/CLAUDE.md` (the pre-existing per-directory file is the template).
- [ ] Regression gates green.

**Commit.** `docs(claude): add per-crate CLAUDE.md for crate-specific gotchas`

---

## Deferred during implementation

(Append-only. Phase 1's audit notes go here.)

## Verification

- `grep -c "Pure Function Rule\|Workspace Loading Parity\|Project Isolation\|Run Pipeline\|Diagnostic Range Encoding" CLAUDE.md` returns the count of one-line pointers in §Architectural invariants (~5), not full rule bodies.
- `find crates editors -name CLAUDE.md` lists the new per-crate files.
- `rg "Pure Function Rule|Workspace Loading Parity|Project Isolation|Run Pipeline|Diagnostic Range Encoding" crates/*/CLAUDE.md editors/*/CLAUDE.md 2>/dev/null` returns zero matches (rules stay at root).
- `/smelt:validate architecture` reports zero drift.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` all green (regression — no code touched throughout).
- A fresh Claude session opened to `crates/smelt-db/` reads both root `CLAUDE.md` (with the pointer summaries) AND `crates/smelt-db/CLAUDE.md` (with the crate-specific gotchas) — both surface in context.
