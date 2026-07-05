# A property-discovery research loop for incremental maintenance

- **Date**: 2026-07-05
- **Status**: research / design (predecessor to `docs/plans/20260705-property-discovery-loop.md`)
- **Author**: Andrew (with Claude; adversarially reviewed by a Fable subagent — findings F1–F9 folded in)
- **Motivates**: the design decisions in `docs/research/20260705-refresh-as-maintenance-plan.md`
  (refresh-as-per-column-maintenance). That paper argues the refresh "modes" are named
  projections of a `(column-group × input-delta)` maintenance plan, and that whether two
  techniques may serve a cell interchangeably is governed by the §4 interchangeability
  theorem. This loop is the **empirical engine** that settles which cells actually hold —
  and, crucially, whether **smelt's own analyzer and emitted maintenance** get them right.
- **Related specs**: `model_maintenance.md`, `model_properties.md`, `batched_models.md`,
  `keyed_models.md`, `sources.md`, `datagen.md`.

---

## 1. Why a loop, and why now

The normal `research → spec → implement` loop has not converged the refresh-as-maintenance
design: the paper makes many *mechanical claims* about which `(SQL-construct × upstream-property)`
combinations admit which maintenance technique, but almost none are empirically established.
§8 of that paper is candid that its own worked example is "a specification target, not a
demonstration of current behaviour." We are designing on assertions.

The remedy is to **measure**: build models that combine real SQL constructs over sources with
real upstream properties, run **smelt's own incremental maintenance** over adversarial
sequences of runs, and diff against a full refresh — establishing, to the same standard smelt
already holds its type system to (property-based tests against a DuckDB oracle), which
maintenance techniques uphold the equivalence contract and where **smelt breaks**.

This is a **research-and-repair engine**: its primary output is *knowledge* — a ledger of which
cells hold, a negative catalogue of which don't and why, and the property tests that constitute the
evidence — and, when it establishes a *clear, test-backed* analyzer/planner bug or improvement, it
**applies the fix to production smelt** (red→green, no-regression gated; policy §8). It does not
autonomously make *behaviour-defining design* decisions — those it records and BLOCKs for human
review.

## 2. The proof architecture: four links, one green gate

Fable's review (F1) killed an earlier three-link design: it proved *"a safe technique exists
in the abstract"* and *separately audited the analyzer statically*, but **never executed
smelt's own maintenance under adversarial schedules** — so the headline prize, an *unsound
acceptance* (smelt maintaining a cell that a run-sequence actually breaks), was unreachable.

The corrected structure has **four links**, and the **execution gate (Link C) is the only
thing that decides greenness.** The other three explain, predict, and localize *why* Link C
passed or failed.

```
  Link 0  Combiner-algebra table   combiner-identity → algebraic property P   (trusted axiom)
  Link A  Abstract pre-filter      (technique, P, source-shape) → witness schedule?   (fast, refute-only)
  Link B  Classification diag.     (SQL construct, source) → facts + SKELETON-COLUMN SET
                                       compared to smelt's analyzer   (diagnostic, not a gate)
  Link C  EXECUTION GATE ★         compile through the REAL analyzer → run smelt's emitted
                                       incremental maintenance over adversarial run schedules →
                                       diff vs full-refresh (multiset + as-of-run oracle, on the
                                       skeleton columns Link B named).   ← decides HOLDS/REFUTED
```

**An unsound acceptance = Link B says "smelt's analyzer classified this safe" AND Link C finds
a breaking schedule.** That is the whole point, and it is now an executable artifact.

### 2.0 Link 0 — combiner-algebra table (trusted axiom, refutation-guarded)

A small, human-auditable table mapping a **combiner identity** to its **algebraic property `P`**:

| combiner | P | faithful fold over append-only? | over retractable/mutable? |
|---|---|---|---|
| `SUM`, `COUNT` | commutative monoid (additive, **non-idempotent**) | yes (ledger: never fold a delta twice) | no (needs signed retraction deltas) |
| `MIN`, `MAX`, `BOOL_OR`, `BOOL_AND`, `EXISTS` | idempotent commutative monoid | yes | **no** (non-invertible) |
| `AVG` | monoid on decomposed `(sum,count)` state | yes, in decomposed state | no |
| `MEDIAN`, `PERCENTILE`, `COUNT(DISTINCT)` | holistic / **non-monoid** | no bounded state | no |
| running `SUM() OVER (… ROWS UNBOUNDED PRECEDING)` | additive over a prefix; **unbounded-forward footprint** | end-state only (paper §7) | no |

