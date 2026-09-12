---
feature: state
status: experimental
last_reviewed: 2026-09-06
owners: [andrew]
---

# State Ownership

> **What this is.** A normative spec for **what state smelt keeps, where each piece is allowed
> to live, and what happens when it is absent**: the inventory of state structures, the two
> residency classes (engine-resident correctness state vs project-local observability state),
> the optionality rule, and the late-resolved degradation contract for projects that run
> without a state store. Out of scope: the `.smelt/` directory layout, manifest formats, and
> serialisation (see `run_state.md`); frontier *semantics* — fold, recompute-reset, grading
> (see `incremental_models.md` §"The frontier"); the `state.mode` configuration key's syntax
> and the environments feature it enables (see `virtual_environments.md`). Those specs own
> their mechanisms; this spec owns the doctrine that classifies them.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is
> the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed.
> Implementation status lives in §Known Divergences with code/research links.

## Overview

smelt is a compiler and orchestrator, not a database — yet several features need memory
between runs: an incremental fold must know which deltas it already absorbed, `--resume` must
know what the last run did, forward propagation must know what landed upstream. This spec
answers one question for every such memory: **who is trusted to keep it, and what does smelt
do when it isn't there?** Other specs that touch state cite §"The residency rule",
§"The degradation contract" or §Diagnostics by name rather than restating the residency class
of a structure or the downgrade rule; this spec is the sole normative statement of both.

The central rule is a two-class split:

- **Correctness state** is state whose loss or staleness could make a maintained table
  *wrong* (a double-counted fold, a replayed merge). It must live **in the data engine,
  in the same backend transaction as the write it describes**. Because it travels with the
  data, it exists whenever the derived plan needs it — it is not part of any opt-in store, and
  the one configuration that touches it (`state.warehouse_tables: none`, below) removes the
  techniques that need it rather than ever running a technique without its bookkeeping.
- **Observability state** is everything else: run history, interval coverage, landed-delta
  records, schema snapshots, environment maps. It lives in the project-local `.smelt/` store,
  is opt-in via `state.mode`, and is always safe to delete — its absence degrades what smelt
  can *tell you* or how *cheaply* it can run, never what the tables *equal*.

The second rule is the **degradation contract**: when a capability needs a state structure
that is unavailable (the posture excludes it, or the backend has no realisation of it), the
plan **downgrades late** — the ideal plan is derived first, then the unavailable-state
downgrade is applied as a recorded, explain-visible step — rather than refusing or silently
changing shape. Declarations stay fail-loud: frontmatter describes what the SQL yields, and a
declaration the SQL supports is never rejected for lack of state; only a declaration whose
*semantics themselves* require state (a `contract.deferral` lag budget) fails loudly under a
posture that cannot supply it.

The degradation contract covers two distinct losses, and conflating them is what produces a
refusal where a downgrade belongs:

- **Losing a technique.** The structure a technique *needs to be correct* is unavailable, so
  the cell downgrades to its recompute-family equivalent and records
  `MaintenanceStateDowngraded`. The maintained table still equals what a full refresh would
  produce; only the cost changes.
- **Losing precision.** The structure carries no correctness obligation of its own but lets a
  downstream consumer narrow its work — the observed output delta is the instance. Here there
  is no cheaper technique to fall back to and nothing to swap: the write proceeds, the record
  is skipped, and the consumer takes its already-defined widen-never-narrow path. A run must
  never refuse for this class of loss. It must also not be silent about it: a skipped record is
  reported as a run-time warning naming the model, the dialect, and the structure, so an operator
  sees the precision the run gave up without raising the log level. This is the one asymmetry
  between the two classes' recording — a lost *technique* is a plan-time fact and reaches
  `MaintenanceStateDowngraded` and `smelt explain`, while a lost *record* is a per-write fact with
  no plan-level cell to hang on (see §Known Divergences).

## Surface

### The state-structure inventory

Every persistent structure smelt reads or writes is classified here. Adding a new structure
without classifying it in this table is a spec violation. "Owner" is the spec that defines the
structure's format and semantics; this table owns only its class.

