# Plan: Backbuild follow-ups — Tier-3 cases, residuals, generative conformance, doc-sync

**Date**: 2026-08-02
**Spec**: none — research-first, same convention as the predecessor plan
[`docs/plans/20260802-backbuild-synthesis.md`](20260802-backbuild-synthesis.md); the
correctness oracle is
[`docs/research/20260802-backbuild-synthesis.md`](../research/20260802-backbuild-synthesis.md)
(§2 contract, §4 catalogue with case IDs, §6 harness). Spec extraction still happens at
wiring time.
**Spec diff**: n/a (research doc is the oracle)
**Tracking PR / branch**: `spec-redraft-incremental-models` (continues the predecessor's branch)
**Docs**: code+docs — the user guide
[`docs-site/docs/guide/backbuild-synthesis.md`](../../docs-site/docs/guide/backbuild-synthesis.md)
gains the new techniques as they land, and Phase 8 makes its emitted-SQL blocks
generated-and-verified

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to
completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read
   `docs/research/20260802-backbuild-synthesis.md` **completely** — it is the correctness
   oracle. §2 (the contract) and §4 (the catalogue) are normative; do not re-derive or
   re-litigate them. Skim the predecessor plan's "Deferred during implementation" section —
   Phases 1–2 here close items recorded there.
2. Confirm you are on branch `spec-redraft-incremental-models`. If not, ask the user
   before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is
   your starting point. If every phase is `done`, run the post-implementation verification
   under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer
subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating the research doc's contract (§2) or a
  catalogue case's stated proof obligation (§4).
- A research-doc assumption turns out to be wrong — update the research doc first, in the
  same commit, and note it under "Deferred during implementation".
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**

- Red-green TDD: failing test before any implementation.
- Oracle tests run against real DuckDB; multiset equality per research §6 (two-way
  `EXCEPT ALL` + name/type check, via the existing
  `tests/backbuild_conformance/harness.rs` helpers). A case admitting several options
  verifies **every** option independently, each against a fresh copy of the staged
  before-table.
- The module remains deliberately **unwired**: no `smelt-runtime`, `smelt-cli`,
  `smelt-db` changes anywhere in this plan. The `examples/` real-fixture convention stays
  replaced by the DuckDB oracle harness for the same reason as the predecessor.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call; failures-only
  output).
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md`: statement single-ownership
  (emitters author, tests execute), fail-loud named refusals, property verdicts consumed
  from existing `analysis::` outputs (per-expression dependency collection stays an
  admissible leaf classifier).
- **Timeless-oracle rule.** Phase vocabulary lives in this plan file only. User-guide and
  research-doc edits describe behaviour as if it has always existed.

---

## Context

The predecessor plan landed the backbuild substrate (diff → classify → requalify → emit,
research §4 cases A0/B1–B4/B7/D1/D2/E1–E4/F1, DuckDB oracle harness) and recorded
follow-ups in two places: its "Explicitly deferred" section (Tier-3 cases B5/B6/F2,
C-sequencing polish, generative recipe sampling) and its "Deferred during implementation"
section (B1/B3 dual-derivability symmetry, F1 edited-survivor branch matching, the C4
reorder corner). This plan closes those, adds the generative property gate the research
doc's §6 explicitly deferred ("generative recipe sampling … à la `maintenance_conformance`"),
and locks the user guide's emitted-SQL examples to the real emitters. Probe-gated G2 stays
deferred (data-dependent contract; now recorded in `docs/ROADMAP.md` What's Next §3).

## Scope

### In scope (research coverage)

- Research §4 Tier-3 cases: **B5** (aggregate column at unchanged GROUP BY grain),
  **B6** (window column over stored columns), **F2** (removed UNION ALL branch via
  discriminator), **C1 sequencing** (populate the H Drop slot from the diff; flag
  doctrine stays with wiring/schema_evolution).
- Predecessor residuals: **B1/B3 dual-derivability** for added columns (§2 "Options, not
  choices" symmetry); **F1 edited-survivor branch matching** (`set_op_diff`
  generalization) including the **C4 reorder corner** guard.
- **Generative conformance gate**: typed before/after recipe generation over constrained
  staged sources, every derived option (and every bounded composed selection) verified
  against a DuckDB full-refresh oracle, `maintenance_conformance`-style
  (deterministic seeding, skip-on-refusal with an admission floor, structural shrinking).
- **Doc-sync**: the user guide's emitted-script blocks become generated from the real
  pipeline and oracle-verified.

### Explicitly deferred

- **Probe-gated G2** — data-dependent admission is a different contract; lands with
  wiring. Recorded in `docs/ROADMAP.md` What's Next §3 item 7.
- **C1/C2/F3 classification ownership** — `ALTER DROP` opt-in doctrine and type-widening
  stay owned by `docs/specs/schema_evolution.md`; F3 needs expansion + fingerprint at
  wiring. Phase 6 only *sequences* a drop atom; it does not decide whether drops are
  permitted (that flag is wiring's).
- **All wiring** — CLI surface, runtime execution, before-SQL sourcing, ledger
  integration, fingerprint refinement, Spark dialect, the cost model (research §7).
- **Spec + full user-docs promotion** — spec extraction at wiring time, per the
  predecessor's header.
- **Multi-hop enrichment beyond B7** (research §7.7) — refuses with named reasons.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 85bf09d2 | 2026-08-02 |
| 2     | done     |        | 2026-08-02 |
| 3     | pending  |        |      |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |
| 7     | pending  |        |      |
| 8     | pending  |        |      |

---

### Phase 1: F1 edited-survivor branch matching + the C4 reorder guard

**Goal.** Generalize `set_op_diff` so a surviving branch with its own edit is matched
against its old self and per-branch diffed (the predecessor's recorded F1 gap), letting a
"B-class edit inside one branch + appended branch" diff classify through the real
classifier instead of hand-assembled atoms — while closing the C4 corner (a
reorder+append edit over branches with identical name-sets but swapped declared orders
must refuse, not slip a positional divergence past a name-keyed no-op check).

**Pre-conditions.** Predecessor plan complete (it is).

**TDD tests to write first.**
- `crates/smelt-logical/tests/backbuild_diff.rs::edited_survivor_branch_pairs_and_diffs` —
  branch 0 gains a SELECT item, branch 1 unchanged, branch 2 appended ⇒ `SetOpDiff`
  reports one *edited surviving* pair (carrying its own inner diff), one unchanged, one
  added — not "removed + 2 added".
- `::edited_nonfirst_branch_is_reported_as_such` — the edit sits in branch 1; the pair is
  reported with its position so classification can refuse by name.
- `backbuild_conformance.rs::h_composite_branch_add_plus_column_add_classifier_driven` —
  a diff combining a B1 add inside branch 0 with an appended UNION ALL branch drives
  through `derive_backbuild_options` (no hand-assembly): both atoms admitted, `assemble`
  emits ALTER/UPDATE before the branch INSERT, oracle-equal. Retire the hand-assembly and
  its doc comments on the two existing H tests
  (`h_composite_add_plus_insert_aligns_columns`,
  `h_composite_with_blocked_atom_yields_only_full_refresh`) in favour of
  classifier-driven construction.
- `backbuild_options.rs::edited_nonfirst_branch_refuses_named` — an edit inside a
  *non-first* surviving branch ⇒ named refusal (the top-level SELECT/WHERE/skeleton diffs
  describe branch 0 only; classifying another branch's edit from them would be unsound —
  research §4 E trap).
- `::c4_swapped_order_reorder_append_refuses` — before-definition contains two branches
  with identical name-sets/expressions/FROM/WHERE but mutually swapped declared column
  orders; the edit reorders branches and appends a third ⇒ named refusal (the pure-diff
  gate must key on declared column *order*, not name-set — predecessor residual "C4
  contrived corner").

**Implementation shape.** `diff.rs::set_op_diff`: after exact-text multiset matching,
pair leftover before/after branches (positional pairing among survivors) and run the
existing per-branch SELECT/WHERE/skeleton diff on each pair, recording an
`EditedBranch { index, diff }` entry; `classify.rs`: extend
`multi_branch_pure_diff_gate` — admit atoms from an edited branch **only** when the
edited branch is branch 0 and every other pair is exact (then the top-level diff *is*
branch 0's diff); tighten the gate's no-op check from name-set to declared-order
equality (C4). F1's `branch_output_column_names` first-branch order check is unchanged.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/{mod,diff,classify}.rs` — `mod.rs` for the shared
  diff-type definitions (`SetOpDiff`/`SelectListDiff` fields)