This table is **trusted axiomatic knowledge** — algebraic laws are ∀-quantified over infinite
inputs and cannot be *confirmed* from finite data (F3). DuckDB spot-checks are **refutation-only
guards** against a mis-*remembered* algebra, never evidence *for* `P`. smelt already encodes
fragments (`discriminants.rs::combiner_discriminants`, `decomposed_state.rs`,
`maintenance_driver.rs` monoid gating); the table makes it explicit and the loop cross-checks
smelt against it.

### 2.1 Link A — abstract contract-safety pre-filter (fast, refutation-only)

Pure-Rust property tests over an abstract technique parameterized by `P` and source-shape. This
is the user's "prove things like *this is a monoid* under any run sequence" — but demoted to a
**pre-filter and hypothesis generator**, not a proof of smelt.

- **Subject:** an abstracted technique — `fold-delta`, `recompute-region`, `column-scoped
  re-derivation`, `read-modify-write region`.
- **Generator — the adversarial run-schedule kinds** (every Link-A and Link-C cell MUST draw
  from these, or it is not `done`; this is the anti-vacuity rule, F-mitigation):
  1. partition the input into arbitrary-size deltas, arbitrary order;
  2. **re-deliver** a delta (idempotency — does re-running double-count?);
  3. **backfill** a region (does a region recompute reset the ledger?);
  4. **late arrival** landing within vs **beyond** the derived horizon;
  5. **retraction / update** (change-feed / mutable sources only).
- **Assertion:** modelled state = batch aggregate over the processed set `S`, on skeleton
  columns. Produces the *witness schedule kinds* that break each `(technique, ¬P)` — which
  Link C then replays against **real smelt**.
- **Not a green gate.** It predicts the admission-matrix boundary abstractly and hands Link C
  the schedules worth trying.
- **Coverage caveat (N4).** Link C only replays what Link A enumerates, so for a construct Link A
  models poorly (correlated `EXISTS`, which no smelt derivation even reads, §8) a generic
  enumeration can miss the breaking sequence and Link C then reports HOLDS on an unbroken cell.
  **Mitigation:** every cell additionally carries a **seeded construct-specific hazard schedule**
  targeting its *known* failure mode, independent of Link A's generic enumeration — e.g.
  late-conversion-within-7d for `SC-1`, back-dated in-place update for `SC-2`. A cell with no
  seeded hazard schedule for a known hazard is not `done`.

### 2.2 Link B — classification diagnostics + the skeleton-column set

- **Subject:** a concrete SQL construct over a source with a declared `MutationProfile`
  (`smelt-core/src/sources.rs`) and shape (unique-key? clocked? bounded lateness?).
- **Oracle:** DuckDB batch semantics (`prop_helpers/duckdb_oracle.rs`) over **proptest-generated
  data** (extending `prop_helpers/generators.rs`, not datagen) + **differential probes** (multiple
  partition orders reveal true combiner behaviour; a widen/narrow clamp-probe reveals true reach and
  footprint) yield the **ground-truth facts**: combiner-identity, reach `(before, after)`,
  footprint, mutation-sensitivity, and — critically — **the skeleton-column set** (which columns
  decide row existence / grouping / dedup / ordering; paper §6).
- **Compared to smelt's analyzer:** `source_bounds.rs::derive_model_bounds` (reach),
  `discriminants.rs` (combiner), `window_independence.rs` (batch safety),
  `input_delta.rs` (delta kind), `join_shape.rs` (fan-out). Verdict per fact: **sound /
  over-conservative / unsound / not-derivable**.
- **Two jobs:** (1) it is the **diagnostic that localizes *why* Link C failed** (which analyzer
  fact was wrong); (2) it **supplies the skeleton-column set as a *floor* to Link C** — resolving
  the circularity Fable flagged (F4): skeleton membership is a Link-B *output*, carried forward as
  a cell input to Link C, never re-derived inside the abstract Link A.
