# Delta signatures as the front door; definition changes as deltas

**Date:** 2026-08-11
**Status:** research — direction agreed (one delta algebra underneath, plan-and-approve workflow on top); spec work not yet planned
**Question:** with the 2026-08-09 rethink's whole sequence landed (rung 2, repair family, probes, output-delta typing, contract lattice v1, spec redraft), what should the incremental-models product become next — and is the "four corners" framing still the right starting point?
**Builds on:** `docs/research/20260809-incremental-rethink.md` (executed in full by the six `docs/outcomes/20260809-*` outcomes); supersedes its §6 sequencing.

---

## Key claims

1. **The gap has moved, not closed.** The 2026-08 outcomes shipped the rethink's entire sequence, and more of it is user-reachable than the rethink predicted its successors would manage: `contract:` genuinely clamps writes and skips runs, `probes:` dispatches live with named diagnostics, `smelt explain` prints delta types, contract points, probe plans, state columns, and repair stanzas. What remains is not missing machinery but three specific disconnects: the scheduler does not consume the delta types it derives; the backbuild synthesis layer is entirely unwired (and unrecorded in the spec's gap list); and the proofs-as-product UX is unbuilt.

2. **The two declared facts are right; the four-corners 2×2 as the opening mental model is wrong.** The grid was never really four (one corner uninhabited, one derived-only and refused at plan derivation), it classifies the *stored table* while everything the outcomes built classifies *change*, and it is model-local while the differentiating capability is now DAG-global. The front door should be the **delta signature**: per column group, the typed change a relation emits. The corners survive as shape profiles — implementation chapters, not the front door.

3. **Definition changes are deltas.** A delta is either a **data delta** (rows changed in an input) or a **definition delta** (the model's own SQL changed). The frontier already treats a definition change as "processed-input vector `∅` over every existing region"; taking that seriously unifies three currently-unrelated things — the live column-add trigger, the unwired 16-technique backbuild layer, and semantic-eclipse fingerprinting — under one algebra with the oracle unchanged: the new definition's full refresh over the processed set.

4. **One algebra underneath, plan-and-approve on top.** The unification is the *semantics*; the *workflow* for definition deltas stays operator-gated. Data deltas fold automatically because they are routine and bounded; definition deltas can be destructive and rare, so their derived plan is **presented and approved** (terraform-plan-shaped), never auto-applied. Both halves read the same plan data; the gate is a workflow property of the delta kind, not a second machinery.

5. **Sequencing:** (a) make the scheduler consume delta types — the composition claim becomes true, not just printed; (b) wire backbuild as the definition-delta arm with the plan-and-approve verb, and resolve the `smelt backbuild` name collision; (c) build the proofs-as-product surface. Each converts something already built into user functionality; none adds new theory.

---

## 1. Where we are (census, 2026-08-11)

All six `docs/outcomes/20260809-*` outcomes are done. What a modeller can actually reach today:

- **Declared surface:** `refresh: incremental` + clock + identity; `contract: {frozen_horizon, deferral, cells[]}`; `columns.<c>.contract: plausible`; `probes: {cadence}`; `maintenance: {defaults.prefer, cells[], scan_bounds}`; top-level `merge_key:`. The `batched:` block and `nondeterministic_columns` are retired fail-loud; `grain: key_per_partition` is refused at parse.
- **Live behaviour:** frozen-horizon write clamps and baseline-comparative late-arrival probes (`smelt-runtime/src/contract_probes.rs`); deferral-licensed run skipping with dependent propagation and ledger-proven subsumption; four fact probes dispatched at pre-write sites; per-group recompute and `diff_patch` admission with fail-closed affected-key discovery; decomposed state (`AVG`, `STDDEV`/`VAR`, `MAX_BY`/`MIN_BY`, once-write) with hidden `__part` columns and compile-time presentation projection; clockless keyed chains maintained incrementally end-to-end via typed model edges.
- **Explain:** delta type per inbound edge (with the degrading operator named), contract point per cell, probe plan, state columns, repair stanzas.

The three disconnects:

**D1 — Typing without scheduling.** Delta types shape admission and are printed, but the graph's dirt currency is still day intervals: a `KeyedUpsert` upstream feeding a `grain: partition` downstream derives a key-addressed cell the run loop never dispatches (registered `KnownBug`); keyed dirt-sets carry key *columns* and provenance, not key *values*; `--since-upstream` takes landed deltas on the command line, no persisted watermark. The §4.1 claim of the 2026-08-09 rethink — incrementality composes through the DAG by construction — is true at the type level and false at the scheduler level.

**D2 — Backbuild unwired and colliding.** `crates/smelt-logical/src/backbuild/` (classify/diff/emit/requalify — 16 techniques migrating a table across a definition change) states in its own module header that nothing outside `smelt-logical` calls it; grep confirms only tests do. The spec's Known Divergences never mentions it — the gap is unrecorded. Meanwhile `smelt backbuild` the CLI verb exists and means something else entirely (ranged rebuild of a model plus upstreams via the ordinary run path), and the *live* definition-change handling is a third module (`analysis/definition_change.rs`, column adds only, three-class policy in the spec's §"The definition-change trigger").

