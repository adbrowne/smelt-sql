---
feature: state
status: experimental
last_reviewed: 2026-09-04
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
| Run manifests + run reports | observability | `.smelt/targets/<t>/runs/`, `reports/` | `run_state.md` |
| Interval ledger | observability | `.smelt/targets/<t>/intervals.json` | `run_state.md` |
| Landed-delta record | observability | `.smelt/targets/<t>/landed_deltas.json` | `run_state.md` |
| Deployed-schema snapshots | observability | `.smelt/targets/<t>/schemas/` | `run_state.md`, `schema_evolution.md` |
| Snapshot / environment store | observability | `.smelt/targets/<t>/snapshots.json` | `run_state.md`, `virtual_environments.md` |
| Source postures | observability | `.smelt/` (per target) | `sources.md` |
| Probe baselines (frozen-band) | observability | `.smelt/` (per target) | `incremental_models.md` §"The contract lattice" |

The class assignment is itself normative: a structure listed as correctness may never be
realised only in `.smelt/`, and a structure listed as observability may never become a
correctness dependency without moving classes here first.

### `state.mode` and what each posture provides

The key's syntax, the capability lattice (`environments ⊇ intervals ⊇ stateless`), and
model-level narrowing are owned by `virtual_environments.md` §"`state.mode`". This spec owns
the consequence table:

| Posture | Observability structures written | Correctness structures |
|---|---|---|
| `stateless` (default) | none — `.smelt/` need not exist | all, whenever the plan derives them |
| `intervals` | manifests, reports, interval ledger, landed deltas, schema snapshots, source postures, probe baselines | all |
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
  feature's spec; this spec requires that one of them is specified.

### The degradation contract

Statefulness is an **admission input resolved late**. Plan derivation proceeds in two steps:

1. **Ideal derivation.** The maintenance plan is derived assuming every classified state
   structure is available: cells get their best technique (an additive keyed fold, a
   ledger-enforced merge) exactly as `incremental_models.md` specifies.
2. **Availability resolution.** Each cell's technique is checked against the structures
   actually realisable for this project: the backend has a builder for the structure, the
   project has not declared `state.warehouse_tables: none` (which makes every engine-resident
   structure unavailable), and — for observability structures only — the posture includes it.
   A cell whose technique
   requires an unavailable structure is **downgraded to the cheapest member of the recompute
   family that preserves the equivalence invariant** (typically per-region or per-key-group
   recompute), and the downgrade is recorded on the cell (`MaintenanceStateDowngraded`).

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

- **The runtime ignores `state.mode` entirely.** `execute_project` unconditionally creates
  the `.smelt/` file store, acquires the lock, and writes manifests, intervals,
  reconciliation entries, landed deltas, and schema snapshots on every run
  (`crates/smelt-runtime/src/execute.rs`) — `StateMode` is parsed (`smelt-core/src/config.rs`)
  but never consulted. The optionality rule is therefore entirely unimplemented: today every
  project behaves as (at least) `intervals`. Tracking:
  `docs/outcomes/20260904-state-residency/outcome.md` (criterion 2).
- **The reconciliation ledger is `.smelt/`-resident, violating the residency rule.** Both
  gradings live in `.smelt/reconciliation.json` (`crates/smelt-state/src/reconciliation.rs`)
  rather than in a backend table transactional with the fold. `run_state.md`
  §"Relationship to the reconciliation ledger" and `incremental_models.md` §Known Divergences
  already record the intended move; this spec makes the end-state normative. Until the move,
  the additive grade's never-fold-twice check rides on `.smelt/`, so deleting `.smelt/`
  today *can* affect correctness for keyed additive folds — the flagship gap this doctrine
  exists to close. Tracking: `docs/outcomes/20260904-state-residency/outcome.md`
  (criterion 1).
- **No availability-resolution step exists in derivation.** Today an additive-graded cell on
  a backend without a ledger builder fails loudly (the ledger's warehouse substrate is
  DuckDB-only, `incremental_models.md` §Known Divergences) instead of downgrading with
  `MaintenanceStateDowngraded`; neither diagnostic code in §Surface is implemented. Tracking:
  `docs/outcomes/20260904-state-residency/outcome.md` (criteria 3-5).
- **`state.warehouse_tables` is unimplemented.** The key (§"Opting out of warehouse
  bookkeeping") is not parsed, and availability resolution — which it feeds — does not exist
  (previous bullet). Decision record:
  `docs/research/20260816-open-questions-triage.md` item 11. Tracking:
  `docs/outcomes/20260904-state-residency/outcome.md` (criterion 5).

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
- **Conformance gate leg for state deletion.** A generative gate variant that interleaves
  `.smelt/` deletion (and, later, downgrade-forcing) between run steps and asserts the
  equivalence oracle still holds — the executable form of the "no correctness state outside
  the engine" invariant. Sensible only after the reconciliation ledger's move makes it pass.

## References

- **Code**: `crates/smelt-state/src/` (the `.smelt/` store: `file_store.rs`, `intervals.rs`,
  `reconciliation.rs`, `landed_deltas.rs`, `schema_tracking.rs`, `snapshot_store.rs`,
  `source_postures.rs`, `frozen_band_baselines.rs`; backend ledger DDL: `ddl_duckdb.rs`,
  `ddl_spark.rs`); `crates/smelt-core/src/config.rs` (`StateMode`);
  `crates/smelt-runtime/src/execute.rs` (state-write sites)
- **Tests**: `crates/smelt-state/tests/`; `crates/smelt-cli/tests/maintenance_conformance.rs`
  (the equivalence oracle this doctrine's gate leg would extend)
- **User docs**: none yet — `docs-site/docs/reference/smelt-yml.md` documents `state.mode`
- **Plans (history)**: none yet — this spec precedes its first implementation plan
- **Related specs**: `run_state.md` (`.smelt/` layout and formats), `incremental_models.md`
  (frontier semantics, equivalence invariant, graph layer), `incremental_shapes.md` (merge
  ledger, partition-grain state ownership), `virtual_environments.md` (`state.mode` surface,
  environments), `sources.md` (fingerprint sidecar, landed deltas), `schema_evolution.md`
  (deployed-schema snapshots), `architecture.md` (maintenance-plan purity, fail-loud
  discipline)