- `crates/smelt-logical/tests/{backbuild_diff,backbuild_options,backbuild_conformance}.rs`

**Docs touched.**
- `docs/research/20260802-backbuild-synthesis.md` §4 F1/E-trap — describe the
  edited-survivor matching and the declared-order gate as the current behaviour
  (timeless); delete the predecessor residual's "not yet recognized" framing.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Edited non-first-branch shapes refuse by name — never classified from a
      first-branch diff that doesn't describe them
- [ ] The C4 gate compares declared column order, not name-sets
- [ ] Hand-assembled H tests replaced by classifier-driven construction
- [ ] No scope creep into later phases

**Commit.** `feat(logical): backbuild set-op diff matches edited surviving branches; C4 order guard`

---

### Phase 2: B1/B3 dual-derivability symmetry for added columns

**Goal.** An added column derivable both from stored columns (B1) and from an upstream
pull-through (B3) returns **both** options, restoring §2 "returns every admissible
technique" symmetry with the D-class (`d_dual_derivable_yields_both_options`).

**Pre-conditions.** None beyond the predecessor.

**TDD tests to write first.**
- `backbuild_conformance.rs::b_dual_derivable_added_column_yields_both_options` — added
  column readable both as a stored bare representative and as an upstream column with a
  proven grain link ⇒ the atom carries both `SelfDerivedColumnAdd` and
  `UpstreamPullthrough`; each independently oracle-verified on a fresh table copy.
