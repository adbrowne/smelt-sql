# Plan: Synthesise derived-table aliases for `TableExpr`-returning calls in FROM position

**Date**: 2026-05-19
**Spec**: [`docs/specs/functions.md`](../specs/functions.md) §"Function call syntax" — boolean-position rules already allow `smelt.<path>(...)` calls anywhere the SQL grammar accepts the call's return sort, FROM position included. The user surface is "you can call a `TableExpr`-returning function in FROM"; the alias is a lowering detail DuckDB requires.
**Spec diff**: none (no spec rule changes; this closes the "TableExpr inlining in FROM position requires derived-table aliases" entry under Known Divergences of the parent plan — currently absorbed into `functions.md`'s "End-to-end execution incomplete" entry).
**Parent plan**: [`docs/plans/20260519-functions-meta-gaps.md`](20260519-functions-meta-gaps.md) Phase 2
**Tracking PR / branch**: `worktree-web_analytics` (this worktree).
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/functions.md` — it is the correctness oracle. Pay particular attention to §"Function call syntax" (the trigger rule is uniform in expression position and FROM position) and the discussion of `TableExpr` return sorts under "smelt.define grammar". Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-web_analytics`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/web_analytics/` or `crates/smelt-cli/tests/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: this phase synthesises a derived-table alias for `TableExpr`-returning calls in FROM position. It does **not** touch JOIN aliasing, CROSS-APPLY / LATERAL constructs, or the dialect-portability of the substituted body's column references.
- Honor architectural invariants from `CLAUDE.md` (no Salsa import added to `smelt-dialect`; printer remains a lowering layer).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/functions.md` and `docs-site/docs/guide/functions.md` describe the feature as if it has always existed — no `### Phase 2 — …` headings, no `(Phase 2)` inline labels.

**Subagent model rule.** Outer orchestrator runs on `opus`. Every delegated subagent — implementer, reviewer, every expert in per-phase reviewer tables — MUST be spawned with `model: "sonnet"`.

---

## Context

Phase 1.2 wired `<call>.*` projection for struct-returning calls in SELECT lists; Phase 1.4 wired a `TableExpr`-returning call in FROM position with an explicit `AS s` alias (`examples/web_analytics/models/silver/sessions.sql`). DuckDB rejects an un-aliased derived table (`FROM (SELECT ...)`), so callers currently must supply the alias themselves. The spec is silent on FROM-position aliasing because it's a lowering implementation detail — the user-visible surface is just "you can call the function in FROM"; whether the user types `AS s` is wart, not contract. This phase removes the wart by synthesising a unique alias at the printer layer when none is present.

## Scope

### In scope (spec coverage)

- **Lowering rule.** When a `SMELT_PATH_CALL` (or `SMELT_PATH_CALL_STAR`, though that one is SELECT-list-only by construction) is printed in FROM position and the substituted body is a SELECT statement, the printer wraps the expansion in `(<expanded>) AS <synthesised-alias>` if and only if there is no user-supplied alias attached to the call. A user-supplied alias suppresses synthesis.
- **Alias naming.** The synthesised alias is `__smelt_t<N>` where `N` is a per-model monotonically increasing counter (or a stable hash of the call's text-range — pick whichever is simpler to implement and explain).
- **Refactor `sessions.sql`.** The `AS s` alias at the call site is removed; the rest of the model is unchanged. Snapshot test asserts the no-alias form materialises the same rowset.
- **Snapshot test.** New e2e test under `crates/smelt-cli/tests/` builds a workspace whose model calls `FROM smelt.functions.sessionize(...)` (no alias) and asserts DuckDB executes it successfully with the expected rowset.
- **User-doc update.** `docs-site/docs/guide/functions.md` named-arg example drops the trailing `AS s` (the example reads cleaner without it); add a one-line note that aliases are optional.

### Explicitly deferred

- **JOIN-position aliasing.** A `JOIN smelt.functions.foo(...) ON ...` shape also needs an alias on DuckDB. If a real fixture demands it, extend in a follow-up plan; for now the synthesis rule covers FROM position only.
- **LATERAL / CROSS APPLY** constructs.
- **Cross-engine validation.** Spark accepts un-aliased derived tables in some positions; the synthesis rule is dialect-agnostic but may produce slightly more verbose SQL than necessary on Spark. Acceptable.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 2.1   | done     | 4275c72a | 2026-05-19 |

---

### Phase 2.1: Synthesise derived-table alias for `TableExpr`-returning calls in FROM position

**Goal.** When a `smelt.<path>(...)` call is expanded in FROM position with no user-supplied alias, wrap the expanded body in `(<body>) AS __smelt_t<N>`. User-supplied aliases suppress synthesis. Update `examples/web_analytics/models/silver/sessions.sql` to drop its explicit `AS s` and add a snapshot test asserting equivalence.

**Pre-conditions.** Phase 1 done. Phase 1.4 shipped `sessions.sql` with an explicit `AS s` alias — this phase removes it.

**TDD tests to write first.**

- `crates/smelt-dialect/src/printer.rs::tests::table_expr_call_in_from_without_alias_synthesises_one` — printer-level unit test: parse `SELECT * FROM smelt.functions.sessionize(x)`, wire a path-call expander that returns a SELECT body, assert the printer output is `SELECT * FROM (SELECT ...) AS __smelt_t<N>` for some N.
- `crates/smelt-dialect/src/printer.rs::tests::table_expr_call_in_from_with_alias_passes_through` — same input but with `AS s` after the closing `)`; printer output uses the user's alias verbatim, no synthesis.
- `crates/smelt-dialect/src/printer.rs::tests::table_expr_call_in_select_list_does_not_synthesise_alias` — a `TableExpr`-returning call appearing in some non-FROM context (most likely an error case, but the printer should not wrap it in `(...) AS …` regardless — fall back to the existing behaviour). If the SQL grammar makes this unreachable, skip this test and document why in the implementer's report.
- `crates/smelt-cli/tests/web_analytics_refactor_snapshot.rs::sessions_rowset_equivalent_without_explicit_alias` — extend the Phase 1.4 test file. Build two mini-workspaces side-by-side:
  - **With alias**: the post-Phase-1.4 `sessions.sql` body (carries `AS s`).
  - **Without alias**: the same body with the `AS s` removed.
  Both must produce identical rowsets. The "without alias" variant must build and execute without error.
- `cargo test -p smelt-cli --test example_diagnostics` continues to pass with zero findings on `examples/web_analytics/`.

**Implementation shape.**

In `crates/smelt-dialect/src/printer.rs`, the existing `SMELT_PATH_CALL` arm:

```rust
if let Some(expanded) = expander(&segs, positional, named) {
    let reparsed = smelt_parser::parse(&expanded);
    print_node(&reparsed.syntax(), ctx, out);
    return;
}
```

becomes context-aware:

1. Walk the `node`'s ancestors to determine FROM position. The relevant CST shape is likely `SMELT_PATH_CALL` → `FROM_CLAUSE_ITEM` (or `TABLE_REF` — implementer audits `crates/smelt-parser/src/syntax_kind.rs` and `parser.rs` to pick the right marker). The marker for "this is a FROM-position table reference" is the immediate parent shape.
2. Check whether the call is followed by an explicit alias (`AS <ident>` or `<ident>` adjacency). The CST may expose this as a sibling node of the `SMELT_PATH_CALL` inside its parent (e.g. `TABLE_ALIAS`). Implementer confirms by parser audit.
3. If in FROM position and no explicit alias is attached, expand the body and emit `(<expanded>) AS __smelt_t<counter>`. Otherwise emit the body verbatim (current behaviour).
4. The counter is a `Cell<u32>` or atomic stored on `PrintContext` so each model gets a fresh counter. (Counter monotonicity across the model is sufficient; nothing outside this printer pass observes the names.)

Choose the counter mechanism by looking at how `PrintContext` is constructed today — if it's `Copy`/borrowed, a `Cell<u32>` field requires lifetime/borrow gymnastics. A simpler alternative: hash the `node.text_range()` into a small ID and emit `__smelt_t_<offset>` — stable per call site, no shared state. Implementer picks; the per-phase plan does not prescribe.

**Critical files (allowed to touch in this phase).**

- `crates/smelt-dialect/src/printer.rs` — the FROM-context detection + alias synthesis.
- `crates/smelt-cli/tests/web_analytics_refactor_snapshot.rs` — extend with the no-alias snapshot.
- `examples/web_analytics/models/silver/sessions.sql` — drop the explicit `AS s`; trim the comment line that says the alias is required.
- `docs-site/docs/guide/functions.md` — drop the trailing `AS s` from the named-arg worked example; add a sentence noting aliases are optional.

**Out of scope — do NOT touch:**

- `crates/smelt-parser/` (parser-level changes). The CST already distinguishes user-supplied aliases from absence — this is purely a printer-side disambiguation.
- `crates/smelt-cli/src/` (compiler / test-pipeline / refs extraction). All needed integration landed in Phase 1.4.
- Any other example workspace.
- JOIN-position handling; LATERAL / CROSS APPLY.

**Docs touched (default — see plan header `Docs: code+docs`).**

- `docs-site/docs/guide/functions.md` — drop the `AS s` from the existing worked example; the existing prose can carry a short addition along the lines of *"The alias is optional — the compiler synthesises one when needed for the target engine."*
- `docs/specs/functions.md` — no change. The Known Divergences entry that was narrowed in Phase 1.2 already excludes this case (it now scopes only cross-engine codegen).

**Review checklist** (material findings only):

- [ ] TDD tests listed above exist and assert what's specified.
- [ ] FROM-position detection is robust: a user-supplied alias suppresses synthesis (no double-aliasing); a call in non-FROM position is unaffected.
- [ ] Synthesised alias names do not collide with user-authored identifiers — `__smelt_t` prefix (or equivalent leading-underscore form) is reserved by the project's existing internal-symbol convention (see `examples/web_analytics/functions/sessionize.sql`'s `_smelt_prev_ts_us` precedent).
- [ ] No regression of the `silver/sessions.sql` rowset (snapshot test asserts byte-equality with the post-Phase-1.4 form).
- [ ] Spec + docs-site edits are timeless — no `Phase 2` headings, no `(Phase 2)` labels.
- [ ] Architectural invariants honored: `smelt-dialect` does not depend on `smelt-db`.
- [ ] No scope creep: parser unchanged; no JOIN/LATERAL handling.

**Commit.** `feat(functions): synthesise derived-table alias for TableExpr calls in FROM position`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets`
- `cargo test --quiet 2>&1 | tail -40`
- `cargo test -p smelt-cli --test example_diagnostics 2>&1 | tail -10`
- `cargo test -p smelt-cli --test web_analytics_refactor_snapshot 2>&1 | tail -10` — both `events_parsed_*` and `sessions_*` tests pass.
- `cargo test -p smelt-cli --test functions_e2e 2>&1 | tail -10`
- `examples/web_analytics/models/silver/sessions.sql` no longer carries an explicit `AS s` on the `sessionize` call.
- `/smelt:validate functions` reports zero drift on the Known Divergences entry.
