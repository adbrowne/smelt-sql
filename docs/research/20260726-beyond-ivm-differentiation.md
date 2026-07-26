# Beyond IVM — what smelt can offer that a native IVM engine structurally cannot

**Date**: 2026-07-26
**Status**: brainstorm for discussion — no decisions. Companion to
`docs/research/20260724-ivm-pattern-gap-catalogue.md`, which surveyed *mechanisms* the field
has and smelt's registry doesn't. This note asks the opposite question: where is smelt's
approach different in kind, such that "just improve Enzyme/pg_ivm/Materialize instead" would
not get you there?

## 1. Framing: the contract is the product, not the delta rules

Start from the honest premise: smelt will not out-engineer the IVM engine authors at delta
derivation. DBSP already unifies the theory; Enzyme, Materialize, Feldera, and Snowflake
dynamic tables are staffed teams shipping exactly that. If smelt's pitch were "a better
incrementalizer", the right move would be to contribute to one of them.

But every native IVM engine sells **one fixed contract**:

> the view equals the query over the *current* base tables, maintained continuously
> (or within a target lag), for the query classes we support, by mechanisms we choose,
> inside our engine.

Everything expensive about IVM is the price of that contract's strongest clauses:
retraction-readiness forever (any base row might be deleted at any time), snapshot
consistency across all inputs, maintenance coupled to ingestion, mechanism opacity, and
engine lock-in. A user who doesn't need one of those clauses still pays for it — the engine
has no vocabulary for *not wanting* it.

smelt's invariant is deliberately weaker and deliberately explicit:

```
incremental_state(S) == full_refresh(inputs ∈ S)
```

Equivalence **up to the state we have fed it** — with `S` a first-class, queryable,
per-input fact. That is not a worse version of the IVM contract; it is a different point in
a **contract lattice**, and the thesis of this note is that smelt's differentiation is the
lattice itself:

- the user **declares knowledge** the engine could never assume (§4);
- the user **relaxes clauses** they don't need, and smelt proves the relaxed contract is
  still honoured (§5);
- the user **keeps control** of when/where/how maintenance runs (§6);
- everything derived is **inspectable and verifiable** (§7);
- smelt plans over the **whole project graph and multiple engines**, not one view in
  one engine (§8);
- and because smelt is a compiler over backends rather than a runtime, a missing
  capability is a **change you can make and test yourself** — extend, fork, or contribute
  at bounded risk, rather than filing a vendor ticket (§9).

An IVM engine is a black box with one guarantee. smelt is a **validator over a space of
guarantees**: you pick the point, smelt proves your SQL and declarations support it, refuses
loudly when they don't, and tells you what each relaxation bought. On this framing, the gap
catalogue's entries (A1 per-group recompute, C1 diff-then-patch, …) are *table stakes* —
mechanism parity worth having — while the sections below are the reason smelt exists.

The organising end-state of the whole argument comes first (§2): a **kernel** of proofs,
state, emission, and testing, with today's incremental-models feature as the **default
kind** built on it. Everything after §2 can be read as filling in what the kernel knows
(§4), what contracts it can validate (§5), what control and visibility it grants (§6–§8),
and who can extend it (§9–§10).

## 2. Kernel and kinds — incremental models as the *default* implementation

The strongest version of the extensibility and layer-surface arguments (developed in §9 and
§10), proposed for examination: factor smelt into a **kernel** (the property/proof layer,
the state/ledger substrate, the transformation/emission layer, the graph protocol, the
conformance harness) and treat today's incremental-models feature — partition grain, key
grain, the composed corner — as the **default model kind** implemented against that kernel.
Other kinds, making *different trade-offs*, could then be implemented by users (or shipped
as non-default extras) without the core contract having to bless them.

### 2.1 The division of ownership

**The kernel owns** (and no kind may reimplement):

- **Properties and proofs**: the walk verdicts (grain, combiner algebra, bounded reach,
  alignment, determinism, FDs), source world-facts, and the obligation vocabulary —
  "admission as a service": *does this SQL + these declarations discharge obligation O?*
- **State substrate**: the processed-set `S` bookkeeping, covered intervals, and grading —
  one authority for "what contract does this region meet right now".
- **Emission discipline**: statements are pure emitter outputs; backends execute, never
  author (the statement-parity rule, unchanged).
- **The graph protocol**: a typed edge interface — given upstream dirt, what does this node
  dirty downstream; given a requested region, what does it need upstream. Kinds implement
  it; the kernel composes it.
- **The harness**: oracle testing as a service — a kind states its invariant; the harness
  drives generated runs against it wherever the claim is testable.

**A kind owns**: its declared surface (frontmatter grammar — or an entire authoring
surface, §2.4), its contract (which lattice point it claims — see §2.3), its plan
derivation (properties → cells → techniques, using kernel proofs), and its grading
semantics. The default kind's contract is exact equivalence-at-`S`; that never weakens.

### 2.2 What could then stay out of core

