# Plan: Typed Meta-Programming — Overall Phase Status

**Date**: 2026-05-09
**Spec**: [`docs/specs/meta_language.md`](../specs/meta_language.md), [`docs/specs/meta_config_loading.md`](../specs/meta_config_loading.md)
**Spec diff**: new specs (Phase 0 scaffold)
**Tracking PR / branch**: PR #117 — `research/typed-meta-programming` (will retitle to `feat: typed meta-programming` after Phase 0)
**Docs**: code+docs

## How to resume in a fresh session

This plan is the **in-repo committed index** for a multi-day implementation across multiple sessions. The across-session source of truth is `/home/andrew/.claude/plans/i-would-like-you-optimized-stallman.md` (the meta-plan). When a fresh session picks this up (whether autonomy-loop fired it via `claude -p "continue"` or the user typed `continue` after `/clear`):

1. Read `/home/andrew/.claude/plans/i-would-like-you-optimized-stallman.md` (the meta-plan; full process, expert-reviewer table, stop-the-line conditions, **sentinel emission contract**).
2. Read this file (the in-repo phase status table).
3. Find the first non-`done` phase below.
4. If the per-phase plan exists (`docs/plans/20260509-meta-language-<X>.md`), read it. Otherwise generate it: run `/smelt:spec meta_language` to author the phase's spec increment, then `/smelt:plan meta_language` to derive the phase plan. **The generated plan MUST end with the "Expert reviewer dispatch loop" phase per the verbatim template in meta-plan §13** (substituting the phase-specific expert subset from §5). Phase A's plan already contains the worked instance — use it as a reference.
5. Run `/smelt:implement docs/plans/20260509-meta-language-<X>.md`.
6. **Execute the per-phase plan's final "Expert reviewer dispatch loop" phase** (e.g. Phase A's Phase 7). This is a *loop*, not a one-shot dispatch: each expert may need multiple rounds. Address material findings (direct edits or implementer subagent), commit per expert (`review(meta-language-<X>): address {expert-name} feedback`), push, and re-dispatch until each expert reports "no material findings". Bounds: max 3 rounds per expert; two different experts flagging the same systemic concern in one round → stop-the-line per meta-plan §7. Record round counts in the plan's "Deferred during implementation" section as the acceptance gate.
7. Run `/smelt:validate meta_language` (and `meta_config_loading` for E1+). Zero drift required.
8. Update the row below: `pending` → `done`, fill `Date` and `Commit`. Push.
9. **Emit `<<PHASE_COMPLETE>>`** as part of the final user-facing message (autonomy-loop wrapper greps for it to fire the next iteration). When all phases are done and meta-plan §10 verification holds, emit `<<ALL_DONE>>` instead. If a stop-the-line condition fires, emit `<<PAUSE_FOR_HUMAN>>` with the reason on the line above. See meta-plan "Sentinel emission contract" for the strict rules.
10. End the session — the next iteration / session resumes from the next pending row.

## Autonomy loop (optional)

To run autonomously: `bash .claude/scripts/autonomy-loop.sh`. The wrapper invokes `claude -p "continue"` in a fresh-context loop, detects the sentinels emitted at the end of each iteration, and either restarts (`<<PHASE_COMPLETE>>`), exits with success (`<<ALL_DONE>>`), or pauses for the user (`<<PAUSE_FOR_HUMAN>>` or unrecognised output). Defaults: max 25 iterations, `bypassPermissions` permission mode, `opus` model. Tunable via `MAX_ITERATIONS`, `PERMISSION_MODE`, `MODEL` env vars. Per-iteration logs land in `~/.claude/logs/meta-language-loop/`.

To run manually instead: `/clear` between phases and type `continue` — the resumability protocol is identical.

**Within-phase reset rule.** If the implementer subagent has been iterating >3 review cycles, the phase scope is wrong. End the session, revise the phase plan or escalate to the user, do not iterate harder.

**Mid-phase commit rule.** Never carry mid-phase state across a session reset. If a phase's commits don't all land together, treat the phase as not-done and re-run `/smelt:implement` from scratch on the same plan.

## Phase status