**D3 — Proofs-as-product unbuilt.** Explain still prints neither the per-column guarantee ledger nor the derived run shape/execution postures; the `prefer` soft-bias ladder and `scan_bounds.on_violation: warn` parse but nothing consumes them; the cost model between two admissible techniques is unbuilt; no `smelt prove` report card, `must_hold:` assertion, or proof-diff-in-CI exists. The refusal discipline (every refusal names the missing fact or machinery) is real; the product around it is not.

## 2. The four corners, demoted

Three reasons the 2×2 should stop being the opening move of the spec and docs:

1. **It was never four.** The no-clock/no-identity corner is uninhabitable, and `key_per_partition` is derived-only and refuses at plan derivation. Two orthogonal facts give three working shapes; the grid promises a symmetry that isn't there.
2. **It types the wrong thing.** The corners classify stored output. Everything the substrate now derives — typed deltas per column group, frontiers graded by combiner algebra, contract points restating the oracle, cells keyed by trigger × changed-input — classifies *change*. The spec even carries a second "corners" 2×2 (read scope × write scope in the plan matrix); the word doing double duty marks the storage taxonomy as a fossil of the pre-delta design.
3. **It is model-local.** "What shape is your table" cannot pose the question the substrate now answers: what does this model *emit*, and what can its consumers do with that.

**The reframe.** Every relation — source or model — has a **delta signature**: per column group, the typed change it emits (`append-only within window(w)` ⊑ `keyed upsert` ⊑ `general`, plus addressing). An incremental model *is* accumulated state plus a fold from its inputs' signatures to its own, under a contract point, with a frontier recording what it has absorbed. Nothing about the declared surface changes: two facts, optional world-facts, optional relaxations; the machinery validates, never chooses. `grain` survives as the friendly name for the output signature's addressing; the corners survive as shape profiles. The user pitch becomes "declare what's true; smelt types your pipeline's changes end-to-end and shows you the plan", replacing "classify your table into a grid".

This is a re-centering of spec, explain, and docs around what is already built — not a semantics change. The claim-inventory method (`docs/specs/CLAUDE.md`) applies: every normative rule survives.

## 3. One delta algebra: data and definition deltas

**The unification.** A cell's trigger today is already `creation | mutation | definition change | backfill` — the algebra just doesn't say so uniformly. Recast:

- A **data delta** is a typed change in an input relation (the existing lattice, addressed by window / key set / whole).
- A **definition delta** is a typed change in the model's own function: per column group, the backbuild classifier's verdict is its transfer function — `Eclipsed` (provably no output change: formatting, reordering, provably-equal rewrites — the semantic-eclipse verdict is exactly "the induced delta is empty"), `PureBackfill` (in-place fill, no upstream read), `UpstreamRederive` (column-scoped recompute from inputs), `SkeletonChange` (grain change — effectively a new relation).
- The **frontier** already handles the boundary case: a definition delta instantiates the affected groups' processed-input vectors at `∅` over every existing region; catch-up advances `∅ → current` under the same fold/recompute-reset rules and the same never-fold-ahead discipline (§"The definition-change trigger"'s group-convergence rule generalises unchanged).
- The **oracle is already right and unchanged**: `incremental_state(S) == full_refresh(new_definition, inputs ∈ S)`. The generative conformance gate extends to definition-delta steps by staging a definition edit mid-sequence — the same harness, one new step kind.
- The 16 backbuild techniques become the **fold family for definition deltas**; shadow-build-and-swap and diff-then-patch are write mechanisms it shares with the data side; per-cell admission, `smelt explain` rendering, and statement-emission single-ownership all apply as-is.