| Structure | Class | Residency | Owner (format/semantics) |
|---|---|---|---|
| Transactional merge ledger | correctness | backend table, transactional with `merge_into` | `incremental_shapes.md` §"The transactional frontier write (merge ledger)" |
| Reconciliation ledger (frontier record) | correctness | backend table, transactional with the fold | `incremental_models.md` §"The frontier record (reconciliation ledger)" |
| Observed output deltas | correctness | backend table, transactional with the conditional write | `incremental_models.md` §"The graph layer" |
| Fingerprint sidecar | correctness | backend table (digest refresh in the maintenance run) | `sources.md` §"The fingerprint sidecar" |
| Tombstone ledger (succession grain) | correctness | per-model sibling table `<presented table>__tombstones`, transactional with the succession-patch `MERGE` | `incremental_shapes.md` §"The tombstone ledger (hidden state)" |
| Run manifests + run reports | observability | `.smelt/targets/<t>/runs/`, `reports/` | `run_state.md` |
| Interval ledger | observability | `.smelt/targets/<t>/intervals.json` | `run_state.md` |
| Landed-delta record | observability | `.smelt/targets/<t>/landed_deltas.json` | `run_state.md` |
| Deployed-schema snapshots | observability | `.smelt/targets/<t>/schemas/` | `run_state.md`, `schema_evolution.md` |
| Snapshot / environment store | observability | `.smelt/targets/<t>/snapshots.json` | `run_state.md`, `virtual_environments.md` |
| Source postures | observability | `.smelt/` (per target) | `sources.md` |
| Probe baselines (frozen-band) | observability | `.smelt/` (per target) | `incremental_models.md` §"The contract lattice" |
| Source-mutation baselines | observability | `.smelt/targets/<t>/source_mutations.json` | `sources.md` |
| Migration approvals | observability | `.smelt/targets/<t>/migration_approvals.json` | `definition_deltas.md` |

The class assignment is itself normative: a structure listed as correctness may never be
realised only in `.smelt/`, and a structure listed as observability may never become a
correctness dependency without moving classes here first.

#### Which dialects realise which structure

A structure is **realisable** on a dialect when that dialect has emitters for it and a
backend seam to run them through. Realisability is per-dialect data, not a property of the
structure:

| Structure | DuckDB | BigQuery | Spark (Delta) |
|---|---|---|---|
| Transactional merge ledger | yes | yes | **no** |
| Reconciliation ledger (frontier record) | yes | yes | **no** |
| Observed output deltas | yes | yes | **no** |
| Fingerprint sidecar | yes | not yet | **no** |
| Tombstone ledger (succession grain) | yes | yes | **no** |

"not yet" is pending work; "**no**" is a permanent, reasoned absence. Spark's is the
latter: Delta provides per-table atomicity and no cross-table transaction, so a ledger
write and its data write cannot be made atomic, and the additive fold's never-fold-twice
refusal has no sound realisation there.

Four facts of BigQuery's **ledger** realisation are load-bearing rather than incidental:

- **The ledger table is addressed by a two-part name.** The ledger lives beside the models
  it records, in the run's own schema, and is named `` `<schema>._smelt_ledger` `` — one
  backticked path, resolving against the job's default project, the same shape every other
  GoogleSQL object this tool emits uses. A schema that already carries a project prefix
  works unchanged.
- **Its `PRIMARY KEY` is declared `NOT ENFORCED`.** GoogleSQL requires the suffix, and the
  declaration is documentation and an optimiser hint — never a constraint. Nothing on
  BigQuery may rely on the key to refuse a duplicate.
- **The re-run-tolerant record is a `MERGE … WHEN NOT MATCHED`.** GoogleSQL has no
  `ON CONFLICT DO NOTHING`, so the idempotent bookkeeping upsert is expressed as a merge
  against a one-row inline source. The statement is a no-op when the window is already
  recorded, which is the same observable behaviour the conflict clause gives on DuckDB.
- **The never-fold-twice refusal is a zero-row abort, not a key violation.** The second
  fact rules out DuckDB's mechanism entirely: there the refusal *is* the `PRIMARY KEY`
  violation, and an unenforced key raises nothing. On BigQuery the additive fold's ledger
  record is the same conditional `MERGE`, and the refusal is its effect — the record, an
  `IF @@row_count = 0 THEN RAISE`, and the fold action run in one multi-statement
  transaction, so a repeat aborts before the action is reached and the transaction rolls
  back. The guarantee is one guarantee with two realisations, not two guarantees.

  This rests on a documented BigQuery property stronger than the snapshot isolation it is
  usually stated alongside: two transactions that mutate rows in the same table cannot run
  concurrently, and a conflicting transaction is cancelled. Both folds of the same delta
  mutate the ledger table, so they cannot both commit — one wins, the other is cancelled
  loudly, and a later re-run reads the committed row and refuses. Were that property to
  weaken, the correct response is to withdraw the realisation, not to fall back on a
  check-then-act probe.

