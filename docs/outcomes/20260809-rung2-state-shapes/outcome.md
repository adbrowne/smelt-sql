# Outcome: Rung 2 — decomposed combiner state

**Created:** 2026-08-09
**Status:** active
**Source:** `docs/research/20260809-incremental-rethink.md` §2 P-A, §6 step 1
**Spec anchors:** `docs/specs/incremental_models.md` (algebraic ladder, column-family catalogue), `docs/specs/model_properties.md` (algebraic discriminants)

## The outcome

Decomposable combiners get concrete auxiliary state: the stored table carries
the state columns a combiner needs to fold correctly (`AVG → (sum, count)`,
`MAX_BY(v, o) → (v, o)`, once-write → written-flag, `stddev`-family →
`(n, Σx, Σx²)`), and a presentation projection hides them from consumers.
Admission then widens to everything rung 2 licenses, and the user-visible
obligations that existed only because rung 2 was unbuilt are deleted.

## Success criteria (checkable)

1. `MAX_BY`/`MIN_BY` admitted **without** the hand-written companion
   `MAX(<ordering>)` projection; the companion-projection obligation is gone
   from spec and docs-site.
2. The once-write family admits the fallback-bearing and multi-candidate
   `COALESCE` spellings that today refuse with "waiting on machinery".
3. `AVG` (and at least one `stddev`-class aggregate) folds incrementally at
   keyed grain instead of refusing.
4. State columns are invisible to downstream consumers (presentation map
   projects them away; `smelt explain` shows them as internal state).
5. `cargo test -p smelt-cli --test maintenance_conformance` generates and
   passes decomposed-state recipes for every newly admitted family.
6. All standing gates green (`verify-phase.sh`, walk_coverage,
   statement_parity); no new whole-text scans (walk rule holds).

## Out of scope

- Ladder rungs 3–4 (change-feed consumption, bounded-domain multiset).
- Approximate-sketch state (HLL) — a later contract-lattice item.
- The `smelt.latest`/`smelt.once`/`smelt.current` pattern functions.

## Phases

One line each — intent only. The planner step details (and may reshape) this
list at the start of every phase; it must not defer work that serves the
success criteria above.

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: decomposed-state semantics — state shapes, presentation projection, widened admissions, obligations to delete | done |
| 2 | Derive concrete state shapes in `smelt-logical` for the decomposable catalogue (`decomposed_state.rs` stops refusing); widen `π` purity to the new shapes; pure state/user column collision detector | done |
| 3 | Storage + emitters: state columns materialised in the stored table, keyed fold over state, `KeyedStateColumnCollision` diagnostic wiring | done |
| 4 | Presentation projection: state columns invisible to `ref()` expansion, `SELECT *`, declared-schema checks, downstream type inference | done |
| 5 | Admission: `MAX_BY`/`MIN_BY` without the companion projection | done |
| 6 | Admission: classify the once-write fallback/multi-candidate spellings onto the derived `(value, written)` state | done |
| 7 | Admission: `AVG`/`STDDEV_*`/`VAR_*` decomposed folds at keyed grain, including the additive-state ledger grade and the state-aware defence-in-depth/preview paths | pending |
| 8 | Conformance-gate recipes for decomposed-state families, each with a downstream `SELECT *` consumer asserting state columns stay hidden end-to-end | pending |
| 9 | Surface cleanup: delete the residual once-write/decomposed-fold obligations from spec + docs-site; `smelt explain` state rendering | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

- 2026-08-09 (plan 1): no reshape — phase list stands as scaffolded. Split of spec work
  fixed: phase 1 writes the normative rung-2 semantics and *rewrites* the three rung-2
  Known Divergences entries to the residual gap; phase 7 deletes them plus the docs-site
  obligations once the code actually behaves that way (success criteria 1–2 need both).
- 2026-08-09 (plan 1): physical-layout decision handed to the phase-1 spec pass — state
  columns live in the same stored table as the presented columns (suffix `__part`),
  hidden from the public schema, rather than a separate state table + presentation view;
  keeps `ref()` a table and leaves backend DDL/atomic-swap paths untouched.