- **The skeleton set is a floor, never a ceiling (N2).** Link B is *not* a gate, and it derives
  skeleton membership from probes — so if it *under*-identifies skeleton (mis-labels a real
  membership/grouping column "payload"), a gate that diffed only Link B's set would silently GREEN
  a diverging cell: a false negative on the headline deliverable. Therefore **Link C diffs *all*
  columns by default** (§2.3); a column is dropped from the diff only when it is *provably payload*
  **and** *declared non-deterministic* (your OQ1 rule — non-determinism must be declared, or it is
  a skeleton error). Link B's skeleton set is the must-include floor of that diff, not its scope.
- **Not a green gate** on its own — static agreement between analyzer and ground truth does not
  prove runtime safety; only Link C does.

### 2.3 Link C — the execution gate ★ (the correctness verdict on smelt)

This is what F1 was missing and is now the spine.

1. **Compile the model through smelt's REAL bound derivation and planner** — no hand-injected
   `WHERE start/end` filter. **Build target (N1, PBT substrate):** the model has **no `WHERE
   start/end` clause**; the framework derives the filter. Link C runs it **in-process through
   `smelt-runtime::execute_project`** over a temp DuckDB (§3a) — **never**
   `incremental/main.rs::run_incremental_sequence` / `execute_model_incremental`, which hand-inject
   the `WHERE` and call the DuckDB backend directly, bypassing the very `source_bounds` derivation
   under test (the F5 sin — the earlier draft wrongly named that bypass suite as the reuse target).
   The `tests/e2e/` binary suite is the semantic *reference* (it also drives the real planner over
   no-`WHERE` models) but not the PBT runtime. The maintenance SQL under test is *what smelt
   actually emits*, including whatever `source_bounds` derived (or mis-derived).
2. **Replay adversarial run schedules** (Link A's kinds + the cell's seeded hazard schedule) via a
   new **RunSchedule driver** (§3). Its defining capability is **between-run source mutation**, not
   just window advancement: between two in-process maintenance runs it can **append a late row** into an
   already-processed event-time range and **update/delete a row in place** — because a source
   fully pre-populated at run 1 masks every lateness/mutation bug (both incremental and full see
   the row; §4 SC-1). Kinds: re-delivery, backfill, reorder, in/out-of-horizon lateness, and
   in-place update / retraction where the source shape allows.
3. **Diff against full-refresh** with the corrected oracle (§3): **`EXCEPT ALL`** (multiset — so
   additive double-counting is visible, F2), computing full-refresh **over the source's state at
   step `k`** (N3) — the driver snapshots the source contents each step, and full-refresh runs
   against that snapshot. For an **append-only** source this coincides with the paper's
   `state = full_refresh(input restricted to S)` invariant (S is the monotone delivered set); for
   a **mutable-snapshot** source there is no monotone `S` to restrict to (an in-place update cannot
   be un-seen by a filter), so "state at step `k`" is the well-posed form and "restricted to `S`"
   is reserved for append-only. The diff runs over **all columns by default** (N2), excluding a
   column only when it is provably payload **and** declared non-deterministic; Link B's skeleton
   set is the must-include floor. CONDITIONAL cells (running-total trajectory, SCD2-over-snapshot)
   declare their weaker contract instead of being mis-coded REFUTED (F2).
- **Verdict:** GREEN iff no adversarial schedule (generic + seeded) produces an all-columns-modulo-
  declared-payload divergence. A divergence on a cell smelt's analyzer marked safe (Link B) is a
  **latent correctness bug** — the deliverable. HOLDS is *relative to the enumerated + seeded
  schedules* (N4), never an absolute proof.

### 2.4 Composition, and honest verdict vocabulary (F3)

Link C decides; Links 0/A/B explain and localize. Verdicts never claim proof:

- **HOLDS** — *no counterexample found* over ⟨N⟩ adversarial schedules; Link B facts agree with
  ground truth. (Never "P proven" — refutation-only.)
- **REFUTED** — Link C found a skeleton divergence; the **witness schedule** and the divergent
  `EXCEPT ALL` rows are recorded, plus the Link-B fact that was wrong. A *positive* result: a
  mapped admission-matrix boundary or a smelt bug.