- `backbuild_options.rs::b_dual_derivable_refusals_still_recorded` — when B3's proof
  fails (e.g. no declared `unique_key`) the B1 option still stands **and** the B3 refusal
  is still recorded on the atom (options and refusals are not mutually exclusive).

**Implementation shape.** `classify.rs::classify_added_column`: attempt B1 and B3
independently instead of `B1 else B3`; merge option lists and inadmissibility records.
The ALTER ADD statement stays identical across both options (same column, same declared
type).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/classify.rs`
- both test files

**Docs touched.**
- `docs/research/20260802-backbuild-synthesis.md` — no change needed (§2 already states
  the rule); predecessor plan's deferred-log entry gets a one-line "closed by
  `docs/plans/20260802-backbuild-followups.md`" note (append-only etiquette: strike
  nothing).

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Both options carried on one atom; each verified independently
- [ ] Refusal records preserved alongside admitted options
- [ ] No scope creep

**Commit.** `feat(logical): backbuild B1/B3 dual-derivability — added columns return every admissible option`

---

### Phase 3: B5 — new aggregate column at unchanged GROUP BY grain

**Goal.** An added aggregate column on a GROUP BY model, admitted when the skeleton is
unchanged and the aggregate's inputs are upstream-available, emitted as a matched-only
column backfill from the after-definition's own skeleton (research §4 B5 — the
re-aggregation carries the full FROM tree and WHERE; no insert arm).

**Pre-conditions.** Phase 2 (added-column classification is option-merging).

**TDD tests to write first.**
- `backbuild_conformance.rs::b5_aggregate_column_backfill` — GROUP BY model gains
  `SUM(o.qty) AS total_qty`; script = ALTER ADD + `UPDATE t SET … FROM (SELECT <keys>,
  <agg> FROM <after skeleton incl. WHERE> GROUP BY <keys>) s WHERE t.<k> = s.<k>`;
  oracle-equal; sibling columns untouched (assert directly).
- `::b5_where_clause_is_carried` — model has a WHERE; staged data includes rows the
  filter drops whose inclusion would change the aggregate ⇒ oracle-equal only because the
  re-aggregation carries the WHERE (the bare re-aggregation over-counts — this is the
  research §4 B5 trap made executable).
- `::b5_update_is_matched_only` — staged upstream contains key groups the stored table
  does not (filtered out before) ⇒ no rows inserted; oracle-equal.
- `backbuild_options.rs::b5_nullable_group_key_refuses` — group keys without a NOT NULL
  proof/declaration ⇒ named refusal (shared key-addressability obligation).
- `::b5_group_key_not_pulled_through_refuses` — the GROUP BY keys are not stored bare
  pull-throughs ⇒ refusal ("no addressable identity").
- `::b5_volatile_aggregate_input_refuses` — `SUM(random())` ⇒ named refusal (after-side
  determinism posture, research §2).
- `::b5_changed_skeleton_refuses` — aggregate add plus a FROM change ⇒ B5 refuses (row-set
  proof), atom lands wherever the skeleton diff sends it.

**Implementation shape.** `classify.rs`: B5 admission for an added SELECT item whose
expression is a registry-recognized aggregate call over upstream-resolvable inputs, on a
GROUP BY model with unchanged skeleton; group-key addressability = every GROUP BY key has
a stored bare pull-through (uniform representative rule) + shared NOT NULL obligation.
`emit.rs`: derived-subquery variant of the Phase-5 update-from emitter
(`emit_column_backfill_update_from_subquery`) — source is `(<keys + new agg> over the
after-definition's FROM/WHERE, GROUP BY keys)`, matched on the keys, **no** insert arm.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/{classify,emit}.rs`
- both test files

**Docs touched.**
- `docs-site/docs/guide/backbuild-synthesis.md` — new tour section "Add an aggregate at
  the model's own grain" (before/after + emitted script; timeless), Current-scope table
  row updated.
- `docs/research/20260802-backbuild-synthesis.md` §4 B5 — drop the "Tier 3" tag; record
  anything learned.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Re-aggregation source carries the model's full FROM tree and WHERE
- [ ] Matched-only (no insert arm); sibling-untouched asserted
- [ ] Aggregate recognition is registry-backed, fail-closed on unknown functions
- [ ] User-guide section timeless
- [ ] No scope creep

**Commit.** `feat(logical): backbuild B5 — aggregate column backfill at unchanged GROUP BY grain`

---

### Phase 4: B6 — new window-function column over stored columns

**Goal.** An added window-function column whose window reads only stored columns becomes
a self-read backfill keyed on row identity (research §4 B6):
`UPDATE t SET c = s.c FROM (SELECT <id>, <window> AS c FROM t) s WHERE t.<id> = s.<id>`.

**Pre-conditions.** Phase 3 (derived-subquery update-from emitter exists).

**TDD tests to write first.**
- `backbuild_conformance.rs::b6_row_number_over_stored_columns` — added
  `ROW_NUMBER() OVER (PARTITION BY status ORDER BY order_id) AS rn`; declared
  `row_identity`; script = ALTER ADD + self-read update-from; oracle-equal; siblings
  untouched.
- `::b6_window_respects_stored_row_set` — model WHERE filters rows; the window computes
  over the *stored* rows only, matching the rebuild (self-read means this holds by
  construction — the test pins it).
- `backbuild_options.rs::b6_no_row_identity_refuses` — no declared `row_identity` and none
  derivable ⇒ named refusal.
- `::b6_nullable_identity_refuses` — identity columns without a NOT NULL
  proof/declaration ⇒ refusal (key addressability).
- `::b6_window_reading_unstored_column_refuses` — window references an input with no
  stored bare representative ⇒ refusal (uniform representative rule).
- `::b6_nondeterministic_order_refuses` — `ROW_NUMBER() OVER (PARTITION BY status)` with
  no ORDER BY (or an ORDER BY that does not total-order within partitions is out of
  scope to prove — refuse only the missing-ORDER BY shape by name) ⇒ named refusal: an
  underdetermined window can never be proven equal to a rebuild's draw.

**Implementation shape.** `classify.rs`: B6 admission for an added item that is a window
call (`OVER` present) whose partition/order/argument references all resolve to stored
bare representatives; requires `row_identity` + NOT NULL; refuses windows without an
ORDER BY (rank-family nondeterminism, fail-closed). `emit.rs`: self-read variant of the
derived-subquery update-from (source subquery reads `t` itself). The predecessor's
blanket "window in added column refuses" narrows to "refuses unless B6's proof admits".

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/{classify,emit}.rs`
- both test files

**Docs touched.**
- `docs-site/docs/guide/backbuild-synthesis.md` — tour section "Add a window column";
  refusal-taxonomy row for underdetermined windows; Current-scope update.
- `docs/research/20260802-backbuild-synthesis.md` §4 B6 — drop "Tier 3"; record the
  ORDER-BY determinism obligation (it is implied by §2 but B6-specific).

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Self-read only — no upstream read anywhere in the B6 path
- [ ] Missing-ORDER BY windows refuse by name (determinism fail-closed)
- [ ] Identity obligations (declared + NOT NULL) enforced
- [ ] User-guide section timeless
- [ ] No scope creep

**Commit.** `feat(logical): backbuild B6 — window-function column via self-read backfill`

---

### Phase 5: F2 — removed UNION ALL branch via discriminator DELETE

**Goal.** A removed UNION ALL branch whose rows are distinguishable in the stored table by
a discriminator (constant column distinct per branch, via `analysis/discriminants.rs`)
becomes `DELETE FROM t WHERE <discriminator predicate>`; otherwise a named refusal
(research §4 F2).

**Pre-conditions.** Phase 1 (branch matching reports removed branches cleanly even
alongside survivors).

**TDD tests to write first.**
- `backbuild_conformance.rs::f2_discriminated_branch_delete` — three-branch UNION ALL,
  each branch carrying a distinct constant `'a'/'b'/'c' AS src`; middle branch removed ⇒
  script is a single `DELETE FROM t WHERE src = 'b'`; oracle-equal.
- `::f2_duplicate_payload_rows_survive_correctly` — the removed branch's rows coincide
  in payload (all columns except the discriminator) with a surviving branch's rows ⇒
  only the discriminated copies are deleted (multiset counts right — the reason a
  content-matching DELETE would be unsound and the discriminator is required).
- `backbuild_options.rs::f2_no_discriminator_refuses` — branches without a distinguishing
  constant ⇒ named refusal ("no provenance predicate").
- `::f2_shared_discriminator_refuses` — two branches carry the *same* constant ⇒ refusal
  (the predicate would also delete a surviving branch's rows).
- `::f2_nonconstant_discriminator_refuses` — the candidate column is a non-constant
  expression in some branch ⇒ refusal (fail-closed; only constant-per-branch
  discriminators are provable from the definitions alone).
- `::f2_plain_union_refuses` — `UNION` (dedup) ⇒ refusal (existing posture extends to
  removal).

**Implementation shape.** `classify.rs`: on a removed-branch diff, consult the
discriminant analysis (`analysis/discriminants.rs`) over the **before**-definition's
branches: a column that is a distinct literal constant in every branch discriminates; the
removed branch's constant builds the predicate. Equality predicate (`src = 'b'`), not
`IS NOT TRUE` — the discriminator is a proven non-NULL literal on exactly the target
rows. `emit.rs`: `emit_discriminated_branch_delete`. H slot: `Delete`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/{classify,emit}.rs`
- both test files

**Docs touched.**
- `docs-site/docs/guide/backbuild-synthesis.md` — tour section "Remove a UNION ALL
  branch" (including the actionable refusal: "add a constant discriminator column to make
  branch removal targetable"); Current-scope update.
- `docs/research/20260802-backbuild-synthesis.md` §4 F2 — drop "Tier 3"; record the
  duplicate-payload rationale.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Discriminance consumed from `analysis/discriminants.rs`, not re-derived ad hoc
- [ ] Duplicate-payload multiset case covered (the soundness argument for requiring a
      discriminator)
- [ ] Refusals actionable where a small model edit would admit
- [ ] User-guide section timeless
- [ ] No scope creep

**Commit.** `feat(logical): backbuild F2 — removed UNION ALL branch via discriminator DELETE`

---

### Phase 6: C1 sequencing — populate the H Drop slot

**Goal.** A dropped column (surviving rename pairing) classifies as a C1 atom carrying an
`ALTER TABLE … DROP COLUMN` option in the H Drop slot — drops run last, after every
statement that might read the column — while classification *policy* (whether drops are
allowed at all, `--allow-column-removal`) stays with wiring/schema_evolution. Option
metadata marks the technique destructive so wiring can gate it.

**Pre-conditions.** Phases 1–5 (composites exercise the full slot order).

**TDD tests to write first.**
- `backbuild_conformance.rs::c1_dropped_column_drops_last` — composite diff: B2 rename +
  B1 add reading the renamed column + C1 drop of a *different* column ⇒ statements in H
  order with the DROP strictly last; oracle-equal.
- `::c1_drop_of_column_read_by_update_sequences_safely` — a D1 rewrite whose
  representative is a column that is *also* dropped in the same diff ⇒ the drop must not
  precede the update; assert order and oracle-equality. (If the uniform representative
  rule already excludes this shape — a dropped column is not "unchanged between both
  definitions" — the test instead pins the refusal by name; the implementer verifies
  which and documents it in the test.)
- `backbuild_options.rs::c1_option_marked_destructive` — the C1 option's metadata records
  a destructive write scope distinct from column-scoped updates (so a wiring-time gate
  can require the opt-in flag without re-classifying).
- `::c1_rename_pairing_still_wins` — a dropped column whose expression matches an added
  one still classifies B2, never C1+B1 (regression guard on pairing order).

**Implementation shape.** `classify.rs`: leftover `select_list.dropped` entries (post
rename pairing) become `AtomicChange::DroppedColumn { name }` atoms with a
`Technique::ColumnDrop` option, `HSlot::Drop`, and destructive metadata. `emit.rs`:
`emit_alter_drop_column`. `mod.rs`: `assemble` already carries the Drop slot; this phase
populates it and extends the slot-order test coverage.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/{mod,classify,emit}.rs`
- both test files

**Docs touched.**
- `docs-site/docs/guide/backbuild-synthesis.md` — tour "Several changes at once" gains
  the drop-last example; a note that drop execution is policy-gated at run time.
- `docs/research/20260802-backbuild-synthesis.md` §4 C1 — record that backbuild sequences
  the drop and marks it destructive; policy ownership unchanged.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Drop strictly last in every composite; interplay with rename pairing pinned
- [ ] Destructive metadata present; no policy decision (flag doctrine) smuggled into the
      pure module
- [ ] User-guide edits timeless
- [ ] No scope creep

**Commit.** `feat(logical): backbuild C1 sequencing — dropped columns populate the H Drop slot`

---

### Phase 7: Generative conformance — property-based backbuild oracle gate

**Goal.** The property the whole module claims, made generative: **for any generated
model and any generated edit, every backbuild option the classifier derives — and every
bounded composed selection — applied to a real DuckDB, is multiset-equal to a full
rebuild of the after-definition over the same staged inputs.** Refusals are skipped but
counted; an over-refusing classifier fails the gate loudly.

**Pre-conditions.** Phases 1–6 (generators cover the full admitted catalogue).

**TDD tests to write first.** (Red state = the test target compiles and the harness smoke
case runs before any generator breadth exists; breadth lands with the generators.)
- `crates/smelt-logical/tests/backbuild_property.rs::generated_options_match_full_rebuild_oracle`
  — the main gate: N seeded cases (default 24, `SMELT_BACKBUILD_CASES` override), each
  drawing a `BeforeRecipe` + 1–3 `EditRecipe`s over the fixed source pool; stage sources
  with generated data; parse both renders; `definition_diff` →
  `derive_backbuild_options`; verify the `FullRefresh` baseline once, then every
  admissible composed selection in the bounded per-atom option product (cap the product;
  `log`/count what the cap drops — no silent truncation) via `harness::verify_script`
  against a fresh staged copy; when any atom's option set is empty assert `assemble`
  returns no targeted script and every refusal carries a non-empty atom + reason.
- `::admission_rate_stays_above_floor` — over the seeded run, the fraction of cases
  yielding at least one targeted option stays above a floor (mirroring
  `maintenance_conformance`'s generator-health guard) and a per-technique coverage tally
  shows every Technique variant exercised at least once at the default case count —
  a generator whose edits never admit (or never reach a technique) fails loudly, not
  silently green.
- `::adversarial_edits_always_refuse_or_verify` — the adversarial edit axis (grain
  change, INNER JOIN add, volatile function, opaque rewrite) never crashes and never
  yields an unverified script: each case either refuses by name or its script passes the
  oracle. (This is the "we never create a wrong plan" property from the other side.)
- `::stale_upstream_documents_precondition_generatively` — one deterministic case per
  run mutates a staged source between `build_before` and script application for an
  upstream-reading option and asserts the *documented divergence* (research §2) — the
  precondition's edge stays tested at the property level, phrased as "diverges exactly
  when the contract says it may".

**Implementation shape.**
- `proptest` joins `smelt-logical`'s `[dev-dependencies]` (it is not there today).
- New test target `crates/smelt-logical/tests/backbuild_property.rs` with modules under
  `crates/smelt-logical/tests/backbuild_property/` (`recipe.rs`, `render.rs`, `data.rs`),
  reusing `#[path = "backbuild_conformance/harness.rs"] mod harness;` for the oracle.
- **Fixed source pool** (the constraint the generators live inside, so staged data always
  exists and declared facts are true by construction): e.g. `src_orders(order_id INTEGER
  NOT NULL /*unique*/, customer_id INTEGER, amount INTEGER, qty INTEGER, status VARCHAR,
  ts DATE NOT NULL)`, `src_dim(dim_id INTEGER NOT NULL /*unique*/, region VARCHAR,
  score INTEGER)`, `src_dim2(zone_id INTEGER NOT NULL /*unique*/, zone VARCHAR)` for B7
  chains. `BackbuildInputs.sources`/`not_null_columns` are derived from the same pool
  constants that emit the DDL — one table, two views, no drift.
- **Data generation** (`data.rs`): bounded ints/dates; deliberate NULLs in every nullable
  column (three-valued-logic coverage for E-class), dim keys with zero and multiple fact
  matches (LEFT JOIN NULL-extension), duplicate payload rows (multiset sensitivity),
  rows on predicate boundaries (E4 widening).
- **`BeforeRecipe`** (structural, shrinkable): projection over one source (bare
  pull-throughs + optional derived columns), optional WHERE conjunct set, optionally one
  of {GROUP BY over pulled-through keys, DISTINCT}, optional pre-existing LEFT JOIN,
  optional second UNION ALL branch with constant discriminator. **`EditRecipe`** mirrors
  the technique axis: AddSelfDerived / AddPullthrough / AddViaNewLeftJoin / AddAggregate /
  AddWindow / Rename / RewriteSelfDerived / RewriteFromUpstream / TightenFilter /
  LoosenFilter / WidenHorizon / AddBranch / RemoveBranch / DropColumn, plus the
  adversarial axis; edits that don't type against the drawn `BeforeRecipe` (e.g.
  AddAggregate on an ungrouped model) re-draw structurally rather than emitting invalid
  SQL. Both rendered by `render.rs` — proptest shrinks the *recipe*, never SQL text
  (the testkit's structural-shrinking convention).
- **Loop idiom**: `TestRunner::deterministic()` + `Strategy::new_tree(...).current()` in
  a plain `for` loop (the `maintenance_conformance` pattern), env-overridable case count,
  `proptest-regressions/` seeds checked in as they appear.
- This gate becomes a standing verification entry (tail section) and is cited from the
  research doc §6 as the generative counterpart of the explicit BB-case suite.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/Cargo.toml` — `proptest` dev-dependency
- `crates/smelt-logical/tests/backbuild_property.rs` + `tests/backbuild_property/*.rs`
- `docs/research/20260802-backbuild-synthesis.md` §6 — harness description gains the
  generative gate (timeless)

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Determinism: seeded runner, no wall-clock/randomness outside proptest
- [ ] Every offered option/selection verified on a fresh staged copy; capped products
      logged, never silent
- [ ] Admission floor + per-technique coverage guard present (no vacuously green gate)
- [ ] Declared facts (unique keys, NOT NULL) true by construction of the staged DDL —
      the generator cannot lie to the classifier
- [ ] Harness executes statements, never authors them
- [ ] No scope creep (no wiring, no smelt-runtime/cli/db changes)

**Commit.** `test(logical): backbuild generative conformance gate — every derived option oracle-equal on DuckDB`

---

### Phase 8: Doc-sync — the user guide's emitted SQL is generated and verified

**Goal.** Every emitted-script SQL block in
`docs-site/docs/guide/backbuild-synthesis.md` is produced by the real
`definition_diff → derive_backbuild_options → assemble` pipeline from the guide's own
before/after SQL, byte-compared by a standing test, and oracle-verified against DuckDB —
so the guide cannot drift from the emitters.

**Pre-conditions.** Phases 1–6 (guide covers the full catalogue this plan admits).

**TDD tests to write first.**
- `crates/smelt-logical/tests/backbuild_docs.rs::doc_examples_match_emitters` — for every
  marked example in the guide (`<!-- backbuild-example(<id>): before | after | script -->`
  HTML markers around the existing fenced blocks): parse the doc's before/after blocks,
  build the example's `BackbuildInputs` from an in-test registry keyed by `<id>`, derive
  options, `assemble` the registry-named selection, and assert the statements equal the
  doc's script block byte-for-byte (modulo the fence itself). Failure output names the
  example id and prints the regeneration hint.
- `::doc_examples_pass_the_oracle` — each example also stages its registry fixture data
  and runs `harness::verify_script` — the doc's scripts are *proven*, not just
  emitter-equal.
- `::every_script_block_is_marked` — the guide contains no emitted-script fenced block
  outside a marker (grep-style sweep of the file within the test) — a new hand-written
  example cannot silently bypass the gate.
- Regeneration mode: `SMELT_REGEN_DOCS=1 cargo test -p smelt-logical --test
  backbuild_docs` rewrites every marked script block in place from the emitters (the
  test then passes trivially); documented in a comment at the top of the test file.

**Implementation shape.** `backbuild_docs.rs` locates the guide via
`concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs-site/docs/guide/backbuild-synthesis.md")`.
A small extractor pulls `(id, role, fenced-block)` triples from the markers; the
per-example registry (inputs + fixture DDL/data + selection) lives in the test file.
Guide edit: add the markers to every existing before/after/script block (invisible in
rendered MkDocs output), fix any block the test proves wrong, and regenerate. New
sections from Phases 3–6 are marked as they exist by now. Prose stays hand-written; only
fenced SQL inside `script` markers is generated.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/tests/backbuild_docs.rs`
- `docs-site/docs/guide/backbuild-synthesis.md` — markers + regenerated script blocks

**Docs touched.**
- `docs-site/docs/guide/backbuild-synthesis.md` — as above; "Why you can trust the
  scripts" section gains one timeless sentence: the SQL shown on this page is generated
  by smelt's own emitters and verified against DuckDB by the test suite.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Every script block marked; sweep test prevents unmarked additions
- [ ] Doc examples oracle-verified, not only string-compared
- [ ] Regeneration mode writes only inside markers (no prose clobbering)
- [ ] Guide edits timeless
- [ ] No scope creep

**Commit.** `docs(site): backbuild guide SQL is emitter-generated and oracle-verified; doc-sync gate`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the plan is satisfied at the end:

- `cargo test -p smelt-logical --test backbuild_conformance --test backbuild_diff --test backbuild_options` — explicit catalogue coverage including the new B5/B6/F2/C1 and composite cases.
- `cargo test -p smelt-logical --test backbuild_property` — the generative gate: seeded recipes, every derived option oracle-equal, admission floor + technique coverage green. Deeper local run: `SMELT_BACKBUILD_CASES=200 cargo test -p smelt-logical --test backbuild_property`.
- `cargo test -p smelt-logical --test backbuild_docs` — user-guide SQL matches the emitters and passes the oracle.
- `bash .claude/scripts/verify-phase.sh` — full pre-commit gate.
- `cargo tree -p smelt-logical` — still no production dependency on `smelt-fingerprint`, `smelt-db`, or `smelt-planner`.
- `docs/ROADMAP.md` What's Next §3 item 7 records probe-gated G2; research doc §4/§6 updated with everything learned (same commits as the learning).
