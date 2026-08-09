# Plan: Keyed frontier — column-family classifier union and the snapshot-reconcile executor

**Date**: 2026-08-09
**Spec**: [`docs/specs/incremental_models.md`](../specs/incremental_models.md) §"The column-family catalogue", §"Per-cell admission" (admission matrix, column-family × run-shape), §"The algebraic maintenance ladder"; [`docs/specs/model_properties.md`](../specs/model_properties.md) §"Algebraic discriminants", the functional-dependency declaration
**Spec diff**: none — implements already-specified behavior (the catalogue, matrix, and refusal codes are settled surface). Ladder rungs 2–4 wiring into keyed columns is **excluded**: the spec flags it "specified ahead of use" and it needs a spec pass first.
**Tracking PR / branch**: `spec-redraft-incremental-models`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

Execute phase by phase with implementer + reviewer subagents; spec sections above are the oracle.

1. Read this plan, then the named spec sections. Confirm branch `spec-redraft-incremental-models`.
2. Next `pending` phase → implementer (red-green) → reviewer (material findings, spec-anchored) → fix → commit with the phase's `Commit.` line → push → record.
3. Per-phase gate: `bash .claude/scripts/verify-phase.sh` + `cargo test -p smelt-cli --test maintenance_conformance` + `cargo test -p smelt-cli --test e2e` + `cargo test -p smelt-runtime --test statement_parity`.
4. Pause for the user on: repeated material finding, spec conflict, pre-existing unrelated failure.
5. Timeless-oracle rule applies to all spec/docs edits.

## Context

The keyed classifier (`rules/cumulative.rs::classify_cumulative`) recognizes only the direct-monoid families (additive, extremal/lattice); order-monotone overwrite (`MAX_BY`), once-write (`COALESCE`), and plain overwrite all fall into `KeyedUnknownCombiner`. Run shape is derived (one clocked source ⇒ window-forward; zero ⇒ refused `KeyedSnapshotPostureUnsupported` at classification — no snapshot-reconcile branch exists). The reconciliation-ledger substrate (`Grade::Additive`/`fold_ledger_delta`) already implements reprocessing refusal but surfaces it as an unnamed error, and `KeyedOnceWriteUnproven` does not exist in code. The conformance keyed pool generates only `SUM`/`MAX`. This plan lands the family union, the named ledger diagnostic, the snapshot-reconcile executor, and the once-write family, each family arriving with its admission-matrix conformance recipes including the refusal directions.

## Scope

### In scope
- Overwrite family (`MAX_BY`/`MIN_BY`, order-monotone) — classification, merge rendering, window-forward admission, snapshot-reconcile refusal direction.
- `KeyedReprocessedWindow` as a real named diagnostic over the existing ledger substrate.
- Snapshot-reconcile run shape: plain-overwrite family + executor (whole-source read, keyed MERGE, retained-departed-keys semantics, window-flag rejection) and the matrix's refusal directions (fold families refuse snapshot-reconcile; plain overwrite refuses window-forward with the `MAX_BY` hint).
- Once-write family (`COALESCE`) with the provenance proof consumed from the functional-dependency verdict (declaration-backed, plus the key-derived special case); `KeyedOnceWriteUnproven` introduced.
- Conformance pool extension: new `KeyedCombiner` variants + snapshot-reconcile recipes exercised end to end.

### Explicitly deferred
- Ladder rungs 2–4 wiring into keyed columns (decomposed state, invertible/retraction, bounded-domain multiset) — needs a spec pass first (`/smelt:spec`); rung 3 additionally depends on change-feed consumption design.
- Pattern functions `smelt.latest`/`smelt.once`/`smelt.current` — the built-in-vs-template decision is an open spec question; the families land on their hand-written SQL spellings, which the pattern functions would later expand to.
- Key deletion/retention semantics beyond retained-departed-keys.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 030b5cd2 | 2026-08-09 |
| 2     | done     | 534d9cde | 2026-08-09 |
| 3     | done     | e437a65f | 2026-08-09 |
| 4     | pending  |        |      |
| 5     | pending  |        |      |

