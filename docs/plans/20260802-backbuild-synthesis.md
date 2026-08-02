# Plan: Backbuild synthesis — optimized migration scripts from model-definition diffs

**Date**: 2026-08-02
**Spec**: none — research-first by explicit decision; the correctness oracle is
[`docs/research/20260802-backbuild-synthesis.md`](../research/20260802-backbuild-synthesis.md)
(the contract in §2, the catalogue in §4). Spec extraction happens at wiring time, after the
implementation has proven the shapes.
**Spec diff**: n/a (research doc is new)
**Tracking PR / branch**: TBD — new branch off `main` at execution time
**Docs**: code-only — nothing user-visible ships (no CLI, no runtime wiring); user docs and
spec land with the wiring plan

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to
completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read
   `docs/research/20260802-backbuild-synthesis.md` **completely** — it is the correctness
   oracle. §2 (the contract) and §4 (the catalogue, with case IDs A0/B1/…/H used throughout
   this plan) are the normative reference; do not re-derive or re-litigate them.
2. Confirm you are on the tracking branch. If not, ask the user before continuing.
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
- Oracle tests run against real DuckDB (the crate's existing `duckdb` dev-dependency);
  multiset equality per research §6 "Conformance harness". A case admitting several
  options verifies **every** option independently, each against a fresh copy of the
  staged before-table.
- The template's real-fixture `examples/` convention is deliberately replaced by the
  DuckDB oracle harness: the module is unwired, so no example workspace can exercise it;
  example coverage arrives with wiring.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call; failures-only
  output) — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md`: statements are emitter-authored in
  `smelt-logical` only (statement single-ownership); classification is fail-closed with
  named refusals (fail-loud); property verdicts are consumed from existing `analysis::`
  outputs, never re-derived by ad hoc scans over raw SQL (property-composition-walk rule —
  per-expression dependency collection is an admissible leaf classifier).
- This capability is deliberately **unwired**: no `smelt-runtime`, `smelt-cli`, `smelt-db`
  changes anywhere in this plan.

---

## Context

Between "fingerprint-equal ⇒ reuse" and "changed ⇒ full refresh" sits a class of model edits
reachable by targeted scripts (research §0). This plan builds the pure derivation:
`(before CST, after CST, BackbuildInputs) → BackbuildOptions → statement strings`, as a
standalone `smelt-logical` module with DuckDB oracle-equivalence tests, priority-ordered per
research §5 (one deliberate regrouping: D2 is pulled forward into Phase 5 to ride B3's
machinery — research §4 D2 names them one admission path). Classification returns **every** admissible technique per atomic change
(research §2 "Options, not choices") — there is no cost model and no chooser in this plan;
callers select, and tests verify each option independently. Wiring (CLI verb,
virtual-environment acceleration, maintained-model ledger integration, Spark dialect, the
cost model) is explicitly out of scope.

## Scope

### In scope (research coverage)

- Research §4 cases: A0, B1, B2, B3, B4, B7, D1, D2, E1, E2, E3 (delivered as E1+E2
  composition; non-factorable rewrites refuse), E4, F1; G-class and CTE-change refusals;
  H composite ordering.
- Option enumeration per research §2 "Options, not choices": per-atom option sets, the
  always-present model-level `FullRefresh` baseline, `assemble(options, selection)`. No
  cost model, no chooser.
- Research §6 architecture: `backbuild/{mod,diff,classify,requalify,emit}.rs` plus the
  `backbuild_conformance` oracle harness.

### Explicitly deferred

- **B5, B6, F2, C-sequencing polish, probe-gated G2** — Tier 3 (research §5); land in a
  follow-up once the substrate has proven itself.
- **C1 / C2 / F3** — `ALTER DROP` and type-widening classification (and their opt-in flag
  doctrine) stay owned by `docs/specs/schema_evolution.md`; F3 (ref repoint) is only
  decidable with expansion + fingerprint at the wiring layer. Until then, a diff whose only
  treatment would be one of these yields `FullRefresh`-only, and the H drop/type slots
  exist in `assemble` but stay unpopulated in this plan.
- **All wiring** — CLI surface, runtime execution, `.smelt/` before-SQL sourcing, ledger
  integration, fingerprint refinement of the no-op judgement, Spark dialect variants
  (research §7 items 4–6).
- **Spec + user docs** — extracted at wiring time per the header.
- **Generative recipe sampling** — explicit BB-case tests only; testkit-style generation
  follows the `maintenance_conformance` precedent later.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | pending  |        |      |
| 2     | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |
| 7     | pending  |        |      |
| 8     | pending  |        |      |
| 9     | pending  |        |      |

---

### Phase 1: Diff foundation — `DefinitionDiff` over before/after CSTs

**Goal.** Parse both definitions and factor their difference into the structured
`DefinitionDiff` (research §6): SELECT-list diff with trivia-insensitive expression
comparison, WHERE conjunct-set diff, skeleton comparison, UNION ALL branch diff. Includes
the A0 whole-definition no-op verdict and the conservative CTE posture (unchanged `WITH`
prefix diffs the final SELECT; changed CTE → an explicit opaque marker on the diff that
classification refuses on).

**Pre-conditions.** None (first phase).

**TDD tests to write first.**
- `crates/smelt-logical/tests/backbuild_diff.rs::a0_formatting_only_is_noop` — whitespace,
  comments, and case-preserving reformat between versions ⇒ `DefinitionDiff::is_noop()`.
- `::added_column_detected` — one added SELECT item lands in `select_list.added` with its
  `Expr`; unchanged items land in `unchanged`.
- `::dropped_and_changed_columns_detected` — a removed item and an edited expression are
  reported separately; the edited pair carries both `Expr`s.
- `::expression_change_is_trivia_insensitive` — reformatting one expression does **not**
  report a change; an actual token change does.
- `::where_conjunct_diff_added_and_removed` — `WHERE a AND b` → `WHERE a AND c` yields
  removed `{b}`, added `{c}`; a non-conjunctive rewrite (`a OR b` → `a`) yields
  `ConjunctDiff::Opaque`.
- `::skeleton_join_add_detected` — added LEFT JOINs with otherwise-unchanged FROM yield
  `SkeletonDiff::AddedLeftJoins` (a list, one entry per added join); a changed join
  condition yields `SkeletonDiff::Changed`.
- `::union_branch_diff` — an added UNION ALL branch is isolated; reordered identical
  branches are unchanged.
- `::changed_cte_is_opaque` — an edit inside a CTE body yields the opaque/refuse-carrying
  variant; an unchanged `WITH` prefix with an edited final SELECT diffs normally.

**Implementation shape.** `backbuild/diff.rs`:
`pub fn definition_diff(before: &File, after: &File) -> DefinitionDiff` (`smelt_parser::File`,
the parser's root AST type). Trivia-insensitive comparison =
token-text sequence equality skipping trivia kinds — one helper
`fn same_modulo_trivia(a: &SyntaxNode, b: &SyntaxNode) -> bool` used by every clause
comparator. Conjunct split at top-level `AND` only. Pure data out; no classification here.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/mod.rs` — module scaffold, `DefinitionDiff` types
- `crates/smelt-logical/src/backbuild/diff.rs` — the diff
- `crates/smelt-logical/src/lib.rs` — `pub mod backbuild;`
- `crates/smelt-logical/tests/backbuild_diff.rs` — tests

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Diff is purely syntactic — no admission judgements smuggled in
- [ ] Fail-closed: every unrecognised shape lands in an `Opaque`/`Changed` variant, never
      silently "unchanged"
