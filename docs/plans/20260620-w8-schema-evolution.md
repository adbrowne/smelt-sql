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

When **backfill-only** is present (no `default:`) — **ADD COLUMN** case:
- The ADD COLUMN statement must omit the `DEFAULT` clause (there is no SQL default).
- The UPDATE backfill statement must follow immediately. For a *new* column this UPDATE
  has **no `WHERE`** (every row is freshly added and must be populated) — this matches the
  existing column-add codegen at `schema_tracking.rs` ~line 1379.
- The column is still NOT NULL — the backfill expression is trusted to populate all rows.

When **backfill-only** is present — **nullability-tighten** (`ChangeNullability` NULL → NOT
NULL) case — the column already exists and already holds non-NULL values in most rows:
- The backfill UPDATE **must be scoped `WHERE <col> IS NULL`** so it fills only the NULL
  gaps and does **not** clobber rows that already hold good data. This is the single guard
  that resolves the code-comment hazard (see §"Spec mismatch (human triage)").
- Then emit `ALTER COLUMN <col> SET NOT NULL`.

When **neither** `default:` nor `backfill:` is present, the change must remain Blocked
(FullRefresh required). The spec says "smelt will not silently insert NULL into a NOT NULL
column."

**Fail-loud, not fail-quiet.** The `ChangeNullability { to_nullable: false }` codegen arm
(`schema_tracking.rs` ~line 1413) currently emits the `UPDATE`/`SET NOT NULL` pair **only
inside** `if let Some(default_val) = column_defaults.get(...)`. The moment classification
admits backfill-only as Safe, that arm falls through and emits **nothing** — silently
dropping the NOT NULL constraint. P1 must restructure this arm to fill from `default` if
present, **else** from `backfill` (`WHERE <col> IS NULL`), and **always** emit the
`SET NOT NULL` on the Safe path. A Safe classification that produces no `SET NOT NULL` is a
bug, not a no-op.