The gap catalogue's §E — patterns surveyed and *deliberately rejected* — reads differently
under this factoring: it is a **kind wishlist**. Snapshot-diff SCD2 with execution-time
stamping (SQLMesh's `SCD_TYPE_2_BY_COLUMN`), declared non-idempotent keyed upsert
(`INCREMENTAL_BY_UNIQUE_KEY`), wall-clock rolling windows (catalogue D4), ignore-retract
postures, approximate/sketch-backed kinds — all are things real users demonstrably want
(SQLMesh ships them), all are things smelt's core rightly refuses to *bless as exact*, and
all are implementable against the kernel by someone who accepts the trade-off — provided
they declare it (§2.3). Core stops being the arbiter of every posture and becomes the
arbiter of *honesty about postures*. The breadth of SQLMesh's kind set is itself the market
evidence that one default kind, however good, will not cover everyone; the difference is
that smelt's kinds would be typed by contract rather than by folklore.

### 2.3 The honesty typing — kinds are points in the contract lattice

The dbt-macro failure mode (an ecosystem of untyped, unverifiable extensions) is the risk.
The defence is to reuse the relaxation lattice (§5) as the **type of a kind**: every kind
must declare which contract it claims — exact-at-`S` / exact-at-reconciliation-points /
exact-modulo-relation / non-idempotent (restatement requires rebuild) / best-effort — and
the kernel enforces the consequences:

- **Testable claims are tested.** An exact kind gets the full generative oracle. A
  reconcile-point kind gets oracle checks at reconciliation points. A non-idempotent kind
  gets its *limitations* verified (restatement genuinely refused, the non-idempotence
  printed in `explain`).
- **Grades propagate.** A downstream exact model consuming a non-exact upstream is tainted
  in the ledger; the graph layer makes weak links visible instead of laundering them.
- **Unknown is safe.** A kind that cannot (or does not) implement the graph edge interface
  degrades to the total-delta posture — over-running, never wrong — exactly as a full
  refresh upstream does today.

This is the load-bearing move: §5 describes relaxations as *parameters of the default
kind*; this section generalises them to *the typing discipline for all kinds*. The lattice
is the kernel's contract language, not a feature list.

### 2.4 Kinds may own the authoring surface — intent nodes, not only SQL nodes

The sections above still carry a hidden assumption: that every node *starts* as SQL, from
which the kernel proves properties. That pipeline direction — SQL → AST/types → property
proofs → plan — is **raising**: recovering intent from its lowered form. Raising is the
compiler's fragile direction; the SCD2-succession work
(`docs/research/20260723-scd2-succession-pattern.md`) is a case study in how much machinery
it takes to *recognise* one pattern in lag-function SQL, and how easily a refactor breaks
the recognition. When the pattern is the whole point of the node, the user should be able
to say so: an **intent node** is a graph node authored in a kind-owned surface — a
declaration, not a query — from which the properties hold **by construction** and the
maintenance plan **lowers** directly.

Concrete candidates, each with live market demand:

- **SCD2 as a declaration** (`scd2: {key, change_ts, attributes, …}`) — dbt *snapshots*
  are exactly this and are heavily used; nobody misses writing the lag-function SQL.
- **Data vault from metadata** — hubs/links/satellites generated from entity/relationship
  config; AutomateDV builds a business on this atop dbt string macros. The user's instinct
  is right: a vault author never wants an "initial SQL stage we then prove something
  about" — the config *is* the model.
- **Declared windowed aggregations** (feature-store style, Feast/Tecton feature views) —
  "sum of spend per customer over trailing 30d, daily grain" is a declaration whose
  combiner algebra, clock, and grain are all axiomatic; it is precisely a maintainable
  fold, stated without SQL.
- **Sessionization / dedup specs** — gap-parameterised sessions, keyed dedup with a
  recurrence window: shapes smelt currently proves out of SQL could instead be declared.

**The one rule that keeps intent nodes honest: every node must have a denotation, and the
denotation is *generated*, never hand-authored.** The equivalence invariant needs a
full-refresh oracle — `full_refresh(inputs ∈ S)` must mean something for every node. For
an intent node the kind *generates* the denotation (typically as SQL) from the intent.
Generating rather than hand-writing kills the drift risk (no second source of truth), and
it lets the generated denotation flow through the **existing pipeline**: type inference
gives the output schema downstream models consume, diagnostics and LSP hover work
unchanged, and the conformance harness runs the generated denotation as the oracle against
the kind's maintenance statements. Better still, running the property *walk* over the
generated denotation becomes a **cross-check**: the kind asserts its properties
axiomatically, the walk re-derives them from the generated SQL, and disagreement is a bug
in the kind's generator — the kind tests itself with kernel machinery.

Graph citizenship is unchanged and is the kernel's real requirement: however a node is
authored, it speaks the contract vocabulary — clock, identity, delta shapes in and out,
the edge protocol. The graph layer never asks how a node was written; it asks what it
dirties and what it needs. (Contrast today's genuinely opaque nodes — a Python model or an
imported dbt model — which degrade to total-delta. Intent nodes are the opposite extreme:
*maximally* transparent, because nothing about them needs recovering.)

Recognition and intent are complements, not rivals: classifiers (succession, top-N) serve
the install base — existing SQL, dbt imports — while intent surfaces serve greenfield; both
lower into the same registry cells and the same ledger. The
recognition-over-declaration philosophy inverts exactly when the pattern stops being an
implementation detail of a query and becomes the node's identity.

Risks, named: **expressiveness cliffs** — every config surface eventually meets a need it
can't express (dbt snapshots' fixed strategies), so intent grammars need typed SQL escape
slots (an expression *inside* a declaration, typed against the generated query) rather
than an all-or-nothing fall-off to raw SQL; **surface proliferation** — kinds must stay
few, and the mandatory generated denotation is the tax that keeps a surface from being
cheap folklore; **per-surface tooling** — schema-validated YAML is easy, but a surface
worth shipping needs its own diagnostics, and the LSP investment is SQL-shaped today.

### 2.5 smelt is already building this kernel — by accident of discipline

