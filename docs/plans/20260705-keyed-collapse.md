# Plan: The keyed collapse — `refresh: keyed` replaces cumulative / latest_value / accumulating_snapshot

**Date**: 2026-07-05
**Master plan**: [`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) — registered in §"Spawned sub-plans" as the first pending row.
**Spec**: [`docs/specs/keyed_models.md`](../specs/keyed_models.md) — **primary oracle** (new spec; the diff is the whole file, commit `f23c5134`). Encodes the decision record [`docs/research/20260705-keyed-collapse-application.md`](../research/20260705-keyed-collapse-application.md) (D1–D16); design: [`docs/research/20260705-unified-keyed-refresh.md`](../research/20260705-unified-keyed-refresh.md), [`docs/research/20260705-model-refresh-review.md`](../research/20260705-model-refresh-review.md).
**Spec diff**: new spec (`keyed_models.md`), plus the companion edits K1 makes per the decision record §3 — each K1 edit is **pre-authorised spec work listed phase-by-phase**; no phase authors a new spec decision.
**Tracking branch**: `worktree-incremental`
**Docs**: code+docs

**Scope boundary (read first).** This sub-plan lands the **keyed collapse**: one `refresh: keyed` mode
(`keyed_models.md`) replacing `cumulative` (built — the seed) and the unbuilt `latest_value` /
`accumulating_snapshot`. It **supersedes** the registry rows for
`docs/plans/20260704-model-updates-l4-latest-value.md` and the un-registered
`docs/plans/20260704-accumulating-snapshot.md`, and **displaces**
`docs/plans/20260704-model-updates-l4-cumulative.md` (rungs 2–4), which is de-registered and
re-scaffolded against `keyed_models.md` after this plan completes — do **not** run it against
`cumulative_aggregate.md`, which K1 retires. It does **not** cover: rungs 2–4 (decomposed state,
retraction, multiset — the re-scaffolded successor's scope), `versioned`
(`docs/plans/20260704-model-updates-l4-versioned.md`, still registered), `materialized_view` emit
(`…-l4-materialized-view.md`, still registered — K1 only fixes its shape wording), batched work
(`…-l4-batched.md`), and everything in the decision record's §5 deferred list (union-of-streams anchor,
observer contract, per-key targeted recompute, `smelt.versions`, property-typed signatures).

---

## Execution prompt (for a fresh Claude session / the autonomy loop)

You are executing this plan phase by phase. It is a sub-plan registered in
[`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) §"Spawned sub-plans".

**Before touching any code:**
1. Read this entire plan, then [`docs/specs/keyed_models.md`](../specs/keyed_models.md) — it is the
   correctness oracle; do not re-open settled decisions (they are argued in
   `docs/research/20260705-keyed-collapse-application.md`). The invariant for every phase is
   **end-state equivalence with the model's own SQL as the oracle** (`keyed_models.md` §"End-state
   equivalence"), with exactly the two named carve-outs (retained departed keys; ordering-key ties).
2. Confirm you are on branch `worktree-incremental`, and that this phase's **Depends on** rows are `done`.
3. Find the next `pending` row in the Progress-tracking table. If every row is `done`, run §Verification,
   flip this sub-plan's registry Status to `done (<today>)` in the master, and stop.

**Per phase, run `/smelt:implement`'s loop:** pre-flight (`cargo build`/`cargo test` green except this
phase's own red target) → implementer subagent (red-green TDD on the listed tests) → reviewer subagent
(material findings only) → iterate → set the row `done` → commit + push with the phase's `Commit.` line.

**The shipped cumulative path is the seed and must never go dark.** Phases K2–K4 rename and generalise
the built `refresh: cumulative` path (`crates/smelt-logical/src/rules/cumulative.rs`,
`crates/smelt-runtime/src/cumulative.rs`, `crates/smelt-runtime/src/maintenance_driver.rs`). The
acceptance gate for **every** phase includes the existing cumulative end-state-equivalence harness
(`crates/smelt-cli/tests/cli_unit/cumulative_equivalence.rs`, `crates/smelt-cli/tests/cumulative*`,
`crates/smelt-cli/tests/e2e/backbuild_cumulative_e2e.rs`) staying green — after K2, under `refresh:
keyed` fixtures. A phase that flips one is a wiring bug, not a spec change; do not update equivalence
expectations to match new output. Equivalence tests need `DUCKDB_LIB_DIR` + `LD_LIBRARY_PATH` (CLAUDE.md).

**Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this file only*. Spec + `docs-site/`
edits describe the feature as if it has always existed; as behaviour ships, **remove or narrow** the
matching `keyed_models.md` §Known-Divergence note rather than annotating it with a phase number.

**Block rule.** On a design decision not answered here or by the spec (see §"Open decisions"), a
dependency capability not yet built, or a pre-flight red unrelated to this phase's target: set the row
`blocked` with a one-line reason, append to §"Blocked phases", restore a clean tree, commit, emit
`<<PHASE_BLOCKED>>`. Otherwise emit `<<PHASE_COMPLETE>>`.

---

## Context

`keyed_models.md` collapses the three keyed patterns into one mode because their differences are all
derived facts of the SQL (per-column combiner families), not contracts — and because combiner intent is
per column, so real models mix families in one table (spec §Design "One mode; the column family is the
pattern"). `refresh: cumulative` is the built seed: its classifier, the windowed-keyed-maintenance
driver, and the per-window `merge_into` execution become the keyed mode's core under a rename, then grow
the overwrite and once-write families, the transactional ledger, and the snapshot-reconcile posture.

## Scope

### In scope (spec coverage)

- **K1** — the companion spec edits (`keyed_models.md` is already authored): retire the three mode
  specs; `models.md` refresh axis + litmus fourth clause; `model_maintenance.md` invariant restatement;
  `batched_models.md` state-doctrine rescope; `model_transforms.md` / `model_properties.md` /
  `materialized_view.md` / `multi_backend.md` / `cli.md` / `data_catalog.md` / `smelt_yml.md` /
  `diagnostics.md` / `run_state.md` touchpoints (decision record §3).
- **K2** — `RefreshStrategy::Keyed` + the `cumulative` removal error + the `Keyed*` diagnostic family +
  the interim unclocked refusal (spec §Surface "Diagnostic codes"; §Known Divergences).
- **K3** — the order-monotone overwrite family (`MAX_BY`/`MIN_BY`), the three-property posture
  derivation, enrichment-join admission, and the `smelt explain` family/posture readout (spec §Surface
  "The column-family catalogue"; §Semantics "Derived execution postures", "Ordering ties",
  "Enrichment joins").
- **K4** — the transactional merge ledger (spec §Semantics "The transactional merge ledger";
  "Reprocessing").
- **K5** — the snapshot-reconcile executor + the plain-overwrite family + per-column admission matrix
  (spec §Semantics "The two run shapes", "Admission matrix").
- **K6** — the once-write family + the pattern functions `smelt.latest`/`smelt.once`/`smelt.current` +
  the docs-site keyed guide (spec §Surface catalogue + pattern-function paragraph).

### Explicitly deferred

- Rungs 2–4 for keyed columns (decomposed state / retraction / multiset) — the re-scaffolded successor
  of the de-registered `l4-cumulative` plan, against `keyed_models.md`.
- Everything in `docs/research/20260705-keyed-collapse-application.md` §5 (union-of-streams anchor,
  observer contract for the refused matrix cells, per-key targeted recompute, settled-key GC,
  `smelt.versions`, consumer-facing `timeseries:` on non-model outputs, mutation-profile tightening,
  self-emitted change feeds, property-typed signatures).
- `versioned` and `materialized_view` delivery — their own registered sub-plans.

## Progress tracking

| Phase | Depends on | Spec anchor | Status |
|-------|-----------|-------------|--------|
| K1 | — (`keyed_models.md` committed) | decision record §3 change list; `keyed_models.md` §References "Related specs" | done |
| K2 | K1 | `keyed_models.md` §Surface "YAML frontmatter", "Diagnostic codes"; §Known Divergences (parse state) | blocked |
| K3 | K2 | `keyed_models.md` §Surface "The column-family catalogue"; §Semantics "Derived execution postures", "Ordering ties", "Enrichment joins" | blocked |
| K4 | K2 | `keyed_models.md` §Semantics "The transactional merge ledger", "Reprocessing"; §Constraints 9, 11 | pending |
| K5 | K3, K4 | `keyed_models.md` §Semantics "The two run shapes", "Admission matrix"; §Constraints 7, 8 | pending |
| K6 | K3, K5 | `keyed_models.md` §Surface catalogue (once-write, pattern functions); §Semantics "Functions inside keyed bodies" | pending |

---

### Phase K1: Companion spec edits — retire the three mode specs, re-point the family

**Goal.** Make the spec set internally consistent with `keyed_models.md`: delete the three replaced mode
specs and land every §3 touchpoint from the decision record, so `/smelt:validate` has one home per rule
before any code moves.

**Pre-conditions.** `keyed_models.md` committed (`f23c5134`).

**TDD tests to write first.** (Docs phase — the "tests" are reference-integrity checks, run red first,
green after the edits:)
- `rg -l 'cumulative_aggregate\.md|latest_value_models\.md|accumulating_snapshot\.md' docs/specs/ docs-site/` returns **no files** (plans/ and research/ may still reference them as history).
- `rg -n '"cumulative"' docs/specs/cli.md docs/specs/data_catalog.md` shows the refresh enum as `full | batched | keyed | versioned | materialized_view`.
- `rg -n 'Cumulative' docs/specs/diagnostics.md` returns no live rows (renamed to `Keyed*` per `keyed_models.md` §"Diagnostic codes"; retired codes removed).

**Implementation shape.** Per the decision record §3, exactly: delete
`docs/specs/{cumulative_aggregate,latest_value_models,accumulating_snapshot}.md` (their Design rationale
already carried into `keyed_models.md`); rewrite `models.md` §"Refresh axis" to the five-value enum +
constraint-table rows + litmus fourth clause + Known-Divergences parse-state fix; restate the invariant
in `model_maintenance.md` (abstract processed-input set; replayability split; the two carve-outs) and
re-scope its horizon write-clamp language to batched; `model_transforms.md` — add the transactional-
merge-ledger row, narrow the dimension-horizon-MERGE licence to derived `H`, update the driver's
consumer list, re-tag eviction/GC deferred-with-late-fact-accounting; `model_properties.md` consumer
notes; `batched_models.md` Constraint 4 rescope + invariant-preserving strategy-choice constraint;
`run_state.md` ledger-vs-observability note; `materialized_view.md` output shape → engine-defined (also
absorbs the D16 item the decision record §4 assigned to the l4-materialized-view plan — note it there);
`multi_backend.md` keyed-mode enumeration; `cli.md` + `data_catalog.md` JSON enum; `smelt_yml.md`
mention; `diagnostics.md` catalogue rename/retire.

**Critical files (allowed to touch in this phase).** Only `docs/specs/*.md`, `docs-site/docs/**` (link
fixes only), `docs/plans/20260704-model-updates-l4-materialized-view.md` (the D16-absorbed note), and
this plan's Progress row. **No code.**

**Docs touched.** The list above *is* the phase. All edits timeless.

**Review checklist** (material findings only):
- [ ] No spec or docs-site file references a retired spec; every §3 touchpoint landed.
- [ ] `models.md` litmus rule has the function clause; refresh table matches `keyed_models.md`.
- [ ] `model_maintenance.md` invariant restatement carries both carve-outs and the replayability split.
- [ ] `batched_models.md` still forbids what it forbade — the rescope names the keyed ledger as the one
      exception, it does not weaken batched's own state doctrine.
- [ ] All edits timeless — no phase vocabulary.

**Commit.** `spec(keyed): retire the three keyed-mode specs; land the collapse's companion edits across the spec set`

---

### Phase K2: `RefreshStrategy::Keyed` — enum, rename, and the interim unclocked refusal

**Goal.** `refresh: keyed` parses and drives the existing (seed) execution path; `refresh: cumulative`
is a hard error pointing at `keyed`; the mode-local diagnostics carry `Keyed*` names; an unclocked keyed
model is refused fail-loud with the interim not-yet diagnostic.

**Pre-conditions.** K1 (specs consistent). The seed path is green under `cumulative` fixtures.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs` unit — `refresh: keyed` deserializes to `RefreshStrategy::Keyed`;
  `refresh: cumulative` fails with a message containing exactly *"`refresh: cumulative` is now
  `refresh: keyed`"*; `latest_value` / `accumulating_snapshot` remain unknown-value errors.
- `crates/smelt-core/src/metadata.rs` unit — `refresh: keyed` + `timeseries:` → `KeyedForbidsTimeseries`;
  `+ batched:` → `KeyedForbidsBatched` (renamed codes, unchanged triggers).
- `crates/smelt-logical/src/rules/` unit — the classifier emits `Keyed*` codes (`KeyedRequiresGroupBy`,
  `KeyedUnknownCombiner`, `KeyedGroupByContainsPartitionColumn`, `KeyedForbidsWindowFunctions`,
  `KeyedForbidsNondeterministic`, `KeyedSqlNotParseable`, `KeyedMultipleDrivingSources`); the
  zero-clocked-sources case emits `KeyedSnapshotPostureUnsupported` (fail-loud not-yet, naming this plan)
  instead of the retired `CumulativeNoDrivingSource`.
- **End-state equivalence (DuckDB harness) — must not regress.** The existing cumulative equivalence +
  backbuild e2e suites pass with their fixtures flipped to `refresh: keyed` (same SQL, same expected
  state). Real fixtures under `examples/` updated in the same commit.

**Implementation shape.** Rename `RefreshStrategy::Cumulative → Keyed` (serialize `"keyed"`); add the
pointing error to the deserializer; rename the diagnostic enum/codes and config plumbing
(`CumulativeDiagnostic → KeyedDiagnostic` etc.); re-point code comments from the retired spec to
`keyed_models.md`; rename the classifier/runtime modules only if trivial (`rules/cumulative.rs` may keep
its filename with a header note, matching the batched precedent in `batched_models.md`-era renames — do
not let a file rename bloat the diff). Update `examples/` fixtures and the smelt-app-builder-visible
docs strings that name `cumulative`.

**Critical files.**
- `crates/smelt-core/src/config.rs`, `crates/smelt-core/src/metadata.rs` — enum, error, constraint codes.
- `crates/smelt-logical/src/rules/cumulative.rs` — diagnostic renames; the interim unclocked refusal.
- `crates/smelt-runtime/src/{cumulative,execute,compile}.rs` — dispatch on the renamed variant.
- `crates/smelt-cli/tests/**`, `examples/**` — fixture flips.

**Docs touched.**
- `keyed_models.md` §Known Divergences — narrow "Nothing named `keyed` parses yet" to the remaining gaps
  (families, ledger, snapshot posture).
- `docs-site/docs/guide/materializations.md` — `refresh: keyed` replaces `refresh: cumulative` in prose
  and examples (timeless).

**Review checklist.**
- [ ] `cumulative` errors with the exact pointer message; `keyed` runs the seed path.
- [ ] Equivalence harness green under `refresh: keyed` fixtures — no expectation edits.
- [ ] No diagnostic code named `Cumulative*` remains emitted; `diagnostics.md` matches.
- [ ] The unclocked case is the fail-loud not-yet refusal, not a model error.

**Commit.** `feat(keyed): RefreshStrategy::Keyed replaces Cumulative; pointing config error; Keyed* diagnostics`

---

### Phase K3: The overwrite family, posture derivation, and enrichment-join admission

**Goal.** `MAX_BY(value, ordering)` / `MIN_BY` classify and execute (incumbent-wins-on-ties merge);
the model's three derived postures (re-run tolerance / order-independence / reprocessing-refusal) are
computed from the column families and surfaced by `smelt explain`; enrichment joins are admitted by the
join-contribution monotonicity proof and refused with `KeyedRetractableContribution` otherwise.

**Pre-conditions.** K2. Consumed by name (built by the fundamentals sub-plan — never re-derived here):
the value/order-monotone discriminant (`analysis::discriminants::combiner_discriminants`),
join-contribution monotonicity (`analysis::join_shape::join_contribution_monotone`), driving-fact
resolution (`analysis::source_bounds::resolve_single_anchor`).

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/cumulative.rs` unit — `MAX_BY(status, updated_at)` classifies to the
  order-monotone overwrite family with an ordering column; a bare column / `ANY_VALUE` non-key
  projection under window-forward refuses (`KeyedUnknownCombiner`) with a message naming `MAX_BY` + an
  ordering column as the fix; a **mixed-family model** (`MIN` + `MAX_BY` + `SUM` in one projection list —
  the spec's §Surface example) classifies with per-column families.
- posture unit — all-idempotent model ⇒ re-run tolerant; any `SUM` ⇒ not; any `MAX_BY` ⇒ not
  order-independent; all-extremal ⇒ both. Postures derived, never declared.
- merge-SQL unit (`crates/smelt-runtime/src/cumulative.rs`) — the overwrite combiner renders
  delta-wins-iff-strictly-greater with incumbent-wins on equality (spec §"Ordering ties"), e.g.
  `CASE WHEN delta.ord > target.ord THEN delta.val ELSE target.val END` for both the value and the
  ordering column.
- join admission unit — a dimension join feeding only extremal/overwrite columns with a proven monotone
  contribution is admitted; a join fanning into a decrementing aggregate refuses
  `KeyedRetractableContribution`; the refusal never fires on join spelling alone.
- **End-state equivalence (DuckDB harness) — extended.** Real fixture in `examples/`: a keyed model
  mixing `SUM` + `MAX_BY` + `MIN` maintained across ≥3 source windows equals a full refresh of its own
  SQL over those windows, including out-of-order arrivals *within* the windows; the rung-1 cases stay
  green. Requires `DUCKDB_LIB_DIR`.
- `smelt explain --json` — reports per-column family and the three model postures for a keyed model.

**Implementation shape.** Extend the classifier's `AggregatorColumn`/combiner map with the overwrite
family (value column + ordering column pair; the merge builder needs both in the delta SELECT); add a
`KeyedPostures` derivation over the classified columns; wire `join_contribution_monotone` as an
admission check on FROM-clause joins feeding non-key projections; extend `build_cumulative_merge_sql`
with the overwrite rendering; extend the explain JSON with `column_families` + `postures`. Sequential
window order is already the driver's only mode — posture 2 requires no scheduler work, only the derived
flag (parallel backfill is a later capability; the flag is what future work consumes).

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — family classification, postures, join admission.
- `crates/smelt-runtime/src/cumulative.rs` — overwrite merge rendering.
- `crates/smelt-cli/src/commands/explain.rs` (+ `src/explain.rs`) — family/posture readout.
- `crates/smelt-cli/tests/**`, `examples/**` — mixed-family fixture + equivalence.

**Docs touched.**
- `keyed_models.md` §Known Divergences — remove the overwrite-family gap; narrow the classifier-union note.
- `docs-site/docs/guide/materializations.md` — the overwrite family + ordering-column recommendation
  (composite tie-free key), timeless.

**Review checklist.**
- [ ] Mixed-family model classifies and its equivalence holds — the collapse's headline case.
- [ ] Tie behaviour is incumbent-wins and documented; no order-independence claimed for overwrite columns.
- [ ] Join admission is semantic (monotone contribution), not syntactic; fail-closed message steers to
      `materialized_view` / DAG composition.
- [ ] Postures appear in explain; nothing declares them.

**Commit.** `feat(keyed): order-monotone overwrite family (MAX_BY/MIN_BY), derived postures, enrichment-join admission`

---

### Phase K4: The transactional merge ledger

**Goal.** Every window-forward keyed run records each merged window in a per-model ledger, atomically
with the merge; additive-posture models exactly refuse a ledgered window's re-run
(`KeyedReprocessedWindow`) and resume mid-crash runs exactly; re-run-tolerant models re-merge freely.

**Pre-conditions.** K2 (the mode exists under its final name). K3 useful but not required (posture
derivation of "additive present" is computable from the rung-1 families alone); **Depends on** K2 only.

**TDD tests to write first.**
- backend unit (`crates/smelt-backend-duckdb`) — `merge_into` + ledger insert commit atomically: a
  failure injected between them leaves **neither** applied.
- `crates/smelt-runtime/src/cumulative.rs` unit — a `SUM` model re-run over a ledgered window refuses
  with `KeyedReprocessedWindow` naming `--full-refresh`; an all-idempotent (`MIN`/`MAX`) model re-merges
  the same window as a no-op.
- **crash-resume e2e (DuckDB harness)** — a run over `[D1, D4)` interrupted after `D2` (inject a failure
  at `D3`) resumes with the same flags and merges exactly `D3` — end state equals the uninterrupted run
  equals a full refresh.
- ledger hygiene unit — the ledger table is not a model, not a dependency target, not selected by
  selectors, and `--full-refresh` truncates it with the target.

**Implementation shape.** Ledger table `<target>__smelt_ledger(window_start, window_end, run_id,
merged_at)` in the target's schema (naming per §"Open decisions"); a `Backend` transactional entry point
that executes the merge statement + ledger insert in one transaction (DuckDB: explicit transaction;
Spark/Delta: single-commit semantics — Spark impl may be a documented `todo!` gated by capability flag,
consistent with the batched Spark posture); the driver consults the ledger before each step and applies
the posture rule; `--full-refresh` drops/truncates the ledger with the target.

**Critical files.**
- `crates/smelt-backend/src/lib.rs` + `crates/smelt-backend-duckdb/src/lib.rs` — the transactional entry
  point.
- `crates/smelt-runtime/src/{cumulative,maintenance_driver}.rs` — ledger consult + refusal + resume.
- `crates/smelt-cli/tests/e2e/` — crash-resume + refuse tests.

**Docs touched.**
- `keyed_models.md` §Known Divergences — remove "The ledger is unbuilt".
- `model_transforms.md` — flip the ledger row's maturity.
- `docs-site/docs/guide/materializations.md` — re-run/resume behaviour of keyed models (timeless).

**Review checklist.**
- [ ] Atomicity proven by the injected-failure test, not asserted.
- [ ] Additive refusal is exact (ledger), not best-effort; idempotent re-merge allowed.
- [ ] Ledger is invisible to selection/deps; truncated on `--full-refresh`.
- [ ] Behaviour change from the seed path (blind `SUM` re-run now refused) is called out in the commit body.

**Commit.** `feat(keyed): transactional merge ledger — exact re-run refusal for additive folds, exact crash resume`

---

### Phase K5: Snapshot-reconcile executor + the admission matrix

**Goal.** An unclocked keyed model runs as a whole-scan reconcile (plain-overwrite family), the
`--event-time` flags are a hard error for it, departed keys are retained, `--auto` treats it as
always-stale, and the per-column admission matrix refuses fold/overwrite/once-write columns over
snapshots (`KeyedSnapshotSourceUnsupportedColumn`), retiring the interim `KeyedSnapshotPostureUnsupported`.

**Pre-conditions.** K3 (family machinery), K4 (posture/ledger split — snapshot models keep no ledger).

**TDD tests to write first.**
- classifier unit — under zero clocked sources: `ANY_VALUE(attr)` classifies to plain overwrite;
  `SUM`/`MIN`/`MAX_BY`/once-write columns refuse `KeyedSnapshotSourceUnsupportedColumn` (message names
  the family and the observer-semantics reason); under window-forward `ANY_VALUE` still refuses with the
  `MAX_BY` fix message (matrix enforced both directions).
- CLI unit — `--event-time-start/-end` on a snapshot-posture model is a hard error with the spec's
  message; a window-forward model still requires them.
- **reconcile e2e (DuckDB harness)** — run 1 over snapshot S1 stores S1's rows per key; mutate to S2
  (change one key, add one, delete one); run 2 stores: changed key updated, new key inserted, departed
  key **retained**; every key present in S2 equals a full refresh of the SQL over S2.
- `--auto` unit — snapshot-posture model is always stale.

**Implementation shape.** A reconcile execution path beside the windowed driver: one delta SELECT over
the whole source, one `merge_into` with the plain-overwrite (incoming-wins) combiner per column; posture
selection at dispatch on the presence of a clocked anchor; no ledger. The retained-keys rule is the
`merge_into` default (no delete clause) — assert it explicitly rather than inheriting it silently.

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — plain-overwrite family + matrix enforcement + interim
  diagnostic retirement.
- `crates/smelt-runtime/src/{cumulative,execute}.rs` — the reconcile path + flag validation.
- `crates/smelt-cli/tests/e2e/` — the S1→S2 reconcile fixture under `examples/`.

**Docs touched.**
- `keyed_models.md` §Known Divergences — remove the snapshot-executor gap and the interim diagnostic.
- `docs-site/docs/guide/materializations.md` — the two run shapes, derived; the retained-keys rule
  (timeless).

**Review checklist.**
- [ ] Matrix enforced per column, both postures; messages name family + reason.
- [ ] Departed keys retained and documented; present keys equal the current-snapshot oracle.
- [ ] Flags error for snapshot posture; `--auto` always-stale.
- [ ] Interim `KeyedSnapshotPostureUnsupported` gone from code and `diagnostics.md`.

**Commit.** `feat(keyed): snapshot-reconcile executor — plain-overwrite family, per-column admission matrix, retained keys`

---

### Phase K6: Once-write family + pattern functions + the keyed guide

**Goal.** The once-write family lands with its canonical spelling settled; `smelt.latest`, `smelt.once`,
and `smelt.current` ship as transparent pattern functions admitted purely by their expansions; the
docs-site keyed guide teaches the patterns (running total / latest value / milestone / mixed lifecycle
table).

**Pre-conditions.** K3 (families + join admission), K5 (plain overwrite exists for `smelt.current`).
Consumed by name: the once-write provenance verdict
(`analysis::functional_dependency::functional_dependency_verdict` + the key-derived case).

**TDD tests to write first.**
- classifier unit — the canonical once-write spelling (settled per §"Open decisions") classifies to the
  once-write family **only** with a provenance proof (key-derived, or declared FD on the driving
  source); unproven → `KeyedOnceWriteUnproven` naming the column and the two provable forms; merge
  renders `COALESCE(target.col, delta.col)`.
- pattern-function units — `smelt.latest(status, updated_at)` expands to `MAX_BY` and classifies
  identically to the hand-written form (assert identical classification output, not just acceptance);
  `smelt.once(first_touch)` expands to the canonical once-write spelling and still requires the
  provenance proof; `smelt.current(tier)` expands to `ANY_VALUE` and is admitted only under
  snapshot-reconcile.
- **End-state equivalence (DuckDB harness) — extended.** A milestone fixture (event stream + conversion
  enrichment, `MIN`/`MAX`/once-write columns) across out-of-order windows equals a full refresh —
  the accumulating-snapshot headline case, expressed in the unified mode.
- `cargo test -p smelt-cli --test example_diagnostics` — the new `examples/` fixtures (mixed lifecycle,
  milestone, snapshot latest-value) are diagnostic-clean.

**Implementation shape.** Add the once-write entry to the family catalogue gated on the provenance
verdict; settle built-in vs template delivery for the three functions (§"Open decisions") — if built-in
transparent bodies are not yet hostable in the function registry, ship them as documented template
snippets in the guide and record that in `keyed_models.md` §Known Divergences; author
`docs-site/docs/guide/keyed-models.md` (replacing the keyed section of `materializations.md`) with the
four recipes and the explain readout.

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — once-write family + provenance gate.
- `crates/smelt-runtime/src/cumulative.rs` — `COALESCE` merge rendering.
- the function-registry / template home settled by the open decision.
- `docs-site/docs/guide/keyed-models.md` (+ `materializations.md` trim), `examples/**`.

**Docs touched.**
- `keyed_models.md` §Known Divergences — remove the pattern-function and once-write gaps; record the
  canonical spelling in §Surface if it differs from the drafted `COALESCE`-first-non-null wording.
- `functions.md` — the three pattern functions, if delivered as built-ins.
- `docs-site/docs/guide/keyed-models.md` — the guide.

**Review checklist.**
- [ ] Pattern functions have zero privileged treatment — classification equals the hand-written form.
- [ ] Once-write fails closed without provenance; both provable forms tested.
- [ ] The milestone equivalence fixture covers out-of-order windows.
- [ ] Guide teaches by pattern, timeless; `materializations.md` no longer describes retired modes.

**Commit.** `feat(keyed): once-write family + smelt.latest/once/current pattern functions + keyed guide`

---

## Blocked phases

- **2026-07-05, K2** — pre-flight `cargo test -p smelt-core --test hardening_budget` (the
  unwrap/expect ratchet gate, CLAUDE.md §"Fail-loud discipline") is red on `main`/this branch's tip
  independent of any K2 change: `.claude/scripts/hardening-budget.sh` reports (a) a REGRESSION —
  `smelt-backend-duckdb expect: current=16 > baseline=15` — and (b) a STALE BASELINE —
  `smelt-cli expect: current=34 < baseline=36`. Neither crate is in K2's critical-file list
  (`smelt-core/src/{config,metadata}.rs`, `smelt-logical/src/rules/cumulative.rs`,
  `smelt-runtime/src/{cumulative,execute,compile}.rs`); the drift predates this sub-plan (bisects to
  somewhere in the batched/backfill commits between `f72c1d7d` and `c6b0f158`, none of which touched
  `.claude/hardening-baseline.txt`). Resolving it requires a human judgment call the gate itself
  reserves for a reviewer sign-off note (CLAUDE.md: "never lower them without a reviewer sign-off
  note in the commit") — either (1) find and convert the new `smelt-backend-duckdb` `.expect(` to a
  classified/justified baseline bump, or (2) revert whichever commit introduced it. The stale
  `smelt-cli` baseline is a pure tightening (`--update` is safe there) but the regression is not.
  Candidate options for the human: run `.claude/scripts/hardening-budget.sh --update` after auditing
  the new `smelt-backend-duckdb` `.expect(` call for infallibility, or bisect+revert. This blocks
  K2 and therefore the rest of the keyed-collapse chain (K3–K6 all depend on K2 being `done`) until
  resolved — no code was touched in this iteration; the tree is unchanged from `HEAD`.

- **2026-07-05, K3** — re-verified pre-flight: `cargo test -p smelt-core --test hardening_budget` is
  still red with the identical regression/stale-baseline pair reported under K2 above (same
  `smelt-backend-duckdb expect: current=16 > baseline=15`, same `smelt-cli` stale baseline). K3's own
  precondition is K2 `done`, which it is not — K3 cannot start (the overwrite-family/posture work is
  built on the renamed `RefreshStrategy::Keyed` enum that K2 lands). Marking K3 `blocked` rather than
  re-litigating the same gate; no code touched, tree unchanged from `HEAD`. Unblocks automatically once
  a human resolves the K2 block above and flips K2 to `done`.

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Open decisions surfaced for the implementer

Settle each in its owning phase; if a choice cannot be made from the spec + decision record, **block**
(do not guess a contract-changing default).

- **Ledger table naming/layout (K4).** Default: `<target>__smelt_ledger(window_start, window_end,
  run_id, merged_at)` in the target's schema, coordinated with `run_state.md`'s naming conventions. A
  storage decision, not a contract fork.
- **Spark ledger transactionality (K4).** DuckDB is the required implementation; if single-commit
  merge+ledger is not achievable on the Spark backend this pass, gate it behind a capability flag with a
  hard error (never a non-atomic fallback) and record it in `keyed_models.md` §Known Divergences.
- **Canonical once-write spelling (K6).** The drafted surface is `COALESCE`-first-non-null; if that
  parses awkwardly as a grouped aggregate, an alternative single-aggregator spelling may be settled
  (e.g. admitting `ANY_VALUE`/`MIN` *as the once-write picker when the provenance proof holds*) — the
  combiner (`COALESCE(target, delta)`) and the provenance gate are the contract; the spelling is the
  open part. Update `keyed_models.md` §Surface to whatever is settled, timelessly.
- **Built-in vs template pattern functions (K6).** Default built-in if the registry can host transparent
  bodies; else templates in the guide + a Known-Divergence note.

## Verification

How to confirm the spec is satisfied at the end:
- `cargo test` (workspace) green; `cargo clippy --all-targets` clean; `cargo fmt --all -- --check`.
- **End-state equivalence harness** green across: the flipped rung-1 fixtures (K2), the mixed-family
  fixture (K3), crash-resume + re-run refusal (K4), the S1→S2 reconcile fixture (K5), and the
  out-of-order milestone fixture (K6) — each equals a full refresh of the model's own SQL over its
  processed inputs (`keyed_models.md` §"End-state equivalence"). Requires `DUCKDB_LIB_DIR`.
- **Fail-closed rejects** green: `cumulative` pointer error; `Keyed*` constraint codes; window-forward
  `ANY_VALUE` refusal; snapshot fold/overwrite/once-write refusals; unproven once-write; retractable
  enrichment; ledgered-window re-run for additive models.
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test
  example_workspaces` green — all new `examples/` fixtures diagnostic-clean.
- `rg -l 'refresh: cumulative' examples/ docs-site/` returns nothing;
  `rg -l 'cumulative_aggregate\.md' docs/specs/ docs-site/` returns nothing.
- `/smelt:validate keyed_models` reports zero drift; `/smelt:validate models`, `/smelt:validate
  model_maintenance`, `/smelt:validate model_transforms` report zero drift on the collapse's touchpoints.