- **CONDITIONAL** — holds only under a named side-condition (bounded lateness declared; payload
  relaxation; end-state-only grain). The traded guarantee is recorded (paper §6 discipline).
- **BLOCKED** — a design fork the loop must not decide, or missing infra (e.g. a source shape the
  driver can't yet emit, §3). Recorded; the loop continues.

## 3. Phase 0 infrastructure — the honest dependency (F6, and the PBT-substrate steer)

Everything is **property-based-test-driven**, in the style of the repo's existing
`crates/smelt-db/tests/proptests/` + `prop_helpers/` suites: **data is produced by proptest
generators**, executed against an in-memory **DuckDB oracle** (`prop_helpers/duckdb_oracle.rs`), and
shrunk on failure. `smelt-datagen` (the synthetic-Parquet CLI) is **not** the substrate — it is a
batch fixture tool, not built for per-case PBT generation; and the `tests/e2e/` binary-shelling
suite is too slow and fixture-heavy to run thousands of proptest cases through a subprocess. Two
pieces do not exist today and gate everything (Phase 0 of the plan, not assumed-solved):

**(a) The in-process real-planner harness — the central Phase-0 task (resolves Fable N1).** Link C
must run smelt's *emitted* incremental maintenance — the SQL smelt produces *after its real bound
derivation*, with **no hand-injected `WHERE`**. The existing incremental unit harness
(`incremental/main.rs::run_incremental_sequence` / `execute_model_incremental`) injects the filter
and calls the DuckDB backend directly, **bypassing the analyzer** (Fable F5/N1) — forbidden here.
The `tests/e2e/` suite *does* go through the real planner (`smelt run
--event-time-start/--event-time-end` over no-`WHERE` models) but only by **shelling the binary** —
right semantics, wrong runtime for PBT. So Phase 0 builds a **proptest harness that drives the
sanctioned real-planner entry point `smelt-runtime::execute_project`** (the run-pipeline-parity
single door, CLAUDE.md) over a temp DuckDB, feeding **generator-produced source rows** and a
**generator-produced run schedule**, and reading back the materialized table. This is nontrivial
in-process plumbing (Config + DependencyGraph + Database + Workspace + BackendFactory) that no test
does today — it is the gating deliverable, built deliberately. The e2e binary suite is the
*reference* for how the real planner derives the filter, not the runtime substrate.

**Location (dev-dependency-cycle-safe).** The Link-C harness lives in **`crates/smelt-cli/tests/
property_discovery/`** (`link_c_harness.rs` = the `execute_project` fixture; `model_shapes.rs` = the
single readable catalogue of every construct's model SQL — the one place the tested model scope
lives; one test module per cell). It must **not** live in `smelt-db`'s tests: `smelt-runtime`
depends on `smelt-db`, so a Link-C harness in `smelt-db` closes a dev-dependency cycle and drags the
whole runtime+DuckDB stack into `smelt-db`'s own type-property suite. `smelt-cli` already depends on
`smelt-runtime` + the DuckDB backend (it is the incremental harness's home too), so it is the clean
home. Link-A (abstract) and Link-B (classification) tests, which need only the analyzer + DuckDB
oracle, stay in `crates/smelt-db/tests/` reusing `prop_helpers/`.

**(b) The run-schedule generator + corrected oracle.** The run schedule is itself a **proptest
value** — a generated sequence of steps, each either *advance the event-time window and run
maintenance* or *mutate the source between runs*. Its defining capability (vs the existing
window-only harnesses, which pre-populate the source at run 1 and only move the window — masking
every lateness/mutation bug) is **between-run source mutation**:
- **append a late row** into an already-processed event-time range (row absent at the earlier run)
  — the SC-1 shape;
- **update / delete a row in place** in an already-processed partition — the mutable-snapshot /
  SC-2 shape;
- **snapshot the source contents at each step** so the oracle can full-refresh over "source state
  at step `k`" (N3).

The oracle: **`EXCEPT ALL`** (multiset, so double-counting shows, F2), full-refresh over **the
source state at step `k`** (for append-only this coincides with "restricted to the delivered set
`S`"; per-cell mode), diffing **all columns by default** and excluding one only if
provably-payload-and-declared-non-deterministic (N2). Generators must **self-check** that data
emitted for a declared `MutationProfile` actually has that shape (F7: the profile is declared,
never verified — an unchecked label silently poisons a verdict).

What each source shape needs:
- **append-only** (append-late-between-runs) and **mutable-snapshot** (in-place update between
  runs) cells are reachable *now* — both are plain generated `INSERT`/`UPDATE` steps, no CDF
  machinery. The initial catalog is scoped to these, and **both seed bugs SC-1/SC-2 live here**
  (§4), so the reachable corner is *not* vacuous.
- **change-feed / CDF / tombstone-retraction / unbounded-lateness** cells need a richer change-feed
  generator — a **named Phase-0 sub-task**; until it lands those cells BLOCK.

The generator vocabulary **extends `prop_helpers/generators.rs`** (append-only monotone streams,
keyed streams, mutable snapshots), not datagen.

## 4. What we test — the catalog and the negative catalogue

The catalog is a checked-in matrix; the loop works cells top-to-bottom. **Axes** (expanded per
Fable's minor-axes note):

- **construct**: pass-through · filter · inner-join enrichment (fact × dim) · left-join
  (null-preservation) · **join fan-out that changes grain** · additive agg (`SUM`/`COUNT`) ·
  idempotent agg (`MIN`/`MAX`/`BOOL_OR`) · holistic agg (`MEDIAN`/`COUNT DISTINCT`) · windowed
  running-total (`ROWS UNBOUNDED PRECEDING`) · **other window frames** (`ROWS BETWEEN n
  PRECEDING AND CURRENT`, `RANGE …`) · correlated `EXISTS` (paper §2) · `UNION ALL` · self-join.
- **source property**: append-only · mutable-snapshot · change-feed/CDF (± retractions) ·
  **single vs composite unique-key** (`join_shape` proves OneToOne on single-column keys only —
  composite-key joins classify OneToMany, so fan-out-bounded claims on composite keys are a
  seeded suspect) · clocked vs unclocked · bounded vs unbounded lateness.
- **candidate technique** (paper §3 2×2): recompute-region · fold-delta · read-modify-write
  region · column-scoped re-derivation.
- **non-determinism / skeleton-leak** (paper §6, your OQ1 FEEDBACK): a **two-model DAG fixture
  family** where a payload column of `M` is consumed in `N`'s `JOIN…ON`/`WHERE`/`GROUP BY`,
  exercising the "non-deterministic column reaches a skeleton position" leak. Single-model cells
  cannot reach this (F4) — it is its own fixture family, or scoped out loudly.

**Seed hypotheses (concrete candidate bugs, verified to exist in code; both reachable in the
scoped corner without the CDF driver, N-refinement):**
- `SC-1` — `source_bounds` "no-bound-found" fallback yields optimistic `(0,0)` for an
  unrecognized construct (correlated `EXISTS`), silently clamping away late conversions →
  predicted **unsound acceptance**. (source_bounds.rs; paper §8.) **Reachability:** needs the
  driver to *append a late conversion between runs* into an already-processed range (§3a) — a
  pre-populated conversion would be seen by both paths and mask the bug.
- `SC-2` — `input_delta.rs:89-91` classifies a **clocked `Mutable`** source as `WindowForward`
  (reads only new partitions, misses back-dated mutations) → predicted **unsound acceptance**.
  **Reachability:** needs an *in-place `UPDATE` of an already-processed partition between runs*
  (§3a) + the step-`k`-snapshot oracle (§3b) — both in the reachable corner, no CDF needed.

**The negative catalogue (`unsupported.md`) — your request.** Every REFUTED and CONDITIONAL cell
is surfaced as a first-class **admission-matrix negative space**: one entry per
`(construct × source-property × technique)` that does **not** support a technique, annotated with
*why* — the witness schedule, the missing property (`Link 0`), or the named guarantee it trades
(CONDITIONAL). This is the "catalogue of things that don't support all options, commented with
why" — the directly reusable output for the spec's admission matrices.

**Cell schema** (`catalog.jsonl` row): `id · construct · source_property · technique · skeleton_cols
· status · hypothesis(expected verdict) · owning_test · appended_from(if agent-grown)`.

**Seeded + agent-may-append** (your call): a human seeds the matrix; an iteration may append ≤2
*adjacent* `pending` cells, each naming the seed cell it descends from (bounded growth).

## 5. What we expect to learn (trimmed for honesty, F8)

1. **★ Headline — smelt's unsound acceptances and over-conservative refusals.** The ranked list
   of cells where smelt's analyzer says "safe" but Link C breaks it (bugs), or "unsafe" for a
   cell that provably holds (missed features). `SC-1`/`SC-2` are the first two hypotheses. This
   is the *sole* novel, actionable deliverable — and the reason Link C had to exist.
2. **The negative catalogue** — the admission-matrix boundaries with witnesses (§4).
3. **The RunSchedule driver + corrected oracle** — reusable infrastructure that outlives the loop
   (multi-batch/CDF replay + multiset/as-of comparison smelt's test suite lacks today).

*Explicitly demoted to regression-guard, not "learning" (F8):* the combiner-algebra table
validation (refutation-only, §2.0); whether the plan factors (paper OQ5 — already answered by
your own feedback that cost forces users to design around it, and a corpus survey not a PBT); the
correlated-`EXISTS` reach comparison (returns `NotDerivable` by construction per §8 — a known
outcome). We record these but do not pretend they are discoveries.

## 6. The loop mechanics

Modelled on `.claude/scripts/autonomy-loop.sh`, single-catalog, research-only.

**One iteration = one catalog cell:** read `catalog`; pick next `pending`; author/extend the Link
it needs (reuse the RunSchedule driver + smelt analyzer); run just that test
(`cargo test -p <crate> --test <t> <name> --quiet 2>&1 | tail -40`, `DUCKDB_LIB_DIR` +
`LD_LIBRARY_PATH` set — unset ⇒ the cell **BLOCKS**, never silently skips); write the verdict +
witness to `ledger.md` and, if REFUTED/CONDITIONAL, an entry to `unsupported.md`; set the cell
`done`/`blocked`; optionally append ≤2 adjacent cells; commit + push; emit one sentinel.

**Sentinels:** `<<PROBE_COMPLETE>>` (verified, committed — loop) · `<<PROBE_BLOCKED>>` (design
fork / missing infra — recorded, loop continues) · `<<CATALOG_EXHAUSTED>>` (no `pending` cell —
stop, surface summary, exit 2).

**Forever wrapper — 10-minute retry (your explicit requirement):** `property-loop-forever.sh`
runs `property-loop.sh`; on any **non-zero, non-exhausted** exit — credit exhaustion, rate-limit,
crash — it **sleeps 600 s and retries**, indefinitely, so it self-starts when credits return next
session. `<<CATALOG_EXHAUSTED>>` (exit 2) and the graceful-stop flag (`.claude/property-loop.stop`)
end it.

**Isolation (F9 — resolved by operational rule).** This loop runs on `worktree-incremental`. That
branch is also driven by the fundamentals autonomy loop, which auto-stashes the dirty tree at
iteration start — so the two must **never run concurrently** (concurrent headless commits race and
clobber half-finished work). Operational rule, agreed: **the fundamentals loop stays paused for the
duration of this research** — the property loop has the branch to itself; there is no live race to
interlock against. What remains enforced regardless is a **CI grep gate** asserting every
`EXPERIMENTAL(property-discovery)` site is under `#[cfg(test)]` or `tests/` (fails the commit
otherwise), so a headless agent cannot wire a throwaway accessor into a production planning/execution
path — code-hygiene, not concurrency.

**Artifacts:** `docs/research/property-discovery/{catalog.jsonl, ledger.md, unsupported.md}` ·
all tests are **proptest-driven** (generators + `DuckDbOracle`, extending
`crates/smelt-db/tests/prop_helpers/`), not datagen/e2e · Link-A tests under
`crates/smelt-db/tests/proptests/maintenance_*` · the **Link-C harness driving
`smelt-runtime::execute_project` in-process** in **`crates/smelt-cli/tests/property_discovery/`**
(`link_c_harness.rs` fixture; `model_shapes.rs` = the single model-SQL catalogue; one module per
cell) over generator-produced rows + a generated run schedule with between-run source mutation
(**not** `crates/smelt-cli/tests/incremental/`, which bypasses the analyzer, N1; **not** `smelt-db`'s
tests, which would close a dev-dep cycle) · Link-B classification diagnostics reusing `smelt-logical`
analysis APIs + the DuckDB oracle · logs `~/.claude/logs/property-discovery/`. Reuses the autonomy
loop's memory-scope / sampler / sync-with-main / usage-log machinery.

## 7. The ledger and negative-catalogue schemas (the deliverables)

`ledger.md`, one block per resolved cell:
```
### CELL <id> — <construct> × <source_property> × <technique>
- verdict: HOLDS | REFUTED | CONDITIONAL | BLOCKED
- P (Link 0): <property>          skeleton_cols (Link B): <set>
- Link B facts: combiner=<…> reach=<(b,a)|Unbounded|NotDerivable> footprint=<bounded|unbounded>
- smelt analyzer: sound | over-conservative | unsound | not-derivable      [← ACTION if not sound]
- Link C: no divergence over <N> schedules | WITNESS: <breaking schedule + EXCEPT ALL rows>
- condition (CONDITIONAL only): <named guarantee traded, paper §6>
- evidence: <test path::name>, <schedule count>, <oracle mode>
```
`unsupported.md`, one line per unsupported `(construct × source × technique)`:
```
<construct> × <source> — technique <T>: UNSUPPORTED — <why: witness | missing P | traded guarantee>
```

## 8. Policy: reusing and extending smelt

**Authority (updated 2026-07-06 — full autonomy, gated by tests).** The loop **may make production
smelt changes** — extend or fix the analyzer/planner/runtime — for any *clear, test-backed*
improvement or bug it establishes, on this (throwaway) branch. This restores the original intent
("I expect this loop will actually extend smelt with extra logic and tests"). The prior test-only
restriction is lifted; what replaces it is a **test gate**, not a location gate:

1. **Red→green.** The change must be driven by a test that first *fails* (reproduces the divergence,
   or asserts the missing/over-conservative derivation) and then *passes* with the change.
2. **No regression.** The full test suite of **every crate whose production code was touched** must
   be green (`cargo test -p <crate>` for each; the command + result recorded in the ledger cell).
   `cargo fmt --all` and `cargo clippy --all-targets` clean.
3. **Recorded.** The ledger cell names the production files/functions changed and the gate output,
   so every autonomous production change is auditable and revertible.
4. **BLOCK genuine design forks.** A change that requires *choosing new maintenance semantics* (e.g.
   wiring a dormant classifier to a new execution path, adding a refresh behaviour) is a design
   decision, not a mechanical fix — record it as a finding and `<<PROBE_BLOCKED>>` for human review
   rather than deciding it autonomously. Bug fixes and derivation tightenings with an unambiguous
   correct answer are in-scope; behaviour-defining design is not.

**Disposable test scaffolding** (harness, generators, oracle) still lives test-target-only and stays
tagged `// EXPERIMENTAL(property-discovery): disposable`, enforced by the §6 grep gate — that tag now
marks *throwaway scaffolding*, and must **not** be applied to a real production change (production
improvements are untagged, permanent-until-reviewed code held to the test gate above). Everything
lands on this branch; merge to product is still a separate human decision informed by the ledger.

## 9. Non-goals and risks

- **Repairs, does not redesign**: it applies test-backed analyzer/planner *fixes and derivation
  tightenings* (§8), but does not author specs, product plans, or new refresh/maintenance
  *behaviour* — a change that would define new semantics is BLOCKed for human review.
- **Empirical, not formal** (F3): PBT gives strong refutation and "no counterexample over N",
  never a machine-checked proof. Verdict vocabulary reflects this.
- **Risk — Link C is only as sound as the driver + oracle.** A driver that can't emit retractions
  or an oracle stuck on set-semantics silently downgrades REFUTED→missed or →CONDITIONAL. Phase 0
  (§3) is therefore the gating deliverable, and cells whose shape the driver can't emit BLOCK
  rather than run against a mislabelled source.
- **Risk — marking its own homework (F1, resolved).** Resolved by Link C running smelt's *emitted*
  maintenance through the *real* analyzer and diffing vs an independent full-refresh oracle — the
  analyzer's verdict (Link B) is an input to the bug report, never a gate on greenness.
- **Risk — unbounded catalogue growth** — bounded by the §4 append cap + descent-rationale.