Three facts of BigQuery's **observed-output-delta** realisation are load-bearing in the same
way:

- **The idempotent replace is a `MERGE`, not a conflict clause.** GoogleSQL has no
  `ON CONFLICT … DO UPDATE`, so re-recording a window is `MERGE … WHEN MATCHED THEN UPDATE
  … WHEN NOT MATCHED THEN INSERT` against the same three-column window key — the same
  observable behaviour as DuckDB's conflict clause, and it does not lean on the unenforced
  `PRIMARY KEY`.
- **The delta's key set is `ARRAY_AGG(DISTINCT CAST(… AS STRING) IGNORE NULLS)`.** Three
  separate GoogleSQL facts collapse into that one expression, and each is a correctness
  requirement rather than a spelling: there is no `FILTER (WHERE …)` clause; `ARRAY_AGG`
  **raises** on a NULL element rather than yielding a NULL array, so `IGNORE NULLS` is what
  keeps an unmatched key from failing the run; and array element types are not coerced on
  write, so a non-string key — or the literal NULL a model with no partition axis projects
  as its partition — must be cast before it can enter an `ARRAY<STRING>` column. The
  `COALESCE` to an empty typed array is kept for the same reason it exists on DuckDB:
  `ARRAY_AGG` over zero rows is `NULL`, and folding that to `[]` is what makes a
  fully-suppressed run record present-and-empty.
- **Empty-versus-absent rests on row presence, never on a column value.** BigQuery cannot
  represent a NULL array at all — a NULL written to an `ARRAY` column reads back as empty —
  so a realisation that encoded "never recorded" in a column would lose the distinction the
  moment it crossed the wire. It does not, and this is a property of the guarantee rather
  than an implementation detail: *absent* means **no row for the window**
  (`incremental_models.md` §"The graph layer" — "Empty and absent are distinct"), the upsert
  always writes exactly one row per recorded window, and the read filters on the window key
  alone. Array flattening can neither manufacture nor destroy a row. For the same reason the
  two array columns are declared without `NOT NULL`: nothing depends on it.

**One shape BigQuery degrades rather than binds atomically.** The seam that records
bookkeeping in the same transaction as its write cannot hold a write group that creates a
permanent entity, because GoogleSQL does not permit that DDL inside a transaction — and a
maintained model's *first* run writes `CREATE TABLE … AS` rather than a merge. There the
statements run unbound: the write first, the bookkeeping record after, and the lost
atomicity is reported. The ordering is what makes the degradation admissible. A record that
reads the target's pre-write state has nothing to read when the write is what creates the
target, so running it second loses nothing; and the surviving exposure — a created table
whose window went unrecorded — costs a redundant re-run, whereas the reverse (a record
claiming a write that never happened) could mislead a later run. This is a degradation of
*atomicity only*: what the cell computes is unchanged.

Four facts of BigQuery's **tombstone-ledger** realisation are load-bearing, and unlike the
two above they are not confined to a bookkeeping module. The tombstone ledger's table is
bookkeeping, but every statement it participates in — the idempotent tombstone insert, the
presented `MERGE`, the rebuild's truncate-and-refill pair, the clock-tie probe — is a
*maintenance* statement, so its dialect plurality lives in the maintenance emitter that
single-owns it rather than in a per-dialect DDL module.

- **The touched-key scoping is a correlated `EXISTS`, not a row-constructor `IN`.**
  GoogleSQL has no row constructor: `(a, b)` is a parenthesised expression there, not a
  tuple, so a multi-column `IN` subquery is a syntax error. The neighbour domain's
  restriction to the keys the batch touches is spelled as a correlated `EXISTS` over an
  aliased relation instead. This is a correctness requirement, not a preference: the
  single-key case would have parsed and every multi-key model would have failed at the
  warehouse.
- **The dedup relation is an explicit `ROW_NUMBER() … WHERE rn = 1`, not `QUALIFY`, and it
  is nested derived tables rather than a `WITH` inside the `MERGE`'s `USING`.** Both of the
  constructs it avoids exist in GoogleSQL, and neither's acceptance in this exact position
  can be established without a warehouse. The nested-derived-table form denotes the same
  relation and is accepted everywhere, so it is what BigQuery gets — the same shape the
  full-rebuild fold already uses.
- **Truncating the ledger says `WHERE TRUE`.** GoogleSQL rejects a `DELETE` with no `WHERE`
  clause. The rebuild's ledger truncation carries the clause GoogleSQL requires; DuckDB's
  keeps the bare form.