- [ ] No scope creep into classification (Phase 3+)

**Commit.** `feat(logical): backbuild diff foundation — CST-level DefinitionDiff over model versions`

---

### Phase 2: Option enumeration, refusals, and the DuckDB conformance harness

**Goal.** The option-set data model (research §2 "Options, not choices"):
`BackbuildOptions` with per-atom option sets and named inadmissibility records, the
always-present model-level `FullRefresh` baseline, `assemble(options, selection)` with the
H-ordering slots (research §4H); G-class and CTE-change refusals; the reusable oracle
harness. End state: A0 yields an empty targeted script plus the verified `FullRefresh`
baseline; a grain change yields `FullRefresh`-only with a named refusal.

**Pre-conditions.** Phase 1 (`DefinitionDiff`).

**TDD tests to write first.**
- `crates/smelt-logical/tests/backbuild_conformance.rs::harness_smoke_a0_noop` — harness
  helpers (`stage_inputs`, `build_before`, `verify_option`, `assert_matches_full_rebuild`)
  work end-to-end on a real DuckDB: the A0 case's targeted script is empty (table already
  equals the full rebuild) and the `FullRefresh` baseline option is present and
  oracle-verified.
- `::g1_group_by_change_refuses` — GROUP BY key added ⇒ named refusal on the atom; the
  model's only option is `FullRefresh`.
