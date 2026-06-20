# Plan: W8 — schema_evolution (D-58)

**Parent (master plan)**: `docs/plans/20260613-spec-impl.md` — the **W8** wave of the
spec-remediation backlog, **schema_evolution** sub-plan. Remediates **D-58** from the
2026-06-13 spec review: NOT NULL column-add reclassification when `backfill:` is present
without `default:`. The decision is already committed to `docs/specs/schema_evolution.md`
§"NOT NULL column-add reclassification"; this wave is **code-only** (no further spec edits
except the close-out KD retraction in P2).

**Date**: 2026-06-20
**Spec**: `docs/specs/schema_evolution.md` §"Change classification" (NOT NULL add row),
§"NOT NULL column-add reclassification" — the correctness oracle.
**Spec diff**: none — the spec already landed in the 2026-06-13 review; the code has not
caught up.
**Tracking branch**: `worktree-spec_review`
**Docs**: code-only. P2 retracts the now-satisfied Known-Divergence entries in
`schema_evolution.md` if applicable and may touch `docs-site/` only if the reviewer
flags a user-facing gap in the backfill/default docs.

---

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then `docs/specs/schema_evolution.md` §"NOT NULL column-add reclassification"
— that section is the correctness oracle; do not re-litigate D-58. Run the next `pending`
phase in the Progress-tracking table (skip `done`/`blocked` rows) using the per-phase
routine below. After the last `pending` phase, flip this sub-plan's Status to `done` in
the master registry (`docs/plans/20260613-spec-impl.md` W8 schema_evolution row) and
commit together. Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>`,
`<<SUBPLAN_ADVANCED>>`, or `<<MASTER_EXHAUSTED>>`.

---

## Goal

The spec says: adding a NOT NULL column (or tightening NULL → NOT NULL) is **Safe** iff at
least one of `default:` or `backfill:` is declared. The current code checks **only**
`column_defaults`; `backfill_exprs`-only is incorrectly classified as **Blocked**
(FullRefresh). Two call sites need to be fixed:

1. `plan_migration_for_backend` in `crates/smelt-state/src/schema_tracking.rs` — the
   `FullRefresh` guard for `AddColumn { nullable: false }` (line ~1290) and for
   `ChangeNullability { to_nullable: false }` (line ~1315).
2. `plan_schema_operations` in the same file — the `full_refresh_reasons` push for NOT NULL
   add without default (line ~902).

When **backfill-only** is present (no `default:`):
- The ADD COLUMN statement must omit the `DEFAULT` clause (there is no SQL default).
- The UPDATE backfill statement must follow immediately.
- The column is still NOT NULL — the backfill expression is trusted to populate all rows.

When **neither** `default:` nor `backfill:` is present, the change must remain Blocked
(FullRefresh required). The spec says "smelt will not silently insert NULL into a NOT NULL
column."

---

## Design decisions (resolved — do not re-litigate)

| Dec | One-line contract (spec is authoritative) |
|-----|------------------------------------------|
| **D-58** | Either `default:` **or** `backfill:` (or both) reclassifies a NOT NULL column add as **Safe**; when **neither** is present, the change is **Blocked** and requires `--allow-full-refresh`. |

---

## Per-phase routine

1. **Pre-flight.** `cargo test -p smelt-state --quiet 2>&1 | tail -40`. If the tree is
   already red on the phase's target test, the bug is confirmed — proceed. If red on
   **unrelated** breakage, treat as a block (§"Block conditions").
2. **Red-green `/smelt:implement`.** Write the failing test(s) first (naming them in the
   phase below), confirm they are red, then implement the fix, confirm green.
   Implementer pass, then reviewer pass (material findings only).
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings);
   `cargo test --quiet 2>&1 | tail -40` green; the dual example gate
   `cargo test -p smelt-cli --test example_diagnostics` +
   `cargo test -p smelt-lsp --test example_workspaces`.
4. **Record + commit.** Set the table row to `done` + date; commit + push tests + impl +
   table with the phase commit message. Emit `<<PHASE_COMPLETE>>` (or roll-up sentinel on
   the last phase).

---

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue)

Set the row to `blocked` with a one-line reason; append a dated entry to §"Blocked phases";
restore the tree to a clean committed state; commit + push; emit `<<PHASE_BLOCKED>>`.
Conditions:

- Pre-flight red on unrelated breakage this phase didn't introduce.
- The spec is genuinely ambiguous for a real case this phase hits (record for human
  review — do not guess).
- The fix cannot be made green without a larger redesign that touches more than
  `schema_tracking.rs` and its tests.

---

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| P1 | `backfill:`-only reclassifies NOT NULL add/tighten as Safe | pending | D-58 | feat(state): backfill-only reclassifies NOT NULL add as Safe (D-58) | |
| P2 | Close-out: KD retraction, master registry, ROADMAP | pending | D-58 close-out | docs(spec-impl): close out W8 schema_evolution (D-58) | |

---

### Phase P1: `backfill:`-only reclassifies NOT NULL add/tighten as Safe

**Goal.** Fix the two call sites in `plan_migration_for_backend` and `plan_schema_operations`
so that the NOT NULL Blocked guard fires **only** when both `column_defaults` and
`backfill_exprs` lack an entry for the column. When backfill-only is present, the ADD COLUMN
statement is emitted without a DEFAULT clause, and the UPDATE backfill statement follows.
When neither is present, FullRefresh is returned as before.

**Critical files.**
- `crates/smelt-state/src/schema_tracking.rs` — two call sites:
  - `plan_migration_for_backend`: the filter closure's `AddColumn { nullable: false }` arm
    (~line 1285) and `ChangeNullability { to_nullable: false }` arm (~line 1310); both
    currently check only `column_defaults.contains_key(name)`.
  - `plan_schema_operations`: the `!nullable && default_expr.is_none()` guard (~line 902)
    that pushes to `full_refresh_reasons`; it must also bail out when `backfill_exprs`
    lacks the key.
  - The DDL emission block inside `plan_migration_for_backend` (~line 1360): the
    `AddColumn` arm already handles the backfill UPDATE — verify the backfill-only branch
    emits `ADD COLUMN … NOT NULL` (no DEFAULT clause) followed by the UPDATE.

**TDD tests to write first** (in `crates/smelt-state/src/schema_tracking.rs` `#[cfg(test)]`):
- `not_null_add_backfill_only_is_safe` — `AddColumn { nullable: false }` + backfill-only
  (no default): asserts `MigrationAction::AlterTable` with exactly two statements: the
  `ADD COLUMN … NOT NULL` (no DEFAULT) and the `UPDATE … SET … = …`.