- 2026-08-09 (implement 1): phase 1 landed the spec text (`incremental_models.md`
  §"Decomposed state (rung 2) in keyed models", catalogue/diagnostics/Known-Divergences
  edits, plus `model_properties.md`/`model_transforms.md` cross-references). Self-review
  caught and fixed a stale Design-section claim ("keyed families sit on the direct-monoid
  rung") and two missing table rows (admission matrix, derived execution postures) for
  the new decomposed-fold family. Only `AVG` is encoded in `decomposed_state.rs` today;
  phase 2 needs to decide whether `MAX_BY`/once-write widen through the same
  `decomposable`-discriminant entry point or need their own.
- 2026-08-09 (plan 2): entry-point decision for the summary's open question — `decompose_to_state`
  stops gating on `combiner_discriminants(...).decomposable` and instead refuses only the
  *holistic-or-unknown* verdict (no monoid fact, no decomposability, no monotonicity). `MAX_BY`/
  `MIN_BY` (`Monotone::Order`) then reach the state-shape match without restating F4's raw facts:
  the discriminants stay exactly as `model_properties.md` defines them, and "has an encoded state
  shape" stays a property of this mechanism, not of the algebra. Rejected: flipping `ArgMax`/
  `ArgMin` to `decomposable: true`, which would corrupt a raw discriminant to serve one consumer.
- 2026-08-09 (plan 2, reshape): once-write is not a `SqlFunction`, so its `(value, written)` state
  needs its own entry point. Phase 2 derives it from an already-classified spelling; the SQL-level
  classification of the fallback/multi-candidate spellings stays phase 5 (row text sharpened).
  `KeyedStateColumnCollision` splits: the pure detector lands in phase 2 alongside the shapes it
  checks, the diagnostic wiring in phase 3 where the plan first carries state columns (row text
  sharpened). No work left the outcome.
- 2026-08-09 (implement 2): `decompose_to_state` now derives variance/stddev `(n, sx, sxx)`,
  `ARG_MAX`/`ARG_MIN` `(v, o)`, and once-write `(value, written)` (via a new
  `decompose_once_write` entry point) state shapes, gated on `Discriminants::is_holistic_or_unknown()`
  per the plan-2 decision. `presentation.rs`'s existing walk needed no new arms — CASE/binary-op/
  scalar-function coverage already accepted the new `π` shapes. All 14 new/updated
  `decomposed_state` tests and 3 new `presentation` tests pass; full `verify-phase.sh` green.

- 2026-08-09 (plan 3, reshape): old row 3 split into two — row 3 (state columns materialised +
  keyed fold over state + collision diagnostic) and a new row 4 (presentation projection: hiding
  state columns from `ref()` expansion, `SELECT *`, declared-schema checks, downstream type
  inference). The hiding half lives in a different layer (`smelt-db`/`smelt-runtime` schema
  resolution) from the storage/emitter half and is what success criterion 4 checks; bundling both
  in one phase made a row that could not be red-green'd coherently. Admission/conformance/surface
  rows shift to 5–8. Nothing left the outcome.
- 2026-08-09 (plan 3): phase 3 carries state through the classification without widening
  admission — the state-bearing shapes stay unreachable from real SQL until rows 5–6 flip
  admission, so phase 3's tests construct the state-bearing classification directly. Rejected:
  folding the `MAX_BY` admission flip into phase 3 to get an end-to-end fixture, which would make
  one phase both mechanism and admission and leave no clean red test for either.
- 2026-08-09 (plan 3): small spec correction queued into phase 3 (spec-first) — the state-shape
  catalogue's once-write "combiner over state" cell currently reads last-write-wins ("`value` is
  the incumbent's unless the delta's `written` is true, in which case the delta's"), which
  contradicts the family's first-write-wins semantics and its own rung-1 `COALESCE(target, delta)`
  form. The fold is `COALESCE(target.value, delta.value)`.

- 2026-08-09 (implement 3): state columns are physically real — `StateColumn` carries its own
  combiner, `AggregatorColumn.state` threads it through (`None` everywhere today),
  `state_augmented_projection` (new pure emitter) appends state select items via CST location,
  and `build_cumulative_merge_sql` expands a state-bearing column into per-state-column folds
  plus a presented column recomputed from the merged state. `KeyedStateColumnCollision` is wired
  end-to-end but unreachable until rows 5-6 widen admission. Caught and fixed in the same pass:
  `CrossPartitionCombiner::OrderMonotone`'s `render` was unconditionally `>`, silently wrong for
  `MIN_BY` (`prefer_greater: bool` added, all call sites updated). Spec cell corrected
  (once-write "Combiner over state": last-write-wins → `COALESCE(target.value, delta.value)`).
  No admission widened; `maintenance_conformance`'s 47 tests stayed green unchanged.

- 2026-08-09 (plan 4, reshape): row 7 sharpened — the decomposed-state conformance recipes must
  each carry a downstream `SELECT *` consumer, because admission is still closed in row 4 and
  criterion 4 therefore has no end-to-end witness until admission widens. The hiding mechanism is
  unit-tested in row 4; row 7 is where it gets proven against a real DuckDB. No work left the
  outcome.
- 2026-08-09 (plan 4): the `SELECT *` leak is real and lives at the *execution* layer only — the
  analysis layer derives a model's schema from its own select list (`smelt-db`'s `model_schema`),
  so state columns, appended by phase 3's `state_augmented_projection` emitter, never enter the
  public schema. Phase 4 therefore rewrites wildcards at compile time into the presented column
  list. Rejected: `SELECT * EXCLUDE (...)` (dialect-specific), resolving `ref()` to a presenting
  derived table (breaks `FROM x AS y` and qualified column references), and a companion
  presentation view (already rejected by the phase-1 physical-layout decision).