The architectural invariants the repo already enforces by CI gate *are* the kernel/kind
boundary: maintenance-plan purity (plans are pure data derived by pure functions —
kind-derivable), statement-emission single ownership (emitters are pure — kernel-owned),
the property-composition walk rule (verdicts come from one shared walk — kernel-owned),
and the generative conformance gate (the harness — kernel-owned). These exist today for
testability; they are the same cuts a kind API needs. The research claim worth recording:
**the kernel should be extracted from the working default implementation, not designed a
priori.** The spec's own design notes already take this posture for the crate boundary
("extraction-mechanical", the rejected `smelt-maintenance` crate) — the kind API is the
same judgment at the next level up.

### 2.6 Precedents and their lessons

- **dbt custom materializations / incremental-strategy macros** — proof of demand for
  exactly this extension point, and proof that shipping it without a correctness story
  yields an ecosystem nobody can trust or upgrade. The kernel's typing (§2.3) is the fix.
- **SQLMesh model kinds** — a closed, vendor-curated kind set; demand evidence and a
  catalogue of trade-offs users accept, but not extensible and not proof-carrying.
- **MLIR dialects over a shared IR + verifier** — the structural analogue: kinds are
  dialects, the property layer is the verifier, and the lesson is that the verifier and the
  shared IR must stabilise *before* the dialect ecosystem opens. Its second lesson backs
  §2.4: high-level dialects exist precisely because *raising* from lowered form is hard —
  semantics carried structurally beat semantics recovered by analysis.

### 2.7 Risks and sequencing

The risks are §9.4's, amplified: premature API design could distort the default
implementation, and the kernel surface is much larger than Tier-0 preference hooks. The
sequencing that manages both: (1) keep hardening the internal boundary the CI gates already
enforce; (2) implement the next internal features *as if they were kinds* —
`materialized_view` delegation is already a de-facto second kind, and the SCD2-succession
classifier is the natural shakedown for a third — extracting the kernel interface each one
actually needed; (3) only then externalise, contract-typing first (§2.3), Rust-internal
kinds before Python kinds. Not a near-term build, but a standing lens (§11): every
invariant kept pure today is kernel surface bought for free.

## 3. What the fixed IVM contract costs (the clauses users pay for)

Enumerated so §5's relaxations have something concrete to relax:

1. **Retraction-readiness.** The engine must be able to un-see any row forever. This is why
   non-invertible aggregates need per-group recompute or domain multisets, why DISTINCT
   needs derivation counts, why state can't be frozen or dropped. Most warehouse sources are
   append-only or bounded-late — the readiness is usually purchased and never used.
2. **Point-in-time cross-input consistency.** The view reflects one snapshot across all base
   tables. Requires multiversioning/coordination. Analytics consumers almost never need it —
   they live with "orders through Tuesday, customers through Monday" every day.
3. **Coupled maintenance.** Ingestion triggers (or target-lag schedules) maintenance; the
   user cannot say "land the data now, repair the expensive join column this weekend".
4. **Query-class cliffs.** An unsupported operator anywhere in the view usually means the
   *whole view* falls off the incremental path (or is rejected). The contract is
   all-or-nothing per view.
5. **Mechanism opacity.** The engine decides fold-vs-recompute; you cannot pin, measure,
   compare, or even reliably see the choice. (Enzyme's cost model chooses per refresh;
   Snowflake documents thresholds; neither is user-steerable.)
6. **Engine lock-in.** The view, its state, and its maintenance live and die inside one
   engine. State is opaque operator state; disaster recovery, migration, and "run the
   backfill somewhere cheaper" are not expressible.
7. **Uniform freshness.** One view, one lag. No "these columns hourly, that enrichment
   column daily".

Each clause maps to at least one smelt differentiation below.

## 4. Knowledge asymmetry — things smelt can know that the engine can't assume

An engine must be sound for arbitrary DML from an adversarial workload. smelt sits where
declarations, orchestration, and the whole project are visible — it can *trust and verify*
what an engine must *defend against*.

### 4.1 Declared source world-facts (already core; underweighted as differentiation)

`append_only`, max lateness, `key_recurrence`, settle bounds. Each declaration deletes an
entire branch of the engine's defensive machinery: append-only deletes retraction handling;
bounded lateness turns "retain everything forever" into a derived finite lookback; key
recurrence bounds dedup state. The spec has all of this — what's worth saying out loud is
that this is not a convenience feature, it is the *primary cost lever*, and no in-engine IVM
can offer it because the engine cannot afford to trust a user claim it can't police.

smelt can, because it can **police cheaply**: emit low-cost audit probes (watermark
monotonicity, spot-check append-only via count/max-rowid deltas, sampled recurrence-window
checks) and fail loudly on violation. Trust-but-verify at a sliver of the cost of
readiness-for-anything. This "declared fact + cheap validator + fail-loud" triple is a
reusable pattern every entry below can follow.

### 4.2 Declared intra-source relationships (the user's date/timestamp example)

Materialized columns often have a semantic relationship the schema doesn't state:
`event_date = CAST(event_ts AS DATE)` (or a timezone-shifted variant), `region` functionally
determined by `country`, `order_id` monotone in `order_ts`, partition column derived from a
payload field. Candidate declarations:

- **Column FDs** (`event_date ← event_ts` via a named expression): lets the planner rewrite
  a predicate on one column into a partition-pruning predicate on the other — constraining
  scans and windows exactly as the user intuited. Also licenses column-group factoring
  (a change to `event_ts` implies the derived column's change; no separate sensitivity).
- **Cross-column monotonicity** (`order_id` non-decreasing in `event_ts`): turns a key-range
  probe into a time-range clamp and vice versa; bounds semijoin scans in mutation cells.
- **Cross-source alignment** (`order_events.event_ts >= orders.order_ts`, bounded skew):
  bounds the join lookback between two feeds — today's horizon derivation must assume the
  worst; a declared skew bound narrows it.
- **Partition-expression truth** (`partition_column = f(event_time_column)` for a declared
  `f`): today the spec checks alignment structurally where it can; a declared `f` extends
  admission to sources smelt didn't create.

All are the same triple: declaration → derived clamp/proof → cheap audit probe + loud
failure. Engines have fragments (Snowflake clustering keys, Oracle dimension/hierarchy
declarations for query rewrite — the closest prior art and worth studying), but none feed an
IVM admission decision with user-declared semantic FDs.

### 4.3 Orchestration knowledge — smelt knows about *other runs*

The engine sees one refresh at a time. smelt sees the schedule and the run plan:

- **Deliberate non-repair**: "this run repairs partition P only; the orchestrator will run
  the other partitions" — the user's example. In smelt this is *already sound* because
  equivalence is per-`S` and the ledger records what's covered; the differentiation is that
  it's expressible at all.
- **Work subsumption**: a scheduled backfill of March subsumes the pending mutation repairs
  inside March — skip them. A pending definition-change backfill subsumes a pending
  dimension repair for the overlapping column group. IVM has no notion of "pending work that
  another job will do"; smelt's graph layer can coalesce obligations across triggers before
  emitting statements.
- **Cadence-fit techniques**: IVM engines optimise for per-transaction or minutes-lag
  updates; smelt knows the model runs daily and can pick techniques whose fixed costs
  amortise at batch cadence (bigger regions, recompute-over-fold pivots at much higher
  delta fractions).
- **Business-calendar horizons**: "books close on the 5th; prior months are immutable after
  close" is orchestration-level truth. Declared as a freeze horizon (§5.3) it deletes
  retraction-readiness for almost all of the table.

### 4.4 Consumption knowledge — smelt knows who reads the output

The project graph names every downstream consumer (and, eventually, BI/query logs name the
external ones):

- **Demand-driven maintenance**: if no consumer reads a column group or a region, its
  repairs can be deferred or elided (with the ledger grading the region stale-by-choice).
  An engine must keep the whole view correct because it can't see readers.
- **Per-consumer freshness**: propagate a delta eagerly along the edge feeding the SLA-bound
  mart, lazily along the edge feeding the weekly report. IVM's target lag is per-view;
  smelt's can be per-edge.

## 5. The relaxation lattice — guarantees the user can decline (the likely centre of value)

The user's hunch, made systematic. Each relaxation names: the clause relaxed, what it buys,
and how smelt keeps the *remaining* contract honest. The recurring design shape: **the
invariant is never silently weakened — the user declares the weaker contract, smelt
validates the declaration, and the ledger/explain surface shows exactly which contract each
region currently satisfies.**

### 5.1 Input-order freedom (relaxes clause 2/3 — already smelt's foundation)

Because equivalence is over the *set* `S`, the user may: backfill history without ingesting
the latest data; process the consumer-visible current partition first and catch up history
later; order inputs by cost or priority. An IVM refresh takes whatever has arrived, all at
once. This is already the spec's core; worth restating as the enabling relaxation from which
the rest follow.

### 5.2 Deferral — decouple landing from repairing (relaxes clause 3)

Let the fact delta fold in now, and *defer* the expensive dimension-fanout repair to a
declared window ("weekends", "next full run", "when delta fraction > x%"). The ledger
already grades regions; deferral is just an admitted state ("pending mutation repair for
column group G") with a scheduling policy attached. Buys: predictable daily runs, expensive
repairs batched into cheap compute windows. Napa's Queryable Timestamp (gap catalogue D1) is
the consistency-preserving form of the same idea — deferral with an explicit "consistent
through" frontier consumers can see.

### 5.3 Frozen horizons — decline retraction-readiness beyond a boundary (relaxes clause 1)

Declare: output older than horizon H is **frozen**. Consequences smelt can derive: state
needed only for retraction (counts, wide lookbacks) is dropped for the frozen region; late
or mutating input targeting a frozen region is **refused with a diagnostic** (or routed to
an explicit operator-approved thaw/backfill) instead of silently costing forever-readiness.
This converts the engine's open-ended liability into a bounded one, and it matches how
warehouses actually operate (books close; reprocessing past the close is an *incident*, not
a Tuesday). The equivalence invariant survives in refined form: equivalence over `S`
restricted to inputs that respect the freeze, with violations loud.

### 5.4 Reconciliation-point equivalence — monotone between true-ups (relaxes clauses 1+2)

Declare a column group **eventually-exact**: maintained by a cheap monotone
under-approximation (append-only fold, retractions ignored) between periodic reconciliation
runs that restore exact equivalence, with the ledger grading intermediate states
"approximate since T, reconciles at R". The contract becomes: *exact at reconciliation
points, monotone progress between*. Nothing in an IVM engine can express "I accept
approximate counts during the day, true them up nightly" — yet that is a very common real
posture (and today users implement it by hand, invisibly and unverifiably). Risky
territory — approximation must never be silent — but the grading machinery is exactly what
makes it honest.

### 5.5 Equivalence modulo declared indifference (relaxes exactness where the user doesn't care)

Generalise the invariant's `==` to a declared equivalence relation: row order (already
implicit), **tie indifference** (any max-by row on tied ordering keys is acceptable —
today's order-monotone overwrite handles ties by proof; a declared "ties are don't-care"
admits more), floating-point tolerance for re-associated sums (declare ε; unlocks fold
techniques over floats that exact equality refuses). Each widens admission at a point the
user certifies they don't observe. The conformance harness already needs comparison
machinery; "compare modulo declared indifference" is a natural extension.

### 5.6 Per-column-group freshness contracts (relaxes clause 7)

The composed corner already gives different cells different techniques; the natural next
step is different cells having different *declared freshness*: transactional columns tight,
enrichment columns loose. One table, several visible freshness contracts, each cell
scheduled to its own budget. `explain` prints it; consumers can query it (a per-column-group
"fresh through" fact — the per-column settle bound generalised to an SLA).

### 5.7 Declared retraction policy per column group (relaxes clause 1, scoped)

Paimon's per-field `ignore-retract` shows the demand: some columns should absorb
retractions exactly (invertible fold), some should refuse them (frozen), some should ignore
them by declared policy (a "lifetime max" that deliberately never un-sees). Today smelt has
exact-or-refuse; a declared per-group policy with the risk stated in `explain` covers the
middle honestly.

### What makes this a lattice rather than a grab-bag

Every relaxation is (a) *declared*, never inferred; (b) *validated* — smelt proves the SQL
and declarations support the relaxed contract, refusing otherwise; (c) *graded* — the
ledger/explain surface states which contract each region/column group currently meets; and
(d) *composable* — freeze the far past (§5.3), reconcile-point the current day (§5.4), keep
exact equivalence in between. The IVM engine's fixed contract is the lattice's top element;
smelt sells the whole lattice with proofs at every point. This is also the honest answer to
"why not just improve Enzyme": an engine could add any one of these as a flag, but the
validator-over-declared-contracts *posture* — refuse-don't-approximate, grade-don't-hide —
is smelt's architecture, not a feature to bolt on.

## 6. Control and flexibility (relaxes clauses 3/5/6)

Mostly already designed; listed to complete the picture and mark the genuinely new bits.

- **Technique choice is user-visible and pinnable** (`prefer`/`technique`, admission-checked)
  and **measurable** (`smelt bakeoff` — real execution, human-reviewed pins). Enzyme chooses
  from history; smelt lets the operator measure and pin. Already spec'd; a real
  differentiator worth marketing as such.
- **Per-run adaptive selection** (gap catalogue D3) composes with pins: static admission
  fixes the *sound* set; per-run cost picks within it.
- **Operational verbs**: backfill a region without ingesting new data; replay a region;
  rebuild one column group; dry-run a plan. IVM offers "refresh". These fall out of per-cell
  addressing + the ledger; the differentiation is having *verbs* at all (deepened in §10.2).
- **Heterogeneous engine placement** (the user's Athena-vs-Photon example): a backfill
  tolerates latency → cheapest scan pricing; the daily run wants latency → premium engine.
  Because state is ordinary tables and exchange is Parquet, the *same cell* can run on
  different engines for different triggers. Also: spot/preemptible tolerance falls out of
  idempotent, ledger-recorded runs — a killed backfill re-runs; killed streaming operator
  state does not. New surface needed: per-trigger (not just per-model) engine placement in
  the plan.
- **No query-class cliffs** (relaxes clause 4): admission is per *cell*, so an unmaintainable
  corner degrades that cell to recompute while sibling cells still fold — and the whole
  model can always fall back to full refresh with identical semantics, because the logical
  SQL never contained maintenance logic. In-engine IVM rejects the view; smelt's floor is
  "correct but slower", refusal reserved for contract violations.
- **Portability**: the maintenance plan is derived from SQL + declarations, so it survives a
  backend migration; an IVM view's incremental behaviour is engine property. (dbt has weak
  portability of *declared* strategies; smelt ports the *derivation*.)

## 7. Transparency and verifiability

The user suspects this is "just the smarter category" — partly, but one piece is structural:

- **The contract is user-checkable.** Because equivalence is against the model's own SQL
  over a recorded `S`, a user can *run the oracle*: `smelt verify <model>` could full-refresh
  into scratch and diff against maintained state (sampled or region-scoped), exactly what
  the CI conformance gate does internally. No IVM engine invites you to audit it — and none
  *could* offer region-scoped audit, lacking the ledger's notion of covered inputs.
- **Reproducibility**: `S` is recorded, so "rebuild the table as it stood when it had
  processed exactly S" is expressible — debugging and audit gold that opaque operator state
  cannot offer.
- **Refusal with reasons + declared-constraint validation**: admission verdicts, clamp
  derivations, and grading are printable (`explain`) and assertable (grain assertions;
  future: declared FDs, freshes, freezes). Transparency is also the *enabler* of §5: you can
  only sell relaxed contracts if you can show which contract currently holds where.
- **Data-quality gating at the boundary**: because maintenance is orchestrated, a delta that
  fails a declared expectation can be quarantined *before* it enters `S` — the region stays
  graded "held", consumers see the old exact state. An engine applies whatever committed.

## 8. Whole-project compilation — the view is not the unit

IVM maintains a view (or a view DAG inside one engine). smelt compiles a project:

- **Shared delta scans / fused repairs**: two consumers of one source's delta share the
  scan; adjacent cells' statements fuse where the planner proves it. Per-query optimisers
  can't see across views.
- **Planner-inserted helper state as visible models**: F-IVM/DBToaster view trees (gap
  catalogue A4) land in smelt as *materialized helper models* — inspectable, costed,
  droppable — not hidden operator state.
- **Bounded deltas across full-refresh boundaries** (gap catalogue D5/C1): an upstream full
  rebuild with row identity emits a diffed, bounded downstream delta; the graph layer owns
  this, and no single-engine view stack has a graph layer that spans materialization
  strategies (views, incremental, full-refresh, external engine MVs) uniformly.
- **Environments**: dev/staging state layered against prod (virtual environments research)
  composes with the ledger — "what would this definition change repair?" answered by plan
  diff before any compute is spent.
- **Delegation, not competition**: where an engine MV genuinely wins (Feldera-class,
  Snowflake DTs for simple shapes), `refresh: materialized_view` delegates — smelt's graph
  layer still owns propagation around it. Beating engines at their own game is an anti-goal;
  surrounding them is the game.

## 9. Extensibility economics — the fork/extend/contribute pitch

A differentiation axis orthogonal to everything above: **who can add a missing capability,
and what do they risk doing so.**

### 9.1 The cost asymmetry

A native IVM engine is a runtime: storage formats, operator state, vectorised execution,
transaction coordination. smelt is a compiler that emits SQL and orchestrates it; execution,
storage, and transactions are the backend's problem. Two consequences:

- **smelt itself is cheap to build relative to its claims** — the investment is in analysis
  (parser, type inference, the property walk, admission) rather than in a runtime. The
  analysis layer is affordable *because of the differential-testing posture*: the parser,
  type oracle, and maintenance plans are all checked against a real engine, so correctness
  is purchased by tests against DuckDB rather than by runtime engineering.
- **A niche capability is a compile-time change.** Adding a write pattern, a source-posture
  classifier, or a scheduling policy touches plan derivation and statement emission — not a
  storage engine. The blast radius of a mistake is a wrong *statement*, which the
  conformance harness is specifically built to catch.

For a user on a vendor platform the asymmetry is absolute: if Databricks/Snowflake lacks
the maintenance behaviour your shape needs, there is no practical path to running your
extension in your prod environment — you file a ticket and wait. Even the open-source
engines (Materialize, Feldera, RisingWave, pg_ivm) require you to *operate a forked
runtime* — a standing operational risk. Forking or extending smelt means carrying a patch
to a compiler whose output you can read (`explain`, emitted SQL) and whose correctness you
can test against your own data. The risk you bear is bounded to the change you made.

### 9.2 The oracle is what makes third-party extension safe

This is the load-bearing link to §1's thesis. The reason vendors can't accept your
maintenance extension is not just process — it's that their correctness argument is
internal and holistic; your patch endangers everyone's views. smelt's correctness argument
is *external and per-model*: `incremental_state(S) == full_refresh(inputs ∈ S)`, checked
generatively against a real backend (the maintenance-conformance gate). An extension that
plugs into the registry inherits the oracle: write the pattern, declare its obligations,
and the same harness that guards core techniques guards yours. "Extensions are testable
against the invariant" is a property no IVM engine can offer, and it is what turns
fork/extend from a liability into a supported posture. (Precedent for the moat: much of
dbt's durable value was its package/macro ecosystem — an extension surface — despite macros
being untyped strings with no correctness story. smelt can offer the ecosystem *with* the
correctness story.)

### 9.3 The planner-rule promise, made concrete

"User-extensible planner rules" (the standing Python-support plan) is currently a promise,
not a truth. If §1's framing is right, the promise should be scoped by **trust tier** —
what a rule can break determines what discipline it needs:

- **Tier 0 — preference.** Choose among techniques core has already admitted as equivalent:
  cost policies, per-run adaptive selection, pins, scheduling/deferral policies, engine
  placement. *Cannot break correctness by construction* (the interchangeability rule is the
  licence). This is where Python rules should land first — it is the safe 80% of the
  practical demand (§9.4), and the API is pleasant: a function from plan + observed stats
  to choices.
- **Tier 1 — registered patterns.** New write patterns / technique realisations that
  declare their required contract facts and equivalence obligations; **core discharges the
  obligations** (admission stays core-owned) and the conformance harness exercises the
  pattern. The registry's "open, partly backend-provided" design is exactly this shape
  already; the extension work is making registration external.
- **Tier 2 — declared truths.** User-supplied world-facts about niche sources (a vendor
  CDC's delete semantics, a Kafka compacted topic's posture, "Fivetran soft-deletes set
  `_deleted` and never physically delete") plus their audit probes. Core trusts, probes,
  and fails loudly — the §4.1 triple, with the declaration itself extensible.
- **Tier 3 — new lattice points.** Extensions to what "correct" *means* (new relaxations,
  new equivalence relations). See §9.4 — probably not an open API; the disciplined form of
  Tier 3 is a whole *kind* (§2).

A rule at any tier never gets to say "trust me" invisibly: whatever it chose or declared
prints in `explain`, and anything above Tier 0 carries obligations core checks or probes.

### 9.4 Can core cover everything practical? (the open question, assessed)

The honest answer is a split, and the split follows the tiers:

- **The invariant, admission, grading, and the relaxation lattice's *primitives* must be
  core.** A user-defined relaxation with a subtle unsoundness would be indistinguishable
  from a smelt bug, and grading is only trustworthy if one authority owns it. But the
  lattice primitives look *composable and parameterisable*: freeze(horizon), defer(policy),
  reconcile-at(points), modulo(relation), per-group-freshness(budget). The bet worth
  examining: a small closed algebra of relaxation primitives, user-*composed* per project,
  covers the practical space — users pick and parameterise lattice points; they don't
  define new ones. If a real user need falsifies this (a relaxation not expressible in the
  algebra), that's a core contribution, and §9.1 says contributing is cheap.
- **The long tail is real, and it is Tier 0–2 shaped.** Where users will genuinely diverge:
  vendor/source idiosyncrasies (CDC dialects, soft-delete conventions, dedup contracts) —
  Tier 2; org-specific physical write conventions and table-format tricks — Tier 1;
  org-specific cost models, schedules, engine-placement and deferral policies — Tier 0.
  None of these require touching what "correct" means, which is why the extension promise
  is plausible at all.
- **Pattern recognizers (succession, top-N, outer-join clean-up) sit on the boundary.**
  They need the property-walk vocabulary and produce admission-relevant verdicts, so they
  are core-shaped today; a plausible end state is recognizers as Tier-1-style plugins over
  a *stable walk vocabulary*, once that vocabulary stops moving. Not worth externalising
  before then.

So: core covers the *meaning* of maintenance; extensions cover the *matter* — which
patterns, which truths, which preferences. The practical risk to the pitch is not soundness
but **API stability**: an extension surface over a plan model that is still being redesigned
would burn early adopters. That argues for sequencing Tier 0 first (smallest stable
surface: choose-among-admitted), Tier 2 second (declarations are already YAML-shaped), and
Tier 1 only once the registry's obligation vocabulary has survived a few internal pattern
additions (succession, C1, B3 as the shakedown cruise).

## 10. Two more product cuts: the property layer and the manipulation layer

Both of smelt's internal layers could be *surfaces*, not just machinery. Neither has an IVM
analogue, and they compose: the properties are what make the manipulations safe to expose.

### 10.1 Provable properties as a product in their own right

Today the property walk (grain, determinism, combiner algebra, bounded reach, partition
alignment, FDs, event-time monotonicity) exists to feed admission. But the verdicts are
valuable independent of maintenance:

- **Queryable contract facts.** "What can be relied on about this model's output?" —
  key-grain proof, determinism, partition alignment — answered from the SQL, not from a
  hand-maintained YAML contract that drifts (the derive-don't-declare posture applied to
  data contracts). Consumers, tests, and BI layers can read them.
- **Refactor safety as a diff.** The highest-leverage form: diff the derived
  properties/plan between two versions of a model. "This change loses the grain proof /
  flips column group G's sensitivity to `customers` / widens the horizon from 2d to ∞ —
  and will therefore trigger a backfill of G" *before* anything runs. This is a code-review
  artifact no engine or dbt can produce, and it operationalises the spec's declaration law
  (silent contract changes become visible plan diffs). CI-able: fail the PR if a declared
  property is lost.
- **Extensible properties.** The tier model (§9.3) extends here: org-specific properties
  ("this column is PII-derived", "this output is idempotent-consumable") as declared facts
  with probes (Tier 2), and eventually custom leaf classifiers over the stable walk
  vocabulary (Tier 1-shaped). The obligations of registered patterns are already *stated
  in* property vocabulary — exposing the vocabulary is a precondition for §9's external
  registration anyway, so the product cut and the extensibility roadmap share one
  investment.

### 10.2 The manipulation layer — verbs over cells, not models

The plan's cell decomposition (column group × trigger × input × region) is today an
internal addressing scheme. Exposed, it becomes an operator algebra IVM structurally cannot
offer, because engines have neither the decomposition nor a coverage ledger to grade the
result:

- **Input-scoped runs**: "update this model from `orders` only" — run only the cells whose
  changed-input is `orders`; the dimension-mutation cells stay pending and graded.
- **Column-scoped backfills**: "backfill just the new enrichment column" — the
  definition-change trigger scoped to one column group, no touch on sibling groups.
- **Region-scoped verbs**: backfill/replay/verify a partition range; freeze/thaw a region.
- **Trigger-scoped policy**: run creation cells hourly, hold mutation cells for the nightly
  window (the §5.2 deferral, expressed as a verb rather than a declaration).
- **Composed selectors**: `--cells 'input=customers,columns=tier_*,window=2026-Q1'` — the
  cell tuple is the selector grammar.

Soundness is the ledger's job and is already designed: every partial manipulation leaves
per-region/per-group coverage facts, so "what contract does the table meet right now"
remains answerable after any sequence of scoped verbs, and equivalence-at-`S` still holds
for the `S` actually covered. dbt's `--select` picks *models*; smelt's unit is the *cell* —
that granularity difference is the whole feature.

### 10.3 Why the two cuts are one story

A manipulation is admissible only where a property licenses it (column-scoped backfill
needs the column-group factoring proof; input-scoped runs need per-edge dirt; region verbs
need the clock). So the property layer is the *type system* of the manipulation layer:
verbs are total over cells the proofs admit and refused elsewhere, with the refusal naming
the missing property. This also closes the loop with §5: a relaxation declaration is the
*standing* form (policy) of what a manipulation verb does *once* (operation) — same
lattice, two tenses. And it sharpens the pitch of §1: IVM sells one verb ("refresh") under
one contract; smelt sells a typed verb algebra whose safety is proven per cell and whose
aftermath is graded.

## 11. Ranked candidates (practical value ÷ new machinery), for discussion

**Tier 1 — high value, mostly existing machinery:**

1. **Frozen horizons (§5.3)** — one declaration; deletes the biggest silent liability;
   ledger + refusal machinery exists. Also the cleanest *story* of the lattice thesis.
2. **Declared intra-source FDs / partition-expression truth (§4.2, first two bullets)** —
   directly serves scan/window constraint (the user's date/timestamp case); admission +
   clamp machinery exists; needs declaration surface + audit probes.
3. **`smelt verify` as a user-facing oracle (§7)** — the conformance harness productised;
   turns the invariant from a promise into a demo. Cheap, high trust value.
4. **Plan/property diff in CI (§10.1)** — near-term, high leverage; machinery mostly
   exists (`explain` twice + diff); the marketable form of the property cut.
5. **Work subsumption in the graph layer (§4.3)** — coalesce pending obligations across
   triggers before emitting statements; pure planning, no new contract.

**Tier 2 — high value, real new surface:**

6. **Deferral windows / per-column-group freshness (§5.2, §5.6)** — the scheduling policy
   axis; needs a declaration grammar and graph-layer scheduling, but the grading machinery
   is the hard part and exists.
7. **Tier-0 Python planner rules (§9.3)** — choose-among-admitted policies; the
   admitted-set + cost hooks exist, and the story ("your niche requirement is a policy
   file, not a vendor ticket") is immediately marketable.
8. **Cell-selector surface for run/backfill (§10.2)** — the manipulation layer's first
   tranche; wants the ledger grading fully landed first.
9. **Per-trigger engine placement (§6)** — backfill-on-cheap-engine; needs multi-backend
   maturity but no new theory.
10. **Equivalence modulo declared indifference (§5.5)** — starts as comparison machinery in
    the conformance harness (ties, float ε), graduates to admission widening.

**Tier 3 — valuable but contract-risky, demand-gated, or stability-gated:**

11. **Reconciliation-point equivalence (§5.4)** — biggest expressiveness win, biggest risk
    of blessing silent approximation; only with grading fully user-visible.
12. **External Tier-2 declaration surface (§9.3)** — gated on the `sources.md`
    declared-relationship family (§4.2) landing first.
13. **Demand-driven maintenance (§4.4)** — wants consumption metadata smelt doesn't
    collect yet.
14. **Cross-source alignment declarations (§4.2, third bullet)** — real horizon wins;
    subtle audit story.
15. **Queryable property/contract facts for consumers (§10.1)** — after the property
    vocabulary stabilises.
16. **External Tier-1 pattern registration (§9.3)** — deliberately last, after the
    obligation vocabulary survives internal shakedown (succession, C1, B3).
17. **Intent-node authoring surfaces (§2.4)** — an SCD2 or windowed-aggregation
    declaration as the pilot, generated-denotation rule from day one; gated on kernel
    stability, but the pilot doubles as the succession classifier's greenfield twin
    (same registry cells, opposite direction) and would settle the escape-slot design
    early.

**Standing lens, not a build item:** the kernel/kind factoring (§2) is the long-game
architecture — it should influence boundary decisions *now* (every invariant kept pure is
kernel surface bought for free), with externalisation sequenced per §2.7.

**Anti-goals, restated:** no Z-set runtime, no per-tuple streaming ambitions, no competing
with engine MVs on continuous freshness — delegate there. The gap catalogue's top-ranked
mechanisms (A1, C1, B3) remain worth adopting, but as parity, not as the pitch.

## 12. Implications for the spec (if this framing survives discussion)

Not spec edits yet; where they would land:

- The **Overview's "one guarantee"** stays the top of the lattice, but the spec could name
  the lattice: declared relaxations as first-class, each with validation + grading. Today's
  spec already contains proto-relaxations (order freedom, per-cell freshness as the "only
  degree of freedom", deferral implicit in the ledger) — the reframe makes them one family.
- **`sources.md`** grows the declared-relationship family (§4.2) alongside the existing
  world-facts, with the declaration → derived clamp → audit probe triple as its template.
- A future **`maintenance_scheduling.md`** (or graph-layer section growth) owns deferral,
  subsumption, freshness budgets, and engine placement — none of which are per-model facts.
- The **conformance harness** grows toward user-facing `smelt verify` and
  comparison-modulo-indifference.
- **Intent nodes (§2.4)** would eventually touch `models.md`'s declaration law with one
  new rule — *every node has a denotation; a non-SQL node's denotation is generated, never
  hand-authored* — and the first intent surface (SCD2 or windowed aggregation) would be
  its own spec file, written against that rule.
- The **kernel/kind factoring (§2)** implies no spec change yet, but boundary decisions in
  `architecture.md` (which invariants are CI-gated, where the plan/emission/walk cuts sit)
  should be reviewed with "is this kernel surface?" as an explicit question, and the next
  quasi-kinds (`materialized_view` delegation, the succession classifier) written against
  the boundary deliberately.

## References

- `docs/specs/incremental_models.md` — the invariant, plan, admission, ledger, graph layer.
- `docs/research/20260724-ivm-pattern-gap-catalogue.md` — mechanism parity survey (the
  complement to this note); production-engine citations for Enzyme, Snowflake, Oracle,
  Materialize, Napa/Mesa, Paimon/Hudi behaviours are collected there.
- `docs/research/20260703-model-updates.md` — rejection catalogue (Part 12).
- `docs/research/20260601-virtual-environments.md` — environments/state layering.
- DBSP (VLDB J. '25) — why delta derivation is commodity theory.
- Oracle dimension declarations / query-rewrite constraints — closest prior art for
  user-declared semantic facts feeding a rewrite/maintenance decision (§4.2).