---

### Phase 1: Overwrite family (order-monotone `MAX_BY`/`MIN_BY`)

**Goal.** Classify `MAX_BY(value, ordering)`/`MIN_BY` as the order-monotone overwrite family (`incremental_models.md` §"The column-family catalogue") and render its keyed merge (`CASE WHEN delta.ord >= target.ord THEN delta.val ELSE target.val` shape with the ordering column carried), admitted window-forward, refused snapshot-reconcile per the matrix. Mixed-family projections (`SUM` + `MAX_BY` + `MIN` in one select list) fold column-wise.

**TDD tests to write first.**
- `crates/smelt-logical/tests/keyed_families.rs::max_by_classifies_as_order_monotone_overwrite` — `FoldSpec` carries the family + ordering column; no `KeyedUnknownCombiner`.
- `crates/smelt-logical/tests/keyed_families.rs::mixed_family_projection_classifies_columnwise` — `SUM` + `MAX_BY` + `MIN` in one model, each column its own family.
- `crates/smelt-logical/tests/keyed_families.rs::bare_nonkey_projection_still_unknown` — a plain non-aggregate projection still refuses `KeyedUnknownCombiner` (plain overwrite is Phase 3's snapshot-shape family, not window-forward).
- `crates/smelt-runtime/tests/technique_lowering.rs::max_by_merge_renders_incumbent_comparison` — rendered MERGE compares ordering values with incumbent-wins tie semantics (`incremental_models.md` ties rule).
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs` — pool extension: `KeyedCombiner::OrderMonotone` generated; equivalence across permuted/redelivered schedules (order-monotone folds are idempotent ⇒ `Grade::Idempotent`, no ledger).
- Refusal direction: snapshot-posture model with `MAX_BY` refuses `KeyedSnapshotPostureUnsupported` (unchanged until Phase 3, then re-asserted against the matrix).

**Implementation shape.** `rules/cumulative.rs`: extend `combiner_for`/`classify_cumulative` via `combiner_discriminants`' `Monotone::Order` arm; `CrossPartitionCombiner` gains the order-monotone variant carrying `(value_expr, ordering_expr)`; `smelt-runtime/src/cumulative.rs::build_cumulative_merge_sql` renders it (emitter stays in `smelt-logical`'s maintenance/emit layer where the statement-parity gate requires). Ledger grading: idempotent.

**Critical files.** `crates/smelt-logical/src/rules/cumulative.rs`, `crates/smelt-logical/src/maintenance/emit.rs` (if the merge body is emitter-owned), `crates/smelt-runtime/src/cumulative.rs`, `crates/smelt-maintenance-testkit/src/recipe.rs`, `crates/smelt-cli/tests/maintenance_conformance/gate.rs`, `crates/smelt-logical/tests/keyed_families.rs`.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences "classifier covers only the direct-monoid families" shrinks (overwrite landed); docs-site keyed page names the family if it enumerates them.

**Review checklist.**
- [ ] Incumbent-wins tie semantics per spec, sequential execution assumption unchanged
- [ ] Column-wise family folding, no whole-model refusal on mixed families
- [ ] Conformance equivalence incl. redelivery/permutation
- [ ] Statement parity green

**Commit.** `feat(keyed): order-monotone overwrite family (MAX_BY/MIN_BY) classified, rendered, oracle-proven`

---

### Phase 2: `KeyedReprocessedWindow` named diagnostic

**Goal.** The ledger's reprocessing refusal (currently an unnamed `bail!`) becomes the named `KeyedReprocessedWindow` diagnostic the spec's table promises, surfaced through the standard diagnostic path with the window and remedy named.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/(driver tests)::reprocessed_window_refusal_names_the_diagnostic` — re-merge of an already-reflected delta surfaces the code, window bounds, and the full-refresh remedy.
- `crates/smelt-db/tests/diagnostics_catalogue.rs` — catalogue entry.
- Conformance redelivery schedule asserts the refusal is loud and state unchanged (existing hazard schedule extended to assert the code).

**Implementation shape.** Name the refusal at the `fold_ledger_delta`/driver seam; catalogue + exhaustiveness gates.

**Critical files.** `crates/smelt-runtime/src/{cumulative.rs,maintenance_driver.rs}`, `crates/smelt-backend/src/lib.rs` (error type only), `crates/smelt-db` diagnostics catalogue files.

**Docs touched.** Diagnostics table row confirmed in `incremental_models.md` §Surface (already listed — mark nothing; entry exists), docs-site diagnostics page if codes are enumerated there.

**Review checklist.**
- [ ] Refusal behavior unchanged, only named
- [ ] Catalogue/exhaustiveness gates compile

**Commit.** `feat(runtime): name the ledger reprocessing refusal KeyedReprocessedWindow`

---

### Phase 3: Snapshot-reconcile run shape — plain-overwrite family + executor

**Goal.** A keyed model over zero clocked sources derives the snapshot-reconcile run shape instead of refusing: plain-overwrite columns admit (incoming row wins), fold families refuse per the matrix (additive double-counts; extremal/order-monotone observer semantics), retained-departed-keys is the asserted default, window flags are rejected loudly.

**TDD tests to write first.**
- `crates/smelt-logical/tests/keyed_families.rs::zero_clocked_sources_derives_snapshot_reconcile` — classification succeeds with run-shape snapshot-reconcile and plain-overwrite columns.
- `crates/smelt-logical/tests/keyed_families.rs::additive_fold_refuses_snapshot_reconcile` — `SUM` under snapshot posture refuses (matrix ✗, "double-counts" reason); `MAX_BY` likewise (observer semantics).
- `crates/smelt-logical/tests/keyed_families.rs::plain_overwrite_refuses_window_forward_with_max_by_hint` — the matrix's other direction (`KeyedUnknownCombiner` message names `MAX_BY` as the fix).
- `crates/smelt-runtime/tests/technique_lowering.rs::snapshot_reconcile_merges_whole_source_no_window` — statement leg: whole-source USING select, keyed MERGE, no delete of departed keys.
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs` — the existing `KeyedRecipe::new_snapshot_reconcile` scaffolding runs end to end: mutate/delete source rows between runs; end state equals the oracle modulo retained departed keys (the documented adjustment `retained_departed_keys_adjusts_the_oracle` already encodes); `--event-time-start/-end` rejected loudly.

**Implementation shape.** `classify_cumulative`'s `NoCandidate` arm becomes the snapshot-reconcile derivation (posture recorded on the classification); executor path parallel to `run_windowed_keyed_maintenance` without `driving_steps` windowing; plain-overwrite renderer (no ledger — idempotent by construction); CLI flag validation at the request boundary.

**Critical files.** `crates/smelt-logical/src/rules/cumulative.rs`, `crates/smelt-runtime/src/{cumulative.rs,maintenance_driver.rs,execute.rs}`, `crates/smelt-maintenance-testkit/src/recipe.rs`, conformance gate files, `crates/smelt-cli` flag validation.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences "snapshot-reconcile executor is unbuilt" deleted; docs-site keyed/snapshot page updated (timeless).

**Review checklist.**
- [ ] Matrix refusal directions asserted BOTH ways (fold ↛ snapshot; plain-overwrite ↛ window-forward)
- [ ] Retained-departed-keys semantics explicit in the oracle adjustment
- [ ] No ledger for idempotent snapshot merges; statement parity green

**Commit.** `feat(keyed): snapshot-reconcile run shape — plain-overwrite family and executor, oracle-proven`

---

### Phase 4: Once-write family (`COALESCE`) with the provenance proof

**Goal.** `COALESCE`-first-non-null keyed columns classify as the once-write family, admitted window-forward only when the once-write provenance proof holds — key-derived expression, or a declared functional dependency consumed from `functional_dependency_verdict_over_vector` — refusing `KeyedOnceWriteUnproven` (new diagnostic) with the three named fixes otherwise.

**TDD tests to write first.**
- `crates/smelt-logical/tests/keyed_families.rs::coalesce_with_declared_fd_classifies_once_write` — declared `functional_dependencies: key → col` admits; the FD verdict is consulted (a `OneToMany` fan-out into the column refuses regardless of declaration).
- `crates/smelt-logical/tests/keyed_families.rs::key_derived_once_write_needs_no_declaration` — a key-derived expression admits without the declaration.
- `crates/smelt-logical/tests/keyed_families.rs::unproven_once_write_refuses_with_three_fixes` — `KeyedOnceWriteUnproven` names key-derived form, declared FD, remodelling.
- `crates/smelt-runtime/tests/technique_lowering.rs::once_write_renders_coalesce_target_first` — `COALESCE(target.col, delta.col)` merge arm.
- Conformance: once-write recipe with late redelivery — first-written value survives; equivalence vs oracle given the FD world-fact holds in staged data; a schedule violating the FD is NOT generated (world-fact contract).
- `crates/smelt-db/tests/diagnostics_catalogue.rs` — new code entry.

**Implementation shape.** Classifier arm for `COALESCE` over a keyed column; provenance proof = `functional_dependency_verdict_over_vector` (first consumer wired — closes that "no consumer" divergence in `model_properties.md`) plus the key-derived syntactic case; renderer + idempotent grading; diagnostic through catalogue/exhaustiveness gates.

**Critical files.** `crates/smelt-logical/src/rules/cumulative.rs`, `crates/smelt-logical/src/analysis/functional_dependency.rs` (consumer seam only), `crates/smelt-runtime/src/cumulative.rs`, testkit + conformance files, `crates/smelt-db` catalogue.

**Docs touched.**
- `docs/specs/model_properties.md` — Known Divergences: FD declaration "no consumer wired" updated (once-write consumer live).
- `docs/specs/incremental_models.md` — catalogue divergence shrinks to the classifier-union residue actually remaining; docs-site keyed page.

**Review checklist.**
- [ ] Declaration widens only the undecidable case (proof-positive multi-valued refuses regardless) — spec's widen-only law
- [ ] The three-fix refusal text matches the diagnostics table
- [ ] Conformance redelivery direction covered

**Commit.** `feat(keyed): once-write family with FD-backed provenance proof; KeyedOnceWriteUnproven`

---

### Phase 5: Divergence closure and doc sync

**Goal.** Sweep `incremental_models.md` key-grain divergences this plan landed (classifier union direct-monoid restriction, snapshot-reconcile executor, `KeyedReprocessedWindow` naming), leaving honest residues (pattern functions + built-in-vs-template decision open; rungs 2–4 spec-first; posture derivation extent); update docs-site keyed pages; `/smelt:validate incremental_models` triage.

**Critical files.** `docs/specs/incremental_models.md`, `docs/specs/model_properties.md`, `docs-site/docs/` keyed pages.

**Review checklist.**
- [ ] Gap-first residues with tracking pointers; timeless lint clean
- [ ] Drift report triaged

**Commit.** `docs(spec): keyed frontier landed — classifier union + snapshot-reconcile divergences closed`

---

## Deferred during implementation

(Append-only.)

## Verification

- `cargo test -p smelt-cli --test maintenance_conformance` — extended keyed pool (order-monotone, snapshot-reconcile, once-write) green
- `cargo test -p smelt-runtime --test statement_parity`, `cargo test -p smelt-logical --test walk_coverage`
- `bash .claude/scripts/verify-phase.sh`
- `/smelt:validate incremental_models` zero drift on the touched sections