- `::g1_distinct_toggle_refuses`, `::g2_join_condition_change_refuses`,
  `::changed_cte_refuses` — named reasons; `FullRefresh`-only.
- `crates/smelt-logical/tests/backbuild_options.rs::atom_without_options_leaves_only_full_refresh`
  — given a **hand-constructed** `BackbuildOptions` value (one atom carrying an option,
  one with an empty option set), `assemble` offers **no** composed targeted script
  (partial application never offered) and `FullRefresh` remains the only model option.
  Hand-construction keeps this phase off Phase 3's admission logic; classification-driven
  coverage of the same rule arrives with Phase 3's first admitted case.

**Implementation shape.** `backbuild/mod.rs`:
`BackbuildOptions { atoms: Vec<AtomAnalysis> }`,
`AtomAnalysis { change: AtomicChange, options: Vec<BackbuildOption>, inadmissible: Vec<BackbuildRefusal> }`,
`BackbuildOption` (technique variant + statement data + the §2 option metadata: write
scope `none`/`column-scoped`/`row-subset`/`full-write`, `reads_upstream`, statement count,
`rerun_safe`; variants filled in by later phases),
`BackbuildRefusal { atom: String, reason: String }`. `classify.rs`:
`pub fn derive_backbuild_options(diff: &DefinitionDiff, inputs: &BackbuildInputs) -> BackbuildOptions`
(this phase: refusal paths, empty-diff ⇒ no atoms, `FullRefresh` baseline).
`assemble(&BackbuildOptions, &Selection) -> Vec<String>` (this phase: empty and
`FullRefresh` paths only). `BackbuildInputs` per research §3. Harness helpers in
`tests/backbuild_conformance/harness.rs` (or a `#[path]` shared module): `verify_option`
applies one option's script to a fresh copy of the before-table; multiset equality via
two-way `EXCEPT ALL` plus column name/type check, per research §6.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/{mod,classify}.rs`
- `crates/smelt-logical/tests/backbuild_conformance.rs` (+ harness module)
- `crates/smelt-logical/tests/backbuild_options.rs`

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Refusals carry atom + named reason (fail-loud); an empty option set leaves
      `FullRefresh` as the only model option — no silent fallback, no partial script
- [ ] Every option carries its own proof path; no chooser/cost logic anywhere
- [ ] Harness asserts multiset + schema equality exactly as research §6 specifies, per
      option on a fresh table copy
- [ ] Options are pure data; harness executes statements, never authors them
- [ ] No scope creep into later phases

**Commit.** `feat(logical): backbuild option enumeration, refusals, and DuckDB conformance harness`

---

### Phase 3: B1 + B2 — self-derivable column adds and rename detection

**Goal.** Admit added columns derivable from stored columns (B1: `ALTER ADD` + in-place
`UPDATE`) and renames (B2: `ALTER RENAME`, zero rows). Rename pairing runs before add/drop
classification.

**Pre-conditions.** Phases 1–2.

**TDD tests to write first.**
- `backbuild_conformance.rs::b1_constant_column` — added `'active' AS status`; script =
  ALTER+UPDATE; oracle-equal to full rebuild.
- `::b1_arithmetic_over_stored_columns` — added `price * qty AS total` where `price`,
  `qty` are stored 1:1; oracle-equal.
- `::b2_rename_touches_no_rows` — renamed column; script is a single
  `ALTER … RENAME COLUMN`; oracle-equal.
- `backbuild_options.rs::b1_opaque_function_refuses` — added column calling an
  unregistered function ⇒ named refusal. NOTE: the existing `collect_dependencies` walk
  recurses into any `FunctionCall`'s arguments and returns `Ok(∅)` for an unknown
  zero-arg function — this test forces the specified extension (registry-backed
  opaqueness check), it does not pass against the walk as-is.
- `::b1_volatile_function_refuses` — added column calling `random()`/`now()` ⇒ named
  refusal (volatility check; research §2 determinism caveat — a volatile backfill can
  never match a rebuild).
- `::b1_subquery_refuses`, `::b1_window_refuses` — per `collect_dependencies` posture.
- `::b2_ambiguous_rename_refuses` — two dropped columns with identical expressions ⇒
  refusal, not a guess (research §7.2).
- `::b2_one_dropped_two_added_pins_lexicographic` — one dropped, two identical added ⇒
  lexicographically-first added name takes the rename, the other classifies as B1
  reading the renamed column (research §4 B2).
- `::b1_dependency_on_upstream_only_column_is_not_b1` — an added column reading an
  upstream column with no stored 1:1 representative is **not** admitted as B1 (it is
  Phase 5's B3; here it must refuse, not misclassify).

**Implementation shape.** `classify.rs`: rename pairing over
`(select_list.dropped × select_list.added)` by `same_modulo_trivia` on expressions, then
B1 admission via the dependency walk (reuse/extract the `collect_dependencies` logic from
`analysis/model_diff.rs` rather than duplicating it — a small `pub(crate)` promotion is in
scope), extended with a registry-backed opaqueness/volatility leaf check
(`smelt_types` function registry; unknown or volatile function ⇒ named refusal, research
§2). Representatives follow the uniform rule (research §4 intro): bare pull-throughs
unchanged between both definitions. `requalify.rs`: first user — requalify a B1 expression's input references to their
stored 1:1 representative columns. `emit.rs`: `emit_alter_add_column`,
`emit_alter_rename_column`, and reuse of the `emit_in_place_update` shape (unregioned
variant).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/{classify,requalify,emit}.rs`
- `crates/smelt-logical/src/analysis/model_diff.rs` — visibility promotion of the
  dependency walk, plus the specified registry-backed opaqueness/volatility check (the
  one sanctioned behaviour change; existing `additive_only_diff` callers must be
  reviewed for the tightened posture)
