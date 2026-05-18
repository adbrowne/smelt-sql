# Plan: Web Analytics Example — Overall Phase Status

**Date**: 2026-05-17
**Spec**: [`docs/specs/datagen.md`](../specs/datagen.md) (extended in Phases 1–2; the example itself is a demo, not a new feature spec)
**Spec diff**: new `json_object` and `linked_choice` generator entries in `docs/specs/datagen.md` (added in Phases 1 and 2 respectively)
**Tracking PR / branch**: `worktree-web_analytics` — open PR when Phase 3 lands
**Docs**: code+docs (example README, docs-site entry under `examples/`, datagen guide updates in Phases 1–2)

## Goal

Build a self-contained example at `examples/web_analytics/` demonstrating:

1. A bronze→silver→gold pipeline over JSON-encoded web events.
2. Incremental sessionization with a 30-minute inactivity + platform-boundary rule.
3. **Three parallel implementations** of cross-session / cross-device user stitching, side-by-side in a single granular event-level table, so the algorithmic tradeoff is observable row-by-row. Compares:
   - `identity_forward_only` — within-session resolution.
   - `identity_backward_fill` — Amplitude-basic, per-device canonical user.
   - `identity_connected_components` — Amplitude-full, union-find over `(device, user)` edges via recursive-CTE label propagation.
4. Late-arriving stitch evidence with a 7-day rolling lookback window.
5. Marts that quantify the difference between the three algorithms (DAU, identification rate).

Reference: <https://amplitude.com/docs/data/sources/instrument-track-unique-users>.

## How to resume in a fresh session

This plan is the **in-repo committed index** for a multi-session implementation. The across-session source of truth is `/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md` (the meta-plan). When a fresh session picks this up (whether autonomy-loop fired it via `claude -p "continue"` or the user typed `continue` after `/clear`):

1. Read `/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md` (the meta-plan; expert dispatch, stop-the-line, **sentinel emission contract**).
2. Read this file (the in-repo phase status table).
3. Find the first non-`done` phase below.
4. If the per-phase plan exists (`docs/plans/20260517-web-analytics-<N>-<slug>.md`), read it. Otherwise generate it: for datagen phases (1, 2) run `/smelt:spec datagen` to author the spec increment, then `/smelt:plan datagen`. For example phases (3–9), skip the spec step and run `/smelt:plan` with the phase scope as input. **The generated plan MUST end with the "Expert reviewer dispatch loop" phase per the meta-plan template** (substituting the phase-specific expert subset).
5. Run `/smelt:implement docs/plans/20260517-web-analytics-<N>-<slug>.md`.
6. **Execute the per-phase plan's final "Expert reviewer dispatch loop"** — a loop, not one-shot. Each expert may need multiple rounds. Address material findings (direct edits or implementer subagent), commit per expert (`review(web-analytics-<N>): address {expert-name} feedback`), push, and re-dispatch until each expert reports "no material findings". Bounds: max 3 rounds per expert; two different experts flagging the same systemic concern in one round → stop-the-line per meta-plan.
7. For datagen phases (1, 2): run `/smelt:validate datagen` — zero drift required. For example phases (3–9): run `cargo test -p smelt-cli --test example_diagnostics` — zero LSP diagnostics required.
8. Update the row below: `pending` → `done`, fill `Date` and `Commit`. Push.
9. **Emit `<<PHASE_COMPLETE>>`** as part of the final user-facing message. When all phases are done and verification holds, emit `<<ALL_DONE>>` instead. Stop-the-line → emit `<<PAUSE_FOR_HUMAN>>` with the reason on the line above. See meta-plan for the strict rules.
10. End the session — the next iteration / session resumes from the next pending row.

## Autonomy loop

To run autonomously: `bash .claude/scripts/autonomy-loop.sh`. The wrapper invokes `claude -p "continue"` in a fresh-context loop, detects the sentinels emitted at the end of each iteration, and either restarts (`<<PHASE_COMPLETE>>`), exits with success (`<<ALL_DONE>>`), or pauses for the user (`<<PAUSE_FOR_HUMAN>>` or unrecognised output). Defaults: max 25 iterations, `bypassPermissions`, `opus`. Tunable via `MAX_ITERATIONS`, `PERMISSION_MODE`, `MODEL` env vars. Per-iteration logs land in `~/.claude/logs/web-analytics-loop/`.

To run manually: `/clear` between phases and type `continue` — the resumability protocol is identical.

**Within-phase reset rule.** If the implementer subagent has been iterating >3 review cycles, the phase scope is wrong. End the session, revise the phase plan or escalate to the user.

**Mid-phase commit rule.** Never carry mid-phase state across a session reset. If a phase's commits don't all land together, treat the phase as not-done and re-run `/smelt:implement` from scratch on the same plan.

