---
feature: keyed_models
status: experimental
last_reviewed: 2026-07-05
owners: [andrew]
---

# Keyed Refresh Mode

> **What this is.** A normative spec for `refresh: keyed` — the smelt-owned key-addressed refresh mode: the stored table is keyed state (one row per key), maintained by folding each run's per-key delta into it via `merge_into`. One mode covers the running-aggregate, latest-value, and milestone/retroactive-enrichment patterns; what distinguishes those patterns is the **column family** of each projection, derived from the SQL, never declared. This spec is a **composition** (`model_maintenance.md` §"The composition contract"): it presents the composition table referencing shared capabilities **by name**, then defines the keyed-**local** machinery in full — the column-family catalogue, the derived execution postures, the run ledger, the two run shapes (window-forward and snapshot-reconcile), the admission matrix, and the classifier. Out of scope, with their own homes: the equivalence invariant and algebraic ladder (`model_maintenance.md`); the discriminants, driving-fact resolution, once-write and join-contribution proofs (`model_properties.md`); keyed `merge_into`, the windowed-keyed-maintenance driver, and source-filter pushdown (`model_transforms.md`); the refresh axis, declaration law, and input-consumption axis (`models.md`); the partitioned mode (`batched_models.md`); the version-history mode (`versioned_models.md`); engine-owned maintenance (`materialized_view.md`); the source clock and mutation profile (`timeseries.md`, `sources.md`); the pattern-function surface (`functions.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).

## Surface

### Composition

Per the composition contract (`model_maintenance.md`), `refresh: keyed` composes as:

| Facet | This mode |
|---|---|
| **Properties required** | algebraic discriminants (is-monoid / needs-inverse / decomposable / value-vs-order-monotone) — they define the column families below; driving-fact / anchor resolution (the single clocked source under window-forward); event-time monotonicity trace (the driving source's clock); once-write provenance (the `COALESCE` family's licence); join-contribution monotonicity (enrichment joins); input-delta discovery (`model_properties.md`) |
| **World-facts consumed** | the **timeseries clock** of a clocked driving source (`timeseries.md`); the **source mutation profile** (`sources.md`) |
| **Transforms driven** | keyed **`merge_into`** (target-as-replica) sequenced by the **windowed-keyed-maintenance driver**, with **source-filter pushdown** on the driving source; the **transactional merge ledger**; for enrichment shapes, the **dimension-driven horizon-bounded MERGE** (all `model_transforms.md`) |
| **Output shape** | **keyed** — one row per `unique_key`, no `partition_column` (`models.md` §"Refresh axis") |
| **Invariant upheld** | end-state equivalence — the end-state specialisation of the processed-input equivalence invariant (`model_maintenance.md` §"The equivalence invariant"); the oracle is the model's **own SQL** (§Semantics) |

### YAML frontmatter (in `.sql` files)

```sql
---
refresh: keyed
---

SELECT
    order_id,
    MIN(event_ts)                 AS placed_at,       -- extremal fold
    MAX_BY(status, event_ts)      AS current_status,  -- order-monotone overwrite
    SUM(item_count)               AS total_items,     -- additive fold
    MAX(shipped_ts)               AS shipped_at       -- extremal fold (milestone)
FROM smelt.order_events
GROUP BY order_id
```

`refresh: keyed` is the entire opt-in; it implies a stored `table` (`models.md` §Design — the modeller does not restate `materialization: table`). No rule-specific config block is read or required.

`refresh: keyed` **forbids** a `timeseries:` block on the model — the output has no partition column (§"Output shape"). This forbids *output* partitioning, not event-time-aware *consumption*: a keyed model over a source that carries a `timeseries:` declaration consumes that source window-forward (§Semantics). It also **forbids** a `batched:` block — keyed and `batched` uphold different specialisations of the equivalence invariant on different output shapes.

