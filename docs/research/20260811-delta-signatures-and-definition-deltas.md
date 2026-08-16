# Delta signatures as the front door; definition changes as deltas

**Date:** 2026-08-11
**Status:** research — direction agreed (one delta algebra underneath, plan-and-approve workflow on top); spec work not yet planned
**Question:** now that the 2026-08 incremental-models workstream has landed, what should the product become next — and is the "four corners" framing still the right starting point?
**Builds on:** `docs/research/20260809-incremental-rethink.md`; supersedes its §6 sequencing.

---

## Background: how smelt's incremental models work today

This section is a self-contained primer so the rest of the doc reads without the spec open.

An **incremental model** in smelt is an ordinary SQL model whose stored table smelt keeps up to date without re-running the SQL from scratch. The modeller declares almost nothing: `refresh: incremental` plus at most two **shape-defining facts** about the output — a **clock** (`timeseries:` — the output has a time axis, so it can be maintained window by window) and an **identity** (`unique_key:` — the output is addressable by key, one row per key). Everything else is *derived* from the model's SQL: which maintenance technique runs, what each run scans, how writes locate stored rows. Where a shortcut can't be proven safe, smelt **refuses with a diagnostic** naming what's missing — it never silently substitutes something else.

The single correctness promise is the **equivalence invariant**: after any sequence of incremental runs, the stored table equals what a full refresh of the model's SQL would produce over the inputs processed so far. Every proof and every technique exists to preserve this equation, and a generative conformance test suite checks it by literally running incremental histories against a full-refresh oracle.

The machinery around that promise, in one paragraph each:

- **The four corners.** The two declared facts vary independently, giving a 2×2 of output shapes the spec currently opens with: clock only (a time-partitioned table, maintained by rewriting touched partitions), identity only (a keyed table, maintained by folding deltas into per-key state), both (a time-partitioned keyed table), neither (not maintainable). The friendly name for a model's corner is its **grain** (`partition` / `key` / `key_per_partition`).
- **The maintenance plan.** For each model, smelt derives a set of **cells** — one per combination of a column group (output columns that change together), a trigger (new data, upstream mutation, definition change, backfill), and the specific input that changed. Each cell records its technique, how its write addresses stored rows, and the bounded window of each input it reads. `smelt explain <model>` prints the plan. Operators can steer choices among *proven-equivalent* techniques via a `maintenance:` frontmatter block (prefer/pin a technique or write pattern per cell) — steering never widens what's admissible.
- **Typed deltas.** Internally, smelt classifies the *shape of change* flowing along each edge of the model DAG, on a three-point scale: `append-only within a window` ⊑ `keyed upsert` ⊑ `general change`. A model over an append-only event feed sees append-only input; a keyed aggregation over it *emits* keyed upserts; a join degrades toward `general`. This typing was recently extended from inputs to **model outputs**, so a chain of models can in principle be incrementally maintained end to end.
- **The frontier.** The bookkeeping record of which deltas each cell has already absorbed — a watermark for windowed shapes, a delta-identity record where re-folding the same delta twice would double-count. It's what makes runs idempotent and resumable.
- **The contract lattice.** The equivalence invariant is the *default* contract, but a modeller can declare named, checked **relaxations** that trade a bounded amount of equivalence for capability. Two shipped so far: `frozen_horizon: H` (partitions older than H are never revisited; a late arrival that would land there raises a diagnostic instead of silently folding) and `deferral: D` (the table may lag its inputs by up to D, which licenses skipping runs whose entire pending input is younger than D). Each relaxation is a triple — a declaration, a precise restatement of what the oracle becomes, and a runtime probe that checks it — and `smelt explain` always prints what was relaxed.
- **Probes.** Declared facts about the world (a source is append-only, a join key is unique, a foreign key always resolves) get cheap runtime tripwires that falsify them, so "declared" means "checked in production", not "trusted forever".
- **Backbuild (the unwired layer).** Separately from data maintenance, there is a synthesis layer that handles **definition changes**: given a diff of the model's SQL, it classifies what each output column needs (nothing, an in-place backfill, a re-derivation from inputs, or a full rebuild because the grain changed) and can emit the migration statements — sixteen techniques for migrating a stored table across a code change without rebuilding it. This layer exists, is fully tested, and is called by nothing outside its own crate.

The 2026-08 workstream (six back-to-back "outcomes") built the last four items above — decomposed aggregate state, a repair family (targeted per-group recompute, diff-then-patch writes), probe-backed facts, output-delta typing, the contract lattice, and a full spec redraft. This doc asks what comes after.

---

## Key claims

