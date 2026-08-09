# Rethinking incremental models from the substrate up

**Date:** 2026-08-09
**Status:** research — fresh rethink, not committed direction
**Question:** with the property proofs and backbuild transforms now unified on one tree walk, what is missing at the bottom, and how should the incremental-models product be reshaped on top of it?

---

## Key claims

1. **The substrate has outgrown the product.** The walk now derives ~30 properties and the emitter layer holds ~20 provably-equivalent transformations, but the user surface still exposes essentially one contract, one run shape per grain, and four techniques. Several proofs are built with zero consumers. The bottleneck is no longer "can we prove it" — it is that the product's shape predates the substrate.

2. **One unbuilt rung causes most of the visible warts.** Decomposed combiner state ("ladder rung 2": storing `(sum, count)` for `AVG`, the ordering column for `MAX_BY`, arrival flags for once-write) is specified but unbuilt. The `MAX_BY` companion-projection obligation, the two-spellings-only `COALESCE` restriction, the unconsumed `bounded_domain` declaration, and the refusal of `AVG`-class aggregates all trace to it. Build it first; several spec sections then simply delete.

3. **The missing properties cluster into three families**, in priority order: **state-shape derivation** (what auxiliary columns must the stored table carry so a combiner folds correctly), **output-delta typing** (what kind of change does this model *emit* — the property that would make incrementality compose through the DAG instead of stopping at each model), and **probe-backed world-facts** (every declared fact about the world gets a cheap runtime tripwire, so declarations are checked promises rather than trusted assertions).

4. **The missing transformations are repairs, not folds.** The fold family is nearly complete for what is foldable. What is missing is everything the current design answers with "full refresh": per-group targeted recompute (the escape hatch every production IVM engine uses for non-invertible aggregates under retraction), diff-then-patch reconciliation writes, key-scoped and column-scoped dirt propagation, and shadow-build-and-swap. These convert today's harshest refusals into bounded costs.

5. **The rethink: make the typed delta the unit of composition, and the contract a lattice instead of a point.** Today every model proves its maintainability against *raw sources*; the DAG layer then degrades everything to day-interval dirt. Instead, each model should derive a typed output delta (append-only window / keyed upsert / general change) from its typed input deltas, per column group — so a chain of models is incrementally maintained end-to-end by construction. On top of that, the single equivalence invariant becomes the *default* point in a small user-composable lattice of contract relaxations (frozen horizons, deferral, declared indifference), each with its own validation obligation. Users get power as *verbs over plan cells* plus *declared facts and relaxations* — never as modes.

6. **Sequencing:** rung 2 (state shapes) → per-group repair → output-delta typing across the DAG → contract lattice → spec redraft around "typed deltas + contract lattice + cells" as the primary abstractions. The redraft is last deliberately: the current spec's accretions (two ledgers under one name, "frontier" with no home, anti-exclusivity polemics) are symptoms; redrafting before the abstractions change would fossilise the wrong shape a second time.

---

## 1. Where we actually are (short)

Bottom layer — strong and recently consolidated:

- **One shared bottom-up walk** over the query tree carries a per-node property vector: source read bounds (a tropical add-along-path / max-across-branches algebra), event-time monotonicity traces, grain and functional dependencies, determinism and comparability lattices, combiner algebra discriminants (monoid / needs-inverse / decomposable / order-monotone), fan-out, mutation and membership sensitivity per column, write footprints, fingerprint projections, skeleton-source closure. Everything fails closed; escape hatches only widen `Undecidable`, never `Disproven`.
- **Transformations** are pure emitters gated by proofs: four maintenance techniques (region delete+insert, keyed fold-merge, column-scoped merge, in-place update), sixteen backbuild techniques that migrate a table across a definition change without rebuild, an open write-pattern registry, and an adjoint forward/backward propagation algebra over the DAG.
- **The contract** is one invariant — incremental state equals full refresh over the processed input set — with the modeller declaring at most a clock and an identity; everything else is derived, and anything unprovable is a named refusal.

