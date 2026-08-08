# Plan: Backbuild/walk/maintenance substrate unification

**Date**: 2026-08-08
**Spec**: [`docs/specs/architecture.md`](../specs/architecture.md) §"Property composition walk rule", §"Constraints & Invariants" #12
**Spec diff**: uncommitted working tree (crate-wide walk rule; single definition-diff engine; backbuild under single-owner emission; `walk_coverage` scope `{analysis,rules,maintenance,backbuild}`)
**Tracking PR / branch**: `spec-redraft-incremental-models`
**Docs**: code-only  <!-- internal unification; no user-visible surface change. Spec edits are the diff above; no docs-site changes. -->

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec sections named above — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `spec-redraft-incremental-models`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Red-green TDD: failing test before any implementation.
- Behaviour-preservation is the default: unless a phase names a behaviour change (Phases 3 and 4 each name exactly one), unification must not change any admission verdict, plan cell, or emitted statement. The existing `backbuild_conformance`, `backbuild_property`, `maintenance_conformance`, and `statement_parity` suites are the safety net — they must pass unmodified except where a phase explicitly says otherwise.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call: fmt + clippy + tests + example_diagnostics, failures-only output) — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md`.
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in this plan file only.

---

## Context

The audit of 2026-08-08 (conversation-level; findings reproduced in the phase notes below) found `crates/smelt-logical/src/backbuild/` is a near-standalone second analysis engine: one import from the rest of the crate (`model_diff::collect_dependencies`), its own single-`SELECT` tree handling beside `analysis/walk.rs`'s `QueryTree`, five crate-wide copies of column-ref collection, two constant-literal recognizers that disagree on typed literals, two definition-diff engines that disagree on whitespace, two "single-owner" emitter modules, and no coverage by either the `walk_coverage` or `statement_parity` gates. The spec diff makes the walk rule and single-owner emission crate-wide; this plan drives the code to match. The walk itself is hardened first (its mutation campaign scored 76.7% with survivors on the fail-closed spine — `docs/TODO.md` "walk.rs campaign residue") because later phases route more consumers through it.

## Scope

### In scope (spec coverage)
- §"Property composition walk rule" crate-wide statement: backbuild proofs consume walk verdicts (lineage, skeleton closure, discriminants, FD); one recognizer per property.
- §"Constraints & Invariants" #12 extension: one definition-diff engine; backbuild emitters in the single-owner families; `walk_coverage` scope `{analysis,rules,maintenance,backbuild}`.
- `docs/TODO.md` "walk.rs campaign residue": tier-2 triage and killing tests for the 38 survivors.

### Explicitly deferred
- **Wiring backbuild** (CLI/runtime consumers, `.smelt/` before-SQL sourcing, executed-statement parity leg for backbuild) — separate capability plan; this plan only unifies the substrate so wiring lands on one engine.
- **Backbuild multi-`SELECT`/CTE descent** (replacing `DefinitionDiff::Opaque` on differing `WITH` prefixes with per-CTE diffing via `QueryTree`) — a capability extension, not unification; becomes tractable after Phase 5 but is not required by the spec diff.
- **`Backend::delete_partitions`/`insert_overwrite` dormant second author** — pre-existing allowlisted gap, unchanged.
- **`rules/cumulative.rs` whole-SQL `OVER(` scan migration** — already tracked in `model_properties.md`; not touched here.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | bfd0453b | 2026-08-08 |
| 2     | done     | 6827ea9e | 2026-08-08 |
| 3     | done     | 9073e141 | 2026-08-08 |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |

---

### Phase 1: Walk fail-closed spine hardening

**Goal.** Triage all 38 `analysis/walk.rs` mutation survivors (full list: `docs/research/20260808-mutation-testing-maintenance-gates.md` §"Bonus campaign") and land killing tests for every genuine gap, starting with the `has_unsupported` cluster.

**Pre-conditions.** None. `cargo mutants` available (`--iterate` state may exist from the prior campaign).

**TDD tests to write first.** One killing test per confirmed-genuine survivor; at minimum these named clusters, each asserting through the public verdict surface (`model_property_vector` / admission outcomes), not private internals:
- `crates/smelt-logical/tests/walk_hardening.rs::unsupported_node_fails_closed` — a model containing an unsupported construct inside a CTE yields a refused/degraded vector; kills `QueryNode::has_unsupported -> false`.
- `crates/smelt-logical/tests/walk_hardening.rs::leaf_transfer_not_default` — a leaf whose properties differ from `Default` propagates them; kills `PropertyTransfer::leaf -> Default` and `PartitionGrainAdmission` leaf mutants.
- `crates/smelt-logical/tests/walk_hardening.rs::intersect_except_degrade` — `INTERSECT`/`EXCEPT` models degrade exactly as specified; kills the deleted `setop_kind_after` arms.
- `crates/smelt-logical/tests/walk_hardening.rs::constant_literal_rejects_function_call` — `is_constant_literal` rejects function calls / accepts typed literals; kills `is_constant_literal -> true`.
- `crates/smelt-logical/tests/walk_hardening.rs::union_discriminator_requires_distinct_tags` — two branches with the *same* literal tag are not discriminated; kills the `union_discriminated_grain` comparison flip.

**Implementation shape.** Test-only phase plus doc-comment classifications; production edits only where triage proves a survivor is a genuine dead arm (delete with a note) — mirror the maintenance-campaign method in `docs/research/20260808-mutation-testing-maintenance-gates.md`. Record per-survivor verdicts in a short research addendum appended to that doc (campaign-record, exempt from timeless rule as a research doc). Update `docs/TODO.md` residue entry to done.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/tests/walk_hardening.rs` — new killing-test suite
- `crates/smelt-logical/src/analysis/walk.rs` — only dead-arm deletions or doc-comment classifications proven by triage
- `docs/research/20260808-mutation-testing-maintenance-gates.md`, `docs/TODO.md`

**Review checklist** (material findings only):
- [ ] Every one of the 38 survivors has a written verdict (killed / advisory / provably-equivalent / deferred-with-reason)
- [ ] Killing tests assert observable verdicts, not implementation details
- [ ] No production behaviour change beyond proven dead-arm removal
- [ ] `cargo mutants --file crates/smelt-logical/src/analysis/walk.rs` kill rate materially improved; new baseline recorded in the research addendum

**Commit.** `test(logical): kill walk.rs mutation survivors on the fail-closed spine`

---

### Phase 2: One column-ref collector, one conjunct splitter

**Goal.** Collapse the five copies of column-ref collection (`analysis/fingerprint.rs:221`, `analysis/skeleton_closure.rs:292`, `maintenance/grouping.rs:265`, `backbuild/classify.rs:3434`, `backbuild/classify.rs:3739`) and three copies of conjunct splitting (`walk.rs:1385`, `backbuild/diff.rs` `split_conjuncts`, `backbuild/classify.rs:3362`) into one shared home each.

**Pre-conditions.** Phase 1 done (the walk's own tests are trustworthy before consumers move onto shared helpers).

**TDD tests to write first.**
- `crates/smelt-logical/tests/expr_util.rs::column_ref_collection_characterization` — one table-driven test capturing the *current* output of each of the five call sites on a shared battery of expressions (qualified refs, aliases, function args, `CASE`, window `OVER` clauses); any behavioural difference between copies is surfaced here first and resolved deliberately (documented in the test), not silently.
- `crates/smelt-logical/tests/expr_util.rs::conjunct_split_characterization` — same for the three splitters, including nested parens, `OR` guards, and `BETWEEN`.

**Implementation shape.** New `crates/smelt-logical/src/analysis/expr_util.rs` (pub(crate)) holding `collect_column_refs` (with a qualifier-aware variant) and `split_top_level_conjuncts`; all eight call sites migrate to it; deleted copies leave no fork behind. Where a copy's behaviour genuinely differed for a reason (e.g. backbuild's `references_alias_or_unproven_bare`), express the difference as a parameter or thin wrapper over the shared core, not a re-implementation.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/expr_util.rs` — new
- `crates/smelt-logical/src/analysis/{fingerprint.rs,skeleton_closure.rs,walk.rs}`, `crates/smelt-logical/src/maintenance/grouping.rs`, `crates/smelt-logical/src/backbuild/{diff.rs,classify.rs}` — call-site migration only

**Review checklist**:
- [ ] All five collector copies and three splitter copies are gone (grep-verifiable)
- [ ] Characterization test documents any deliberate behaviour reconciliation
- [ ] Existing conformance/property suites pass unmodified

**Commit.** `refactor(logical): single shared column-ref collector and conjunct splitter`

---

### Phase 3: One constant-literal recognizer, one union discriminator

**Goal.** Delete backbuild's `bare_literal`/`find_branch_discriminator` (classify.rs:1068/1563) in favour of the walk's `is_constant_literal`/`union_discriminated_grain` machinery, fixing the known disagreement on typed literals.

**Pre-conditions.** Phase 1 (the walk's recognizers are the hardened ones).

**TDD tests to write first.**
- `crates/smelt-logical/tests/backbuild_property.rs::typed_literal_branch_discriminator` — a UNION model whose branch tags are `DATE '2026-01-01'` / `DATE '2026-02-01'`: backbuild recognises the discriminator (today it does not — this is the phase's single named behaviour change, in the accepting direction).
- `crates/smelt-logical/tests/backbuild_property.rs::function_call_never_discriminates` — a `CURRENT_DATE` branch tag is rejected by both layers (guards against the unification widening too far).

**Implementation shape.** Expose the walk's recognizer at the right granularity (a pure `is_constant_literal(expr)` + a per-branch discriminator helper over already-parsed branch SELECT lists, reusable without a full `QueryTree` if backbuild's inputs don't warrant one); backbuild delegates; correct the misleading doc comment at classify.rs:1532-1544 to name the walk as the sibling. DuckDB oracle equivalence in `backbuild_conformance` must hold for the newly-admitted typed-literal case.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/walk.rs` — visibility/factoring of the recognizer only
- `crates/smelt-logical/src/backbuild/classify.rs` — delete `bare_literal`/`find_branch_discriminator`, delegate
- `crates/smelt-logical/tests/backbuild_property.rs`, `crates/smelt-logical/tests/backbuild_conformance.rs`

**Review checklist**:
- [ ] Exactly one constant-literal recognizer remains crate-wide (grep-verifiable)
- [ ] The newly-admitted typed-literal case is oracle-verified (DuckDB equivalence), not just unit-asserted
- [ ] No other admission verdict changed (conformance suites unmodified otherwise)

**Commit.** `refactor(logical): backbuild union discriminator delegates to the walk's literal recognizer`

---

### Phase 4: One definition-diff engine

**Goal.** Rebase `analysis/model_diff.rs`'s `additive_only_diff` onto the backbuild diff's token-stream comparison so exactly one text-equality notion exists, fixing the known false positive: a pure whitespace/trivia reformat of a column expression is currently `NotAdditive` (model_diff.rs:76 raw `.text().trim()` compare) but must be a no-op.

**Pre-conditions.** Phase 2 (shared splitter available to both diff layers).

**TDD tests to write first.**
- `crates/smelt-logical/tests/model_diff.rs::whitespace_reformat_is_not_a_change` — same column expression reformatted (line breaks, comment trivia) yields `AdditiveOnly` with no changed columns (red today — this is the phase's single named behaviour change).
- `crates/smelt-logical/tests/model_diff.rs::token_change_is_a_change` — a genuine one-token expression change still classifies as changed.
- `crates/smelt-logical/tests/backbuild_docs.rs` (or equivalent) — assert `same_modulo_trivia` has exactly one definition, reachable from both consumers.

**Implementation shape.** Move `same_modulo_trivia` (and the token-stream walk under it) from `backbuild/diff.rs` into `analysis/` (e.g. `expr_util.rs` or a small `analysis/token_eq.rs`); `model_diff.rs` and `backbuild/diff.rs` both consume it. Do **not** merge the two diff *representations* (`ModelDiff` vs `DefinitionDiff`) in this phase — they answer different questions at different granularity; this phase unifies only the equality primitive, which is where the disagreement lives. Representation-level convergence is re-examined in Phase 5's review.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/{model_diff.rs,expr_util.rs}` (or new `token_eq.rs`)
- `crates/smelt-logical/src/backbuild/diff.rs` — consume the moved primitive
- `crates/smelt-logical/tests/model_diff.rs`

**Review checklist**:
- [ ] One `same_modulo_trivia` (grep-verifiable); `.text().trim()` equality comparison gone from `model_diff.rs`
- [ ] `ColumnAdded` trigger derivation unchanged for all existing conformance recipes
- [ ] The whitespace no-op case is covered end-to-end (a `ModelInputs.column_add_proof` built from a reformatted definition derives no `ColumnAdded` cells)

**Commit.** `refactor(logical): single token-stream equality primitive for definition diffs`

---

### Phase 5: Backbuild proofs consume walk verdicts

**Goal.** Replace backbuild's private re-derivations with the walk's: `resolve_representative` provenance → `ColumnLineage`; `try_b1`/`try_d1` "derivable from stored data" → skeleton-closure verdicts; `admit_added_left_join`'s inline at-most-one-match check → `functional_dependency_verdict`; `classify_skeleton_reason`'s lowercased-string scan over an English reason → structured `SkeletonDiff` data.

**Pre-conditions.** Phases 2–4. This phase threads walk-derived inputs into `BackbuildInputs` (the classify.rs:3273 deferral note names exactly this: "revisit once wiring supplies the definition").

**TDD tests to write first.**
- `crates/smelt-logical/tests/backbuild_property.rs::provenance_chases_renames` — an added column derivable from a stored column *through a CTE/derived-table rename* is admitted (today `resolve_representative`'s flat triple cannot chase the rename — an accepting-direction change; oracle-verified).
- `crates/smelt-logical/tests/backbuild_property.rs::fd_verdict_shared` — the left-join enrichment admission and the maintenance FD path agree on the same fixture (one uniqueness verdict, two consumers).
- `crates/smelt-logical/tests/backbuild_property.rs::skeleton_reason_structured` — G1-vs-G2 catalogue classification is driven by `SkeletonDiff` variants, asserted by constructing a diff whose *prose* mentions "join" but whose structural cause is a grain change (kills the string-scan).
- All existing `backbuild_conformance` oracle cases pass unmodified.

**Implementation shape.** `BackbuildInputs` gains optional walk-derived facts (a `PropertyVector`/lineage handle per side where SQL is available; the plain-data `not_null_columns`/`unique_key` fields remain as the declared-fact fallback). `resolve_representative` becomes a thin adapter over `ColumnLineage`. `classify_skeleton_reason(reason: &str)` is deleted; `SkeletonDiff` carries a structured cause enum and the catalogue label derives from it. Behaviour-preserving except the named rename-chasing admission widening.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/backbuild/{mod.rs,classify.rs,diff.rs}`
- `crates/smelt-logical/src/analysis/{walk.rs,skeleton_closure.rs,functional_dependency.rs}` — visibility/adapter factoring only
- `crates/smelt-logical/tests/backbuild_{property,conformance}.rs`

**Review checklist**:
- [ ] `PropertyVector`/lineage consumed, not re-derived (the classify.rs:3280 "why not" comment is gone because the answer is now "we do")
- [ ] No lowercased free-text `.contains` classification remains in backbuild admission paths
- [ ] Accepting-direction changes are each oracle-verified
- [ ] Reviewer re-examines whether `ModelDiff` should now become a view over `DefinitionDiff` (record verdict in Deferred if not)

**Commit.** `refactor(logical): backbuild admission proofs consume walk lineage, skeleton, and FD verdicts`

---

### Phase 6: Emitter unification and gate extension

**Goal.** Bring backbuild emission under the single-owner families and extend both structural gates to the spec's scope: `walk_coverage` scans `{analysis,rules,maintenance,backbuild}`; the `statement_parity` no-authoring leg also scans `smelt-logical` outside the two emitter modules.

**Pre-conditions.** Phases 1–5.

**TDD tests to write first.**
- `crates/smelt-logical/tests/walk_coverage.rs` — extend `SCANNED_DIRS` to include `maintenance` and `backbuild`; the test fails red on today's untagged sites (e.g. `backbuild/classify.rs:247` — deleted in Phase 5 — and the ~50 `maintenance/` sites), then each surviving site gets a `Leaf classifier`/`Advisory heuristic` classification or is refactored; no blanket `KNOWN_NONCOMPLIANT` additions without a per-file reason.
- `crates/smelt-runtime/tests/statement_parity.rs::no_maintenance_statement_authoring_outside_the_emitter` — scan scope extended to `crates/smelt-logical/src` excluding `maintenance/emit.rs` and `backbuild/emit.rs`; red if any statement shape is authored elsewhere.
- `crates/smelt-logical/tests/emit_statements.rs::backbuild_unregioned_update_is_the_maintenance_emitter` — backbuild's unregioned `UPDATE` output is byte-identical to `maintenance::emit::emit_in_place_update` invoked with no region bound (the emit.rs:46-51 sibling fork is gone).

**Implementation shape.** Generalize `emit_in_place_update`'s region parameter to `Option<Region>` (or an equivalent absent-region form) and delete backbuild's sibling; audit the remaining backbuild emitters for other maintenance-family overlap (delete+insert shapes) and delegate where the family matches, keeping genuinely backbuild-only shapes (rename, pull-through, window backfill) in `backbuild/emit.rs` as their single owner. Tag or refactor every `.contains(` site the widened `walk_coverage` gate flags — emit.rs SQL-construction sites are not free-text scans and should not need tags; if the gate's pattern misfires on them, tighten the gate's pattern rather than tagging noise.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/tests/walk_coverage.rs`, `crates/smelt-runtime/tests/statement_parity.rs`
- `crates/smelt-logical/src/maintenance/emit.rs`, `crates/smelt-logical/src/backbuild/emit.rs`
- `crates/smelt-logical/src/maintenance/{locality.rs,choice.rs,mod.rs}` — doc-comment classification of flagged scan sites only
- `docs/specs/architecture.md` — remove the "in progress" gap entry once the gates are green (the target-state text already reads timelessly)

**Review checklist**:
- [ ] Both gates green at the widened scope; no unclassified scan sites, no blanket exemptions
- [ ] Emitter delegation is byte-identical where families merged (asserted, not assumed)
- [ ] The architecture.md gap entry for this plan is deleted; remaining backbuild gap (executed-statement parity pending wiring) recorded in behavioural terms
- [ ] `docs/TODO.md` updated

**Commit.** `refactor(logical): backbuild emitters under single-owner families; walk_coverage and no-authoring gates widened`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **Phase 2 — `maintenance::grouping` keeps the pre-unification, un-gated column-ref collector.**
  Characterization found `analysis::skeleton_closure` and `maintenance::grouping`'s copies had
  silently diverged from the other three (missing an `EXPRESSION`-kind guard, causing a bare
  function call to be misread as a column reference while silently dropping every real reference
  among its arguments). `skeleton_closure` was repointed to the fixed collector with zero
  conformance regression. `grouping` could not be: the fix flips two `maintenance_conformance`
  cases to a different maintenance technique, an admission-verdict change outside Phase 2's
  behaviour-preservation contract. `grouping` therefore calls the deliberately-kept
  `expr_util::collect_column_refs_ungated`. Tracked as a follow-up in `docs/TODO.md`
  ("`maintenance::grouping`'s column-ref collector keeps a known under-collection bug").
- **Phase 2 — `analysis::walk`'s `collect_self_conjunct_ranges` stays a distinct function**, now
  implemented as a thin consumer of `expr_util::split_top_level_conjuncts` rather than folded into
  its signature: its own output is text ranges for region carving (blanking excluded ranges out of
  a scope's own SQL text), not `Vec<Expr>` — a genuinely different shape from the two unified
  `Vec<Expr>`-returning splitters (`backbuild::diff`'s `split_conjuncts`,
  `backbuild::classify`'s `split_top_level_and`, which were byte-identical and are now one
  function).

## Verification

How to confirm the spec is satisfied at the end:
- `cargo test -p smelt-logical --test walk_coverage` green with `SCANNED_DIRS = {analysis,rules,maintenance,backbuild}`
- `cargo test -p smelt-runtime --test statement_parity` green with the widened no-authoring scope
- `cargo test -p smelt-logical --test backbuild_conformance` and `--test backbuild_property` green (oracle equivalence preserved through the unification)
- `cargo test -p smelt-cli --test maintenance_conformance` green
- `rg -c 'fn collect_column_refs|same_modulo_trivia|is_constant_literal' crates/smelt-logical/src` shows one home each
- `bash .claude/scripts/verify-phase.sh`