**Precedence when both `default:` and `backfill:` are present.** Per the spec
(§"NOT NULL column-add reclassification" — "backfill takes precedence for pre-existing
rows"), the backfill value wins the gap fill. For the tighten case that means: if backfill
is present, fill gaps with the backfill expression (`WHERE <col> IS NULL`); `default:` only
governs the ADD COLUMN default clause, which does not apply to an already-existing column.

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
  - The DDL emission block inside `plan_migration_for_backend`: the `AddColumn` arm
    (~line 1360) already handles the backfill UPDATE — verify the backfill-only branch
    emits `ADD COLUMN … NOT NULL` (no DEFAULT clause) followed by the no-`WHERE` UPDATE.
  - The `ChangeNullability { to_nullable: false }` arm (~line 1413): restructure so it
    fills gaps from `default` if present, **else** from `backfill` with `WHERE <col> IS
    NULL`, and always emits `SET NOT NULL` on the Safe path (see §Goal "Fail-loud, not
    fail-quiet"). Delete or rewrite the stale comment at ~line 1414 that asserts backfill
    does not apply to nullability changes — it does, scoped to the NULL gaps.

**TDD tests to write first** (in `crates/smelt-state/src/schema_tracking.rs` `#[cfg(test)]`):
- `not_null_add_backfill_only_is_safe` — `AddColumn { nullable: false }` + backfill-only
  (no default): asserts `MigrationAction::AlterTable` with exactly two statements: the
  `ADD COLUMN … NOT NULL` (no DEFAULT) and the `UPDATE … SET … = …`.
- `not_null_tighten_backfill_only_is_safe` — `ChangeNullability { to_nullable: false }` +
  backfill-only (no default): asserts `AlterTable` with exactly two statements — the
  gap-scoped `UPDATE … SET <col> = <expr> WHERE <col> IS NULL`, then
  `ALTER COLUMN … SET NOT NULL`. The `WHERE <col> IS NULL` is **load-bearing** (it is the
  guard that resolves the code-comment hazard); assert it is present in the UPDATE string.
- `not_null_tighten_backfill_only_does_not_clobber_existing` — same setup; assert the
  emitted UPDATE is scoped (`WHERE … IS NULL`) and **not** an unscoped
  `UPDATE … SET <col> = <expr>` (which would overwrite already-populated rows). This is the
  regression guard against reusing the column-add (no-`WHERE`) codegen for tightening.
- `not_null_tighten_both_default_and_backfill_uses_backfill_for_gap` — both present:
  assert the gap fill uses the **backfill** expression (precedence), `WHERE <col> IS NULL`,
  then `SET NOT NULL`.
- `not_null_add_neither_default_nor_backfill_is_blocked` — `AddColumn { nullable: false }`,
  empty `defaults` and empty `backfills`: asserts `FullRefresh`.
- `not_null_tighten_neither_default_nor_backfill_is_blocked` —
  `ChangeNullability { to_nullable: false }`, empty `defaults` and empty `backfills`:
  asserts `FullRefresh` (the Blocked path is unchanged).

The existing tests `test_plan_migration_not_null_column_with_default` and
`test_plan_migration_not_null_with_default_and_backfill` must still pass unchanged — they
cover the `default:`-present and both-present cases respectively.

**Spec-mismatch risk — RESOLVED (human triage 2026-06-20, do not re-litigate).** The spec
at §"NOT NULL column-add reclassification" applies the `default:`/`backfill:` symmetry to
both `AddColumn` and `ChangeNullability` (NULL → NOT NULL tighten). The code comment at
~line 1413 ("backfill expressions are for recomputing column values from other columns …
a different semantic") was reviewed and found to flag a **real codegen hazard but the wrong
conclusion**:

- The hazard is concrete: the column-add backfill UPDATE at ~line 1379 has **no `WHERE`**
  (overwrites every row). That is correct for a brand-new column, but reused verbatim on an
  existing column it would clobber already-populated, non-NULL rows.
- The resolution is **not** to exclude backfill from nullability tightening (the spec is
  right that it should unlock the change, and a derived `backfill:` expression — e.g.
  `coalesce(prior, 'unknown')` — is strictly more useful here than a constant `default:`).
  The resolution is to scope the tightening UPDATE `WHERE <col> IS NULL` so it fills only
  the gaps. With that guard, backfill satisfies the constraint without touching good data.

Therefore P1 **applies the spec for both `AddColumn` and `ChangeNullability`** as detailed
in §Goal. Do **not** raise `<<PHASE_BLOCKED>>` for this case — the question is answered.
(Full triage write-up references: classification sites `schema_tracking.rs:1291` and
`:1316`; codegen `:1379` add / `:1413` tighten; spec `schema_evolution.md:137,147`;
decision `docs/research/20260613-spec-remediation-decisions.md:429`.)

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

## Spec mismatch (human triage) — RESOLVED 2026-06-20

**Question.** The code comment at `schema_tracking.rs` ~line 1413 argued that `backfill:`
should **not** be used for `ChangeNullability` (only for new-column backfill), while the
spec classification row "Change NULL → NOT NULL … unless `default:` and/or `backfill:` is
set" says it should.

**Resolution: apply the spec (option A) for both `AddColumn` and `ChangeNullability`.** The
comment's underlying concern — that `backfill:` for a *new* column is an unscoped
`UPDATE SET col = expr` (no `WHERE`) which would clobber existing non-NULL rows if reused
for an *existing* column — is real, but it is a codegen-scoping issue, not a reason to
exclude backfill. The fix is to scope the tightening UPDATE `WHERE <col> IS NULL` (fill only
the NULL gaps). This:

1. Satisfies the NOT NULL constraint (every former-NULL row gets a value).
2. Leaves already-populated rows untouched (no data clobbering).
3. Gives users the more expressive primitive exactly where it matters — tightening fills are
   almost always *derived* (`coalesce(...)`, computed from a sibling column), which a constant
   `default:` cannot express. Forcing `default:` here would push users to a meaningless
   constant or a full table rewrite — the penalty the spec's own design rationale
   (`schema_evolution.md:213,217`) exists to avoid.

**Implication for P1, beyond the classifier change:** the `:1413` codegen arm must be
restructured so it never silently no-ops on the Safe path (it currently emits `SET NOT NULL`
only when a `default:` exists — see §Goal "Fail-loud, not fail-quiet"), and the stale
comment at `:1414` must be deleted/rewritten. No `<<PHASE_BLOCKED>>` for this item.

---

## Blocked phases

_(none)_

---

## Verification

After P1 passes:
- `cargo test -p smelt-state --quiet 2>&1 | tail -40` — all new `not_null_*` tests green
  (add backfill-only, tighten backfill-only, tighten no-clobber, tighten both-precedence,
  add neither-blocked, tighten neither-blocked); all existing `test_plan_migration_not_null_*`
  tests still green.
- `cargo test --quiet 2>&1 | tail -40` — workspace green.
- `cargo test -p smelt-cli --test example_diagnostics` — no regressions.
- `cargo test -p smelt-lsp --test example_workspaces` — no regressions.
- Reviewer confirms: backfill-only ADD produces no DEFAULT clause and a no-`WHERE` UPDATE;
  backfill-only **tighten** produces a `WHERE <col> IS NULL`-scoped UPDATE plus
  `SET NOT NULL` (never a silent no-op, never an unscoped clobbering UPDATE); neither-present
  still returns FullRefresh for both add and tighten; both-present still produces DEFAULT +
  UPDATE for add and a backfill-precedence gap fill for tighten.