**Subagent model rule.** The outer orchestrator runs on `opus` (autonomy loop default). Every delegated subagent — implementer, reviewer, every expert in the per-phase reviewer table (`datagen-expert`, `sql-expert`, `docs-reviewer`, etc.) — MUST be spawned with `model: "sonnet"` on the Agent tool. **Do not omit the `model` parameter** — without it, the subagent inherits `opus` from the parent and silently burns budget.

## Phase status

| # | Phase | Status | Plan path | Date | Commit |
|---|-------|--------|-----------|------|--------|
| 0 | Foundation: this overall plan + meta-plan + autonomy-loop repoint | done | this file | 2026-05-17 | *(this commit)* |
| 1 | datagen: `json_object` generator (spec + impl + docs) | done | `docs/plans/20260517-web-analytics-1-datagen-json-object.md` | 2026-05-17 | `57ce9489` |
| 2 | datagen: `linked_choice` joint-distribution generator (spec + impl + docs) | done | `docs/plans/20260517-web-analytics-2-datagen-linked-choice.md` | 2026-05-18 | `8fd0955c` |
| 3 | Example scaffolding: `smelt.yml`, `datagen.yaml`, bronze view, `silver/events_parsed`, `functions/parse_event_payload.sql` | done | `docs/plans/20260517-web-analytics-3-scaffold.md` | 2026-05-18 | `51800515` |
| 4 | Sessionization: `functions/sessionize.sql`, `silver/sessions.sql` (incremental, 7-day lookback), `silver/device_user_edges.sql` | done | `docs/plans/20260517-web-analytics-4-sessionize.md` | 2026-05-18 | `52e507f1` |
| 5 | `identity_forward_only` + initial `eventstream_with_identity` (single column) | done | `docs/plans/20260517-web-analytics-5-forward-only.md` | 2026-05-18 | `9421a636` |
| 6 | `identity_backward_fill` (extends `eventstream_with_identity`) | pending | `docs/plans/20260517-web-analytics-6-backward-fill.md` | | |
| 7 | `identity_connected_components` (recursive-CTE label propagation, extends eventstream) | pending | `docs/plans/20260517-web-analytics-7-connected-components.md` | | |
| 8 | Marts (`daily_active_users_by_method`, `identity_method_comparison`) + README + docs-site link | pending | `docs/plans/20260517-web-analytics-8-marts-readme.md` | | |
| 9 | (Deferred / optional) Replace connected-components iteration cap with true fixed-point | pending | `docs/plans/20260517-web-analytics-9-fixed-point.md` | | |

## In scope

- Everything listed in Goal §1–5.
- New datagen generators (`json_object`, `linked_choice`), with unit tests, round-trip tests, and spec/docs entries in `docs/specs/datagen.md` + `docs-site/docs/guide/datagen.md`.
- `examples/web_analytics/` complete and passing `cargo test -p smelt-cli --test example_diagnostics`.
- Inline `.test.sql` assertions for each identity algorithm's defining invariant (see per-phase plans for the exact assertions).
- `examples/README.md` and `docs-site/docs/examples/` updated to link the new example.

## Out of scope

- Cross-device probabilistic stitching (heuristic / ML-based). The three algorithms shipped are deterministic.
- A live JSON ingestion endpoint. Source data is generated to parquet on disk and read via `smelt.sources`.
- Real-time / streaming sessionization. The example is batch with daily partitions.
- Schema evolution of the JSON payload (the payload schema is fixed across the 60-day dataset).
- `time_to_identity` mart — interesting but adds a fourth concept and isn't needed for the side-by-side comparison.

## Verification

The work is **done** when:

- All phases 1–8 show `done` above (Phase 9 is optional).
- `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` all pass.
- `cargo test -p smelt-cli --test example_diagnostics` reports zero diagnostics for `examples/web_analytics/`.
- `/smelt:validate datagen` reports zero drift after Phases 1 and 2.
- Running `smelt-datagen --config examples/web_analytics/datagen.yaml --scale-factor 0.01 && smelt build --project-dir examples/web_analytics` succeeds end-to-end on a fresh checkout.
- The mart `daily_active_users_by_method` shows the expected monotonic relationship `forward_only ≤ backward_fill ≤ connected_components` on every day in the synthetic dataset.

## Phase 0 — what landed in this commit

This Phase 0 commit lands:

1. `docs/plans/20260517-web-analytics-example.md` — this file.
2. `/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md` — the meta-plan.
3. `.claude/scripts/autonomy-loop.sh` — docstring + log dir repointed at this plan.

No code in `crates/` or `examples/` changes in Phase 0. The next session begins Phase 1 (`json_object` generator).
