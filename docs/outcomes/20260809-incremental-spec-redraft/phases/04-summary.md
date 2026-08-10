# Phase 4 summary — shape profiles redraft + composed key+time corner table

**Shipped:**
- `docs/specs/incremental_models.md` §"Shape profiles" through §"Interactions" (spec lines
  ~1195–1618) redrafted from 657 to 424 lines (35% cut). All 29 pre-existing headings preserved
  verbatim; `#### What the composed shape enables` absorbed into a single composition table
  under `#### Key temporal locality (the time-partitioned output)`.
- The composed key+time corner is now exactly one table: capability rows (merge-target scan
  pruning, propagation admissibility, key→partition dirt projection, no-op write elimination,
  settle × observed-delta composition, consumer-visible output shape) × bare-keyed/composed
  columns, replacing three overlapping prose sections.
- `phases/04-claims.md`: 184-claim inventory of the pre-redraft range.
- `phases/04-check.sh`: 8 automated checks (structure, orphan-refs, claim fixture, diagnostic
  codes, budget, timeless grep, no-split-code-spans, one-composition-table) — all green.
- Fixed 6 dangling `§"What the composed shape enables"` citations elsewhere in the same file
  (§Overview, §Design ×2, the graph-layer section, §Surface, §Known Divergences) to point at
  `§"Key temporal locality (the time-partitioned output)"` — a real cross-reference defect the
  phase-1 outline's blast-radius check missed (it found only a historical `docs/plans/` citer).
- `bash .claude/scripts/verify-phase.sh` full gate green.

**Decisions:**
- Budget set to 424 lines, not the plan's 330 (itself already a deviation from the phase-1
  outline's 305). With all 11 diagnostic codes and all 29 headings required verbatim, 330 was
  not reachable without cutting claims outright. Rationale and the exact restore list are
  recorded as a comment in `04-check.sh` at the budget check.
- First redraft pass hit 126/184 "preserved" on adversarial verify (56 weakened, 2 lost, 0
  diagnostic codes missing). Restored: both outright losses (batch-unit rendering note, the
  "only proofs prune" governing principle); the highest-value weakenings (window-functions
  escape-hatch name, the `Bounded(c, before, after)` readout row + formula, the CTE/body-structure
  cross-reference, the declared-shape exclusivity sentence, `CREATE TABLE AS SELECT` + read-in-full
  detail, the `[event_ts, event_ts + H]` interval notation). Left dropped: purely illustrative
  worked examples and rationale color (the `GREATEST` idempotence example, the
  `session_start_date` skew specifics, the `(updated_at, source_seq)` tie-free example) — per the
  craft doc's own guidance that per-section throwaway examples cost more context than they buy,
  and consistent with cutting restatement/history rather than rule strength.

**For the next planner:**
- The dangling-heading-reference defect found and fixed here (§"What the composed shape
  enables") suggests re-running a `§"…" `-citation sweep over the *whole* file once phases 5–7
  land, not just the range each phase edits — a heading removed in one phase can be cited from a
  section another phase owns.
- 34 of the original 56 "weakened" claims were left weakened by design (illustrative examples,
  restated rationale). Not restored, and not expected to be — flagging only so a future
  completeness pass doesn't treat the claims file as 100%-preserved-or-bust.
- File total is now 2619 lines. The phase-1 outline's ≤1800 target still requires phases 5–6
  (Design/Constraints/Known Divergences/References) to carry more slack than originally planned,
  as phase 4's own decision log already flagged before this phase ran.

**Gates:**
- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/04-check.sh` → 8/8 PASS.
- Adversarial claim verification (independent subagent): 126/184 preserved on first pass, 0
  diagnostic codes missing; after restores, all outright losses and highest-value weakenings
  fixed (see Decisions).
- `bash .claude/scripts/verify-phase.sh` → PASS (fmt, clippy, full `cargo test`,
  `example_diagnostics`), run with `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH`/`LIBRARY_PATH` set to
  `~/.local/lib/duckdb`.
