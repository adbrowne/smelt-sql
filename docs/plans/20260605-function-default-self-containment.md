# Plan: Function default self-containment — enforce Semantics #9 (a default must not reference other parameters)

**Parent (master plan)**: `docs/plans/20260530-feature-sweep.md` — a **sub-plan** spawned from the feature sweep to remediate the "functions self-contained defaults" cluster of its ledger findings: a single finding, **BUG-003** (the functions Semantics #9 rule "A default expression must not reference other parameters" is specified but unenforced — a default like `b: Expr<Int> = a + 1` that references sibling param `a` is silently accepted). The autonomy loop works this sub-plan phase by phase and rolls up to the master only when it is exhausted.

**Date**: 2026-06-05
**Spec**: `docs/specs/functions.md` §Semantics #9 ("A default expression must not reference other parameters.") and §"Diagnostic codes" table (functions.md:157-174; ownership note at functions.md:260 — descriptions live alongside `DiagnosticCode`).
**Spec diff**: **applied mid-flight by this plan** — P1 adds one row (`DefaultReferencesParameter`, the new code) to the functions.md §Diagnostic codes table with a one-line trigger description, kept timeless (no phase/ledger vocabulary). No other spec section changes; Semantics #9 already states the rule normatively, so P1 only documents the code that enforces it. No close-out spec retraction is needed (#9 was always normative — it was simply unenforced).
**Tracking branch**: `worktree-test_features`
**Docs**: code+docs. The spec §Diagnostic codes table gains the new row (P1). No user-facing docs-site page changes today: there is no `docs-site/` diagnostics catalogue page yet (diagnostics are tracked for a future `diagnostics.md`); P1 confirms this and records it rather than inventing a page.

## Execution prompt (for a fresh session / autonomy iteration)

Read this file. Run the next `pending` phase in the Progress-tracking table (skip `done` and `blocked` rows) using the per-phase routine below (pre-flight → spec increment **only if the row lists one** → red-green `/smelt:implement` on the phase's tests, spec as oracle, implementer + reviewer → verification gates → update the table row → commit + push with the phase's commit message). Emit exactly one sentinel: `<<PHASE_COMPLETE>>` (phase done), `<<PHASE_BLOCKED>>` (decision/off-target-red recorded; see §"Block conditions"), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>` (sub-plan exhausted; see the loop's roll-up rule), or `<<ALL_DONE>>`. There is no hard-stop: a block is recorded and the loop continues to the next pending phase.

## Goal

Make functions Semantics #9 real. A `smelt.define` default expression that references a **sibling parameter** must be a hard declaration-time error, surfaced identically in CLI and LSP.

- **BUG-003 (correctness, major)** — `smelt.define f(a: Expr<Int>, b: Expr<Int> = a + 1)` is silently accepted today. The default `a + 1` references sibling param `a`. Per Semantics #9 a default must be self-contained. The current implementation type-infers the default against an **empty** `TypeContext` (`crates/smelt-db/src/queries/function_diagnostics.rs:1620-1622`; comment at 1598-1602 says "self-contained per research §3"), so a sibling-param reference resolves to `Unknown` against the empty context and produces **no** diagnostic — there is no validator enforcing self-containment. Enforce it: add a new `DefaultReferencesParameter` diagnostic code and an AST-side validator that flags a default expr referencing any sibling parameter name, anchored at the default expr's range.

## Design decisions (resolved — do not re-litigate)

- **Implement option (a): add the diagnostic + validator** — do NOT downgrade Semantics #9 to advisory. It is a clear, contained correctness rule already stated normatively in the spec; the only gap is enforcement.
- **The validator runs AST-side, co-located with `default_type_lookup`** (`crates/smelt-db/src/queries/function_diagnostics.rs`), NOT in the `ParamSpec`-only path (`emit_duplicate_param_diagnostics` at `crates/smelt-db/src/function_body_check.rs:432`). Rationale: `ParamSpec` (`crates/smelt-types/src/signatures.rs:1566`) records name / name_range / has_default but **not** the default expr text or range. The default expr is only recoverable from the AST via `Param::default_value_expr()` → `Expr` (`crates/smelt-parser/src/ast.rs:310`) — which is exactly how `default_type_lookup` already re-parses it. The validator must run where BOTH the per-param default exprs AND the sibling param names are co-available; that is AST-side.
- **The check is "references a sibling PARAMETER name", not "references any identifier".** Column refs, literals, and other identifiers that happen not to match a sibling parameter name are fine (`= 1`, `= some_column + 1` where `some_column` is not a param). Mechanism: collect the signature's parameter names → for each param that has a default, scan the default expr's identifier / column-ref tokens for any token matching a sibling param name → emit `DefaultReferencesParameter` anchored at the default expr's range. (A param's default referencing *its own* name is impossible in well-formed surface — it isn't in scope before it is declared — but the "sibling" framing covers it regardless: any param-name match is an error.)
- **`DefaultReferencesParameter`, Error severity, owned by functions.md's §Diagnostic codes table** (the same table that owns the other declaration-time function codes). The colliding default does not change behaviour silently — it is rejected at the declaration.
- **No new spec section, no Known-Divergence churn.** Semantics #9 is already normative; this plan adds the enforcing code's documentation row and the validator. There is nothing to retract at close-out.

## Per-phase routine
1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. If red, check **what** is red: if the failure is this phase's own acceptance target (the fixture/test this phase exists to make green), that is expected — **proceed**. If the red is unrelated breakage, treat it as a block (record + continue, per §"Block conditions"); do not build on a broken baseline.
2. **Spec increment** (only the phases that list one — here, only P1's §Diagnostic codes table row): edit the named spec section first; keep it timeless (no phase/ledger vocabulary in the spec body).
3. **Red-green `/smelt:implement`.** Write the phase's failing test(s) first, then the implementation, spec as oracle. Implementer pass, then reviewer pass (material findings only).
4. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green; plus the phase's gates: `cargo test -p smelt-cli --test example_diagnostics` **and** `cargo test -p smelt-lsp --test example_workspaces` (the dual gate — the LSP gate catches CLI/Salsa divergence and confirms the new diagnostic surfaces through the real LSP backend).
5. **Record + commit.** Update the status-table row to `done` + date; commit and push tests + impl + (spec at P1) + table together. Emit `<<PHASE_COMPLETE>>` (or `<<ALL_DONE>>` on the last green phase).

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue, no hard-stop)
When a phase hits a condition below, **do not halt**. Instead: (1) set the row to `blocked` with a one-line reason; (2) append a dated entry to §"Blocked phases" (phase id, reason/decision, candidate options); (3) restore the tree to a clean committed state; (4) commit + push; (5) emit `<<PHASE_BLOCKED>>`. The next iteration skips the blocked row.

Conditions:
- The phase needs a design decision **not** answered by this plan or the spec. The one anticipated judgment call: the exact **name** of the new diagnostic code (`DefaultReferencesParameter` is the proposed name) and the precise **scope** of the rule — specifically whether a default may legitimately reference a CTE-like / non-parameter construct vs. only sibling parameters being forbidden. The settled decision above is "forbid sibling-parameter references only; everything else is fine"; if implementation reveals a surface form where that boundary is genuinely ambiguous (e.g. a construct that is both a param name and a valid column ref in a real workspace), block for a product call rather than guess.
- Pre-flight is red on **unrelated** breakage (not this phase's own target).
- The tree can't be returned to green after the phase.

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| P1 | Spec: add the `DefaultReferencesParameter` row to the functions.md §Diagnostic codes table (functions.md:157-174) with a one-line trigger ("A `smelt.define` default expression references another parameter in the same signature."), kept timeless. Confirm no `docs-site/` user page needs the new code today (no diagnostics catalogue page exists yet — tracked for a future `diagnostics.md`); record that in the plan rather than inventing a page. | done (2026-06-06) | BUG-003 (1/2) | docs(functions): document DefaultReferencesParameter diagnostic for Semantics #9 (BUG-003) | 2026-06-06 |
| P2 | Code (red-green): add the `DefaultReferencesParameter` variant to `DiagnosticCode` (`crates/smelt-db/src/diagnostics_types.rs:9`); add the AST-side validator near `default_type_lookup` (`crates/smelt-db/src/queries/function_diagnostics.rs`) that collects the signature's parameter names and, for each param with a default, scans the default expr's identifier/column-ref tokens for a sibling-param-name match → emits `DefaultReferencesParameter` anchored at the default expr's range; wire it into the function-diagnostics production path so it surfaces in `file_diagnostics`/LSP. **Red test FIRST**: a `crates/smelt-db` unit test asserting `f(a: Expr<Int>, b: Expr<Int> = a + 1)` emits exactly one `DefaultReferencesParameter` anchored at the default expr, and that a self-contained default (`= 1`) emits none. Inspect existing `broken_function_diagnostics` fixture cases first; if they match, add a keepable broken fixture under `examples/` covering the sibling-reference case. | pending | BUG-003 (2/2) | feat(db): enforce self-contained function defaults via DefaultReferencesParameter (closes BUG-003) | |
| P3 | Close-out: flip BUG-003 to `fixed` in `docs/plans/20260530-feature-sweep.md`'s ledger with the regression-test name; update the master sub-plan registry + `docs/ROADMAP.md`; full dual gate (`example_diagnostics` + `example_workspaces`) green. | pending | — | docs(function-default-self-containment): close out — Semantics #9 enforced, ledger + roadmap updated | |

**Status values**: `pending`, `done`, `blocked`. A phase is `done` only when its tests are red-green confirmed and all gates are green. A `blocked` phase has a dated §"Blocked phases" entry and returns to `pending` once a human resolves it.

## Blocked phases

Append-only log of phases the loop recorded as `blocked` and continued past. Each entry: date, phase id, reason/decision, candidate options. *(None yet.)*