The body **must** be an aggregated `GROUP BY` query: `unique_key` is the `GROUP BY` column list, and every non-key projection must classify into exactly one column family (below). A bare, un-aggregated projection is not a keyed model — the SQL must itself express the per-key semantics, so that a full refresh of the SQL is the mode's correctness oracle (§Design "The SQL is the oracle").

### `smelt.yml` (project-level overrides)

```yaml
models:
  order_lifecycle:
    refresh: keyed
```

Frontmatter wins over `smelt.yml` when both set `refresh`. The same forbid-`timeseries:` / forbid-`batched:` constraints apply.

### CLI

Which flags apply is determined by the model's derived **run shape** (§Semantics):

```
smelt run       --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]   # window-forward
smelt backbuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]   # window-forward
smelt run       [selectors]                                                             # snapshot-reconcile
```

- **Window-forward** (the model's driving source is clocked): both flags are required; they apply to the **driving source's** `partition_column` / `granularity` — not to any column on the keyed output. Format and alignment rules follow `batched_models.md` §CLI.
- **Snapshot-reconcile** (no clocked source): the flags are a **hard error** — *"model has no clocked driving source; run without event-time flags"*. Each run is a whole reconciliation.

### The column-family catalogue

The classifier assigns each non-key projection to exactly one **column family**. The family fixes the cross-window combiner (a lookup off the aggregator — authors never declare combiners) and every derived property:

| Family | Per-key aggregators | Cross-window combiner | Idempotent (re-run safe) | Order-independent | Invertible | Run shapes admitted | Extra licence |
|---|---|---|---|---|---|---|---|
| **additive fold** | `COUNT(...)`, `SUM(...)`, `BIT_XOR(...)` | `+` / `xor` | no | yes | yes | window-forward only | ledger-enforced re-run refusal (§Semantics) |
| **extremal / lattice fold** | `MIN`, `MAX`, `BOOL_AND`, `BOOL_OR`, `BIT_AND`, `BIT_OR` | `LEAST`/`GREATEST`/`AND`/`OR`/`&`/`\|` | yes | yes | no | window-forward only | — |
| **order-monotone overwrite** | `MAX_BY(value, ordering)`, `MIN_BY(value, ordering)` | max/min-by-ordering (§"Ordering ties") | yes | up to ordering-key ties | no | window-forward only | — |
| **once-write** | `COALESCE`-first-non-null over the group | `COALESCE(target, delta)` | yes | yes (given the proof) | no | window-forward only | once-write provenance proof (`model_properties.md`): key-derived, or a declared functional dependency |
| **plain overwrite** | `ANY_VALUE(...)` | incoming row wins | yes | n/a — one row per key per scan | no | **snapshot-reconcile only** | — |

Any other aggregate, any non-aggregate non-key expression, and any composite expression over aggregates (`SUM(x) + 1`) is rejected (`KeyedUnknownCombiner`). Add columns for the underlying aggregates and derive downstream.

The pattern functions `smelt.latest(value, ordering)` (→ `MAX_BY`), `smelt.once(value)` (→ the once-write canonical spelling), and `smelt.current(value)` (→ `ANY_VALUE`) are the intent-naming sugar for the overwrite, once-write, and plain-overwrite families; they are ordinary transparent functions (`functions.md`) whose expansions are admitted on exactly the same terms as hand-written calls.

### Diagnostic codes (owned by this spec)

| Code | Severity | Trigger |
|---|---|---|
| `KeyedRequiresGroupBy` | Error | The model SELECT has no `GROUP BY` — there is no unique key to derive. |
| `KeyedForbidsTimeseries` | Error | The model declares both `refresh: keyed` and a `timeseries:` block. |
| `KeyedForbidsBatched` | Error | The model declares both `refresh: keyed` and a `batched:` block. |
| `KeyedUnknownCombiner` | Error | A non-key projection is not a direct call to a catalogued aggregator. Names the offending expression; when the projection is a bare column or `ANY_VALUE` under window-forward, the message names `MAX_BY` + an ordering column as the fix. |
| `KeyedGroupByContainsPartitionColumn` | Error | The `GROUP BY` contains the driving source's `partition_column` — that is the partitioned (batched) shape; the diagnostic suggests `refresh: batched` + `timeseries:`. |
| `KeyedForbidsWindowFunctions` | Error | The outer SELECT body uses `OVER (...)`. The keyed state *is* the window. |
| `KeyedForbidsNondeterministic` | Error | The SQL uses `NOW()`, `RANDOM()`, or other non-deterministic functions. Cross-window merge requires deterministic per-window output. |
| `KeyedSqlNotParseable` | Error | The model body cannot be parsed into the shape the classifier reads. |
| `KeyedMultipleDrivingSources` | Error | More than one timeseries-tagged source appears in the FROM clause. Lists the candidates. |
| `KeyedOnceWriteUnproven` | Error | A once-write (`COALESCE`) column has no once-write provenance proof — the value is not provably a per-key constant. Names the column; suggests the key-derived form, a declared functional dependency, or remodelling. |
| `KeyedRetractableContribution` | Error | An enrichment join's per-key contribution is retractable — it feeds a decrementing aggregate or a value that must be un-seen. Steers to `refresh: materialized_view` or DAG composition. Does **not** fire on the join spelling alone (§Semantics). |
| `KeyedSnapshotSourceUnsupportedColumn` | Error | A column family inadmissible under snapshot-reconcile (§"Admission matrix") appears in a model with no clocked driving source. Names the column, the family, and why the current-snapshot oracle cannot hold for it. |
| `KeyedReprocessedWindow` | Error | A run window covers a ledgered window of a non-re-run-tolerant model, or `--auto` detects changed input under an already-merged window (§"Reprocessing"). Points at `--full-refresh`. |

There is **no** `safety_overrides:` block. Every rejection above guards the equivalence invariant itself, not a partial-correctness optimisation — there is nothing safe to waive (§Design).

## Semantics

### The two run shapes (derived, never declared)

The run shape is the keyed application of the input-consumption axis (`models.md` §"Input-consumption axis"), derived from the driving source:

- **Window-forward** — the FROM clause contains exactly one source whose resolved target declares `timeseries:` (the **driving source**, resolved by the shared driving-fact / anchor proof; zero clocked sources means snapshot-reconcile below; two or more is `KeyedMultipleDrivingSources`). The run steps over the source partitions covered by `[run_start, run_end)` **in temporal order**; for each partition, source-filter pushdown injects the partition's window onto the driving source's reference, the per-partition delta SELECT executes, and a `merge_into` folds the delta into the target with the per-column combiner map. Non-timeseries sources (lookups / dimensions) are read in full each step. If the output table does not exist at the first step, it is created from that step's delta (`CREATE TABLE AS SELECT`).
- **Snapshot-reconcile** — no clocked source. The run re-scans the source whole, computes the per-key aggregation, and `merge_into`s the result: matched keys are overwritten, unmatched inserted. A key present in the store but **absent from the incoming scan is retained** unchanged; deletion requires an explicit mechanism (out of scope, §Known Divergences).

Out-of-order, parallel, or sliced-backfill window application is admitted **iff the model is order-independent** (below); otherwise windows must be applied sequentially in temporal order.

### Derived execution postures

Three model-level properties are folded from the column families; each is derived, surfaced by `smelt explain`, and never declared:

1. **Re-run tolerance** — may an already-merged window be blindly re-merged over *unchanged* input? Holds iff every column is idempotent, i.e. **no additive-fold column**. For re-run-tolerant models a repeated window converges (`GREATEST(x, GREATEST(x, y)) = GREATEST(x, y)`); for additive models it double-counts and must be refused (the ledger, below).
2. **Order-independence** — may windows be applied out of order or in parallel? Holds iff every column's combiner is order-independent: the extremal/lattice and proven once-write families qualify; the **order-monotone overwrite family does not** (its order-independence holds only up to ordering-key ties, which are not statically excludable — §"Ordering ties"), so any model with an overwrite column executes windows sequentially in temporal order.
3. **Reprocessing refusal** — a window whose *input changed* since it was merged must not be re-merged for **any** family: an irreversible fold cannot un-see a removed contribution, and an overwrite cannot retract a superseded-by-nothing value. Detection and mitigation below.

### The transactional merge ledger

Every **window-forward** keyed model maintains a per-model **ledger** — a small backend table recording each merged window — written **in the same backend transaction** as that window's `merge_into`. Its role by posture:

- **Additive-fold models** (not re-run tolerant): a run whose window is already ledgered is **refused** (`KeyedReprocessedWindow`) — exactly, not best-effort. Crash resume merges only unledgered windows; a run interrupted at window *k* of *n* resumes correctly by re-running the same range.
- **Re-run-tolerant models**: a ledgered window may be re-merged (a no-op on unchanged input); the ledger serves reprocessing detection and `--auto` bookkeeping, not refusal.

Snapshot-reconcile models keep no ledger — each run is a self-contained reconciliation and re-running is always safe. The ledger is backend-resident and transactional with the write it describes; it is a **correctness structure**, distinct from the opt-in run-state observability surface (`run_state.md`).

### Admission matrix (column family × source shape)

Which families a model may use depends on its run shape. This is the enforced form of the event-vs-state distinction: fold families consume **events** (each row contributes exactly once); overwrite families consume **observations** (each row supersedes). The matrix is checked per column:

| Column family | window-forward (clocked source) | snapshot-reconcile (mutable snapshot) |
|---|---|---|
| additive fold | ✓ (ledger) | ✗ — re-folding state double-counts |
| extremal / lattice fold | ✓ | ✗ — observer semantics (below) |
| order-monotone overwrite | ✓ | ✗ — observer semantics (below) |
| once-write | ✓ (provenance proof) | ✗ — observer semantics (below) |
| plain overwrite | ✗ — order-dependent over events (`KeyedUnknownCombiner` names the `MAX_BY` fix) | ✓ |

The three snapshot ✗ cells marked *observer semantics* are not double-count hazards — those families re-merge safely — they are **equivalence failures**: `MIN(price)` folded over successive snapshots computes *min ever observed* while a full refresh over the current snapshot computes the *current* min; `MAX_BY(attr, updated_at)` retains a stale incumbent forever if a mutation regresses the ordering value; `COALESCE`-once-write captures *first observed*, unrecoverable from the current snapshot. Each is a different contract (a history *observation*, not a recomputation) and is refused (`KeyedSnapshotSourceUnsupportedColumn`) rather than admitted silently.

### End-state equivalence: the SQL is the oracle

`refresh: keyed` upholds the **end-state specialisation** of the processed-input equivalence invariant (`model_maintenance.md` §"The equivalence invariant"), and because the body is required to be the aggregation itself (§Surface), the oracle is executable for every admitted model — it is the model's **own SQL**:

- **Window-forward:** for any set `S` of processed driving-source partitions and any admitted ordering over `S`, the stored state equals the model SQL evaluated over `source.where(partition ∈ S)`. Order-independence beyond sequential-temporal application holds per posture 2 above; for overwrite columns it holds **up to ordering-key ties** (§"Ordering ties").
- **Snapshot-reconcile:** the stored row for every key **present in the current snapshot** equals the model SQL evaluated over that snapshot. Keys absent from the snapshot are retained (a named divergence from the oracle relation — the stored table is the oracle's rows plus retained departed keys).

There is **no write-eligibility clamp**: a run merges **every** delta row it scans, into whatever key it names, however old that key is. A derivable forward reach is computed and reported (`smelt explain`) but never gates admission and never bounds which keys a run may touch — so no scanned input is ever silently dropped.

### The maintenance boundary

On the algebraic ladder (`model_maintenance.md` §"The algebraic maintenance ladder") the keyed families sit on the **direct-monoid rung**: every catalogued combiner folds `(state, delta)` with no inverse and no history re-read. The additive family is additionally a **group** (invertible), which is what a future subtract-then-add reprocessing path would exploit; the idempotent families are monoids but not groups (a folded contribution cannot be un-seen), which is why reprocessing is refused for them. Rungs 2–4 (decomposed state + presentation view for `AVG`-class aggregates; group-rung retraction; the opt-in bounded-domain multiset for exact holistic aggregates) grow this mode without changing its contract; the transforms are catalogued in `model_transforms.md` and the `bounded_domain:` budget declaration in `model_properties.md`. Beyond the ladder — general-operator retraction over joins, unbounded non-additive state — is delegated to `refresh: materialized_view`.

### Reprocessing

If a merged window's source data changes, re-running it does not produce correct state for any family (posture 3). The rule refuses at planning time when it can detect it — the ledger says the window was merged; `--auto` staleness says the input changed — with `KeyedReprocessedWindow` pointing at the two mitigations: `--full-refresh` (truncate-and-rebuild), or a manual cascade rebuild. Subtract-then-add for all-invertible models is a future path (§Known Divergences).

### Ordering ties (order-monotone overwrite)

The pairwise combiner for `MAX_BY(value, ordering)` is: the delta wins iff `delta.ordering > target.ordering` (strict); **on equality the incumbent (target) wins**. This is deterministic given the processing history but **not order-independent when ties occur across windows** — which is why overwrite columns force sequential execution (posture 2). The recommended modelling practice is a composite, provably-tie-free ordering expression (e.g. `(updated_at, source_seq)`); the classifier cannot verify uniqueness and does not claim to.

### Enrichment joins

A fact-to-dimension join that brings an enriching event in as a separately-arriving relation is admitted when its per-key contribution is **provably monotone** — the join-contribution monotonicity proof (`model_properties.md`): the contribution feeds only extremal, order-monotone, or once-write columns and does not fan into a decrementing aggregate. The maintainability line is monotone-vs-retractable **semantics, not join-vs-union spelling** — the join form is normalised to the same keyed-monoid merge as the union form. Only a genuinely retractable contribution is refused (`KeyedRetractableContribution`). A **re-scanned existence flag** additionally requires the dimension source to be declared `append_only` (`sources.md`); extremal milestones are safe regardless. Where a dimension batch's forward reach `H` is **derivable from the model's SQL**, the dimension-driven horizon-bounded MERGE (`model_transforms.md`) may clamp the enrichment *recompute* to `[event_ts, event_ts + H]` — a scan-side bound that cannot under-cover because it is derived; where `H` is not derivable, the transform is not licensed and the enrichment evaluates through the ordinary widened scan. No declared value ever truncates a recompute or a write.

### Output shape

One row per `unique_key`; column names are the projection's `AS` aliases (or source column names). **No** `partition_column`, **no** `event_time_column`, **no** `timeseries:` on the model. Downstream consumers see the output as a lookup table read in full each run, identical to any non-timeseries source.

### Functions inside keyed bodies

Function expansion (`expansion.md`) runs **before** the classifier. Projection reading, GROUP-BY inspection, FROM-clause walking, family classification, and pushdown operate on the expanded CST. A `smelt.define`-resolved call is admitted iff its expanded body produces a catalogued aggregator at the outermost expression position — the pattern functions (§Surface) are admitted exactly this way, with no privileged treatment. Opaque calls (`smelt.extern`, non-inlinable built-ins) in the projection list are rejected via `KeyedUnknownCombiner`.

### Interaction with `--auto` / staleness

- **Window-forward:** stale driving-source windows are re-processed subject to posture — re-run-tolerant models re-step exactly the stale windows (safe by idempotence); additive models refuse re-processing of ledgered windows (`KeyedReprocessedWindow`) and steer to `--full-refresh`.
- **Snapshot-reconcile:** the model is treated as always-stale; every `--auto` run reconciles.

## Design

**One mode; the column family is the pattern.** The running-aggregate, latest-value, and milestone patterns share the output shape (keyed), the invariant (end-state equivalence), the transform (`merge_into` via the one windowed driver), and the key derivation — they differ only in per-column combiner algebra, and every consequence of that difference (re-run tolerance, ordering, ledger, reprocessing) is derivable from the SQL. By the litmus rule (`models.md` §Design), facts that change only execution posture under an unchanged contract are **derived, never declared** — so they must not multiply the refresh enum. Splitting them into peer modes was rejected for a second, decisive reason: combiner intent is **per column, not per model** — the §Surface example mixes an additive fold, an overwrite, and two extremal milestones in one table, a shape no per-pattern mode can express without materialising the same keyed state several times. Full derivation: `docs/research/20260705-unified-keyed-refresh.md`; decision record: `docs/research/20260705-keyed-collapse-application.md`.

**The SQL is the oracle.** The body must be the aggregation itself so that `full_refresh(model SQL)` is an executable correctness oracle for every admitted model. A bare-projection surface with mode-imposed dedup was rejected: its full refresh is not one row per key, so the equivalence invariant would have no executable oracle and the mode would add semantics the SQL does not carry (`docs/research/20260705-model-refresh-review.md` §1.1). The plain-overwrite family (`ANY_VALUE`) exists to give the snapshot posture an honest aggregated spelling under this rule.

**Derive `unique_key` and combiners from the SQL, not frontmatter.** The `GROUP BY` names the key; each projection names its aggregator; the combiner is a fixed lookup. A config block restating them re-introduces metadata-vs-SQL drift (`docs/research/20260521-incremental-as-planner-rule.md`). If it is in the SQL, it is not also in YAML.

**No write-eligibility clamp.** A horizon-clamped merge (only keys newer than `run_start − H` are eligible) was rejected: it silently drops *scanned* inputs — the one silent-data-loss point in the maintained family — and it is not needed for correctness, since merge work is proportional to delta size. What a clamp would buy (settled-key GC, a work bound) is deferred optimisation and must arrive as a package with late-fact accounting (`docs/research/20260705-keyed-collapse-application.md` D6).

**The ledger is the deliberate exception to "smelt does not own state".** The batched-side doctrine (backend owns watermarks/run history; `batched_models.md`) rejected a watermark *store* because it duplicates engine state and opens a sync-correctness window. The keyed ledger has neither defect: it is backend-resident and written in the same transaction as the merge it describes, so it cannot drift from the state it records. Without it, additive-fold models cannot detect a double-counting re-run and any mid-run crash forces a full rebuild — an unacceptable operational cliff for the family's most common combiners (`SUM`/`COUNT`).

**Observer semantics are refused, not smuggled.** Folding state observations (a mutable snapshot) into `MIN`/`MAX`/once-write columns yields min-ever / first-observed values no full refresh can reproduce — a genuinely different contract (a history observer). Admitting it silently would put two contracts behind one mode, the exact dbt-`strategy:` failure the refresh peers exist to avoid. The refused cells name the observer contract as the future opt-in path.

**Ties: honest boundary, not fake proof.** Incumbent-wins plus mandatory sequential execution makes overwrite columns deterministic-given-history without claiming an order-independence no static analysis can prove. A last-processed combiner (no ordering column, order-dependent for *all* rows) was rejected outright; the snapshot posture's plain-overwrite family serves that need where it is well-defined (one row per key per scan).

**No `safety_overrides:`.** Batched offers per-check overrides because some of its rejections guard partial-correctness properties a modeller may knowingly waive. Every keyed rejection guards the equivalence invariant itself — a bypass would produce silently order-dependent or double-counted state that is impossible to debug. The escape from a rejection is to remodel, or to move to `refresh: materialized_view`.

**One windowed executor, shared.** The window-forward step loop is the windowed-keyed-maintenance driver (`model_transforms.md`), parameterised by `(classifier, merge-SQL builder)`. A per-pattern copy of the loop was rejected as four-way drift risk; a consequence is that the mode inherits the driver's granularity support (§Known Divergences).

## Constraints & Invariants

1. **Opt-in is `refresh: keyed` alone** (storage implied `table`). No config block; no `safety_overrides:`.
2. **`timeseries:` and `batched:` blocks are forbidden on the model.** Diagnostics `KeyedForbidsTimeseries`, `KeyedForbidsBatched`.
3. **The body is an aggregated `GROUP BY` query; `unique_key` is derived from `GROUP BY`; every non-key projection classifies into exactly one column family.** The combiner is a fixed lookup; authors never declare combiners.
4. **The catalogue is closed and the classifier is fail-closed.** Unrecognised aggregators, composite expressions, unproven once-write columns, and retractable contributions are refused — never approximated, never silently downgraded (`model_maintenance.md` §"Validator, not chooser").
5. **End-state equivalence holds with the model's own SQL as the oracle**, with exactly two named carve-outs: retained departed keys under snapshot-reconcile, and ordering-key ties on overwrite columns.
6. **No write-eligibility clamp.** A run merges every delta row it scans; no scanned input is silently dropped. Any future clamp or settled-key GC must ship together with late-fact accounting.
7. **The run shape is derived from the driving source** (clocked ⇒ window-forward; unclocked ⇒ snapshot-reconcile) and surfaced by `smelt explain`; it is never declared.
8. **The admission matrix is enforced per column.** Fold and once-write families require a clocked (replayable) driving source; the plain-overwrite family requires the snapshot posture.
9. **Window-forward models maintain the transactional merge ledger**, written atomically with each window's merge. Additive-fold models must refuse a ledgered window's re-run; re-run-tolerant models may re-merge. Snapshot-reconcile models keep no ledger.
10. **Ordering and parallelism follow the derived postures.** Out-of-order/parallel/sliced backfill only for order-independent models; overwrite columns force sequential temporal order.
11. **Reprocessing changed input is refused for every family** when detected; the mitigation is `--full-refresh` (or a manual cascade rebuild).
12. **Exactly one clocked driving source under window-forward.** Zero clocked sources selects snapshot-reconcile; two or more is refused.
13. **No `partition_column` on the output.** Downstream consumers treat the keyed table as a lookup.
14. **The windowed step loop is the shared driver**, not a per-pattern copy (`model_transforms.md`).

## Known Divergences / Open Questions

- **Nothing named `keyed` parses yet.** `RefreshStrategy` (`crates/smelt-core/src/config.rs`) accepts `full` / `batched` / `cumulative` / `materialized_view`. The shipped `refresh: cumulative` is this mode's built seed: the direct-monoid classifier (`crates/smelt-logical/src/rules/cumulative.rs` — additive + extremal/lattice families only), the windowed-keyed-maintenance driver (`crates/smelt-runtime/src/maintenance_driver.rs`), and the per-window `merge_into` execution (`crates/smelt-runtime/src/cumulative.rs`). The rename (`cumulative` removed with a pointing error, `Keyed*` diagnostic family), the classifier union (overwrite + once-write + plain-overwrite families), and the posture derivation are delivered by the keyed-collapse plan (decision record: `docs/research/20260705-keyed-collapse-application.md`; sub-plan to be registered in `docs/plans/20260704-model-updates.md`).
- **The ledger is unbuilt.** Today reprocessing detection is best-effort and crash resume of an additive model is not exact; `KeyedReprocessedWindow` semantics require the ledger. Until it lands, the shipped behaviour matches `cumulative` today (refuse when detectable; `--full-refresh` otherwise).
- **The snapshot-reconcile executor is unbuilt.** Until it lands, an unclocked keyed model is refused fail-loud with a not-yet-supported diagnostic (`KeyedSnapshotPostureUnsupported`) naming the delivering plan — it is not treated as a model error.
- **The pattern functions (`smelt.latest`, `smelt.once`, `smelt.current`) are unshipped**, as is the decision whether they are built-ins or template files; the canonical once-write spelling is fixed alongside them. Tracked in the keyed-collapse plan.
- **Driver granularity is `day`/`week` only** (`maintenance_driver.rs::driving_steps` refuses others) — a property of the shared driver inherited by every consumer; widening it is driver work, not mode work.
- **`--auto` staleness fidelity** for all-invertible models ("exactly the changed windows") needs the delta-history mechanism of the group rung; the v1 answer is conservative. Carried from the cumulative-era divergence list.
- **Self-referential keyed models** (`state += delta − decay`; the model joining its own target) are rejected — the self-reference is not an admissible input. An explicit input/state-distinction design would be needed to admit them. Carried.
- **Run-pinning alignment**: `NOW()`/`CURRENT_*` are rejected outright rather than compile-time-pinned as batched does; adopting the pinning transform here is a deferred alignment. Carried.
- **Key deletion is unresolved beyond retention.** Snapshot-reconcile retains departed keys; window-forward has no delete signal short of a change feed with delete events. Tombstones, opt-in hard delete, and the observer contract for the refused matrix cells are recorded as deferred in the decision record (§5 there).
- **Rungs 2–4 are specified ahead of this mode's use of them** (`AVG` via decomposed state + presentation view; group-rung retraction; the bounded-domain multiset). The mechanisms live in `model_transforms.md` / `model_properties.md`; wiring them into keyed columns is future composition work.

## References

- **Code**: `crates/smelt-core/src/config.rs` (`RefreshStrategy`); `crates/smelt-logical/src/rules/cumulative.rs` (the built classifier seed — combiner lookup, GROUP-BY key derivation, driving-source resolution); `crates/smelt-runtime/src/maintenance_driver.rs` (the windowed-keyed-maintenance driver, `WindowedKeyedRule`); `crates/smelt-runtime/src/cumulative.rs` (per-window merge execution); `crates/smelt-backend/src/lib.rs` (`merge_into`), impls in `crates/smelt-backend-duckdb`/`-spark`.
- **Tests**: the cumulative classifier unit tests (`smelt-logical/src/rules/cumulative.rs`); the keyed end-state-equivalence harness; `smelt-backend-duckdb` `merge_into` tests.
- **User docs**: `docs-site/docs/guide/materializations.md` (to be replaced by a keyed-models guide with per-pattern recipes).
- **Plans (history)**: `docs/plans/20260523-cumulative-aggregate.md` (the built seed); `docs/plans/20260704-model-updates.md` (the mode-vertical master; hosts the keyed-collapse sub-plan registration).
- **Research**: `docs/research/20260705-model-refresh-review.md`; `docs/research/20260705-unified-keyed-refresh.md`; `docs/research/20260705-keyed-collapse-application.md` (the decision record this spec encodes); `docs/research/20260704-monotone-join-maintenance.md` (the monotone-vs-retractable boundary); `docs/research/20260703-model-updates.md`.
- **Related specs**: `model_maintenance.md` (invariant, ladder, composition contract, validator-not-chooser); `model_properties.md` (discriminants, anchor resolution, once-write and join-contribution proofs, `bounded_domain:`); `model_transforms.md` (`merge_into`, the driver, pushdown, the ledger, dimension-horizon MERGE); `models.md` (refresh axis, declaration law, litmus rule, input-consumption axis); `batched_models.md` (the partitioned peer); `versioned_models.md` (the version-history keyed peer); `materialized_view.md` (engine-owned maintenance; the retractable-slice fallback); `timeseries.md`, `sources.md` (the clock and mutation-profile world-facts); `expansion.md`, `functions.md` (expansion order; the pattern-function surface).