Top layer — showing its age. The spec is ~2,600 lines of which 12% is divergence essays; two distinct ledger concepts share one name; "frontier" (the roadmap's own headline word) exists nowhere as an abstraction; three proofs and two declarations have no consumer at all; and users hit obligations (hand-projecting `MAX(ordering)` next to `MAX_BY`) that exist only because an implementation rung is missing. Details and citations in §5.

---

## 2. Missing properties

Ranked by leverage. Each is a derivation the walk could produce; none requires new theory.

**P-A. State-shape derivation (rung 2).** For each decomposable combiner, derive the auxiliary state columns the stored table must carry and the projection that recovers the user-visible value: `AVG → (sum, count)`, `MAX_BY(v, o) → (v, o)`, `COALESCE`-once-write → `(v, written_flag)`, `stddev → (n, Σx, Σx²)`. The discriminants already classify decomposability; what is missing is the concrete state schema and the rewrite that hides it (state columns are internal; the presentation map projects them away). This single property dissolves: the companion-projection obligation, the once-write spelling restrictions, the `AVG`/`stddev` refusals, and unlocks approximate-sketch state (HLL for `COUNT(DISTINCT)`) as a user-optable contract relaxation later.

**P-B. Output-delta typing.** Derive, per model and per column group, the *shape of change the model emits*: `append-only within window(w)` ⊑ `keyed upsert` ⊑ `general` (a lattice the walk already uses internally for inputs). The transfer rules are classical: append-only in → append-only out through selection/projection/UNION ALL; append-only in through a keyed aggregation → keyed upsert out; joins take the worst input shape times fan-out. This is the property that makes incrementality *compositional*: a downstream model consuming a `keyed upsert` input is exactly the change-feed case (rung 3), and today it cannot even be posed because the DAG layer types every edge as day-interval dirt. Also directly unlocks: keyed dirt-set propagation (today refused), engine change-feed consumption, and streaming lowering (the same typed deltas are the streaming operators' input contract).

**P-C. Probe-backed world-facts.** Every declared fact (source posture, functional dependency, `referential_integrity`, bounded domain, declared recurrence bound, declared lateness) should derive a cheap runtime probe that falsifies it — the way the recurrence bound and count-preservation probes already work. Today `referential_integrity` is trusted unverified (an admitted narrowing), and declared lateness reaches no live scan. Rule worth adopting: *a declaration is admissible only if a probe exists for it*; "declared" then means "checked at run time" rather than "trusted forever", which makes the whole declared-facts surface safe to grow.

**P-D. Retraction accounting.** Track per-key derivation counts (the classical counting algorithm) so `DISTINCT`, `EXISTS`, and semijoin-shaped models stay maintainable under deletes/retractions instead of refusing. Cheap to state as a state-shape (a count column — i.e. a special case of P-A) plus a fold rule; high value once mutable sources are common.

**P-E. Column-scoped and key-scoped dirt.** Propagation edges today carry whole-partition day intervals. Column lineage per edge is already derived (fingerprint projections, sensitivity groups); typing edges as (columns × keys-or-window) would let an upstream change to one payload column trigger only the downstream column-scoped cells. This is a property of the *edge*, and all its inputs exist — it is purely unconsumed.

**P-F. Key lifecycle.** Nothing today speaks about key deletion, retention, or departure beyond the retained-departed-keys carve-out. A small property/declaration pair — is the key domain grow-only, or can keys be retired, and with what retention obligation — would give snapshot-reconcile models an honest deletion story and give the contract lattice its retention axis.

**P-G. Semantic-eclipse fingerprinting.** "This definition change provably does not change stored output" (formatting, reordering, provably-equal rewrites) — the backbuild diff engine already computes most of this; surfacing it as a verdict gates no-op migrations, semantic deploy gates, and CI caching. Adjacent, not core, to maintenance.

Deliberately *not* pursued: full Z-set/DBSP delta rules as a runtime (delegate to engine MVs past the ladder), and any property whose only consumer would be a cost estimate (measure instead — `smelt bakeoff` exists).

## 3. Missing transformations

**T-A. Per-group targeted recompute.** When a non-invertible aggregate receives a retraction, recompute *only the affected groups* from their bounded input slice instead of refusing to full refresh. Gated by: derivable group key (the walk's grain), bounded per-group read footprint, and delta discovery naming the affected keys. This is the single highest-value missing technique — it converts the design's harshest refusal into a bounded repair, and it is what every surveyed production IVM engine does.

**T-B. Diff-then-patch write.** Compute the new value for a slice, diff against stored state, write only the difference. One new write-pattern registry entry; serves reconciliation runs, drift repair after a probe fires (P-C's remedy path), and cheap idempotent re-runs.

**T-C. Change-feed consumption end-to-end.** The input-delta classifier already names `ChangeFeed`; nothing consumes it. With P-B typing model outputs, a maintained model's own emitted delta becomes its consumer's change feed — closing the loop is a transformation (fold-over-upsert-delta) plus ledger support, not new analysis.

**T-D. Shadow-build-and-swap.** Build the new state adjacent, verify (count/fingerprint probes), swap atomically. The honest generic fallback for every "refused: full refresh" — same cost as full refresh but with zero downtime and a verification hook; also the natural executor for backbuild's destructive options.

**T-E. Backfill choreography.** Compose what already exists — backward resolution (`required_inputs`), backbuild options, ledger catch-up — into one resumable plan: minimal upstream read set, ordered repair of downstream cells, restartable via the ledger. All parts are built and unwired; the transformation is the composition.

**T-F. Work subsumption.** When a pending small run is implied by a scheduled larger one (its input set is a subset and the technique is idempotent-graded), skip it. Pure plan-level reasoning over the ledger; an easy, visible operational win.

## 4. The rethink

Three moves, ordered by how much they change.

### 4.1 Typed deltas as the unit of composition

Recast what a maintained model *is*: not "a table with a refresh mode", but **a typed function over deltas** — for each column group, a mapping from typed input deltas to a typed output delta, with the stored table as accumulated state. The type is the delta-shape lattice (append-only-in-window ⊑ keyed-upsert ⊑ general) plus addressing (window / key set / whole).

- A model over raw clocked sources gets `append-only window` inputs — today's window-forward shape, unchanged.
- A model over a maintained keyed upstream gets a `keyed upsert` input — today an unposed question; with P-B it is just another admissible input type with its own fold rules.
- The DAG layer stops being a separate day-interval dirt system and becomes *delta type propagation*: an edge carries the upstream's output-delta type projected through the downstream's sensitivity. Day intervals survive as the addressing of one delta type, not as the universal currency.

This is the move that makes the "keyed frontier" a real abstraction rather than a plan name: **a frontier is the ledger's record of which typed deltas a cell has absorbed** — a watermark for window types, a key set or feed offset for upsert types, graded by combiner algebra exactly as the reconciliation ledger already grades storage. One concept; the two current ledgers become its two realizations (the record and its transactional write).

It is also the door to everything beyond batch: streaming lowering consumes the same typed deltas continuously; engine MVs are the delegation target for `general`-typed cells; `smelt serve` freshness endpoints read frontiers.

### 4.2 The contract as a lattice, not a point

Keep the equivalence invariant as the **default** contract — it is the right default and the oracle that makes everything testable. But let users *declare relaxations per column group × input*, each a named point with a validation obligation and honest grading in `smelt explain`:

- **frozen horizon** — partitions older than H are never revisited; late data outside H is diagnosed, not folded (today's silent-late-arrival gap becomes a declared, checked policy);
- **deferral** — this group may lag its inputs by up to D (licenses work subsumption and batching);
- **reconciliation points** — equivalence is promised at declared points (end of day), not after every run (licenses cheap approximate intermediate folds plus a T-B patch at the point);
- **declared indifference** — equivalence modulo stated tie-breaks or tolerances (generalises the two existing carve-outs: departed keys, ordering ties — which are then no longer special cases in the spec but ordinary lattice points);
- **retention** — P-F's key-departure policy.

The design bet (inherited from the differentiation research, still right): a *small closed algebra* of relaxation primitives that users compose — they pick and parameterise lattice points, they do not define new ones. Every relaxation must answer: what does the oracle become, and what probe checks it in production. Anything that cannot answer both is not admitted to the lattice.

This is what "flexibility and power" should mean here — not more modes, but a contract the user can weaken *explicitly, per cell, with the weakening printed*.

### 4.3 The user surface: facts, verbs, and proofs — never modes

- **Facts:** the model declares what is true (clock, identity, world-facts, relaxations). All admissible facts are probe-backed (P-C).
- **Verbs:** operational intent addresses *plan cells*, not models — prefer/pin a technique, pin a write pattern, freeze a horizon, force a repair, choreograph a backfill. The `maintenance:` override ladder is the seed of this; extend it rather than inventing a mode system.
- **Proofs as product:** every refusal names the missing fact ("declare X and it folds") or the missing machinery (a tracking link) — this discipline exists and is the UX to invest in: `smelt prove` report cards, `must_hold:` assertions failing compilation, proof-diffs in CI. The property layer is only worth its 31 rows if users can see it.

Consequence for architecture: incremental models become the default **kind** over a kernel of (walk properties, typed deltas, plan cells, frontier ledger, single-owner emitters). Streaming, MV delegation, and the dbt adapter are other kinds over the same kernel. The kernel is extracted from the working implementation — the existing CI gates (plan purity, single-owner emission, walk rule, conformance oracle) already draw its boundary; keep hardening that boundary and implement the next feature *as if* it were a kind before externalising anything.

---

## 5. Detail — evidence for the "substrate outgrew the product" claim

Pointers into the corpus; this section is the only one that assumes repo access.

- **Unconsumed proofs:** change comparability, region row identity, window independence, fingerprint projection, `bounded_domain`, join-contribution monotonicity, `functional_dependency_verdict_over_vector` (outside locality route 2), decomposed-state stubs — built, unwired (`docs/specs/model_properties.md` §Known Divergences; `crates/smelt-logical/src/maintenance/mod.rs:298,312,478`). Backbuild is entirely unwired outside `smelt-logical` (`backbuild/mod.rs:9`).
- **Rung-2-shaped warts:** `MAX_BY` companion projection, once-write two-spellings restriction, `bounded_domain` with no consumer, `AVG`-class refusals (`docs/specs/incremental_models.md` §column-family catalogue and §Known Divergences; `analysis/decomposed_state.rs:46,89`).
- **Spec accretions:** two ledgers under one name (reconciliation vs transactional merge, plus "guarantee ledger"); "frontier" absent as an abstraction; §Known Divergences at ~12% of the file with settled-decision essays; anti-exclusivity polemics arguing with retired drafts; `grain: key_per_partition` declared but refusing at derivation; a backend strategy enum with two dead variants; `batched.*` config fossils. (All in `docs/specs/incremental_models.md`; the 2026-07-15 consolidation note in `docs/ROADMAP.md` marks where the layering originated.)
- **Coarse DAG layer:** propagation is whole-partition day intervals; keyed dirt-sets refused (`maintenance/propagate.rs:31,351`); keyed outputs carry an *unverified* footprint mirror into propagation (`maintenance/derive.rs:719` residue). Column lineage per edge exists but no edge consumes it.
- **Fold-family near-complete, repair-family absent:** the technique enum is four fold/recompute variants; there is no targeted repair, no diff-patch, no swap (`maintenance/mod.rs:169`). The IVM survey's top-ranked gaps are exactly these (`docs/research/20260724-ivm-pattern-gap-catalogue.md` §A1, C1).
- **Declared-but-unchecked facts:** `referential_integrity` licenses a narrowing with its runtime tripwire unbuilt; declared source lateness reaches no live scan (`model_properties.md` §Known Divergences; `analysis/temporal.rs:564`).

Prior research this doc builds on (and where it diverges): `20260726-beyond-ivm-differentiation.md` supplies the relaxation lattice and kernel/kinds framing — adopted here; `20260724-ivm-pattern-gap-catalogue.md` supplies T-A/T-B/P-D — adopted; `20260809-creative-ideas.md` Themes A/C supply the proofs-as-product and beyond-batch items — adopted selectively. The divergence: those docs treat the DAG/delta question as one relaxation among many; this doc promotes **output-delta typing (P-B / §4.1) to the central move**, because it is the only item that changes what the abstraction *is* rather than adding to it.

## 6. Sequencing and open questions

Order of work (each step independently shippable, each dissolving named warts):

1. **Rung 2 state shapes (P-A)** — deletes the companion-projection and once-write-spelling surface; unlocks `AVG`-class folds.
2. **Per-group repair + diff-patch (T-A, T-B)** — converts the harshest refusals to bounded costs; gives probes a remedy path.
3. **Probe-backed declarations (P-C)** — makes the declared-facts surface safe to grow before the lattice grows it.
4. **Output-delta typing + typed edges (P-B, T-C, P-E)** — the compositional core; subsumes keyed dirt-sets and change feeds.
5. **Contract lattice v1 (frozen horizon + deferral first)** — the two relaxations with the clearest oracles.
6. **Spec redraft** around typed deltas / contract lattice / cells-and-verbs; fold both ledgers into the frontier concept; delete the accretions.

Open questions to settle before step 4:

- **Is the delta type per model or per column group?** Sensitivity analysis says groups differ; per-group typing is more precise but makes edge types vectors. Leaning per-group (it matches the plan matrix), accepting vector edges.
- **Where does the equivalence oracle live under the lattice?** Each relaxation redefines the oracle for its cells; the generative conformance gate must parameterise over lattice points or it silently tests only the default. This is a gate-design question as much as a spec question.
- **Kernel externalisation timing** — the extension-surface risk stands: publishing verbs/kinds APIs over a plan model still being reshaped by steps 1–5 would burn early adopters. Externalise nothing before step 6.