1. **The gap has moved, not closed.** More of the new machinery is user-reachable than expected — contracts genuinely clamp writes and skip runs, probes dispatch live, explain prints delta types and contract points. What remains is not missing machinery but three specific disconnects: the *scheduler* doesn't consume the delta types the *analyzer* derives; the backbuild layer is entirely unwired (and its absence isn't even recorded in the spec's gap list); and the "proofs as product" UX — making the derivation visible and assertable — is unbuilt.

2. **The two declared facts are right; the four-corners 2×2 as the opening mental model is wrong.** The grid was never really four (one corner is uninhabitable, one is derived-only and currently refused), it classifies the *stored table* while everything new classifies *change*, and it is model-local while the differentiating capability is now DAG-global. The front door should be the **delta signature**: per column group, the typed change a relation emits. The corners survive as implementation profiles, not the front door.

3. **Definition changes are deltas.** A delta is either a **data delta** (rows changed in an input) or a **definition delta** (the model's own SQL changed). The frontier already models a definition change as "this column group has processed nothing yet, over every existing region"; taking that seriously unifies three currently-unrelated mechanisms — the live column-add handling, the unwired backbuild layer, and no-op-change detection — under one algebra, with the correctness oracle unchanged: the *new* definition's full refresh over the processed inputs.

4. **One algebra underneath, plan-and-approve on top.** The unification is the *semantics*; the *workflow* for definition deltas stays operator-gated. Data deltas fold automatically because they are routine and bounded; definition deltas can be destructive and are rare, so their derived migration plan is **presented and approved** (terraform-plan-shaped), never auto-applied. Both halves read the same plan data; the gate is a property of the delta kind's workflow, not a second machinery.

5. **Sequencing:** (a) make the scheduler consume delta types, so end-to-end incrementality becomes true rather than merely printed; (b) wire backbuild as the definition-delta arm behind a plan-and-approve verb; (c) ship the next contract-lattice points (retention first); (d) build the proofs-as-product surface. Each converts something already built into user functionality; none adds new theory.

---

## 1. Where we are (census, 2026-08-11)

What a modeller can actually reach today:

- **Declared surface:** the two shape facts; `contract:` with `frozen_horizon`, `deferral`, and per-cell refinements; per-column contract annotations; `probes:` cadence control; the `maintenance:` steering block; `merge_key:`. Several legacy config spellings were retired with hard errors that name their replacement.
- **Live behaviour:** frozen-horizon write clamps with a late-arrival probe; deferral-licensed run skipping with correct propagation to dependents; fact probes dispatched before writes; targeted per-group recompute and diff-then-patch writes admitted where provable; decomposed aggregate state (`AVG`, `STDDEV`, `MAX_BY`, once-write patterns) stored as hidden state columns with the user-visible value projected at read; keyed model chains maintained incrementally end to end in the simple cases.
- **Explain:** per-edge delta types (with the operator that degraded the type named), per-cell contract points, the probe plan, state columns, repair details.

The three disconnects:

**D1 — Typing without scheduling.** Delta types shape admission and are printed, but the DAG scheduler's currency for "what needs re-running" is still whole day-intervals. Concretely: a keyed-upsert upstream feeding a partition-grain downstream derives a key-addressed repair cell that the run loop never dispatches (a registered known bug — the result is correct but not incremental); propagation tracks *which key columns* are affected but not *which key values*; and cross-model runs (`--since-upstream`) require the operator to state what landed upstream on the command line, because no per-source watermark is persisted. The headline claim — incrementality composes through the DAG by construction — is true at the type level and false at the scheduler level.

**D2 — Backbuild unwired and colliding.** The definition-change synthesis layer (`crates/smelt-logical/src/backbuild/`) states in its own module header that nothing outside the crate calls it; only its tests do. The spec's gap list never mentions it — the gap is unrecorded. Meanwhile a CLI verb named `smelt backbuild` exists and means something else entirely (rebuild a model and its upstreams over a time range — an ordinary ranged re-run), and the *live* handling of definition changes is a third, narrower mechanism that covers column additions only. Three unrelated things share one territory and two share one name.

**D3 — Proofs-as-product unbuilt.** The refusal discipline is real — every refusal names the missing fact or machinery — but the product around it is not: explain still omits the per-column guarantee summary and the derived run shape; the declared technique-preference ladder and the "warn instead of error" scan-bound option parse but nothing consumes them; there is no cost model choosing between two admissible techniques; and there is no `smelt prove` report card, no `must_hold:` assertion that fails compilation, no proof-diff in CI.

## 2. The four corners, demoted

Three reasons the 2×2 should stop being the opening move of the spec and docs:

1. **It was never four.** The no-clock/no-identity corner is not maintainable, and the clock-plus-key-recurring-across-partitions corner (`key_per_partition`) is derived-only and currently refuses at plan derivation. Two orthogonal facts give three working shapes; the grid promises a symmetry that isn't there.
2. **It types the wrong thing.** The corners classify stored output. Everything the recent work derives — typed deltas per column group, frontiers, contract points, cells keyed by trigger and changed input — classifies *change*. The spec even carries a *second*, unrelated 2×2 also called "corners" (read scope × write scope, inside the plan matrix); one word doing double duty marks the storage taxonomy as a fossil of the pre-delta design.
3. **It is model-local.** "What shape is your table" cannot even pose the question the substrate now answers: what does this model *emit*, and what can its consumers do with that?

**The reframe.** Every relation — source or model — has a **delta signature**: per column group, the typed change it emits (append-only within a window ⊑ keyed upsert ⊑ general, plus how that change is addressed — by window, by key set, or whole-table). An incremental model *is* accumulated state plus a function from its inputs' signatures to its own, under a contract point, with a frontier recording what it has absorbed. Nothing about the declared surface changes: still two facts, optional world-facts, optional relaxations; the machinery still validates and never chooses. `grain` survives as the friendly name for the output signature's addressing; the corners survive as **shape profiles** — implementation chapters. The user pitch becomes "declare what's true; smelt types your pipeline's changes end to end and shows you the plan", replacing "classify your table into a grid".

This is a re-centering of spec, explain, and docs around what is already built — not a semantics change. Every existing normative rule survives the rewrite (the spec repo has an established claim-inventory method for verifying exactly that).

## 3. One delta algebra: data and definition deltas

The plan already indexes cells by trigger, and one of the four triggers is "definition change" — the algebra just doesn't treat it uniformly. Recast:

- A **data delta** is a typed change in an input relation: the existing lattice.
- A **definition delta** is a typed change in the model's own function. Per column group, the backbuild classifier's verdict is its transfer function:
  - **eclipsed** — the change provably does not alter stored output (formatting, reordering, provably-equal rewrites): the induced delta is *empty*, so the correct migration is nothing;
  - **pure backfill** — a new or changed column computable in place from columns already stored, no upstream read;
  - **re-derive** — a column that must be recomputed from inputs, column-scoped;
  - **skeleton change** — the change alters which rows exist or the model's grain: honestly a new relation, refused as an in-place migration (exactly as today).
- The **frontier** already handles the boundary case: a definition delta sets the affected column groups' processed-input record to "nothing yet" over every existing region; catch-up advances it region by region under the same fold rules and the same never-fold-ahead discipline that governs an ordinary new column today. A definition-delta migration is therefore *resumable* for free.
- The **oracle is unchanged**: after migration (or mid-catch-up, per region), the stored table must equal the *new* definition's full refresh over the processed inputs. The generative conformance suite extends to this by staging a definition edit mid-history — the same harness, one new step kind.
- The sixteen backbuild techniques become the **fold family for definition deltas**; shadow-build-and-swap and diff-then-patch are write mechanisms shared with the data side; per-cell admission, explain rendering, and the single-owner statement-emission rule all apply as-is.

**What this buys.** The unwired layer gets a principled wiring path instead of a bolt-on — plan derivation gains a definition-delta trigger source feeding the same plan matrix, and the maintenance driver dispatches its cells like any others, behind the gate below. The spec gets smaller: the definition-change section, the backbuild design, and no-op-change detection stop being three separate stories.

**The boundary that stays.** Retraction handling and change-feed consumption remain data-side future work; nothing here depends on them. Skeleton changes remain refused as in-place migrations.

## 4. Plan-and-approve on top

What distinguishes the two delta kinds is their *workflow*, not their semantics:

- **Data deltas fold automatically.** Routine, bounded, oracle-checked; that is the whole point of maintenance.
- **Definition deltas are planned and approved.** On detecting one, smelt derives the migration plan — per column group: the verdict, the technique, the regions touched, a cost class, or "provably no-op" — and **presents it**. A verb approves and executes; nothing destructive runs unapproved. Terraform-shaped: plan, review, apply, resume via the frontier if interrupted.

Concrete surface consequences:

- **Rename the ranged-rebuild verb.** Today's `smelt backbuild` (rebuild a range of a model plus upstreams) must not share a name with definition-delta migration. Either free the word for the migration verb or retire it in favour of `plan`/`apply`-style naming; decide at spec time — the collision itself is the bug.
- **CI mode falls out.** The plan in `--json` plus an exit-code contract makes "definition changed, with non-trivial unapproved migration cells" a CI-visible state, while eclipsed-only changes pass silently — a semantic deploy gate, essentially free.
- **Verification hooks live here too.** Destructive migration options (drop-and-rebuild legs, table swaps) surface their count/fingerprint verification probes in the presented plan, before the swap.

## 5. The lattice's next points

The contract lattice shipped with two points (frozen horizon, deferral). The deferred points each land *stronger* under the delta framing, in priority order:

- **Retention / key departure.** The sharpest user-facing gap on record: a keyed model reconciled against a snapshot source retains a departed key forever — there is no deletion mechanism at all. As a lattice point, declared retention gives deletion a contract home (licensing tombstones or hard deletes with a stated policy), and the current "departed keys are retained" special case becomes the default point's honest statement rather than a footnote. In delta terms it is the missing **retraction row** of the delta lattice — which is also the prerequisite shape for maintaining aggregates under deletes and for consuming change feeds with delete events. One point, two roadmaps.
- **Reconciliation points** — equivalence promised at declared moments (say, end of day) rather than after every run, licensing cheap approximate folds in between. Its mechanism now exists: the diff-then-patch write *is* the reconciliation, and elegantly, the diff at the declared point is simultaneously the probe (it measures the drift) and the remedy (it repairs it). Cheapest remaining point to ship.
- **Declared indifference** — equivalence modulo stated tie-breaks or tolerances. Mostly spec hygiene with real payoff: the two carve-outs currently special-cased inside the equivalence invariant (departed keys, ordering ties) become ordinary declared points. The genuine cost: the conformance suite's comparator must quotient by the declared relation — the first point where a plain set-difference comparison stops sufficing.
- **Per-column-group freshness** is not a separate point to plan: it is blocked on the same per-cell frontier bookkeeping gap already recorded for per-cell deferral, and rides that work.

**Interaction with definition deltas (must be decided, not discovered):** a definition delta touches *all* regions — including partitions a frozen-horizon contract says are never revisited. The plan-and-approve gate is the natural resolution: the migration plan surfaces the conflict, and approval is either explicit consent to cross the horizon or the plan clamps its backfill to the horizon and says so. Similarly, a deferral-licensed skip must never silently defer definition-delta catch-up — catch-up progress belongs on the presented plan, not the ambient scheduler.

## 6. Sequencing

Each step independently shippable, each converting built machinery into user functionality:

1. **Scheduler consumes delta types** (closes D1). Dispatch the key-addressed cells that already derive (the registered known-bug route first); let propagation carry key values where discovery admits them; persist a per-source watermark so cross-model runs stop requiring hand-fed deltas. Exit criterion: the conformance suite drives a keyed-upstream → partitioned-downstream chain incrementally with no command-line delta.
2. **Definition-delta unification + plan-and-approve** (closes D2). Spec first (one algebra, two workflows, verb renames), then wire the backbuild layer behind the gate; extend the conformance harness with definition-edit steps; record the layer's current unwired state in the spec's gap list immediately (it is missing today).
3. **Lattice v2** — retention, then reconciliation points; both consume step 2's machinery (approved destructive legs; diff-then-patch).
4. **Proofs as product** (closes D3). Explain completes (guarantee summary, run shape, per-input scope maps); preference/cost-model consumption; then `smelt prove`, `must_hold:` assertions, proof-diff in CI. Deliberately last: steps 1–3 change what the proofs say — and the proof surface should print the full lattice, not two points.
5. **Spec re-architecture**, alongside or just after step 2: (i) delta signatures and composition; (ii) the contract lattice; (iii) the plan — cells, verbs, frontier; (iv) shape profiles (the corners live here); (v) definition deltas — likely its own spec file (the incremental spec is 2,400 lines *after* a 30% cut).

## 7. Open questions

- **Verb naming.** What replaces `smelt backbuild` for ranged rebuild, and does migration live under `smelt diff` + `smelt apply` or one `smelt migrate` verb with `--plan`/`--apply`? Decide in the step-2 spec diff.
- **Approval persistence.** Is an approval a stored artifact (plan hash recorded; apply refuses if the plan has drifted — terraform-like) or a flag on the invocation? Leaning stored-hash: it is what makes CI mode honest.
- **Spec boundary.** Where the definition-delta spec draws its line against the model-declaration spec and the existing `schema_evolution:` frontmatter — whose full-refresh escape currently bypasses the atomic migration gate, a recorded divergence the unification should subsume rather than inherit.
- **Explain's primary line.** Does `grain` remain the headline label, or does the reframe justify printing the full signature (`emits: keyed upsert over [order_id], window 2d`) as the first thing explain says about a model? Leaning the latter; cheap once edges carry it.