- **The ledger table's columns are the model's own inferred types, rendered in GoogleSQL.**
  A type with no GoogleSQL spelling is refused with the column and the reason named, never
  substituted — a substituted type is a column the next write cannot fill. The declared
  `PRIMARY KEY (k…, t)` says `NOT ENFORCED` and, as everywhere else on BigQuery, refuses
  nothing: the tombstone insert's idempotence is its own anti-join, which never depended on
  an enforced key in either dialect.

**One shape the succession rebuild degrades on BigQuery.** The `--full-refresh`/`smelt
rebuild` group re-derives the presented table and the ledger in one transaction on DuckDB.
It opens with a `CREATE TABLE … AS`, so on BigQuery the same permanent-entity-DDL rule
applies and the three statements run as separate jobs. This is admissible here in a way it
is not for an additive fold, and for a reason specific to the statement: the rebuild is a
pure function of the whole retained source, so a partially-applied rebuild is repaired by
re-running it, and no bookkeeping row can outlive a write that never happened. It is a
degradation of *atomicity only* — a rebuild interrupted midway leaves a presented table and
a ledger that disagree until the next rebuild, and never a ledger that claims a fold that
did not occur.

**One shape BigQuery refuses rather than realises.** BigQuery does not permit DDL creating
or dropping permanent entities inside a transaction, and an additive fold's *first* action
against a not-yet-existing target is a `CREATE TABLE … AS`. There is no safe degradation —
committing the ledger record and the create separately would let a crash between them leave
the ledger claiming a fold that never happened — so that first step is refused, naming the
construct and the remedy: run once with `--full-refresh` to materialise the target, after
which every step is a merge the transaction can hold. The condition is the backend's
transactional-DDL capability, not its dialect.

**Concurrency is the price of the same isolation rule.** BigQuery's protection against a
double fold is write-conflict detection: a transaction that mutates a table another
in-flight transaction is also mutating is *cancelled*. Every maintained model's bookkeeping
transaction mutates the one ledger table, so the guarantee that makes a repeat fold
impossible also makes two of a run's own models' bookkeeping writes mutually exclusive.
Two rules follow, and neither weakens the isolation:

- **One run serialises its own ledger transactions.** A backend whose ledger writes are
  transactional and single-table opens at most one such transaction at a time, so a
  parallel run (the default `--jobs` is the host's core count) does not race itself. The
  cost is that maintained models' bookkeeping-bound writes run one at a time on that
  backend; the alternative is losing models at random to cancellation.
- **A conflict from anywhere else is transient, and retried.** A cancellation raised by a
  writer this run does not control — a second `smelt` process, an external job — is a
  *transient* backend error, not a deterministic one: the engine rolls the whole
  transaction back before cancelling it, so re-issuing the identical statement group is the
  documented remedy and the ordinary bounded retry performs it. This is the one backend
  error class whose remedy is retry by the engine's own definition.

Two rules bind this table to the implementation, and they are the whole point of stating
it:

1. **A claim implies a builder.** Declaring a structure realisable on a dialect that
   cannot build it is worse than declaring nothing: availability resolution records no
   downgrade for a structure it believes present, so the run reaches the execution path,
   finds no builder, and *refuses* — losing exactly the graceful degradation the
   degradation contract promises.
2. **An absence implies a downgrade, never a refusal.** Where a structure is unrealisable,
   the execution path degrades and records the degradation. A technique needing that
   structure downgrades to its recompute equivalent (below). A structure that only carries
   *precision* — the observed output delta is the instance — is simply not recorded: the
   write still happens, and consumers fall back to their widen-never-narrow path, because
   an absent delta is already defined as a legal fallback trigger
   (`incremental_models.md` §"The graph layer" — "Empty and absent are distinct"). Costing
   a downstream recompute its narrowness is a permitted degradation; refusing the run is
   not.

### `state.mode` and what each posture provides

The key's syntax, the capability lattice (`environments ⊇ intervals ⊇ stateless`), and
model-level narrowing are owned by `virtual_environments.md` §"`state.mode`". This spec owns
the consequence table:

| Posture | Observability structures written | Correctness structures |
|---|---|---|
| `stateless` (default) | none — `.smelt/` need not exist | all, whenever the plan derives them |
| `intervals` | manifests, reports, interval ledger, landed deltas, schema snapshots, source postures, probe baselines, source-mutation baselines, migration approvals | all |
| `environments` | everything in `intervals` plus the snapshot/environment store | all |

Correctness structures are identical in every row: they are a property of the *plan*, not of
the posture. The only surface that touches them is the warehouse-tables opt-out below — an
orthogonal key, not a `state.mode` value.

### Opting out of warehouse bookkeeping (`state.warehouse_tables`)

Some organisations forbid tool-authored objects in the target schema. A project declares that
constraint with a sibling of `state.mode` in `smelt.yml`:

```yaml
state:
  warehouse_tables: allowed   # default — engine-resident correctness structures are created
                              # as the derived plan needs them
  # warehouse_tables: none    # smelt authors no tables of its own in the target backend
```

Under `warehouse_tables: none`, every engine-resident correctness structure is treated as
**unavailable** during availability resolution (§"The degradation contract"): each cell whose
technique requires one downgrades to its recompute-family equivalent, recorded and printed as
`MaintenanceStateDowngraded` like any other availability downgrade, and a declaration whose
semantics require such a structure refuses with `DeclaredContractRequiresState`. The knob is
project-wide and binary — there is deliberately no per-table or per-model granularity — and it
never changes what any maintained table equals, only what it costs to maintain.

### Diagnostics

| Code | When it fires |
|---|---|
| `MaintenanceStateDowngraded` | Advisory, plan derivation: a cell's derived technique requires a state structure with no available realisation on the target backend, and the cell was downgraded to its recompute-family equivalent. Names the cell, the original technique, the missing structure, and the reason (§"The degradation contract"). Printed by `smelt explain`; surfaced as a warning-level diagnostic, never an error. |
| `DeclaredContractRequiresState` | Validation, fail-loud: a declared contract point whose semantics require a state structure (e.g. `contract.deferral`'s ledger-measured lag) is declared in a project whose posture, backend, or `state.warehouse_tables: none` opt-out cannot supply it. Names the declaration and the missing structure. |

`smelt explain <model>` prints every downgraded cell with both the executed technique and the
technique that *would* run were the missing structure available — the downgrade is a visible
plan fact, not a silent substitution.

## Semantics

### The residency rule

A **correctness structure** must be resident in the same backend as the data it describes and
written in the same backend transaction as the write it records. A data write that commits
without its bookkeeping, or bookkeeping that commits without its write, must be impossible by
construction — not recovered from after the fact. `.smelt/` must never hold correctness state:
every file under `.smelt/` must be deletable with no effect on what any maintained table
equals after the next run.

Corollary — **the trust boundary**: smelt trusts a correctness structure's content because
smelt is its only writer and it commits atomically with the data. State kept anywhere else
(another database, a project file) cannot carry that trust and therefore cannot be
correctness-bearing.

### The optionality rule

Observability state is opt-in via `state.mode` and its absence must never change what a
maintained table equals. Concretely:

- Under `state.mode: stateless`, smelt writes nothing under `.smelt/` and does not require
  the directory to exist (`run_state.md` §Semantics "Stateless writes nothing").
- Correctness structures are exempt from the posture: a keyed model's merge ledger exists
  under every posture, because it lives in the backend alongside the table it protects.
- A capability that consumes an observability structure the posture excludes must either
  **degrade to a coarser, always-correct behaviour and say so** (forward propagation without
  landed deltas recomputes the full dirty set and reports why) or **refuse loudly by name**
  (`--resume` with no manifest refuses, `run_state.md` §"`--resume` semantics") — never
  silently pretend the state was empty. Which of the two applies is owned by the consuming
  feature's spec; this spec requires that one of them is specified. Under `state.mode:
  stateless`, `--resume` is always in the refuse-by-name case: there is no manifest to resume
  from *by posture*, not by accident, so the error names the posture rather than reading as an
  ordinary "no partially-failed run found".

### The degradation contract

Statefulness is an **admission input resolved late**. Plan derivation proceeds in two steps:

1. **Ideal derivation.** The maintenance plan is derived assuming every classified state
   structure is available: cells get their best technique (an additive keyed fold, a
   ledger-enforced merge) exactly as `incremental_models.md` specifies.
2. **Availability resolution.** Each cell's technique is checked against the structures
   actually realisable for this project: the backend has a builder for the structure, the
   project has not declared `state.warehouse_tables: none` (which makes every engine-resident
   structure unavailable), and — for observability structures only — the posture includes it.
   The required structure is a function of the **cell**, not of its technique alone: a
   `PerGroupRecompute` cell addressed by a key-addressed model edge (`incremental_models.md`
   §"Upstream model edges") requires the **fingerprint sidecar**, because its affected-key
   discovery is a group-grain sidecar diff — a plain, clamp-bounded `PerGroupRecompute` cell
   requires nothing. A cell whose technique
   requires an unavailable structure is **downgraded to the cheapest member of the recompute
   family that preserves the equivalence invariant** (typically per-region or per-key-group
   recompute), and the downgrade is recorded on the cell (`MaintenanceStateDowngraded`). For the
   key-addressed cell this means the whole per-group route is what the missing sidecar denies,
   so the cell downgrades to `DeleteInsert` (full-region recompute), never a no-op "downgrade"
   back to the same `PerGroupRecompute` technique it already carries. No consumer re-derives
   this requirement at run time — it is resolved once, here, and any run-time check for the
   same fact is a defensive guard against inconsistent inputs, not a second source of truth.
   The recompute-family fallback a `key_scope`-carrying cell downgrades to also depends on the
   key scope's own discovery route: `UpstreamKeyed` and `DownstreamGrainOverUpstream` both
   address a real `PerGroupRecompute` cell the key-addressed driver dispatches, but
   `EnrichmentKeyed` (the value-enrichment join shape) addresses only a `ColumnScopedMerge`
   cell — no execution route dispatches an `EnrichmentKeyed` `PerGroupRecompute` cell, so a
   `ColumnScopedMerge` cell of this shape whose merge ledger is unavailable downgrades straight
   to `DeleteInsert`, never through an unexecutable `PerGroupRecompute` intermediate.

The downgrade is sound by construction: every recompute-family technique satisfies the same
equivalence invariant (`incremental_models.md` §"The equivalence invariant"), so availability
resolution changes a cell's *cost*, never its *result*. This is the same shape as SQL-driven
degradation — a delta signature degrading to `general` downgrades the consumer's technique —
and the two must be recorded and printed uniformly.

Resolving late is mandatory, not an implementation choice: the ideal plan must exist as a
derived object even when it will not run, because diagnostics, `smelt explain`, and future
tooling must be able to show what the project *would* get with state — the counterfactual is
part of the product. An implementation that prunes state-requiring techniques during ideal
derivation (early resolution) violates this spec even if it executes identical SQL.

### Declarations stay fail-loud

Frontmatter declares facts about the model's output (`grain`, `timeseries:`, `unique_key:`,
contract points), and validation checks those facts against the **SQL**, exactly as today
(`incremental_models.md` §"Validator, not chooser"). State availability never enters that
check: a declaration the SQL upholds is valid under every posture, and the plan beneath it
degrades per the contract above. The one exception is a declaration whose semantics *are* a
statement about state: `contract.deferral` promises a bounded, ledger-measured lag, which
cannot be measured without the frontier — declaring it where the frontier has no realisation
is `DeclaredContractRequiresState`, a validation error, because silently skipping the
measurement would turn a declared guarantee into an unverified hope.

## Design

**Two classes, not a spectrum.** Every structure is either transaction-coupled to the data or
freely deletable; nothing in between. The rejected middle ground — "important but external"
state, e.g. a required project-local ledger — is exactly the shape that produces
wrong-after-crash tables: external state can always be lost, restored from backup, or edited
independently of the warehouse, so anything correctness-bearing kept there needs a
reconciliation protocol that the transactional design makes unnecessary. This promotes what
was previously an exception (`incremental_shapes.md` §"State ownership" carved out the merge
ledger as "the one deliberate exception") into the rule: the merge ledger is the *model*
citizen, and every correctness structure follows its pattern.

**Comparison with SQLMesh, and why the trust model is inverted.** SQLMesh keeps snapshots,
per-snapshot processed intervals, and environment maps in a mandatory state database
(`state_connection`, with an OLTP engine such as Postgres recommended for production), and
that state is correctness-bearing: the interval records decide what work runs. That is sound
for SQLMesh because its incremental models are interval-idempotent by construction (re-running
an interval overwrites it), so a crash between the data write and the interval record is
recoverable by re-running. smelt's admission space is wider — an additive keyed fold is *not*
re-run tolerant (folding a delta twice double-counts) — so the SQLMesh posture of
correctness-bearing state in a separate, non-transactional store would be unsound here.
Inverting the split keeps the wider admission space safe: what decides correctness commits
with the data; what lives outside the engine is only ever observability. The visible cost of
the inversion is bookkeeping tables in the user's warehouse; the benefit is that a smelt
project needs no state database at all.

**Optional-by-downgrade rather than optional-by-refusal.** A project without a state store
should get *most of smelt*, not a wall of errors — the same principle that admits a model
whose SQL degrades its delta signature. Refusal is reserved for declarations (a human said
something the system cannot honour), because refusing derived behaviour would make the state
store a de-facto requirement, and silently narrowing admission would hide from the user what
turning state on would buy them.

**Late resolution buys the counterfactual.** Resolving availability after ideal derivation
costs one extra pass and keeps two plans conceptually alive, but it is what lets every
diagnostic and `smelt explain` answer "what would change if I configured state / moved to a
backend with a ledger builder?" — turning the degradation contract into an adoption funnel
instead of a cliff. Early pruning was rejected for exactly this reason.

**The warehouse-tables opt-out is an availability input, not a third state class.** Modelling
`state.warehouse_tables: none` as one more input to availability resolution keeps the doctrine
intact: correctness structures remain transaction-coupled wherever they exist, and where the
project forbids them the *techniques* go, never the coupling. Project-wide binary granularity
was chosen over per-table or per-model knobs deliberately — the constraint it models is an
organisational policy about the target schema, not a tuning decision, and per-table opt-outs
would reintroduce exactly the silent, partially-stateful middle ground the two-class split
rejects. The decision record is `docs/research/20260816-open-questions-triage.md` item 11.

**`state.mode` stays where it is.** This spec deliberately does not take over the
`state.mode` key from `virtual_environments.md`: the key's original job (gating environments)
is unchanged, and moving surface ownership while the doctrine is young would churn references
for no behavioural gain. Revisit if a state-store-selection surface (§Future Extensions) ever
lands.

## Constraints & Invariants

- **No correctness state outside the engine.** Deleting `.smelt/` entirely, at any moment,
  must never change what any maintained table equals after subsequent runs. This is the
  doctrine's single most checkable consequence and the natural conformance-gate extension:
  the maintenance conformance oracle remains valid under interleaved `.smelt/` deletion.
- **Transaction coupling.** Every correctness-structure write shares a backend transaction
  with the data write it describes. A correctness write on its own transaction is a bug even
  when "nothing went wrong".
- **Classification before use.** A new persistent structure must be added to §"The
  state-structure inventory" (with a class) before any feature depends on it.
- **Downgrades are recorded, never silent.** Every availability downgrade appears on the
  derived plan and in `smelt explain`; no execution path may substitute a technique without
  the plan recording why.
- **Equivalence is posture-independent.** `incremental_state(S) == full_refresh(inputs ∈ S)`
  holds under every `state.mode` and every downgrade — the degradation contract changes cost
  only. The conformance gate must therefore pass with state structures made unavailable.
- **Maintenance-plan purity extends to availability resolution.** Step 2 of the degradation
  contract is a pure function over (ideal plan × backend capabilities × posture), living in
  `smelt-logical` with the rest of the plan derivation (`architecture.md` §"Constraints &
  Invariants" item 12); consumers never re-derive availability.

## Known Divergences / Open Questions

- **A skipped precision record reaches the operator as a log warning, not as structured run
  state (#202).** §"The degradation contract"'s precision class is reported per occurrence at warn
  level, which is what a live BigQuery run showed was missing entirely. It is not yet carried in
  the run manifest or the run report, and `smelt explain` cannot be asked what a given target
  would give up (`explain` takes no `--target`), so there is no offline way to see the loss
  before a run and no machine-readable record of it after one. Both are the same missing piece:
  a per-model precision-downgrade record derived from the availability layer, which already
  knows the answer statically. Tracked in #202; found by
  `docs/outcomes/20260906-bigquery-dogfood-spine/phases/12-summary.md` finding 4.

Otherwise none open — `state.mode` is honoured by `execute_project`, the reconciliation ledger
is engine-resident, and `state.warehouse_tables` is parsed and feeds availability resolution, as
this spec describes normatively above.

## Future Extensions

- **Pluggable observability store (OLTP backend).** The `.smelt/` JSON store is single-writer
  and machine-local; a team sharing state (concurrent CI runs, shared dev history, a hosted
  UI) would want the *observability* half in a shared OLTP database (Postgres/SQLite), which
  is precisely SQLMesh's `state_connection` shape. Under this doctrine that extension is
  low-risk by construction — the pluggable store could only ever hold observability state, so
  a mis-configured or lost state database degrades capability, never correctness. Open: the
  store trait boundary in `smelt-state`, locking semantics across writers, and whether
  `state.mode` grows a `store:` sibling key. Not now: single-user file state has no felt pain
  yet, and the trait is cheap to extract later.
- **Ledger builders for further backends (Spark first).** The engine-resident ledgers have a
  transactional realisation on DuckDB only; on other backends availability resolution takes
  the recorded downgrade path, which is the intended steady state, not a stopgap. A
  Spark-dialect ledger builder is built when a real Spark-targeted incremental workload
  demands the fold-family techniques the downgrade forgoes — not speculatively before.
  Decision record: `docs/research/20260816-open-questions-triage.md` item 12.
- **Downgrade-forcing conformance leg.** A generative gate variant that forces an availability
  downgrade (`MaintenanceStateDowngraded`) mid-schedule and asserts the equivalence oracle
  still holds across the switch — the state-deletion leg (§References → Tests) proves ledger
  residency; this residual proves the recompute-family fallback itself.
- **Detected-inconsistency healing as an alternative to cross-table atomicity.** Some
  backends will permanently lack a cross-table transaction — Delta's per-table atomicity
  (§"Which dialects realise which structure") is one instance of a property no per-backend
  ledger builder (the extension above) can ever change, and a future backend may share it.
  For those backends the standing degradation is the recompute-family downgrade, chosen for
  safety over throughput. A narrower alternative is possible in principle: keep the
  technique's incremental write, drop its atomicity guarantee, and replace it with detection
  — order the data write before the correctness-structure record, so an interrupted pair
  fails in the safe direction (an unrecorded write costs a redundant re-run, the same
  direction BigQuery's create-table degradation already accepts above, rather than a record
  claiming a write that never landed) — then periodically verify the two agree and heal a
  mismatch with a one-time full recompute of the divergent window, recorded on the plan as a
  downgrade exactly like any other. Verification need not be a bespoke probe against smelt's
  own ledger: a table format that keeps its own commit history (Delta's transaction log, in
  particular) already records independently whether a given write landed, which is a second
  source of truth free of an extra write smelt would otherwise have to make and maintain.
  Open: which structures this is even safe for (the safe-direction ordering above does not
  by itself cover the tombstone ledger, whose danger is a tombstone record with no matching
  delete — the presented `MERGE` and the tombstone insert are two writes with no natural
  data-first ordering between them); where the periodic verification runs (it is a run-time
  healing step, not a planning-time one — a standing plan-time resolver reading backend state
  would violate maintenance-plan purity); and whether a backend's transaction-log metadata is
  a durable enough interface to build on, or only an engine-specific escape hatch. Not
  decided; no plan targets it.

## References

- **Code**: `crates/smelt-state/src/` (the `.smelt/` store: `file_store.rs`, `intervals.rs`,
  `reconciliation.rs`, `landed_deltas.rs`, `schema_tracking.rs`, `snapshot_store.rs`,
  `source_postures.rs`, `frozen_band_baselines.rs`; backend ledger DDL: `ddl_duckdb.rs`,
  `ddl_spark.rs`); `crates/smelt-core/src/config.rs` (`StateMode`, `parse_warehouse_tables`);
  `crates/smelt-runtime/src/execute/` (state-write sites); `crates/smelt-logical/src/maintenance/availability/`
  (the pure availability-resolution step)
- **Tests**: `crates/smelt-state/tests/`; `crates/smelt-cli/tests/maintenance_conformance/`
  (the standing equivalence-oracle gate; `state_deletion.rs` is the leg that interleaves
  `.smelt/` deletion between run steps for every maintained recipe, the executable form of
  "no correctness state outside the engine")
- **User docs**: `docs-site/docs/reference/state.md` (state-structure inventory, per-posture
  write set, recovery playbook), `docs-site/docs/reference/smelt-yml.md` §"State Configuration"
  (`state.mode`, `state.warehouse_tables`, both diagnostics), `docs-site/docs/guide/targets.md`
  (per-backend ledger availability), `docs-site/docs/guide/incremental-models.md` (per-model
  maintenance-plan downgrade reporting), `docs-site/docs/reference/smelt-explain.md`
  (`MaintenanceStateDowngraded` printing)
- **Plans (history)**: `docs/outcomes/20260904-state-residency/outcome.md`
- **Related specs**: `run_state.md` (`.smelt/` layout and formats), `incremental_models.md`
  (frontier semantics, equivalence invariant, graph layer), `incremental_shapes.md` (merge
  ledger, partition-grain state ownership), `virtual_environments.md` (`state.mode` surface,
  environments), `sources.md` (fingerprint sidecar, landed deltas), `schema_evolution.md`
  (deployed-schema snapshots), `architecture.md` (maintenance-plan purity, fail-loud
  discipline)