- both test files

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Rename pairing precedes add/drop classification (drop+add never misread)
- [ ] Dependency walk shared with `model_diff.rs`, not forked
- [ ] Requalification is a CST rewrite with its own unit tests, not string replacement
- [ ] B1 derivability = stored 1:1 representative exists for every dependency (research
      §4 B1/D1 subtlety), not name-coincidence
- [ ] No scope creep into later phases

**Commit.** `feat(logical): backbuild B1/B2 — self-derivable column adds and rename detection`

---

### Phase 4: D1 — column-scoped rebuild for stored-derivable expression changes

**Goal.** A changed existing-column expression whose new expression is derivable from
stored columns becomes a single-column in-place `UPDATE` (the "fix one column of a huge
table" case). Includes the formatting-only guard.

**Pre-conditions.** Phase 3 (derivability + requalification + UPDATE emission exist).

**TDD tests to write first.**
- `backbuild_conformance.rs::d1_changed_expression_updates_one_column` — `amount_usd`
  formula fixed; script = one `UPDATE t SET amount_usd = …`; oracle-equal; sibling
  columns byte-identical to pre-script values (assert directly — the point of D1 is
  *not* touching them).
- `::d1_formatting_only_change_is_noop` — reformatted expression ⇒ no step for that
  column (trivia-insensitivity end-to-end).
- `backbuild_options.rs::d1_new_expr_reading_own_old_value_refuses` — new expression whose
  input has no stored representative other than the changed column itself ⇒ refusal, never
  self-substitution.
- `::d1_swapped_columns_refuse` — `x AS a, y AS b` → `y AS a, x AS b`: both changed
  columns fail the uniform representative rule (research §4 intro) ⇒ refusal. (The weaker
  "not its own value" rule would admit mutually-invalidating updates.)
- `::d1_lateral_alias_to_changed_sibling_refuses` — a changed/added expression referencing
  a lateral alias of a *changed* sibling ⇒ refusal (representative must be unchanged).
- `::d1_distinct_model_refuses` — D-class under `SELECT DISTINCT` ⇒ named refusal: an
  UPDATE cannot merge rows the rebuild's DISTINCT would (research §4 grain guards).
- `::d1_upstream_dependency_refuses_until_d2` — changed expression needing an upstream
  read refuses in this phase (admitted in Phase 5 as D2).

**Implementation shape.** `classify.rs`: route `select_list.changed` through the same
derivability check as B1 under the uniform representative rule (research §4 intro:
representatives are bare pull-throughs **unchanged between both definitions** — this
subsumes "never its own value" and excludes changed siblings/swaps), plus the D-class
DISTINCT and LIMIT grain guards. Reuses Phase 3 emission unchanged.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/classify.rs`
- both test files

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Sibling-untouched assertion present (not just multiset equality)
- [ ] Self-substitution explicitly inadmissible
- [ ] No scope creep into upstream-read (Phase 5)

**Commit.** `feat(logical): backbuild D1 — column-scoped rebuild for stored-derivable expression changes`

---

### Phase 5: B3 + D2 — upstream pull-through and upstream-read expression changes

**Goal.** Added (B3) or changed (D2) columns whose expressions read an upstream already in
the FROM tree, admitted via the grain-link proof (output carries a 1:1 pull-through of the
upstream's declared unique key), emitted as a column-scoped `UPDATE … FROM`.

**Pre-conditions.** Phases 3–4.

**TDD tests to write first.**
- `backbuild_conformance.rs::b3_upstream_pullthrough` — model selects from `orders`;
  added `o.discount AS discount`; `BackbuildInputs` declares `orders.unique_key =
  [order_id]` and `order_id` is pulled through; script = ALTER + `UPDATE … FROM orders`;
  oracle-equal.
- `::b3_respects_model_filter` — before-def has a WHERE; backfill touches only surviving
  rows; oracle-equal.
- `::d2_changed_expression_from_upstream` — existing column's formula now reads a
  different upstream column; single-column `UPDATE … FROM`; oracle-equal; siblings
  untouched.
- `::d_dual_derivable_yields_both_options` — a changed expression derivable **both** from
  stored columns (D1) and from upstream (D2) returns both the in-place `UPDATE` and the
  `UPDATE … FROM` options; each independently oracle-verified on a fresh table copy.
- `::b3_stale_upstream_documents_precondition` — upstream mutated after `build_before`
  (precondition §2 violated); the test **demonstrates the divergence**: the backfilled
  column reflects current upstream while sibling columns reflect the stale build, so the
  result ≠ a full rebuild against current inputs. Comment cites research §2 "Why the
  precondition is load-bearing" — this is the contract's edge made visible, not a bug.
- `backbuild_options.rs::b3_missing_key_pullthrough_refuses` — output lacks the upstream key
  column ⇒ named refusal ("no addressable identity").
- `::b3_undeclared_unique_key_refuses` — no `unique_key` in inputs ⇒ refusal.
- `::b3_nullable_key_refuses` — key columns without a NOT NULL proof/declaration ⇒
  refusal: SQL UNIQUE admits NULLs, an equality backfill never addresses a NULL-keyed
  row, and the rebuild fills it (research §4 "Key addressability").
- `::b3_self_join_binds_per_alias` — `orders o1 JOIN orders o2`: adding `o2.discount`
  with only `o1.order_id` pulled through ⇒ refusal (the proof binds per FROM-tree alias,
  not per table; a table-level match would backfill o1's discount where the rebuild wants
  o2's).

**Implementation shape.** `classify.rs`: grain-link proof = lineage over *unchanged*
SELECT items, bound **per FROM-tree alias** (the pulled-through key and the added
expression must resolve to the same alias), with the NOT NULL key obligation consumed
from `analysis::not_null` or declared facts, fail-closed. `requalify.rs`: statement-context aliasing (`t.` / `u.`). `emit.rs`:
`emit_column_backfill_update_from` (`UPDATE t SET c = <expr> FROM <ups> u WHERE t.k = u.k`,
composite keys ANDed) — the `emit_column_scoped_merge` shape adapted to the standalone
(unregioned, unledgered) setting.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/{classify,requalify,emit}.rs`
- both test files

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Grain-link proof is lineage-based (CST), fail-closed on computed pull-throughs
- [ ] Stale-input case asserts the contract's actual guarantee, not a vaguer one
- [ ] D2 reuses B3 machinery (one admission path, two triggers)
- [ ] No scope creep into later phases

**Commit.** `feat(logical): backbuild B3/D2 — upstream pull-through and upstream-read expression changes`

---

### Phase 6: B4 — join-enrichment backfill with the row-set-preservation proof

**Goal.** An added LEFT JOIN feeding only added columns, admitted iff the join provably
cannot change the row set (LEFT + unique dimension key + no other clause references the
alias), emitted as a fan-out backfill.

**Pre-conditions.** Phases 1–5.

**TDD tests to write first.**
- `backbuild_conformance.rs::b4_left_join_enrichment_fanout` — fact×dim with genuine
  fan-out (many fact rows per dim row); bare `d.x` pull ⇒ **both** emitter shapes offered
  (`UPDATE … FROM` and scalar-subquery); each independently oracle-verified.
- `::b4_unmatched_rows_null_extend` — fact rows with no dim match end NULL, matching the
  rebuild exactly.
- `::b4_general_expression_null_extension` — added column is `COALESCE(d.x, 'none')`;
  only the **per-reference substituted** scalar-subquery option is offered:
  `SET c = COALESCE((SELECT d.x FROM dim d WHERE d.jk = t.jk), 'none')`. Assert the
  option set, then oracle-verify with an unmatched fact row — it must end `'none'`, not
  NULL. Both naive shapes are the traps this test pins: bare `UPDATE … FROM` skips the
  row, and the whole-expression subquery `SET c = (SELECT COALESCE(…) FROM …)` yields
  NULL because a zero-row scalar subquery nulls the *whole* expression (research §4 B4).
- `backbuild_options.rs::b4_join_key_not_stored_refuses` — fact-side ON column has no
  stored bare representative ⇒ named, actionable refusal.
- `::b4_on_beyond_bare_key_equality_refuses` — ON carries an extra dim-side conjunct
  (`… AND d.active`) or a non-equality comparison ⇒ refusal.
- `::b4_nullable_join_key_refuses` — join key without a NOT NULL proof/declaration ⇒
  refusal (research §4 "Key addressability").
- `backbuild_options.rs::b4_inner_join_refuses` — added INNER JOIN ⇒ refusal (can drop
  rows).
- `::b4_nonunique_dim_key_refuses` — no declared/derived uniqueness ⇒ refusal.
- `::b4_alias_referenced_in_where_refuses` — new alias in WHERE ⇒ refusal (row set no
  longer preserved).

**Implementation shape.** `classify.rs`: consumes `SkeletonDiff::AddedLeftJoins` with
exactly one element (two or more is Phase 9's B7 — refuse here with a named reason);
uniqueness from `BackbuildInputs.sources[dim].unique_key` or
`analysis::functional_dependency` where derivable; alias-reference sweep over every
non-added clause. `emit.rs`: shape *enumeration* is expression-driven — bare column pull
⇒ both `UPDATE … FROM` and scalar-subquery options; any other expression ⇒ the
per-reference substituted scalar-subquery form only (each dim-column reference replaced
by its own scalar subquery — research §4 B4, including the free multiplicity guard).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/{classify,emit}.rs`
- `crates/smelt-logical/src/backbuild/requalify.rs` — only if the scalar-subquery shape's
  embedded expression fragment needs statement-context requalification beyond Phase 5's
  rewriter (otherwise reused unchanged)
- both test files

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Row-set-preservation proof requires all three legs (LEFT, uniqueness, no stray
      references) — dropping any one is a demonstrated refusal
- [ ] Shape enumeration is expression-driven; the NULL-extension case demonstrates a
      *narrowed* option set, and every offered option is oracle-verified
- [ ] Uniqueness consumed from declared facts / existing FD analysis, not re-derived ad hoc
- [ ] No scope creep into later phases

**Commit.** `feat(logical): backbuild B4 — join-enrichment backfill with row-set-preservation proof`

---

### Phase 7: E1 + E4 — predicate tighten DELETE and horizon-extension INSERT

**Goal.** Conjunct-diff-driven row-set repair: an added conjunct over stored columns
becomes `DELETE … WHERE <q> IS NOT TRUE`; a relaxed range predicate on one column becomes
a region-scoped difference `INSERT`.

**Pre-conditions.** Phases 1–2 (conjunct diff); Phase 5's requalifier.

**TDD tests to write first.**
- `backbuild_conformance.rs::e1_tighten_deletes_only` — added conjunct; script is a
  single DELETE; oracle-equal.
- `::e1_null_semantics` — rows where the new conjunct evaluates NULL are deleted (the
  `IS NOT TRUE` form; a bare `NOT` would keep them — this test is the regression trap).
- `::e4_horizon_extension_inserts_region` — `ts >= '2025-01-01'` → `ts >= '2024-01-01'`;
  script inserts exactly `[2024-01-01, 2025-01-01)` from upstream; oracle-equal.
- `::e4_idempotent_with_identity` — declared row identity ⇒ anti-join guard; running the
  script twice still oracle-equal.
- `backbuild_options.rs::e1_conjunct_over_unstored_column_refuses` — added conjunct
  referencing an input with no stored representative ⇒ refusal.
- `::e_opaque_predicate_rewrite_refuses` — `a OR b` → `a` (non-conjunctive) ⇒ refusal.
- `::e_group_by_model_refuses` — predicate change on a GROUP BY model ⇒ named refusal (a
  slice INSERT double-counts existing groups; the anti-join guard would silently skip
  them instead — research §4 E grain precondition).
- `backbuild_conformance.rs::e4_group_key_range_admits` — the carve-out: E4 where the
  range column **is a group key** (extending history on a date-keyed aggregate) admits
  and is oracle-equal — every group lies wholly inside or outside the region.
- `backbuild_options.rs::e_distinct_model_refuses`, `::e_limit_model_refuses` — DISTINCT
  and LIMIT presence refuse E-class atoms (research §4 grain guards).
- `::e4_mixed_operator_refuses` — `ts > X` → `ts >= Y` ⇒ refusal (range classification
  requires the same comparison operator; boundary semantics are not literal arithmetic).

**Implementation shape.** `classify.rs`: the E-class grain guards first (no GROUP
BY/DISTINCT/LIMIT, with the E4-on-group-key carve-out); E1 = added conjuncts, each
requalified to stored columns; E4 = removed+added conjunct pair that are range predicates
on the same column with the same operator and a provably widened literal — the range view
classifies only, the **emitted predicate is always the complement form** (research §4 E4:
new conjunct present, `AND (<old conjunct>) IS NOT TRUE`), which gets boundaries and NULLs
right by construction. `emit.rs`: `emit_predicate_delete` (`IS NOT TRUE` form),
`emit_difference_insert` (the after-definition SELECT body with the difference predicate
appended, an **explicit column list** on the INSERT, plus the identity anti-join guard
when identity is available).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/{classify,emit}.rs`
- both test files

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Three-valued-logic form used and regression-tested
- [ ] E4 range widening proven from literals, fail-closed otherwise
- [ ] INSERT guard present exactly when identity exists; one-shot posture recorded on
      the option data when not
- [ ] No scope creep into later phases

**Commit.** `feat(logical): backbuild E1/E4 — predicate tighten DELETE and horizon-extension INSERT`

---

### Phase 8: E2 + F1 — general loosen INSERT, union-branch INSERT, and composites

**Goal.** Round out the predicate and structural cases: removed conjunct ⇒ difference
`INSERT` (general E2 — with Phase 7's E1 this also delivers E3 by composition), added
UNION ALL branch ⇒ branch `INSERT` (F1), and the H composite ordering proven end-to-end.

**Pre-conditions.** Phases 1–7.

**TDD tests to write first.**
- `backbuild_conformance.rs::e2_loosen_inserts_difference` — removed conjunct `q`; insert
  slice is after-SELECT `AND (q IS NOT TRUE)`; oracle-equal.
- `::f1_union_branch_insert` — added UNION ALL branch inserted alone; oracle-equal.
- `::h_composite_rename_add_tighten` — one diff combining B2 + B1 + E1; assemble a
  selection (one option per atom); statements in the H order (rename → alter/add →
  delete → update); oracle-equal. The ordering is the assertion — shuffled statements
  must fail the oracle (verify by construction in a comment, not a second test).
- `::h_composite_add_plus_insert_aligns_columns` — B1 add (mid-declared-list position) +
  F1 branch INSERT in one diff: after `ALTER ADD` the physical column order differs from
  the declared order, and a positional INSERT silently misassigns same-typed columns —
  the emitted INSERT carries an explicit column list (or `BY NAME`) and the composite is
  oracle-equal (research §4H).
- `::h_composite_with_blocked_atom_yields_only_full_refresh` — same composite plus a G1
  edit ⇒ no composed targeted script; refusals name the G1 atom; `FullRefresh` remains
  the only model option.
- `backbuild_options.rs::f1_plain_union_refuses` — `UNION` (dedup) ⇒ refusal; only
  `UNION ALL` admits.

**Implementation shape.** `classify.rs`: E2 from removed conjuncts (difference predicate
in `IS NOT TRUE` form); F1 from `set_ops` branch diff. Every INSERT-family emitter takes
an explicit target column list (research §4H — never positional). `mod.rs`: `assemble`
finalises the H ordering (`rename → alter → delete → update/merge → insert → drop`) as a
total order over the selected options' variants.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/{mod,classify,emit}.rs`
- both test files

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] H ordering is a property of `assemble` (one place), not per-case emission
- [ ] `UNION` vs `UNION ALL` distinction enforced
- [ ] Catalogue cases admitted so far are all conformance-tested; refusal reasons cover
      every inadmissible branch touched in this plan
- [ ] No scope creep into later phases

**Commit.** `feat(logical): backbuild E2/F1 and composite ordering`

---

### Phase 9: B7 — sequential multi-join enrichment

**Goal.** Two or more added LEFT JOINs backfilled as ordered steps (research §4 B7): each
join passes the full B4 proof, a later join's key may reference a column an earlier step
backfills (stored by the time the step runs), and the steps run in derived dependency
order within the update slot.

**Pre-conditions.** Phases 6 (B4 proof + emitters) and 8 (`assemble` ordering).

**TDD tests to write first.**
- `backbuild_conformance.rs::b7_two_joins_sequential_backfill` — fact + dim1 + dim2 where
  dim2's ON keys on a column dim1 provides *and* the model stores; script backfills
  dim1's columns first, then dim2's keyed on the now-stored column; oracle-equal.
- `::b7_independent_joins_either_order` — two added joins each keyed on already-stored
  columns (no inter-dependency); both backfills emitted; oracle-equal.
- `backbuild_options.rs::b7_unstored_intermediate_refuses` — dim2 keys on a dim1 column the
  model does **not** store ⇒ named refusal whose message names the column to add
  (actionable refusal, research §2; multi-hop: research §7.7).
- `::b7_nonbare_intermediate_refuses` — the stored carrier of dim1's column is wrapped
  (`COALESCE(d1.c, 0) AS c`) ⇒ refusal: the carrier stores `0` where the rebuild has
  NULL, so a later join on it can hit a dim row the rebuild misses (research §4 B7
  bareness).
- `::b7_per_join_proof_still_enforced` — second join is INNER, or its alias leaks into
  WHERE ⇒ refusal naming that join (B4's three legs hold per join, not just for the
  first).

**Implementation shape.** `classify.rs`: consume the full `SkeletonDiff::AddedLeftJoins`
list; build the reference-dependency order (a later join's ON referencing an earlier
join's output columns); run the B4 proof per join with the derivability environment
extended by earlier steps' backfilled columns; cycle or unresolvable ordering ⇒ refusal.
`mod.rs`: within-slot ordering of B7 steps in `assemble` (the one data-dependent ordering;
research §4H).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/{mod,classify,emit}.rs`
- both test files

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Per-join proof is the unmodified B4 proof plus only the stored-by-then extension,
      and the stored intermediate is required **bare** (never a wrapped carrier)
- [ ] Dependency ordering derived from CST references, fail-closed on cycles
- [ ] Unstored-intermediate refusal names the join and the missing column
- [ ] No scope creep beyond the catalogue cases this plan admits

**Commit.** `feat(logical): backbuild B7 — sequential multi-join enrichment`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the plan is satisfied at the end:

- `cargo test -p smelt-logical --test backbuild_conformance` — every admitted catalogue
  case oracle-equal against DuckDB, including the stale-input and composite cases.
- `cargo test -p smelt-logical --test backbuild_diff --test backbuild_options` — diff and
  refusal coverage.
- `bash .claude/scripts/verify-phase.sh` — full pre-commit gate.
- Grep gate: no production dependency added from `smelt-logical` to `smelt-fingerprint`,
  `smelt-db`, or `smelt-planner` (`cargo tree -p smelt-logical` clean).
- The research doc's §4 catalogue and §7 open questions updated with anything learned
  (same commits as the learning).