| # | Phase | Status | Plan path | Date | Commit |
|---|-------|--------|-----------|------|--------|
| 0 | Foundation: spec skeletons + overall plan | done | this file | 2026-05-09 | *(this commit)* |
| A | `List<T>`, literals `[a,b,c]`, spread `...` | done | `docs/plans/20260509-meta-language-A.md` | 2026-05-10 | (pending commit) |
| B | HOFs, lambdas, pipe `\|>`, contextual reducers, `smelt.config.var` | pending | `docs/plans/20260509-meta-language-B.md` | — | — |
| C | Reflection narrow: `smelt.columns_of`, `ColumnRef` | pending | `docs/plans/20260509-meta-language-C.md` | — | — |
| D | Reflection wide: `smelt.models.*`, `smelt.sources.*` | pending | `docs/plans/20260509-meta-language-D.md` | — | — |
| E1 | Records, `Map<K,V>`, YAML/JSON loaders | pending | `docs/plans/20260509-meta-language-E1.md` | — | — |
| E2 | Multi-model production: `generates: models` frontmatter, `ModelDef` | pending | `docs/plans/20260509-meta-language-E2.md` | — | — |
| F | Polish: parameterised reducers, multi-arg lambdas, ternary | pending | `docs/plans/20260509-meta-language-F.md` | — | — |
| G | LSP completeness sweep, docs-site rewrite, `/smelt-loop` tier-3 | pending | `docs/plans/20260509-meta-language-G.md` | — | — |

## In scope

- Every phase A–G surface listed in `docs/specs/meta_language.md` and `docs/specs/meta_config_loading.md`.
- Per-phase examples under `examples/`, all passing `cargo test -p smelt-cli --test example_diagnostics`.
- User docs at `docs-site/docs/meta-language/` covering every shipped construct.
- LSP support for every shipped construct: hover, goto-def, completion, diagnostics-with-frame-stacks.
- `smelt-app-builder` skill update per phase.
- `/smelt-loop` extension: smaller asks added to medium tier as Phases A–B land; new large tier added in Phase E2; full run in Phase G with skill diffs landed.
- The killer per-cohort union demo (`examples/per_cohort_union/`) lands in Phase E2 — see meta-plan §8 for the concrete file shape.
- Cross-spec touches per the meta-plan §6 table.

## Deferred (out of plan)

Per the meta-plan §3 theoretical-completeness ledger:

- `flat_map`, `zip_with`, `take`, `drop`, `length`, `index_of`, `any`, `all`, `find`, `partition` — speced as derivations, shipped only if examples force them.
- Tuples — rejected in favour of records.
- Pipe-SQL extension (research §4.6 alt b) — separate spec.
- User-defined reducers — Phase F if room; otherwise post-plan.
- `infer_schema` codegen mode — post-plan.
- Generators-of-generators — forbidden in v1.
- Heterogeneous lists / sum types — out of scope.

## Verification

The work is **done** when every condition in the meta-plan §10 holds. Notably:

- All phases A–G show `done` above.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, `cargo test -p smelt-cli --test example_diagnostics` all pass.
- `/smelt:validate meta_language` and `/smelt:validate meta_config_loading` report zero drift.
- `examples/per_cohort_union/` and `examples/staging_from_sources/` build and pass acceptance tests.
- `/smelt-loop` `large` tier completes a clean run.
- LSP support for every shipped construct (hover, goto-def, completion, diagnostics, rename for new constructs by Phase G).
- PR #117 retitled to `feat: typed meta-programming` with phase checklist updated as we went.

## Phase 0 — what landed in this commit

This Phase 0 commit (the one introducing this plan file) lands:

1. `docs/specs/meta_language.md` — skeleton with all SPEC_TEMPLATE.md sections; framing in Design and Constraints filled in; per-phase Surface entries marked `[deferred to Phase X]`.
2. `docs/specs/meta_config_loading.md` — skeleton; framing filled in; surface body deferred to Phase E1.
3. `docs/plans/20260509-meta-language-overall.md` — this file.
4. `.claude/commands/smelt/implement.md` — single edit pinning inner implementer/reviewer subagents to `model: sonnet` per the user's instruction to use simpler models for delegated work.
5. (Optional, post-commit) `gh pr edit 117 --title 'feat: typed meta-programming'` — retitle to reflect that this branch is now the implementation branch.

No code in `crates/` changes in Phase 0. The next session begins Phase A.
