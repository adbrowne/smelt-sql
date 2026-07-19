# The Relation Contract and per-cell write addressing

Date: 2026-07-16
Status: design proposal (predecessor to spec edits across `incremental_models.md`,
`models.md`, `sources.md`, `timeseries.md`)
Owners: andrew

> **One-line thesis.** `grain:` today conflates two independent things — *what a stored row is*
> (a consumer-facing shape, legitimately declared) and *how a write physically addresses rows*
> (region rewrite vs keyed merge, which should be per-cell). Split them: a model's output declares
> the **same world-fact vocabulary a source declares** (one shared **Relation Contract**), the
> declared facts fix the output shape, and **physical write addressing is derived per
> `(column-group × trigger × changed-input)` cell** with user pins — validated against the
> equivalence invariant, never silently chosen.

## Motivation

Two modelling scenarios the current single-`grain:`-per-model declaration cannot express, both
raised in review of `incremental_models.md` (the FEEDBACK note at the head of §Surface):

- **A — mixed addressing by *which input changed*.** An output is partition-addressed with
  respect to its main fact table (new fact rows → rewrite the touched partitions), but when a
  *different* table changes — a late dimension attribute correction — the right write is a keyed
  MERGE targeting the specific affected rows across many partitions, not a whole-partition
  rewrite.
- **B — mixed addressing by *trigger*.** Normal runs merge deltas by key (cheap, proportional to
  delta size); a **backfill** of a region wants DELETE+INSERT (a clean region reset), not a
  row-by-row merge.

In both, the operator wants **control** over how the update happens, keyed by *what changed* and
*which trigger fired* — not a single model-wide addressing verdict.

## What the machinery already does — and where the framing lies to itself

The plan machinery is *already* per-cell along the axes that matter:

- **The plan matrix** factors output columns into groups and picks, per `(group × trigger)` cell,
  a corner of **read-scope × write-scope**, where write-scope is exactly
  `{targeted addresses, region-overwrite}`. The technique enum spanning the corners —
  `DELETE`+`INSERT` region recompute, keyed fold `MERGE`, column-scoped `MERGE`, in-place
  `UPDATE` — already covers both region and keyed addressing.
- **Scope maps** already name the per-input dispatch: the driving fact's delta folds forward; a
  dimension delta probes and horizon-merges; a definition diff backfills columns; a self-edge
  forces ordering.
- **Per-cell admission** already says a region recompute *supersedes and resets* what folds wrote
  (interchangeable at fixed processed-input set `S`, modulo the ledger).

So "region vs keyed" is already a per-cell fact in the *plan*. The defect is the **surface
framing**: `grain:` is defined as "how output rows are addressed for update" — a per-write notion
— but pinned model-wide, contradicting the per-cell plan beneath it. Identity-free-ness is a
property of a *write*, not of a *model*: a partition-grain model's dimension-change write wants
row identity (which is why `unique_key` is quietly available on it); a composed keyed+clocked
model's backfill write can be an identity-free slice rewrite.

## The litmus rule already draws the line — `grain` straddles it

`models.md`'s litmus rule sorts every fact by *who fixes it*:

- changes *the freshness owner / equivalence contract* → a new **refresh peer** (`full` /
  `incremental` / `materialized_view`);
- changes *what a stored row is* → a **grain** (declared-and-checked, never derived);
- changes *which technique serves a cell* → **derived** per cell;
- changes *how deltas are discovered / how much is scanned* → **derived** from the source.

Today's `grain` values violate this by carrying *both* halves. `grain: partition` declares "a
stored row is one row of a complete clocked table" (*what a row is* — declared, correct) **and**
"addressed by whole-partition rewrite" (*addressing* — should be per-cell). The fix puts each
half on its correct side of the litmus line: keep *what a row is* declared; move *addressing*
fully into derived-per-cell, where the litmus rule already places technique. The reframe is
**more** litmus-aligned, not less.

## Part 1 — output shape is two orthogonal declared facts (grain dissolves)

"What a stored row is" is fully captured by two facts a **source already declares**:

- **clock** — a `timeseries:` block (`event_time_column` / `partition_column` / `granularity`),
  or its absence;
- **identity** — a `unique_key:`, or its absence — *including whether `partition_column` is a
  member of the key*.

The current three grains (+ the composed shape) are exactly the corners of these facts:

| declared facts | shape (old name) | rows |
|---|---|---|
| clock, no key | complete clocked table (`grain: partition`) | 1 / partition |
| key, no clock | bare lookup (`grain: key`) | 1 / key |
| clock + key, **partition ∉ key** | keyed lookup with a home slice (`grain: key` + temporal locality) | 1 / key |
| clock + key, **partition ∈ key** | trajectory (`grain: key_per_partition`) | 1 / (key, partition) |

`partition ∈ unique_key` is the single fact that distinguishes a trajectory (the natural key
recurs across partitions) from a keyed lookup whose key has a fixed home slice. This **unifies**
the key-temporal-locality routes with `key_per_partition`:

- locality **route 1** ("`partition_column` is a `unique_key` column") **is** the partition-∈-key
  (trajectory) row;
- locality **route 2** ("partition is a per-key constant, functionally dependent on the key, not
  in it") **is** the partition-∉-key (home-slice) row.

All four corners are declared facts *about the stored row*, never about addressing.

### `grain` survives only as a derived label (+ optional check-only assertion)

`grain` is **not declared** as a driver. It becomes a **derived, reported** classification
computed from `(clock?, identity?, partition∈key?)`, printed by `smelt explain` — and computed
for **sources too** (a source also has an effective grain: clocked-fact, keyed-dimension, …). A
modeller who wants the friendly name in frontmatter may write it only as a **check-only
assertion** (like `scan_bounds`): it errors on mismatch with the derived facts and *drives
nothing*. This keeps a shared, human-readable shape name that can never disagree with the facts —
resolving the rejected "per-edge grain declarations (two declarations can disagree)" objection,
because the declared *facts* stay one-per-node and only *derived* addressing varies.

## Part 2 — the shared Relation Contract (one vocabulary, two providers)

Both a source and a model output are **a relation a downstream consumer reads**, and the graph
layer already treats an upstream-model edge and a source edge as "the same standing." So define
**one named contract vocabulary** that *both* providers fill, with identical field paths for the
shared slots. Sources and models are **two providers**, not a symmetric pair — the asymmetries
are explicit.

| contract slot | shared field shape | source fills by | model fills by |
|---|---|---|---|
| **schema** (cols / types / nullability) | `columns:` | declared | derived (type inference) |
| **clock** | `timeseries:` (event_time / partition / granularity) | declared | **declared-and-checked** |
| **identity** | `unique_key:` (incl. partition∈key) | declared | **declared-and-checked** |
| **mutation / arrival** | `mutation:` (append_only / mutable / change_feed; lateness; redelivery; retractions; ordered; delta_identity) | declared (trust rule) | **derived** from SQL + upstream facts |
| **completeness / settle** | `watermark:` / settle bound | declared | **derived** from the plan |
| **replay bound** | replayability | `retention:` | always replayable (rebuildable) |
| *source-only* | external-name `name:` routing | declared | — |
| *model-only* | per-column `contract:`, `data_latency`, definition-change trigger | — | declared / derived |

### Three fill-modes

- **Declared** — a source's world-facts: external, unprovable, governed by the existing **trust
  rule** (widening facts like lateness trusted; narrowing facts admitted only with a verification
  tripwire; undeclared → strictest-but-correct).
- **Derived** — a model's facts proven from its SQL and plan: trusted because proven.
- **Declared-and-checked** — a model's *shape-defining* facts (the clock and identity) where pure
  derivation would let a projection refactor silently flip consumer semantics; declared, then
  checked against the SQL, error on mismatch.

A consumer reads **one contract** and never cares which mode filled a slot. This is the honest
mechanism behind "an upstream maintained model is a plan edge of the same standing as a
`sources.*` ref."

### Sources restructuring (chosen: shared named block, both use it)

`sources.md` is restructured so the shared slots (`timeseries:`, `unique_key:`, `mutation:`,
`watermark:`) have **identical field paths** to the model-output contract. Today's source
`mutation_profile:` bundle aligns to the shared `mutation:` slot; `timeseries:` and `unique_key:`
are already shared shapes. Source-only (`name:` routing, `retention:`) and model-only (per-column
`contract:`) remain as explicitly-documented asymmetries. This is the largest edit in the
proposal and is the deliberate cost of the strongest unification.

## Part 3 — per-cell write addressing (the original ask)

With addressing decoupled from the now-derived grain, every
`(column-group × trigger × changed-input)` cell derives its physical write from the **currently
known** pattern set — *an open list, not a closed enum* (see "The write-pattern set is open"
below):

```
{ region DELETE+INSERT, keyed MERGE, column-scoped MERGE, in-place UPDATE, full rebuild, … }
```

**Available-addressings rule** — a write mechanism is admitted for a cell iff:

> `available = (which contract facts the output declares) × (what the trigger/changed-input needs) × (the equivalence invariant)`

- keyed MERGE / column-scoped MERGE / in-place UPDATE require a declared `unique_key` (row
  identity);
- region DELETE+INSERT requires a declared partition axis (`timeseries:`) to delete by;
- a bare lookup (key, no clock) has no region → only keyed merge or full rebuild;
- a bare partition table (clock, no key) has no identity → only region rewrite (declaring
  `unique_key` on it is what *unlocks* targeted keyed addressing — now **load-bearing**, not a
  dedup footnote);
- SCD2's close-out cell has **only** keyed MERGE available, because its write provably escapes any
  time window — derived per-cell, fail-loud if the facts can't support it, no bespoke grain
  needed.

**Addressing mechanism vs execution span — a keyed write on a clocked model is still
partition-scoped.** Choosing *keyed MERGE* (or column-scoped MERGE / in-place UPDATE) for a cell
picks how a row is *found* — by identity — not that the statement runs unbounded over the whole
table. When the output also declares a `timeseries:` axis, the write stays **bounded to (and,
where the backend benefits, iterated over) the affected partitions**: the changed-input delta is
first resolved to the set of touched partitions, and the keyed MERGE is emitted per-partition (or
with a partition predicate) against just those. So "dimension change → keyed MERGE" on a
partition-clocked model is a keyed merge *scoped to the partitions the correction lands in*, not a
single table-wide MERGE/UPDATE scan. A genuinely window-free keyed write — one whole-table MERGE —
is the exception, reached only when the cell **provably cannot** be bounded to a partition set
(the SCD2 close-out above, whose affected rows escape any time window); that unboundedness is
itself a derived per-cell fact, fail-loud, not a default. Partition-scoping is orthogonal to the
addressing corner: region and keyed writes alike ride the same partition-pruning the plan already
computes.

**User pins.** The existing override ladder is extended to name the write mechanism per trigger:

```yaml
maintenance:
  cells:
    - columns: [<col>, ...]
      on: <source-address> | backfill      # the trigger this cell handles
      technique: fold | recompute | rederive_columns
      write:     region | keyed | column | update   # hard per-cell addressing pin (optional)
```

A pin is **validated against the equivalence invariant** (§"Per-cell admission") — an
addressing that cannot uphold equivalence for that cell is **refused with a diagnostic**, never
silently honored. This keeps the whole feature inside *validator, not chooser*.

### The two scenarios, resolved

- **A (mixed by input):** the output declares **both** `timeseries:` and `unique_key:`. The
  creation-trigger cell (main fact delta) derives a region rewrite / fold; the dimension-change
  cell derives a keyed column-scoped MERGE — available *because* `unique_key` is declared — still
  **scoped to the partitions the correction touches** (per the note above), not a whole-table
  merge. Pin either if the cost model picks wrong.
- **B (mixed by trigger):** the output declares `timeseries:` (± `unique_key`). Creation/mutation
  cells derive keyed merge / fold; the `backfill` cell is pinned
  `on: backfill, technique: recompute, write: region` → DELETE+INSERT. Licensed by the existing
  fixed-`S` interchangeability rule (a recompute supersedes and resets what folds wrote).

### The write-pattern set is open (and partly backend-provided)

The five patterns above are the ones we understand *today*. The set will grow — partition/atomic
swap (Delta/Iceberg `REPLACE PARTITION`), copy-on-write vs merge-on-read variants, MERGE with
`WHEN NOT MATCHED BY SOURCE` prune, staged-upsert, incremental MV refresh, and backend-specific
primitives we haven't met yet. Treating the list as closed would bake today's engines into the
surface. So the design's durable contract is deliberately **not** the enumeration; the enumeration
is data. Three ramifications follow, and each is a reason the reframe is *more* robust to growth,
not less:

- **The invariant is the admission function, not the enum.** The stable, load-bearing thing is the
  available-addressings rule — `available = (declared contract facts) × (trigger/changed-input
  needs) × (the equivalence invariant)` — and the doctrine that we *validate, never choose blind*.
  A new pattern is admitted purely by declaring **which contract facts it requires** (identity? a
  partition axis? ordered arrival?) and **discharging the equivalence proof obligation**
  (`incremental_state(S) == full_refresh(inputs ∈ S)` for the cells it serves). Nothing else in
  the framing has to move: grain stays derived, the contract stays the vocabulary, the cost model
  ranks whatever candidates the rule admits. Extensibility is *safe by construction* because the
  equivalence gate is the price of entry, so a new mechanism can never be less correct than the
  ones it joins.
- **The pattern set is backend-relative, so the rule gains a fourth factor.** Not every engine
  offers every primitive (DuckDB, Delta, Iceberg, BigQuery diverge sharply on atomic partition
  swap, true `UPDATE`, merge-on-read). Admission is therefore
  `available = contract facts × trigger needs × equivalence × **backend capability**`: the write
  layer queries a **capability registry** the backend fills, and a pattern the target can't
  execute is simply not a candidate. This makes the write-pattern registry the natural home for
  backend-specific optimizations to be *contributed* rather than special-cased in the planner —
  and keeps a portable project from silently depending on a primitive only one engine has.
- **The `write:` pin is an open, fail-loud vocabulary — not a fixed keyword set.** Because pins
  name patterns and patterns are extensible, `write:` cannot be a sealed
  `region|keyed|column|update` enum in the surface; it is an **open name resolved against the
  registry**. An unrecognised pin, or one naming a pattern the target backend can't provide, is
  **refused with a diagnostic** (fail-loud discipline), never silently downgraded to a default.
  This is the same "validator, not chooser" stance applied to the naming layer: the surface admits
  new pattern names the moment a backend registers them, and rejects everything it cannot honour.

Net: the enum is a **snapshot of a registry**, the admission rule + equivalence gate + capability
factor are the **contract**. New write patterns are expected, plug in without reopening the
grain/contract reframe, and each carries its own correctness proof — so growth costs a registry
entry, not a redesign.

## Blast radius (spec edits this design drives)

- **`incremental_models.md`** — rewrite §Surface "declared shape axis" and the addressing framing;
  demote grain to derived label; add the available-addressings rule and the `write:` pin; recast
  the "two axes are orthogonal" table as the actual declaration; fold `key_per_partition` and
  locality routes into the partition∈key mechanism. Frame the write-pattern set as an **open
  registry** (non-exhaustive enum), state the admission-function-not-enumeration invariant, and
  document `write:` as an open, fail-loud pin name rather than a sealed keyword set.
- **`architecture.md`** — the write-pattern **capability registry** (backend-filled) and the
  fourth admission factor (backend capability) are an architectural extension point; note who owns
  the registry and that unsupported/unrecognised patterns fail loud, consistent with the
  maintenance-plan-purity invariant (backends execute registered patterns, never author).
- **`models.md`** — refresh axis keeps its trichotomy; grain moves from declared selector to
  derived label + check-only assertion; litmus-rule wording updated to name addressing as
  derived-per-cell.
- **`sources.md`** — restructure world-facts onto the shared Relation Contract slot names.
- **`timeseries.md`** — the clock slot is already the shared shape; cross-reference the contract.

## Rejected / superseded alternatives

- **Fully per-cell grain with no declared shape anchor** — reopens the silent consumer-semantics
  flip the declaration law exists to prevent. Rejected: the shape-defining facts (clock, identity)
  stay declared-and-checked.
- **Keep `grain` a declared pure-shape label** (approach C) — smaller diff, but keeps a model-only
  token instead of unifying with sources; under-delivers on the shared-vocabulary goal.
- **Explicit full `output:` world-fact block mirroring sources 1:1** (approach B) — maximal
  symmetry, but invites re-declaring derivable facts and adds surface. The shared *named slots*
  give the unification without forcing a redundant block.

## Open questions

- Exact field-path reconciliation between today's source `mutation_profile:` and the shared
  `mutation:` slot (naming, defaults, the trust-rule annotations) — to settle during the
  `sources.md` edit.
- Whether the derived `grain` label is worth surfacing in `smelt explain` for sources as well as
  models, or models only.
- Whether `write:` is a distinct pin or a refinement of `technique:` (the four techniques already
  imply an addressing; `write:` may be redundant for all but the region-vs-keyed ambiguity).
- Where the **write-pattern capability registry** lives and how a backend contributes a new
  pattern: the shape of a pattern's declaration (required contract facts + equivalence proof
  obligation + backend-capability predicate), and whether new patterns can be registered
  out-of-tree or must land in `smelt-logical`'s maintenance layer to keep the emitter single-owner.