**What this buys.** The unwired layer gets a principled wiring path instead of a bolt-on: `derive.rs` gains a definition-delta trigger source feeding the same plan matrix, and the maintenance driver dispatches its cells like any others — behind the gate below. The spec gets smaller: the definition-change trigger section, the backbuild plan docs, and semantic eclipse stop being three stories.

**The boundary that stays.** Retraction/rung-3 and change-feed consumption remain data-side future work; nothing here depends on them. `SkeletonChange` remains refused as an in-place migration (honest answer: new relation), exactly as `MaintenanceSkeletonColumnAdded` refuses today.

## 4. Plan-and-approve on top

The workflow property that distinguishes the two delta kinds — not their semantics:

- **Data deltas fold automatically.** Routine, bounded, oracle-checked; this is the whole point of maintenance.
- **Definition deltas are planned and approved.** On detecting a definition delta, smelt derives the migration plan — per column group: verdict, technique, regions touched, cost class, or "provably no-op (eclipsed)" — and **presents it**. A verb approves and executes it; nothing destructive runs unapproved. Terraform-plan-shaped: `smelt diff` (already "pending schema changes") grows into the presentation surface; the approval verb executes the stored plan, resumable via the frontier.

Concretely on the CLI surface:

- **Rename the ranged-rebuild verb.** `smelt backbuild` today means "rebuild a range of a model + upstreams" — a data-side operational verb (closest to the rethink's backfill choreography). It must not share a name with definition-delta migration. Candidate: `smelt rebuild` (or fold into `smelt run --rebuild`); "backbuild" is either freed for the migration verb or retired entirely in favour of `smelt diff` / `smelt apply`-style naming. Decide at spec time; the collision itself is the bug.
- **CI mode:** the plan in `--json` + an exit-code contract makes "definition change with non-eclipsed, non-approved cells" a CI-visible state; eclipsed-only changes pass silently — the semantic deploy gate the 2026-08-09 rethink's P-G wanted, for free.
- The same presentation is where **destructive backbuild options** (drop-and-rebuild legs, swap) surface their verification hooks (count/fingerprint probes from the probe layer) before the swap.

## 5. Sequencing

Each step independently shippable, each converting built machinery into user functionality:

1. **Scheduler consumes delta types (closes D1).** Key-addressed dispatch from the run loop for the derived cells that exist today (the registered `KnownBug` route first); keyed dirt carries key values where discovery admits; persisted per-source watermark so `--since-upstream` stops requiring hand-fed deltas. Exit criterion: the conformance gate drives a keyed-upstream → partition-downstream chain incrementally with no command-line delta.
2. **Definition-delta unification + plan-and-approve (closes D2).** Spec first (one algebra, two workflows, verb renames), then wire `backbuild/` behind the gate; extend the conformance harness with definition-edit steps; record the layer's current unwired state as a Known Divergence immediately (it is missing today).
3. **Proofs as product (closes D3).** Explain completes (guarantee ledger, run shape, postures, scope maps); `prefer`/cost-model consumption; then `smelt prove` / `must_hold:` / proof-diff. Deliberately last: steps 1–2 change what the proofs say.
4. **Spec re-architecture**, alongside or just after step 2: (i) delta signatures and composition; (ii) the contract lattice; (iii) the plan — cells, verbs, frontier; (iv) shape profiles (the corners live here); (v) definition deltas — likely its own spec file (`incremental_models.md` is 2,400 lines *after* a 30% cut), cross-owned with the same care as `model_properties.md`/`model_transforms.md`.

## 6. Open questions

- **Verb naming.** What replaces `smelt backbuild` (ranged rebuild), and does the migration flow live under `smelt diff` + `smelt apply`, or one `smelt migrate` verb with `--plan`/`--apply`? Decide in the spec diff for step 2.
- **Approval persistence.** Is an approval a stored artifact (plan hash recorded, `apply` refuses on drift — terraform-like) or a flag on the invocation? Leaning stored-hash: it is what makes CI mode honest.
- **Where the definition-delta spec draws its boundary** with `models.md`'s declaration law and the existing `schema_evolution:` frontmatter (whose `full_refresh` escape currently bypasses the atomic migration gate — a recorded divergence the unification should subsume rather than inherit).
- **Does `grain` remain a printed label only**, or does the delta-signature reframe justify printing the full signature (`emits: keyed upsert over [order_id], window 2d`) as the primary explain line? Leaning the latter; cheap once edges carry it.