- 2026-08-09 (plan 4): the unexpandable-wildcard case is a hard compile error following
  `check_native_ivm_gate`'s precedent, not a new diagnostic code — it arises on the build path
  where no `KeyedDiagnostic` is being collected, and inventing a code there would split the
  fail-loud surface for one unreachable-until-row-5 condition.

- 2026-08-09 (implement 4): `presentation_projection` runs on the pre-print SQL text inside
  `compile()`/`compile_with_sql*()` (`smelt.models.*` still literal), not on `apply_type_casts`'s
  post-print SQL (already rewritten to physical table names) — the only point in the pipeline
  where a refusal can still name the user's ref path, matching the plan's instruction. Rejected:
  rewriting after printing and reconstructing the original path from `cross_engine_refs`/schema
  metadata, which would make refusal messages depend on backend-specific naming.
- 2026-08-09 (implement 4): `SqlCompiler::state_bearing_models` is a bare `BTreeSet<String>`
  (membership only); the presented-column list is derived on demand from the compiler's own
  `upstream_schemas.models` inside a private `presentation_map()` rather than duplicated into the
  set at `execute.rs`'s classification site — task 3's "no new source of truth for which columns
  are presented" reading taken literally: the set answers "is this model state-bearing", the
  already-existing public schema answers "what are its columns".
- 2026-08-09 (implement 4): test 9's diagnostic assertion uses `type_diagnostics`/
  `UndeclaredColumn` rather than `file_diagnostics`/`ColumnTypeUnresolved` as the plan's own line
  sketched — a hand-written `agg.avg_amount__sum` reference against a resolvable upstream model
  fires `UndeclaredColumn` (the column-existence check), the more precise diagnostic and what the
  harness actually produced; `ColumnTypeUnresolved` fires for a different failure shape
  (unresolvable *type*, not a missing column). Also discovered along the way (not fixed, out of
  scope): `TestDb::file_diagnostics` resolves cross-model refs via `resolve_ref_path`, which needs
  fuller project/address-index setup than `model_schema`/`type_diagnostics`'s `resolve_ref` does —
  a legitimately-defined upstream ref reports a spurious `UndefinedModelRef` through
  `file_diagnostics` alone in a minimal `TestDb` harness. Pre-existing, unrelated to this phase.

- 2026-08-09 (plan 5, reshape): the order-monotone surface cleanup moves *into* phase 5 rather
  than waiting for row 8 — the "no decomposed-state storage wired in" Known Divergence and the
  docs-site companion-projection obligation both become false statements the moment phase 5's
  code lands, and a spec that lies for three phases is worse than a slightly wider phase. Row 8
  text sharpened to the residual once-write/decomposed-fold obligations plus `smelt explain`
  rendering. No work left the outcome.