- `not_null_tighten_backfill_only_is_safe` — `ChangeNullability { to_nullable: false }` +
  backfill-only (no default): asserts `AlterTable` with two statements (UPDATE to fill NULLs,
  then `ALTER COLUMN … SET NOT NULL`). _(Note: the spec says `backfill:` populates rows for
  both add and tighten; if tightening with backfill-only is genuinely ambiguous — see §Design
  "backfill expressions are for recomputing column values from other columns, which is a
  different semantic (used for new column additions, not nullability changes)" at line ~1414
  — the implementer must flag it as a spec question and raise `<<PHASE_BLOCKED>>` rather
  than guessing._
- `not_null_add_neither_default_nor_backfill_is_blocked` — `AddColumn { nullable: false }`,
  empty `defaults` and empty `backfills`: asserts `FullRefresh`.

The existing tests `test_plan_migration_not_null_column_with_default` and
`test_plan_migration_not_null_with_default_and_backfill` must still pass unchanged — they
cover the `default:`-present and both-present cases respectively.

**Spec-mismatch risk.** The spec at §"NOT NULL column-add reclassification" applies the
`default:`/`backfill:` symmetry to both `AddColumn` and `ChangeNullability` (NULL → NOT NULL
tighten). However, the comment in the code at line ~1413 says: "We use `column_defaults`
(not `backfill_exprs`) here because the goal is to fill NULL gaps with a safe constant —
backfill expressions are for recomputing column values from other columns, which is a
different semantic." If the implementer judges that the spec intends `backfill:`-only to
unlock `ChangeNullability` as well (which the classification table row "Change NULL → NOT
NULL | **Blocked** — requires `--allow-full-refresh` unless `default:` and/or `backfill:`
is set" implies), apply it. If the distinction in the code comment makes the correct
behaviour genuinely unclear, flag as `<<PHASE_BLOCKED>>` and note it in §"Blocked phases"
for human resolution.

---

### Phase P2: Close-out

**Goal.** Retract any Known-Divergence note in `docs/specs/schema_evolution.md` that was
tracking the D-58 gap (none currently exists for this specific point, so this phase is
a no-op on the spec). Update the W8 schema_evolution row in the master plan
(`docs/plans/20260613-spec-impl.md`) to `done (<date>)`. Update `docs/ROADMAP.md` to
record W8 schema_evolution as complete.

---

## Deferred

None. D-58 is a narrow, self-contained fix.

---

## Spec mismatch (human triage)

Possible ambiguity: the code comment at `schema_tracking.rs` ~line 1413 argues that
`backfill:` should **not** be used for `ChangeNullability` (only for new-column backfill),
while the spec classification table row "Change NULL → NOT NULL … unless `default:` and/or
`backfill:` is set" implies it should. The implementer should apply the spec as written
unless the argument in the code comment surfaces a genuine correctness concern (e.g.,
`backfill:` expressions reference other columns that may not correctly fill pre-existing
NULLs in a nullability-tightening context). If uncertain, raise `<<PHASE_BLOCKED>>` rather
than guessing.

---

## Blocked phases

_(none)_

---

## Verification

After P1 passes:
- `cargo test -p smelt-state --quiet 2>&1 | tail -40` — all three new tests green; all
  existing `test_plan_migration_not_null_*` tests still green.
- `cargo test --quiet 2>&1 | tail -40` — workspace green.
- `cargo test -p smelt-cli --test example_diagnostics` — no regressions.
- `cargo test -p smelt-lsp --test example_workspaces` — no regressions.
- Reviewer confirms: backfill-only produces no DEFAULT clause in the ADD COLUMN DDL;
  neither-present still returns FullRefresh; both-present still produces DEFAULT + UPDATE.
