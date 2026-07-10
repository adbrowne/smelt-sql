# Handoff — emit unification → web-analytics maintenance demo

**Date:** 2026-07-10
**Branch:** `worktree-incremental` (worktree at `.claude/worktrees/incremental`), pushed to origin.
**Build/test status:** `verify-phase.sh --fast` ALL GREEN at `223962ba`. No code changes this session — docs/specs/plans only.

## What this session did (all committed + pushed)

The 2026-07-10 review session confirmed the MP-series maintenance work and executed the resulting hygiene + planning steps:

1. `8d190c5c` — docs-site drift purge: removed the phantom "key temporal locality" conditional at all seven sites; fixed `--since-upstream` mislabelled "unbuilt"; documented `maintenance:`/`scan_bounds` frontmatter, `smelt explain <model>` mode, reconciliation ledger; fixed `mutation_profile` wire name.
2. `8ff61e2d` — spec consolidation: `docs/specs/model_maintenance.md` **deleted**, its normative contract folded into `docs/specs/maintenance_plan.md` (same §-names, grain vocabulary); ~24 files' references retargeted (`docs/plans/` and `docs/research/` deliberately left historical); stale Known-Divergences/banners in keyed/batched/versioned/models specs corrected against `config.rs` ground truth. Also records the review results in `docs/plans/20260710-web-analytics-maintenance-demo.md` §6 and **reverses its decision 3**: the demo ships on the partition-grain reframe, keyed temporal locality comes later as its own spec-first plan.
3. `223962ba` — spec-first emit-unification: `maintenance_plan.md` §"Statement emission (single owner)", `cli.md` `--show-sql` surface, `architecture.md` invariant 12 clause + CLAUDE.md mirror, and the phased plan **`docs/plans/20260710-emit-unification.md`** (6 phases, all `pending`).

Key review findings a fresh session should know (full detail in the demo plan §6):
- `emit.rs` emitters are test-only; production authors its own SQL (`cumulative.rs`, `execute.rs`, backends). The conformance HOLDS legs prove the wrong copy ≡ full refresh. `emit_column_scoped_merge`'s shape never matched production.
- Keyed temporal locality has **zero** code (`establish_locality`/`KeyedRecurrenceBoundViolated` absent; `key_recurrence` parses, unconsumed). `KeyedForbidsTimeseries` fires unconditionally (`metadata.rs:530-539`).

## Next work, in order

1. **Implement `docs/plans/20260710-emit-unification.md`** — it has an "Execution prompt (for a fresh Claude session)" section at the top; follow it (per-phase implementer + reviewer subagents per `/smelt:implement`, commit + push per phase, update the Progress table).
2. **Rewrite `docs/plans/20260710-web-analytics-maintenance-demo.md`** from DRAFT into a real phased plan (`/smelt:plan`) on the **partition-grain reframe** (§3 decision 3 as reversed; §6 sequencing). Its tutorial phase consumes `--show-sql` from step 1.
3. **Implement the demo plan** (Andrew wants to be involved here).
4. Later, separately: keyed temporal locality spec-first plan (demo becomes its showcase).

## Environment / process notes

- `export DUCKDB_LIB_DIR=~/.local/lib/duckdb LD_LIBRARY_PATH=~/.local/lib/duckdb:$LD_LIBRARY_PATH` before any cargo command (system `/usr/local/lib` does NOT have the lib on this machine).
- Verification gate: `bash .claude/scripts/verify-phase.sh` (`--fast` skips full `cargo test`; run the full gate before marking a code phase done).
- Andrew's orchestration preferences this effort: manage work via subagents; route mechanical implementation to cheaper models (Sonnet/Haiku); Fable forks above medium effort are approved where judgment/context genuinely pays (e.g. delicate spec writing, plan review). Stop and ask for a compact/clear when context becomes the constraint.