- 2026-08-09 (plan 5): `MAX_BY`/`MIN_BY` takes a *single* admission path — always decomposed
  `(v, o)` state; `order_monotone_companion` is deleted along with both its call sites
  (`classify_order_monotone_column`, `derive_fold_spec`). Rejected: keeping the companion
  projection as a stateless fast path, which would leave one family with two admission modes and
  two stored-table shapes, and preserve the duplicated proof that `faithful_fold`'s module doc
  and the `derive_fold_spec` doc both have to warn about. A model that already projects
  `MAX(ord)` keeps it as an ordinary extremal-fold output column.
- 2026-08-09 (plan 5): the existing 47 `maintenance_conformance` recipes already generate
  `MAX_BY` models, so phase 5 gets an end-to-end DuckDB witness for free — those recipes will
  execute with materialised `(v, o)` state and must still match the full-refresh oracle. Row 7
  still owns the *new* decomposed-state recipes and the downstream `SELECT *` consumers.

- 2026-08-09 (implement 5): `MAX_BY`/`MIN_BY` admit on hidden `(v, o)` state with no companion
  projection required — single admission path, `order_monotone_companion` deleted. Caught and
  fixed two latent bugs surfaced by the first real end-to-end state-bearing execution:
  `expand_aggregator_column_folds`'s sequential identifier substitution corrupted the presented
  column when one state column's merged expression named a sibling column (fixed with a single
  simultaneous-pass substitution); `state_augmented_projection` was applied to the compiled,
  cast-wrapped SQL instead of the raw pre-compile SQL, so a state expression needing raw source
  columns couldn't resolve them (fixed by augmenting before compiling in both keyed executors).
  `maintenance_conformance`'s `assert_keyed_equivalence` needed a presented-columns-only SELECT
  helper since the physical table now carries hidden state columns the oracle doesn't produce.

- 2026-08-09 (plan 6, reshape): old row 6 split into row 6 (once-write fallback/multi-candidate
  spellings) and row 7 (`AVG`/`STDDEV_*`/`VAR_*` keyed folds). They widen two disjoint arms of
  `classify_cumulative` (the `GroupByKey`/`COALESCE` arm vs. the `OtherAggregate`/`combiner_for`
  arm) and have disjoint blast radii: once-write's state combiners (`OnceWrite` + `BoolOr`) are
  idempotent and change nothing downstream, whereas `AVG`'s state is `Sum` — the first *additive*
  state, which `WindowedKeyedRule::ledger_grade` reads off `cross_partition_combiner` alone and
  would grade `Idempotent`, silently dropping the ledger refusal a reprocessed window needs. That
  fix (plus `refuse()`'s monoid allowlist and `smelt-runtime/src/diagnostics.rs`'s `KeyedFold`
  preview folds, both of which ignore `state` today) belongs with the family that makes it
  reachable, not in a later "audit" row. Old rows 7/8 shift to 8/9; the ledger-grading audit
  merges into row 7 as required work. No work left the outcome.
- 2026-08-09 (plan 6): only the fallback-bearing and multi-candidate spellings become
  state-bearing; the key-derived and bare `COALESCE(MAX(col))` spellings stay stateless with the
  `COALESCE(target, delta)` combiner they have today. Rejected: routing all four spellings through
  `decompose_once_write` for uniformity — it would rewrite the stored shape of every already-
  admitted once-write model (and all 47 conformance recipes) to buy nothing the spec asks for,
  since §"The column-family catalogue" states the bare spellings' combiner *is* the direct fold.

- 2026-08-09 (implement 6): once-write's fallback-bearing and multi-candidate spellings admit
  onto hidden `(value, written)` state per candidate — `classify_once_write` parses the leading
  run of `MAX(...)`/`MIN(...)` candidates plus one trailing fallback, proves each independently
  (first failure names that candidate), and calls `decompose_once_write` when a fallback or a
  second candidate is present; the bare-reduction and key-derived spellings stay stateless.
  `OnceWriteAdmission::Admitted` gained a `state` payload; `classify_once_write` gained an
  `output_name` parameter to name the derived columns. No new bugs surfaced — once-write's state
  columns never cross-reference each other, so both phase-5 traps (fold-substitution corruption,
  pre-cast state augmentation) were re-verified clean rather than hit again. `KeyedStateColumnCollision`
  reaches its second family for free (the detector is already generic over `aggregator_columns`).

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
