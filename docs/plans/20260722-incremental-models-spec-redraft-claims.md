# Claim inventory — incremental_models.md redraft verification oracle

Scaffolding for `docs/plans/20260722-incremental-models-spec-redraft.md` (Phase 1). Extracted
from `docs/specs/incremental_models.md` at commit `135c307a` (the last commit before the
redraft, on branch `spec-redraft-incremental-models`). Line numbers refer to THAT revision.

Phase 5 verifies every entry against the redrafted spec (verdict: preserved / weakened / lost /
strengthened; anything not `preserved` is fixed or justified in the plan). Deleted in the
follow-up sweep PR once verification passes.

Notes:
- Extraction ranges overlap slightly at seams (A/B at ~541–545, B/C at ~957–965); duplicated
  claims across a seam are expected and harmless.
- Section E classifies Known-Divergences entries (LANDED/LIVE/MIXED) to drive the Phase 4
  pruning; LANDED evidence is verified against gates/tests before deletion (plan Appendix B).
- Section F item on "Conditional maintenance" contains built-status claims inside a Future
  Extension; the redraft relocates those (they are not not-decided ideas).

# Claim inventory A — incremental_models.md lines 1–545 (header blockquote + ## Surface + §"The equivalence invariant" opening)

### A1 (lines 10) [Header blockquote]
This file is the **single normative spec** for maintained models and owns, in order: (a) the maintenance contract every maintained (non-`full`) model upholds (equivalence invariant, algebraic ladder, windowed-scan/horizon contract, validator-not-chooser, composition contract); (b) the derived per-model **maintenance plan**, a matrix indexed by `(output-column-group × trigger)` whose cells choose maintenance techniques; (c) the **graph layer** (forward propagation and backward resolution); (d) the **shape profiles** of `refresh: incremental`: `grain: partition`, `grain: key`, and `versioning: interval` (SCD2).

### A2 (lines 12) [Header blockquote]
A modeller declares `refresh: incremental` plus the output's **shape-defining facts** — clock (`timeseries:`) and/or identity (`unique_key:`) — and everything else (the `grain` label, technique, physical write addressing, clamps, windows, ledgers, propagation edges) is *derived* per `(column-group × trigger × changed-input)` cell. The shapes are not competing modes; they share one invariant, one plan machinery, one graph layer.

### A3 (lines 12) [Header blockquote]
Physical write addressing (region rewrite vs merge by key) is **not** a model-wide verdict: one model can be region-addressed with respect to its main fact table yet keyed-addressed when a different input changes.

### A4 (lines 12) [Header blockquote]
This spec supersedes and retires four earlier specs: `maintenance_plan.md`, `batched_models.md`, `keyed_models.md`, `versioned_models.md`.

### A5 (lines 14) [Header blockquote]
Ownership carve-outs (out of scope here, with named homes): provable SQL properties → `model_properties.md`; physical transform mechanisms → `model_transforms.md`; the `refresh:` axis, declaration law, and litmus rule → `models.md`; source world-fact declarations → `sources.md`; `event_time_column`/`partition_column`/`granularity` declaration → `timeseries.md`; engine-owned maintenance → `materialized_view.md` (the one shape profile that stays a separate spec, because the engine, not smelt, is the maintainer); backend capability flags → `multi_backend.md`.

### A6 (lines 16–18) [Header blockquote]
Process rules: spec-first (edit this file before the implementation plan; the spec diff is the change description) and timeless-oracle (implementation status lives only in §Known Divergences or §References → Plans).

### A7 (lines 20) [Header blockquote — Status]
`refresh: incremental` + `grain: partition` is the live surface. `refresh: batched` is a **hard error with a fix-it**; the `.sql` frontmatter `batched:` sub-block is retired outright — a **hard error with a per-key fix-it** — while the `smelt.yml` model-override spelling of `batched:` still parses (Known Divergence).

### A8 (lines 20) [Header blockquote — Status]
Implementation status claims: partition grain's DuckDB DELETE+INSERT path implemented and tested; key grain's additive-fold and extremal/lattice-fold families implemented against the windowed-keyed-maintenance driver; overwrite, once-write, plain-overwrite families, the snapshot-reconcile run shape, and the time-partitioned output are specified ahead of implementation; the transactional merge ledger is DuckDB-only; `versioning: interval` is specified ahead of implementation and **does not parse today**.

### A9 (lines 26–34) [The declared shape axis]
The entire declared shape surface of an incremental model is exactly: the two shape-defining facts of the Relation Contract (`models.md` §"The Relation Contract") — clock and identity — plus the optional `versioning: interval` sub-declaration. Grammar: `refresh: incremental`; `timeseries: { ... }` (the clock); `unique_key: [ ... ]` (the identity, including whether `partition_column` is a member); `versioning: interval` (optional; requires identity + no model clock; SCD2); `grain: partition | key | key_per_partition` (optional, CHECK-ONLY, drives nothing).

### A10 (lines 36–38) [The declared shape axis]
Corner 1 — clock, no identity → **partition-addressed table**, derived `grain: partition`, one row per `(partition_column, …)`, a complete table whose *default* cell addressing is region rewrite (DELETE+INSERT).

### A11 (lines 39) [The declared shape axis]
Corner 2 — identity, no clock → **bare keyed state**, derived `grain: key`: one row per `unique_key`, read in full by consumers, kept current by folding deltas into stored state (keyed `merge_into`). One profile covers the running-aggregate, latest-value, and milestone patterns; what distinguishes them is each projection's **column family**, derived from the SQL, never declared.

### A12 (lines 40) [The declared shape axis]
Corner 3 — clock + identity with `partition_column` ∉ key → **keyed state with a home slice** (derived `grain: key`, time-partitioned): one row per key, each key's partition value a fixed per-key constant; **admitted iff key temporal locality is established**.

### A13 (lines 41) [The declared shape axis]
Corner 4 — clock + identity with `partition_column` ∈ key → the **trajectory**, derived `grain: key_per_partition`: one row per `(key, partition)`; the natural key recurs across partitions.

### A14 (lines 42) [The declared shape axis]
`versioning: interval` attaches to the identity-no-clock corner: keyed state **plus history** — every version of a key kept, stamped with a non-overlapping validity interval (SCD2). It requires identity and **no model clock**: a `timeseries:` block on the model is a **hard error** (the close-out escapes every time window). It is deliberately not a shape/grain of its own — row addressing stays by key; the interval is structure within the key.

### A15 (lines 44) [The declared shape axis]
The `refresh:` axis itself (including `full`, `materialized_view`) and the declaration law are **owned by `models.md` §"Refresh axis"**; this spec covers only the `incremental` shapes.

### A16 (lines 44) [The declared shape axis]
Declarations name **shape-defining facts only**. Which technique realizes which part of the output, and how it physically addresses rows, are properties of `(column-group × trigger × changed-input)` cells, never of the model as a whole; the machinery **validates the declared facts rather than choosing them** (validator, not chooser).

### A17 (lines 46–48) [Grain is a derived label]
`grain` is **not declared as a driver**: it is a derived classification computed from `(clock?, identity?, partition_column ∈ key?)`, reported by `smelt explain`, and computed for **sources too** (a source also has an effective grain). A modeller may write it only as a **check-only assertion** (like `maintenance.scan_bounds`): it **errors on mismatch** with the derived facts (`models.md` §"Constraint violations") and drives nothing.

### A18 (lines 48) [Grain is a derived label]
The single fact `partition_column ∈ unique_key` distinguishes a trajectory (`key_per_partition`) from a keyed lookup with a fixed home slice. Key temporal locality's **route 1** ("`partition_column` is a `unique_key` column") *is* the partition-∈-key case; **route 2** ("partition is a per-key constant, functionally dependent on the key") *is* the partition-∉-key case.

### A19 (lines 52–61) [The two axes are orthogonal]
The two shape-defining facts vary independently (orthogonal). The keyless, clockless combination has **no maintainable shape**. The inhabited combinations and derived labels: no-identity + clock → `grain: partition`; identity + clock → `grain: key` (time-partitioned, locality-admitted, `partition_column` ∉ key) or `grain: key_per_partition` (`partition_column` ∈ key); identity + no clock → `grain: key` bare.

### A20 (lines 63–68) [The two axes are orthogonal]
A model with both a key and a partition axis is a **first-class shape**; several capabilities exist **only** in that composed form. Both axes are orthogonal to **input consumption**: a bare keyed model over a clocked source still consumes window-forward; a composed model's *output* clock is a property of its own stored shape, not of its sources.

### A21 (lines 70–74) [The two axes are orthogonal]
Normative correction rule: any text — in this spec, sibling specs, research, or plans — that frames "partitioned" and "keyed" as mutually exclusive alternatives or as disjoint populations **is wrong and must be corrected against this section**.

### A22 (lines 76–78) [The composition contract]
The composition contract is **system surface**: its callers are the shape profiles (the shape sections of this spec and `materialized_view.md`) and the planner/analysis layer, not the modeller directly.

### A23 (lines 80–85) [The composition contract]
A maintained model is a **composition** of: **Properties** (proven/declared facts about the SQL — `model_properties.md`); **Transforms** (physical mechanisms a property licenses — `model_transforms.md`); **Output shape** (declared via `grain:` per `models.md` §"Refresh axis": partition-addressed, or key-addressed, optionally time-partitioned under key temporal locality); and **Scope maps**.

### A24 (lines 85) [The composition contract]
Definition — **scope map**: for each input of a model, the derived mapping from that input's delta to the affected output addresses and the transform that runs for it. Fixed dispatch: driving source's delta → windowed fold; mutable dimension's delta → delta-driven probe + dimension-driven horizon-bounded MERGE; self-edge → ordered execution; model-definition diff → targeted column backfill (all in `model_transforms.md`). Which map applies follows from input-delta discovery (`model_properties.md`) and the input's declared world-facts (`sources.md`); a **run is the union of its inputs' scope maps**, and the per-input answer is surfaced by `smelt explain`.

### A25 (lines 87) [The composition contract]
Every shape profile **must** present a **composition table** stating the properties it requires, world-facts it consumes, transforms it drives (differentiated per input class — its scope maps), and its output shape. A profile's normative content is exactly (a) that table, referencing shared capabilities **by name**, plus (b) its own local machinery defined in full. A profile **must not re-specify** a capability owned by a capability spec or a shared section.

### A26 (lines 89–106) [The plan (derived, reported)]
Every non-`full` model has a **maintenance plan**: a set of **cells**, each keyed by `(output-column-group × trigger × changed-input)`, carrying: the **corner** of the read-scope × write-scope 2×2 (write-scope = the cell's physical write addressing, `{targeted addresses, region-overwrite}`); the **technique** (e.g. `DELETE`+`INSERT` region recompute, keyed fold `MERGE`, column-scoped `MERGE`, in-place `UPDATE`) drawn from the **open write-pattern registry**; the **write mechanism** (derived by the available-addressings rule, or a validated user `write:` pin); the **derived scan clamps** — per read source, the `(partition_col, before, after)` window anchored to the output region; the **partition-locality verdict** per source; and the cell's **obligations and traded guarantees** (per-column, two-dimensional: equivalence contract × settle bound).

### A27 (lines 107–110) [The plan (derived, reported)]
The plan is **derived, never declared**. What stays declared is the model's shape-defining facts (clock and identity, `models.md`), which are **validated against the plan — an error on mismatch, never a silent flip**; the `grain` label is derived from those facts.

### A28 (lines 109–111) [The plan (derived, reported)]
`smelt explain` prints the plan: every cell, its addressing, clamps and locality verdicts, the per-column guarantee ledger, and — at the graph level — the model's inbound edges.

### A29 (lines 114–120) [Triggers]
Four trigger classes index the plan's columns, with these definitions: **creation** — new rows arrived in the driving source; **mutation** — a post-creation delta in a source some column group is mutation-sensitive to; **definition change** — the model gained output fields while sources stood still; **backfill** — an explicit region recompute from replayable input.

### A30 (lines 122–127) [Triggers]
Each trigger is paired with the **changed-input** it fires for (a specific source, self-edge, or definition diff) — the third axis of the plan's cell key. The same column group under the same trigger class can derive **different** physical write addressing for different changed inputs (e.g. creation delta on the driving fact rewrites/folds a region; mutation delta on a dimension merges by key). The scope maps are this axis's per-changed-input dispatch.

### A31 (lines 129–137) [Upstream model edges]
A maintained model's ref to another maintained model **in the same project** is a plan edge of the **same standing as a `sources.*` ref**. The upstream model's own `timeseries:` declaration supplies the event-time clock the downstream creation-trigger cell is clamped by; scan bounds compose through the chain exactly as the propagation graph composes them.

### A32 (lines 137–139) [Upstream model edges]
An upstream-model ref whose clock cannot be derived (upstream declares no `timeseries:` and none is inferable) is a **recorded refusal** on that cell — `MaintenanceReachNotDerivable`, naming the edge — never a silent drop.

### A33 (lines 139–141) [Upstream model edges]
A ref to a `full`-mode or view upstream derives **no creation cell** (no incremental delta to receive); it participates in mutation/backfill triggers only.

### A34 (lines 143–145) [Upstream model edges]
For forward propagation, `--source <address>` accepts either a declared source **or** an upstream maintained model; a model's landed delta is the output window a completed run wrote for it.

### A35 (lines 148–160) [Frontmatter]
Surface grammar — `maintenance:` block: `defaults.prefer: recompute | fold | suppress | unconditional | auto` (per-model soft default; `auto` = cost model). `cells[]` entries carry: `columns: [<col>, ...]` (names any member of a derived column group); `on: <source-address> | backfill` (the trigger + changed-input the cell handles); `prefer: fold | recompute | suppress | unconditional` (soft per-cell bias; cost model still refines); `technique: fold | recompute | rederive_columns | suppress | unconditional` (hard per-cell pin, bypasses the cost model); `write: <pattern>` (optional hard per-cell addressing pin; OPEN name resolved against the write-pattern registry, e.g. `region | keyed | column | update`; unknown or backend-unavailable → refused).

### A36 (lines 161–168) [Frontmatter]
Surface grammar — `maintenance.scan_bounds`: `require: partition_local | none` (**default: `partition_local`**); `on_violation: error | warn` (**default: `error`**); `per_source.<source-address>.max_lookback: '<interval>'` (ceiling on the derived scan span for that source); `per_source.<source-address>.allow_full_scan: true` (named acceptance of a full read of that source).

### A37 (lines 170–171) [Frontmatter]
The override ladder is `defaults.prefer` → `cells[].prefer` → `cells[].technique`, narrower scope winning; `technique:` alone bypasses the cost model.

### A38 (lines 172–178) [Frontmatter]
`suppress`/`unconditional` are an orthogonal dimension from `fold`/`recompute`: they never change which technique family a cell resolves to, only whether a suppressible cell's (`ColumnScopedMerge`/keyed fold) matched arm writes conditionally. `technique: suppress` on a cell whose write-suppression proof did not hold (no proven row identity, or a compared column not proven comparable across runs) is **refused** the same way a family pin naming an unadmitted technique is; `technique: unconditional` **never refuses**.

### A39 (lines 179–185) [Frontmatter]
`cells[].write` is a hard per-cell addressing pin resolved as an **open name** against the write-pattern registry (not a sealed keyword set). Every pin is **validated against the equivalence invariant** for its cell — an addressing that cannot uphold equivalence is refused with a diagnostic, never silently honoured — and an unrecognised name, or one the target backend cannot execute, is **refused fail-loud, never silently downgraded**.

### A40 (lines 186–187) [Frontmatter]
`cells[].columns` naming columns that span two derived column groups is an **error** (it would silently re-partition the plan).

### A41 (lines 188–190) [Frontmatter]
`scan_bounds` is **check-only**: it never modifies a clamp; it only refuses (or warns) when the derived plan exceeds the stated expectation. A project-level default in `smelt.yml` sets the baseline; per-model blocks refine it.

### A42 (lines 191–193) [Frontmatter]
A sibling **top-level** frontmatter key `horizon_ceiling: '<interval>'` (**partition grain only**) declares a ceiling on the derived horizon — a **compile-time warning threshold, never a clamp modification**.

### A43 (lines 197–202) [Partition-grain declaration]
The partition-grain profile's default plan corner is recompute-a-region per touched partition, driven by DELETE+INSERT — **not a mode the modeller selects**: technique is a per-`(column-group × trigger)`-cell property, never model-wide. (Historical name: "batched".)

### A44 (lines 204–216) [Partition-grain composition]
The partition-grain composition table binds: output shape `grain: partition` (complete table with a monotone `partition_column`, addressed by partition, not key — home `models.md`); required properties (event-time monotonicity trace, column nullability gate, unified bound/reach derivation, frame-reach taxonomy, injection-point/pushdown-depth, scoped partition alignment, driving-fact/anchor resolution, determinism run-vs-row + nondeterminism predicate + taint, body-structure classifier, set-operation distribution, static-seed detection, window-independence/ordered-execution — home `model_properties.md`); consumed world-facts (timeseries clock, source mutation profile + lateness margin, per-column `columns.<c>.contract`); default-plan transforms (source-filter pushdown, partition DELETE+INSERT, output-window derivation with partition-column skew inversion, outer output-clamp, two-layer widened-scan + exact output clamp, compile-time pinning — home `model_transforms.md`); admission = §"Per-cell admission" instances for the recompute corner; invariant = per-partition equivalence.

### A45 (lines 217) [Partition-grain composition]
The partition-grain profile's local (profile-owned) machinery is: batch-safety roll-up, column-locality of the equivalence, event-time outer-visibility, backfill chunking, run/partition granularity alignment, and the partition-grain surface (`grain: partition`, `timeseries:` requirement, `safety_overrides`, per-source-clamp observability).

### A46 (lines 219–243) [Partition-grain frontmatter]
Partition-grain `.sql` frontmatter surface: `refresh: incremental`; optional check-only `grain: partition`; `timeseries:` block with `event_time_column`, `partition_column`, `granularity`; optional `safety_overrides:` with boolean sub-keys `allow_window_functions`, `allow_having`, `allow_subqueries` (each bypassing a specific safety check); optional `columns.<name>.contract: plausible`.

### A47 (lines 243) [Partition-grain frontmatter]
`refresh: incremental` + a `timeseries:` clock + **no declared identity** is the opt-in for the partition shape; the stored `table` materialization is implied (not restated). A written `grain: partition` is the check-only assertion of the shape the facts already fix.

### A48 (lines 243, 325, 539) [Partition-grain frontmatter / Key-grain frontmatter / Diagnostics]
`safety_overrides` is a top-level frontmatter key (`models.md` §"YAML frontmatter keys") **admitted only on a partition-shaped output**; on a key-addressed output (`grain: key`, or once a `unique_key` reshapes the model) it is a **hard error** (`models.md` §"Constraint violations"). (Stated at lines 243, 325, and 539.)

### A49 (lines 243) [Partition-grain frontmatter]
Declaring a `unique_key` on a partition-shaped model does **not** add a "dedup aid": it declares identity, which reshapes the output to the composed clock-and-identity keyed corner. A model that wants only whole-partition rewrites declares no identity.

### A50 (lines 245) [Partition-grain frontmatter]
Missing the `timeseries:` block on a model asserting `grain: partition` is a **hard error** (rule home: `models.md` §"Constraint violations"; code `TimeseriesRequiredForPartitionGrain` per §Diagnostics).

### A51 (lines 245) [Partition-grain frontmatter]
The declared `partition_column` **must be monotone**, validated by the event-time monotonicity trace (`model_properties.md`). Monotone admits either a timestamp or an ever-increasing integer (sequence id / offset / watermark): a constant shift over such a column (`batch_id + 5`, `batch_id - 5`) is recognised on the same footing as a constant `INTERVAL` shift over a timestamp, while a non-monotone integer transform (`batch_id % n`, `batch_id * n`) is **rejected fail-closed, naming the construct**.

### A52 (lines 247) [Partition-grain frontmatter]
`columns.<c>.contract` is declared per `models.md` §"`columns:` — column metadata" but its **semantics are owned by this spec**: `contract: plausible` exempts that output column from the determinism requirement (replacing the pre-cut `nondeterministic_columns` list). Listing `event_time_column`, `partition_column`, or a `unique_key` column as `plausible` is a **configuration error** — a skeleton position must be deterministic.

### A53 (lines 249–263, 353–363) [smelt.yml overrides — both grains]
The same declaration keys may be set per model in `smelt.yml` under `models.<name>`; **frontmatter wins over `smelt.yml` when both set the same field**. (Stated for partition grain at 251 and key grain at 363; the key-grain restatement adds that the same `timeseries:`-admission constraint applies to the override spelling.)

### A54 (lines 264–266) [Granularity values]
The granularity enum is a **closed enum owned by `timeseries.md`** §"Granularity values"; this profile consumes the granularity declared in the model's `timeseries:` block.

### A55 (lines 268–279) [Strategy enum (backend-internal)]
Strategy is **not declared on the model** — it is derived per cell. For the recompute corner, backends pick a physical strategy from model config and their capabilities, from the internal enum `IncrementalStrategy { DeleteInsert, Append, InsertOverwrite }`. DuckDB currently always uses `DeleteInsert`.

### A56 (lines 280) [Strategy enum (backend-internal)]
A partition-shaped output's creation/backfill cells are **region-addressed** (DELETE+INSERT); UPSERT (`MERGE`) is not the addressing of those cells, and a pure partition grain (no declared identity) has **no keyed addressing at all**. Keyed `MERGE` is the addressing a *dimension-change* cell derives on a composed clock-and-identity output (declared `unique_key`), scoped to the touched partitions — `MERGE` is per-cell, driven by what changed, not tied to one grain.

### A57 (lines 282–289) [Key-grain declaration]
The key-grain profile: stored table is keyed state, one row per `unique_key`, kept current by the derived per-cell plan rather than a declared strategy. One profile covers running-aggregate, latest-value, and milestone/retroactive-enrichment patterns; the distinguishing **column family** of each projection is derived from the SQL, **never declared**. (Historical names: "keyed", "cumulative".)

### A58 (lines 291–302) [Key-grain composition]
The key-grain composition table binds: output shape `grain: key` (end-state per key, addressed by `unique_key`, not partition); required properties (algebraic discriminants — is-monoid / needs-inverse / decomposable / value-vs-order-monotone — which define the column families; driving-fact/anchor resolution, the single clocked source under window-forward; event-time monotonicity trace of the driving source's clock; once-write provenance for the `COALESCE` family; join-contribution monotonicity for enrichment joins; input-delta discovery; key temporal locality for a time-partitioned output); consumed world-facts (driving source's timeseries clock; source mutation profile; a declared **key-recurrence bound** from `sources.md` where the recurrence-bounded locality route is declared rather than derived); default plan (keyed `merge_into` target-as-replica sequenced by the windowed-keyed-maintenance driver, source-filter pushdown on the driving source, the transactional merge ledger, dimension-driven horizon-bounded MERGE for enrichment shapes, slice-pruned merge target under established locality); admission = §"Per-cell admission" instances for the fold-a-delta corner (§"Admission matrix"); invariant = end-state equivalence with the model's **own SQL as oracle**.

### A59 (lines 304) [Key-grain composition]
The key-grain profile's local machinery is: the column-family catalogue, the derived execution postures, the transactional merge ledger, the two run shapes, the key-temporal-locality routes for the time-partitioned output, and the key-grain surface (`grain: key`, `timeseries:` admission, the classifier).

### A60 (lines 325) [Key-grain frontmatter]
`refresh: incremental` + a declared `unique_key` (with **no clock, or a clock admitted under key temporal locality**) is the opt-in for the key shape; the stored `table` is implied (the modeller does not restate `materialization: table`). A written `grain: key` is the check-only assertion of the shape the identity fact already fixes.

### A61 (lines 325) [Key-grain frontmatter]
`unique_key` must **restate the `GROUP BY` column list**; the classifier checks the two agree. No rule-specific config block is read or required for the key shape.

### A62 (lines 327) [Key-grain frontmatter]
By default the key-grain output carries **no partition column**. A model **may** declare a `timeseries:` block to time-partition its keyed output — admitted **iff key temporal locality is established**, refused otherwise with `KeyedForbidsTimeseries` naming the missing route.

### A63 (lines 327) [Key-grain frontmatter]
Output partitioning is independent of event-time-aware **consumption**: a key-grain model over a source carrying a `timeseries:` declaration consumes that source window-forward whether or not its own output declares a clock.

### A64 (lines 327) [Key-grain frontmatter]
`grain: key_per_partition` is a **different grain**, not a sub-declaration of `grain: key` — it stores the per-partition trajectory, not the end-state the key-grain profile maintains.

### A65 (lines 329–349) [Key-grain frontmatter]
The time-partitioned key-grain form's flagship shape is event-grain dedupe over a bounded redelivery window, where the driving source declares `key_recurrence` (`sources.md`); in that form the output `timeseries:` columns (e.g. `first_seen_at`/`first_seen_date`) are themselves extremal-fold projections.

### A66 (lines 351) [Key-grain frontmatter]
The key-grain body **must** be an aggregated `GROUP BY` query: `unique_key` is the `GROUP BY` column list and every non-key projection must classify into exactly one column family. A bare, un-aggregated projection is **not** a key-grain model — the SQL must itself express the per-key semantics so that a full refresh of the SQL is the profile's correctness oracle.

### A67 (lines 365–367) [The column-family catalogue]
The classifier assigns each non-key projection to **exactly one column family**. The family fixes the cross-window combiner — a lookup off the aggregator; **authors never declare combiners** — and every derived property.

### A68 (lines 369–371) [The column-family catalogue]
Family **additive fold**: aggregators `COUNT(...)`, `SUM(...)`, `BIT_XOR(...)`; combiner `+`/`xor`; **not idempotent** (not re-run safe); order-independent; **invertible**; run shape **window-forward only**; extra licence: **ledger-enforced re-run refusal**.

### A69 (lines 372) [The column-family catalogue]
Family **extremal / lattice fold**: aggregators `MIN`, `MAX`, `BOOL_AND`, `BOOL_OR`, `BIT_AND`, `BIT_OR`; combiners `LEAST`/`GREATEST`/`AND`/`OR`/`&`/`|`; idempotent; order-independent; **not invertible**; window-forward only.

### A70 (lines 373) [The column-family catalogue]
Family **order-monotone overwrite**: aggregators `MAX_BY(value, ordering)`, `MIN_BY(value, ordering)`; combiner max/min-by-ordering (ties per §"Ordering ties"); idempotent; order-independent **up to ordering-key ties**; not invertible; window-forward only.

### A71 (lines 374) [The column-family catalogue]
Family **once-write**: `COALESCE`-first-non-null over the group; combiner `COALESCE(target, delta)`; idempotent; order-independent **given the proof**; not invertible; window-forward only; extra licence: the **once-write provenance proof** (`model_properties.md`) — key-derived, or a declared functional dependency.

### A72 (lines 375) [The column-family catalogue]
Family **plain overwrite**: aggregator `ANY_VALUE(...)`; combiner "incoming row wins"; idempotent; run shape **snapshot-reconcile only**.

### A73 (lines 377) [The column-family catalogue]
Any other aggregate, any non-aggregate non-key expression, and any composite expression over aggregates (e.g. `SUM(x) + 1`) is **rejected** (`KeyedUnknownCombiner`); the remedy is to add columns for the underlying aggregates and derive downstream.

### A74 (lines 379) [The column-family catalogue]
Pattern functions: `smelt.latest(value, ordering)` → `MAX_BY`; `smelt.once(value)` → the once-write canonical spelling; `smelt.current(value)` → `ANY_VALUE`. They are ordinary transparent functions (`functions.md`) whose expansions are admitted on **exactly the same terms** as hand-written calls.

### A75 (lines 381–386) [Interval-versioned declaration]
The SCD2 profile is `refresh: incremental` + declared `unique_key:` + `versioning: interval`, with **no model clock** (derived `grain: key`): keyed state plus history — every version of a key kept, each stamped with a non-overlapping validity interval. Deliberately **not a third grain**: row addressing is still by key; the interval is structure within the key. (Historical name: "versioned".)

### A76 (lines 388–399) [Interval-versioning composition]
The SCD2 composition table binds: output shape (identity + `versioning: interval`, no model clock, derived `grain: key`); required properties (algebraic monotonicity/ordering discriminants for tracked-attribute change detection; event-time monotonicity trace — **validity is stamped from source event-time, never the run clock**; driving-fact/anchor resolution; window-independence/ordered-execution — the combiner reads versions in event order); consumed world-facts (the timeseries clock of an update-events/CDC feed **or** a mutable snapshot's source mutation profile — **one of the two, derived from the source's shape, never declared on the model**); default plan (keyed `merge_into` via the windowed-keyed-maintenance driver, source-filter pushdown, folding through the profile-local close-old/open-new interval combiner); admission = §"Interval-versioning admission"; invariant = end-state equivalence in its **interval-keyed specialisation** — the visible set of `(key, version, validity interval)` rows equals a full rebuild from the same processed snapshots, independent of merge order.

### A77 (lines 401) [Interval-versioning composition]
The SCD2 profile's local machinery is: the close-old/open-new combiner, the smelt-managed validity columns, tracked-attribute selection, event-time stamping, and deletion handling.

### A78 (lines 420) [Interval-versioned frontmatter]
`versioning: interval` is admitted **only** where the output declares identity (`unique_key:`, the key-shaped corners — `models.md` §"Constraint violations") and is a **hard error together with a `timeseries:` block on the model itself**. This forbids output partitioning, **not** event-time-aware consumption: a `versioning: interval` model over a clocked source (update-events/CDC feed) consumes it window-forward.

### A79 (lines 422) [Interval-versioned frontmatter]
The model's SELECT projects the natural key and the tracked attribute columns as they are *now*; **smelt maintains the version history**: each `smelt build` compares incoming rows against the stored current version per key and, where a tracked attribute changed, closes the prior version and opens a new one.

### A80 (lines 424–426) [Interval-versioned output shape]
The SCD2 stored table carries the projected columns plus **smelt-managed** validity columns — a `valid_from`/`valid_to` interval and an `is_current` flag (**exact names/types are an Open Question**). A key with three successive states yields three rows: two closed intervals and one open (`valid_to` NULL/sentinel, `is_current = true`).

### A81 (lines 430–433) [CLI]
`smelt explain <model>` prints the plan (cells, clamps, locality, guarantee ledger, edges). With `--show-sql`, it additionally prints each cell's emitted maintenance statements — **the same emitters' output a run executes** (§"Statement emission (single owner)"); the flag surface itself is owned by `cli.md`.

### A82 (lines 434–440) [CLI]
`smelt run --since-upstream --source <address> --landed <start>..<end>` (`--source`/`--landed` repeatable, one pair per source) is forward propagation: the caller declares what landed per source; the graph reflects those declared per-source deltas through the edges and runs **exactly** the propagated per-edge regions with their trigger cells. **No per-invocation delta is computed automatically** — a `--source` without a matching `--landed` delta propagates nothing for that invocation. The mode is opt-in (intended default posture once trusted) and **prints the dirty set before acting**.

### A83 (lines 441–442) [CLI]
`smelt build <model> --period <start>..<end> --include-upstreams` is backward resolution: print the per-ancestor required slices and build order; optionally execute the bounded build.

### A84 (lines 443–455) [CLI]
`smelt bakeoff <model>` measures every admissible technique for a set of cells against a representative window of real data and reports cost. `--cells` defaults to **every cell with two or more admissible techniques** (a single-technique cell has nothing to bake off). `--runs N` (**default 3**) splits the driving source's event-time extent into `N` sequential windows replayed in order per technique — each replay a **real `execute_project` run** against the project's actual data, not a synthetic sample. Each technique runs against a scratch target cloned in-memory under schema `smelt_bakeoff_<model>_<technique>` (schema flows from `config.targets[target].schema` — no runtime schema seam), dropped after measurement unless `--keep`.

### A85 (lines 452–455) [CLI]
After each bakeoff window, the measured techniques' outputs are cross-checked against each other with `EXCEPT ALL` **in both directions** — the equivalence invariant is verified rather than assumed. `--target` selects which declared target to clone (defaults to the active target).

### A86 (lines 456–460) [CLI]
`smelt bakeoff --pin` emits the winning `cells[]` entry (or a complete `maintenance:` block when the model has none) as ready-to-paste YAML **on stdout** — it **never rewrites the model's `.sql` file**; the user reviews and commits the pin. An applied pin is an ordinary override **re-validated through admission on every compile**: an inadmissible pin fails loud rather than silently running.

### A87 (lines 462–466) [CLI]
`cells[].technique` pins and `defaults.prefer`/`cells[].prefer` are honoured at execution — the same choice ladder (`resolve_cell_choice`/`effective_override`, §"Validator, not chooser") that governs bakeoff's measurement targets also resolves a live run's technique — and **admission still binds: an override can never select an inadmissible technique**.

### A88 (lines 468–475) [Partition-grain run flags]
`--event-time-start` and `--event-time-end` are **both required** for any direct (`--event-time`-driven) partition-grain `smelt run`/`smelt backbuild`; a forward-propagation run (`--since-upstream`) instead derives its regions from the supplied `--landed` intervals. Format: ISO-8601 (`2026-03-20`, `2026-03-20T00:00:00Z`).

### A89 (lines 476) [Partition-grain run flags]
The end bound is **exclusive**: `--event-time-end 2026-03-25` does not include `2026-03-25`.

### A90 (lines 477) [Partition-grain run flags]
The supplied `[start, end)` range is the **run window**; it must be a positive integer multiple of `timeseries.granularity`, aligned to granularity boundaries (`timeseries.md` §"Granularity arithmetic"). Run-window size may exceed partition granularity.

### A91 (lines 478) [Partition-grain run flags]
`smelt backbuild` uses the model's **classified batch safety** (§"Batch safety classification") to expand or split the requested range.

### A92 (lines 480–490) [Key-grain run flags]
Which key-grain run flags apply is determined by the model's derived **run shape**. Under **window-forward** (clocked driving source): both event-time flags are required and apply to the **driving source's** `partition_column`/`granularity` — never to any column on the keyed output, including an admitted output `timeseries:` block (**run flags always address the source's clock**); format/alignment rules follow the partition-grain flags.

### A93 (lines 491) [Key-grain run flags]
Under **snapshot-reconcile** (no clocked source), the event-time flags are a **hard error** with the message *"model has no clocked driving source; run without event-time flags"*; each run is a whole reconciliation.

### A94 (lines 495–499) [Diagnostics]
All diagnostic codes are **catalogued in `diagnostics.md`; this spec owns their semantics**. Partition-grain rejections: `TimeseriesRequiredForPartitionGrain` (missing `timeseries:` block; the rule lives in `models.md` §"Constraint violations"), `PartitionGrainNotSafe` (the batch-safety classifier), `EventTimeColumnNotVisibleAtOuterSelect` (event-time outer-visibility, partition-grain-local).

### A95 (lines 503) [The `Maintenance*` family]
`MaintenanceNoAdmissibleTechnique` — no technique survives a cell's admission; names the cell.

### A96 (lines 504) [The `Maintenance*` family]
`MaintenanceReachNotDerivable` — a required scan bound is neither derivable nor declared. (Also the recorded-refusal code for an upstream-model ref with underivable clock, per A32.)

### A97 (lines 505–506) [The `Maintenance*` family]
`MaintenanceScanUnbounded` — the K8 guardrail: a scan/footprint cannot be partition-bounded (or exceeds a declared `max_lookback`) and no `allow_full_scan` acceptance exists.

### A98 (lines 507–508) [The `Maintenance*` family]
`MaintenanceUnboundedFootprint` — a targeted write was requested for a cell whose write footprint is unbounded (e.g. a stored trajectory under late data).

### A99 (lines 509–510) [The `Maintenance*` family]
`MaintenanceSkeletonColumnAdded` — a field was added in a skeleton position: a grain change, **refused** as a column backfill.

### A100 (lines 511–512) [The `Maintenance*` family]
`MaintenanceGraphUnsupportedNode` — a keyed-grain or self-referential node in the propagation graph; **refused fail-loud**.

### A101 (lines 513–516) [The `Maintenance*` family]
`MaintenanceWriteAddressingRefused` — a `maintenance.cells[].write` pin names an addressing that cannot uphold the cell's equivalence invariant (e.g. keyed on an output with no identity, or a region write on a cell whose footprint escapes any partition set); names the cell and the refused pattern.

### A102 (lines 517–519) [The `Maintenance*` family]
`MaintenanceWritePatternUnavailable` — a `write:` pin names an unrecognised pattern, or one the target backend's capability registry does not provide; names the pattern and the backend; **never a silent downgrade**.

### A103 (lines 525) [Key-grain diagnostic codes]
`KeyedRequiresGroupBy` (Error) — the model SELECT has no `GROUP BY`; there is no unique key to derive.

### A104 (lines 526) [Key-grain diagnostic codes]
`KeyedForbidsTimeseries` (Error) — the model declares a `timeseries:` block but key temporal locality cannot be established (no route applies; the routes require the window-forward run shape). The message names the **three routes** and the nearest missing fact.

### A105 (lines 527) [Key-grain diagnostic codes]
`KeyedUnknownCombiner` (Error) — a non-key projection is not a direct call to a catalogued aggregator; names the offending expression. When the projection is a bare column or `ANY_VALUE` under window-forward, the message names `MAX_BY` + an ordering column as the fix.

### A106 (lines 528) [Key-grain diagnostic codes]
`KeyedGroupByContainsPartitionColumn` (Error) — the `GROUP BY` contains the driving source's `partition_column` and the model declares **no** `timeseries:` block (ambiguous between partition-grain and key-embedded time-partitioned key-grain). The diagnostic suggests **both** fixes: `grain: partition` + `timeseries:`, or declaring `timeseries:` on the model to stay `grain: key`.

### A107 (lines 529) [Key-grain diagnostic codes]
`KeyedForbidsWindowFunctions` (Error) — the outer SELECT body uses `OVER (...)`; the keyed state *is* the window.

### A108 (lines 530) [Key-grain diagnostic codes]
`KeyedForbidsNondeterministic` (Error) — the SQL uses `NOW()`, `RANDOM()`, or other non-deterministic functions; cross-window merge requires deterministic per-window output.

### A109 (lines 531) [Key-grain diagnostic codes]
`KeyedSqlNotParseable` (Error) — the model body cannot be parsed into the shape the classifier reads.

### A110 (lines 532) [Key-grain diagnostic codes]
`KeyedMultipleDrivingSources` (Error) — more than one timeseries-tagged source appears in the FROM clause; lists the candidates.

### A111 (lines 533) [Key-grain diagnostic codes]
`KeyedOnceWriteUnproven` (Error) — a once-write (`COALESCE`) column has no once-write provenance proof (the value is not provably a per-key constant); names the column; suggests the key-derived form, a declared functional dependency, or remodelling.

### A112 (lines 534) [Key-grain diagnostic codes]
`KeyedRetractableContribution` (Error) — an enrichment join's per-key contribution is retractable (feeds a decrementing aggregate or a value that must be un-seen); steers to `refresh: materialized_view` or DAG composition. It does **not** fire on the join spelling alone.

### A113 (lines 535) [Key-grain diagnostic codes]
`KeyedSnapshotSourceUnsupportedColumn` (Error) — a column family inadmissible under snapshot-reconcile appears in a model with no clocked driving source; names the column, the family, and why the current-snapshot oracle cannot hold for it.

### A114 (lines 536) [Key-grain diagnostic codes]
`KeyedReprocessedWindow` (Error) — a run window covers a ledgered window of a non-re-run-tolerant model, or `--auto` detects changed input under an already-merged window; points at `--full-refresh`.

### A115 (lines 537) [Key-grain diagnostic codes]
`KeyedRecurrenceBoundViolated` (Error) — fires at **runtime**, window-forward, **declared-recurrence route only**: a merged delta row matched (or would duplicate) a stored key outside the run's derived slice, violating the driving source's declared `key_recurrence`. The run's **transaction rolls back**; the message reports the violation count and sample keys. **Derived** locality routes cannot fire it.

### A116 (lines 539) [Diagnostics]
Every key-grain rejection above guards the **equivalence invariant itself**, not a partial-correctness optimisation — **there is nothing safe to waive**.

### A117 (lines 543–545) [Semantics — The equivalence invariant]
The parent contract: every maintained (non-`full`) model upholds **one** invariant over an abstract processed-input set `S`: `incremental_state(S) == full_refresh(source | input ∈ S)` — an incremental run produces what a full refresh would, restricted to the inputs processed so far. `S` is a set of *source rows/partitions the run has scanned*, not necessarily clock-addressed; the **partition-set form** (`source | partition_col ∈ S`) is the **clocked specialisation**, available whenever the driving source carries a `timeseries:` clock; an unclocked (snapshot) source's specialisation is stated per shape profile (e.g. over "keys present in the current snapshot").

# Claims inventory B — incremental_models.md lines 541–965 (shared machinery)

### B1 (lines 543–545) [The equivalence invariant]
Every maintained (non-`full`) model upholds exactly **one** invariant, stated over an abstract processed-input set: for processed input set `S`, `incremental_state(S) == full_refresh(source | input ∈ S)`.

### B2 (line 545) [The equivalence invariant]
`S` is a set of *source rows/partitions the run has scanned*, not necessarily a clock-addressed partition set. The **partition-set form** (`source | partition_col ∈ S`) is the **clocked specialisation**, available whenever the driving source carries a `timeseries:` clock; an unclocked (snapshot) source has no partition set to slice by, and its specialisation is stated per shape profile (e.g. over "keys present in the current snapshot").

### B3 (line 547) [The equivalence invariant]
Order/set-determinacy is a corollary of the invariant and holds for **every** shape profile including the partition grain: the right-hand side depends only on the *set* `S`, never processing order, so any conforming profile is order-independent. For partition grain the combiner is disjoint union (commutative monoid), making the property trivial but present.

### B4 (line 549) [The equivalence invariant]
Shape profiles differ not in *which* equivalence they satisfy but in **how their writes address rows**. Addressing is a **per-cell** fact; each profile names the addressing of its *dominant* (creation/default) cell, and a model may derive the other addressing for a different `(trigger × changed-input)` cell (e.g. a composed clock-and-identity output: dimension-change cell keyed, fact-creation cell region-rewrites).

### B5 (line 551) [The equivalence invariant]
**Partition-addressed** (identity-free; the partition shape's default cell): output addressed by `partition_column`; a source partition maps to an output partition rewritten wholesale (DELETE+INSERT), no row identity needed. Equivalence is additionally checkable slice-by-slice — **per-partition equivalence** — a **strengthening** of the one invariant, available because each output slice depends only on its own source partition (partition-local).

### B6 (line 552) [The equivalence invariant]
**Key-addressed** (identity-requiring; the key shape's default cell, `versioning: interval`, `refresh: materialized_view`): output addressed by a key; each processed input contributes a delta merged into keyed state (`merge_into`). The write reaches stored rows **by key, wherever they live** — it is *not* bounded by the incoming data's time window (SCD2 close-out is the sharp case: closing the previously-open version may touch a row arbitrarily far outside the input window) — which is exactly why a key-addressed write cannot be maintained as a per-partition rewrite. Equivalence is checked on the end-state.

### B7 (line 554) [The equivalence invariant]
Key-addressing admits a **derived refinement**: a key-addressed output that also carries a `timeseries:` partition column, admitted when **key temporal locality** is established — every stored row a run's deltas can touch provably lies within a derived slice of the output's time axis. The write remains a keyed `merge_into`; locality licenses pruning the merge's *target scan* to the slice, and makes **per-slice equivalence** available as a strengthening. This is a per-model *established fact*, not a key-grain default (some key-addressed writes intrinsically escape every time window).

### B8 (line 556) [The equivalence invariant]
Per-partition equivalence is not a peer of a separate "end-state equivalence" — it is a strengthening of the single invariant. Key-addressed shapes discharge the *same* invariant on their end-state. Every property is proven in service of this invariant; every transform is licensed **because it preserves it**.

### B9 (line 556) [The equivalence invariant]
For smelt-driven shapes the invariant is discharged by the generative equivalence oracle (the family's regression net); for `refresh: materialized_view` it is discharged by the **engine's** native IVM, not the smelt oracle — smelt runs no combiner for that shape.

### B10 (line 558) [The equivalence invariant — replayability split]
Full equivalence — an executable `full_refresh` oracle a test can run — holds only for **replayable inputs**: a set `S` the model can re-evaluate its own SQL over (a clocked source's processed partitions; a snapshot's keys currently present). v1 admits **only** combinations whose oracle is executable this way; §"Admission matrix" enforces this per column.

### B11 (line 558) [The equivalence invariant — replayability split]
The designed-but-**unshipped** third column for non-admitted combinations (non-replayable input under a partitioned output; a fold family needing unreplayable history) is an **observer / prefix-consistency contract** — a different, *weaker* equivalence (a property of the observation sequence, not a re-runnable full refresh) that a future **opt-in** could state and admit explicitly, never smuggled in under the executable-oracle invariant. It is not specified here; each shape profile's Known Divergences records where it would apply.

### B12 (lines 560–562) [The equivalence invariant — carve-outs]
Named carve-out 1 of exactly two (both consequences of the executable-oracle requirement, not gaps): **retained departed keys** under an unclocked (snapshot-reconcile) posture — a key present in stored state but absent from the current snapshot is retained, not deleted; the stored table is *the oracle's rows plus retained departed keys*, a documented divergence from a hypothetical delete-on-absence oracle.

### B13 (line 563) [The equivalence invariant — carve-outs]
Named carve-out 2: **ordering-key ties** on an order-monotone overwrite column (`MAX_BY`/`MIN_BY`) — equivalence holds up to ties on the ordering expression, because the classifier cannot statically prove ordering-key uniqueness.

### B14 (line 565) [The equivalence invariant]
The interval-versioned profile's oracle is its end-state equivalence in the **interval-keyed specialisation**: the user-visible set of `(key, version, validity interval)` rows equals what a full rebuild would compute from the same processed snapshots, independent of merge order.

### B15 (line 569) [The algebraic maintenance ladder]
Ownership split: the ladder's *discriminants* (is-monoid, needs-inverse, decomposable, value-vs-order-monotone) are raw SQL properties **owned by `model_properties.md`**; the ladder itself — the ordering *and* the maintainable-vs-delegated cutoff — is the maintenance consequence **owned here** (in `incremental_models.md`, with the invariant). What a key-addressed model can maintain is fixed by the algebra of its combiners, not by any backend feature; the ladder's ordering criterion **is** invertibility → maintainability.

### B16 (line 569) [The algebraic maintenance ladder]
The equivalence invariant holds **unconditionally on every rung**; only the state representation and its size change across rungs, never the fidelity of the user value.

### B17 (line 571) [The algebraic maintenance ladder]
Rung 1 — **Direct monoid**: the stored column *is* the answer; the combiner is a commutative monoid (associative, commutative, identity = empty partition): `SUM`/`COUNT` (`+`, 0), `MIN`/`MAX` (±∞), `BOOL_*`, `BIT_*`.

### B18 (line 572) [The algebraic maintenance ladder]
Rung 2 — **Decomposed monoid**: the user value is `π(state)` for a richer monoid element and a pure presentation map `π` (`AVG` = `(sum, count)` presented `sum/count`; variance = a Welford triple; approximate distinct = an HLL register vector). Kept in a state table, exposed through a presentation view.

### B19 (line 573) [The algebraic maintenance ladder]
Rung 3 — **Group**: when inputs can change (corrections, reprocessing, deletes) the combiner must be **invertible** — a commutative group (`SUM`, `COUNT`, `BIT_XOR`). Monoids that are not groups (`MIN`/`MAX`/`BOOL_*`/`BIT_AND`/`BIT_OR`) cannot un-see a contribution and cannot be reprocessed without a full refresh.

### B20 (line 574) [The algebraic maintenance ladder]
Rung 4 — **Opt-in bounded-domain multiset**: holistic aggregates needing all rows (exact `MEDIAN`/`PERCENTILE`/`MODE`/quantiles, exact `COUNT(DISTINCT)`, `DISTINCT`-modified aggregates) are maintained by storing the per-key value→count multiset (a bounded-domain Z-set); its **signed** form makes retraction free even for the otherwise-irreversible `MIN`/`MAX`. **Opt-in and fail-loud**: state is `O(active domain)`, so an unbounded-state aggregate is **default-refused** (suggesting the approximate form or `refresh: full`) unless the modeller supplies a bounded-domain budget, and the runtime caps the multiset with a full-refresh fallback.

### B21 (line 576) [The algebraic maintenance ladder]
The ladder is the boundary: rungs 1–4 are what smelt maintains itself (a `merge_into` loop, optionally with a presentation view). Beyond it — general-operator retraction over joins, unbounded non-additive state — is **not** smelt-driven-maintainable and is **delegated** to the engine's native incremental-view maintenance via `refresh: materialized_view`.

### B22 (line 580) [Windowed maintenance and the horizon]
Maintenance runs over a **bounded input window by default** — a full scan is the degenerate fallback, not the baseline. A run reasons about two windows, **always with `scan ⊇ write`**.

### B23 (lines 582–583) [Windowed maintenance and the horizon]
Definitions: the **write window** is the partitions or keys written this run; the **scan window** is the input rows read to produce that write window correctly.

### B24 (line 585) [Windowed maintenance and the horizon]
The scan window is bounded **where the model carries a `timeseries:` clock**: input-delta discovery is window-forward (only the new window plus a lookback is read; stored state stands in for history). Without a clock the source can only be snapshot-diffed, so the scan degrades to a full read. This is orthogonal to output addressing: a clocked key-addressed model still windows its **scan** even though its **write** reaches back by key outside that window.

### B25 (line 585) [Windowed maintenance and the horizon]
Bounding the scan never weakens the invariant — the engine evaluates the model, joins included, over the widened scan window and the write is **clamped** to the exact write window ("widened scan + exact clamp", `model_transforms.md`), leaving join optimisation to the engine rather than smelt hand-computing a minimal delta.

### B26 (line 587) [Windowed maintenance and the horizon]
The **horizon** — a **write-eligibility clamp** (a bound on which keys/partitions a run may *write to*) — applies **only to the partition grain**: the far edge of the maintained window, past which inputs are no longer folded in. It is **derived, never trusted from a declaration**: clamp bounds are computed from the model's own reach (lookback, window frames, join contribution — `model_properties.md`), because a declared horizon smaller than true reach would make the clamp drop rows that should have been rewritten.

### B27 (line 587) [Windowed maintenance and the horizon]
A modeller **may** declare a horizon *ceiling* (frontmatter key `horizon_ceiling:`, e.g. `horizon_ceiling: '30 days'`) — smelt **warns at compile time** when the derived horizon would exceed it — but the clamp **always uses the derived value**.

### B28 (line 589) [Windowed maintenance and the horizon]
Because the horizon is derived, a genuinely late arrival — landing after its natural partition passed the horizon — is **silently excluded** from the maintenance run, **not diagnosed** (smelt cannot fail loud on a row it never scans; rows outside the derived-horizon-bounded scan window are outside "inputs processed so far" by construction). **Surfacing lateness is a model-author concern, not a maintenance guarantee**; the documented pattern is to fold the late row into the current partition (re-stamping its partition time) with a lateness/validity flag and let a data-quality check raise on flagged rows. The maintenance layer clamps; it does not police lateness.

### B29 (line 591) [Windowed maintenance and the horizon]
**The key grain has no write-eligibility clamp.** A `grain: key` run merges **every** delta row it scans, into whatever key it names, however old — no bound on which keys a run may touch. A **derived forward reach** is still computed and reported (via `smelt explain`) for observability, but it **never gates admission and never bounds a write**. Rationale is normative: a write clamp would silently drop scanned inputs — the one thing the equivalence invariant forbids.

### B30 (line 591) [Windowed maintenance and the horizon]
What a keyed clamp would buy (settled-key GC, bounded working set) is deferred optimisation that, **if ever introduced, must ship together with late-fact accounting** (`docs/research/20260705-keyed-collapse-application.md` D6).

### B31 (line 591) [Windowed maintenance and the horizon]
The narrow principle beneath both grain stances: **only proofs prune; a declared bound is admitted only checked (fail-loud on violation); no unproven bound ever refuses a write.** Target-scan slice pruning under established key temporal locality conforms — derived routes prune by proof, the declared key-recurrence route prunes only under a transactional runtime check, and every scanned delta row still merges.

### B32 (lines 593–598) [Three pruning categories]
The only-proofs-prune rule admits **exactly three** categories of narrowing. Category 1 — **Target-scan slice pruning** (read-side): rows the write provably cannot touch are removed from the merge's *read* of stored state; licensed by the key-temporal-locality proofs or the transactionally-checked recurrence declaration.

### B33 (lines 599–610) [Three pruning categories]
Category 2 — **No-op write elimination** (write-side): a maintenance write may be skipped **iff** the row's applied effect is proven to be the identity, proven per row *by evaluation*: an exact `IS DISTINCT FROM` comparison over every column that can differ under the cell's trigger (the mutation-sensitive group — comparing only it is sound *because* the other groups are proven insensitive).

### B34 (lines 602–605) [Three pruning categories]
Suppression may **never** skip **evaluating** a scanned input — restricting what is *computed* is a separate concern with its own static licence (§Future Extensions, "Conditional maintenance without a change feed").

### B35 (lines 605–608) [Three pruning categories]
A compared column must be a pure function of the processed inputs; a column that legitimately varies run to run (`contract: plausible`, run-pinned `NOW()`) is **incomparable**, and a cell containing one **refuses the conditional technique (fail-closed)**.

### B36 (lines 608–610) [Three pruning categories]
At a fixed processed-input set `S`, the suppressed and unconditional variants produce identical state — interchangeable in the strongest sense of §"Per-cell admission" — so choosing between them is squarely a cost-model/`prefer`/`technique` matter.

### B37 (lines 611–615) [Three pruning categories]
`model_transforms.md` catalogues exactly two physical realisations of category 2: **change-suppressed MERGE** (matched-arm `IS DISTINCT FROM` predicate on keyed `merge_into` or column-scoped merge, dialect-split on the unmatched-by-source side) and the **staged-candidate conditional DELETE+INSERT** (merge-less, for a backend without `MERGE`) — both licensed by region row identity plus per-column change comparability on the compared group.

### B38 (lines 616–617) [Three pruning categories]
Category 3 — **Write-eligibility clamps**: **forbidden on the key grain, derived-only on the partition grain** (the horizon).

### B39 (lines 619–623) [Three pruning categories]
Categories 1–2 preserve the equivalence invariant **bit-for-bit at fixed `S`**; category 3 is different in kind — it bounds which inputs are *in* `S` at all. A suppressed write is the write-side dual of slice pruning (the proof is the per-row equality just evaluated), **not a clamp, and must never be argued into one**.

### B40 (line 623) [Three pruning categories]
Two `model_transforms.md`-catalogued transforms read a **derived (never declared)** forward reach without being write clamps: the dimension-driven horizon-bounded MERGE (a *scan/recompute* bound on the enrichment recompute, not the write) and the horizon settled-delay/tail-rewrite mechanism (partition-grain forward-reach machinery).

### B41 (lines 625–627) [Validator, not chooser]
The machinery **validates** the declared shape — the `refresh:` value plus shape-defining facts (clock `timeseries:`, identity `unique_key:`) and any check-only `grain:`/`write:` assertion — against the derived properties, and **rejects (fail-loud)** when the SQL cannot uphold the shape's contract. It **never chooses or silently switches** the shape or the addressing. A full refresh is the honest fallback **surfaced as a diagnostic, never an automatic downgrade**.

### B42 (lines 627, 686–695) [Validator, not chooser / Per-cell admission]
Per-cell technique choice among proven-interchangeable techniques operates strictly **inside** validator-not-chooser: it may change *which `S` is reflected* (freshness), **never observable bits at a fixed processed-input set**. (Stated at line 627 and restated at lines 693–695.)

### B43 (lines 631–636) [The plan matrix]
The plan factors output columns into **column groups** by shared mutation-sensitivity; the proof and its degenerate-collapse rule are **owned by `model_properties.md`** (§"Per-column mutation-sensitivity / column provenance") — this spec consumes the groups as the plan's column axis. Creation is shared by every column (all columns of a new row computed together); mutation is what partitions them.

### B44 (lines 637–644) [The plan matrix]
Each `(group × trigger × changed-input)` cell picks a corner of the 2×2 of **read scope** (delta+state vs the region's full upstream input) × **write scope** (the cell's physical write addressing: targeted addresses vs region overwrite). The four corners: read delta+state × write targeted = **fold-a-delta**; read delta+state × write region-overwrite = **read-modify-write region**; read full-input × write targeted = **column-scoped re-derivation**; read full-input × write region-overwrite = **recompute-a-region**.

### B45 (lines 646–649) [The plan matrix]
The write-scope column *is* the addressing corner; which concrete write pattern realizes it (keyed `MERGE`, column-scoped `MERGE`, in-place `UPDATE`, region `DELETE`+`INSERT`, or a backend-provided variant) is drawn from the **open write-pattern registry** by the available-addressings rule.

### B46 (lines 649–652) [The plan matrix]
Recompute-a-region is **contract-agnostic and unconditionally valid over replayable input**; the fold corner is **contract-specific** (it needs a combiner algebra per the ladder).

### B47 (lines 652–653) [The plan matrix]
Where the interchangeability conditions hold, a recompute of a region **supersedes and resets** what folds had written there.

### B48 (lines 655–659) [The plan matrix]
"Unconditionally valid" is a correctness claim, **not an admission or cost claim** — it holds even in the degenerate case where the region is the whole table (a whole-table recompute is a region taken to its limit). Whether that degenerate recompute is *admitted* into the plan is a separate question, **gated by the K8 partition-locality guardrail**.

### B49 (lines 663–664) [Per-cell admission]
A technique enters a cell's plan space **only when all of its obligations discharge (fail-closed; an unrecognised construct refuses, never defaults)**.

### B50 (lines 666–667) [Per-cell admission]
Obligation 1 — **Replayable input** (recompute family): the source is re-readable at its current processed set; declared posture, owned by `sources.md`.

### B51 (lines 668–671) [Per-cell admission]
Obligation 2 — **Faithful fold** (fold family): the fold's two independent conditions (source posture × combiner algebra) hold (`model_properties.md` §"Faithful-fold conditions"); a replayable feed carrying retractions into a non-invertible combiner passes the first condition and fails the second, and **either failure alone refuses the fold family for this cell**.

### B52 (lines 672–673) [Per-cell admission]
Obligation 3 — **Combiner algebra class**: derived, fail-closed (`model_properties.md` discriminants); a holistic or unrecognised combiner leaves **only the recompute family**.

### B53 (lines 674–677) [Per-cell admission]
Obligation 4 — **Bounded reach**: the cell's scan bound `(clock_col, before, after)` per source is derived (`model_properties.md` §"Unified bound / reach derivation") or declared-and-checked; absent both, **full-input techniques only**, and diagnostic `MaintenanceReachNotDerivable` when the trigger requires a bound.

### B54 (lines 678–681) [Per-cell admission]
Obligation 5 — **Bounded footprint** (targeted writes): the write-scope reflection of the scan bound is bounded (`model_properties.md` §"Footprint reflection / bounded write footprint"); a trajectory column's unbounded forward footprint fails this (`MaintenanceUnboundedFootprint`).

### B55 (lines 682–684) [Per-cell admission]
Obligation 6 — **Well-defined groups**: the mutation-sensitivity partition is computable (`model_properties.md`); degenerate collapse is **surfaced, never silent**.

### B56 (lines 686–689) [Per-cell admission — interchangeability]
Two techniques may serve one cell interchangeably **iff**, at a fixed processed-input set `S`, they produce identical state **on the columns that decide which rows exist** (the `S`-indexed refinement of the equivalence invariant). `S` is a **per-input vector** once the plan factors.

### B57 (lines 689–692) [Per-cell admission — interchangeability]
For faithful idempotent columns the choice is **bit-preserving**; for additive columns it is state-preserving **modulo the ledger**, whose real obligation is *never fold a delta already reflected in the state*: **fold-then-recompute is safe** (the recompute resets the region's ledger); **recompute-then-refold double-counts**.

### B58 (lines 692–695) [Per-cell admission — interchangeability]
Technique choice among proven-interchangeable techniques belongs to the cost model or the operator (via `prefer`/`technique`); it may change only which `S` is reflected (freshness), never observable bits at fixed `S`.

### B59 (lines 699–705) [Per-cell write addressing]
Every `(column-group × trigger × changed-input)` cell derives its **physical write** from the currently known write-pattern set — an **open registry, not a closed enum**: `{ region DELETE+INSERT, keyed MERGE, column-scoped MERGE, in-place UPDATE, full rebuild, … }`.

### B60 (lines 707–709) [Per-cell write addressing]
**The available-addressings rule**: a write mechanism is admitted for a cell iff `available = (which contract facts the output declares) × (what the trigger/changed-input needs) × (the equivalence invariant) × (backend capability)`.

### B61 (lines 711–712) [Per-cell write addressing]
The first three factors of the available-addressings rule are structural; the fourth is the target engine's **capability registry** (owned by `architecture.md`).

### B62 (lines 714–715) [Per-cell write addressing]
Keyed `MERGE`, column-scoped `MERGE`, and in-place `UPDATE` **require a declared `unique_key`** (row identity).

### B63 (line 716) [Per-cell write addressing]
Region `DELETE`+`INSERT` **requires a declared partition axis** (`timeseries:`) to delete by.

### B64 (line 717) [Per-cell write addressing]
A **bare lookup** (identity, no clock) has no region → **only keyed merge or full rebuild** are available.

### B65 (lines 718–722) [Per-cell write addressing]
A **bare partition table** (clock, no identity) has no identity → **only region rewrite or full rebuild**. To gain keyed dimension-change addressing the output must **declare a `unique_key`**, which makes it the composed clock-and-identity keyed shape (derived `grain: key`, time-partitioned) — declaring identity is **load-bearing** (it admits keyed writes), never a dedup footnote.

### B66 (lines 723–725) [Per-cell write addressing]
SCD2's close-out cell has **only** keyed `MERGE` available, because its write provably escapes any time window — derived per-cell, fail-loud if the facts can't support it, **no bespoke shape needed**.

### B67 (line 727) [Per-cell write addressing]
A cell with no admissible write mechanism is refused with `MaintenanceNoAdmissibleTechnique`, **naming the cell**.

### B68 (lines 729–736) [Per-cell write addressing — addressing vs span]
A keyed write on a clocked model is still **partition-scoped**: choosing keyed `MERGE` (or column-scoped `MERGE` / in-place `UPDATE`) picks how a row is *found* (by identity), not that the statement runs unbounded. When the output also declares a `timeseries:` axis, the write stays **bounded to (and, where the backend benefits, iterated over) the affected partitions**: the changed-input delta is first resolved to the set of touched partitions and the keyed `MERGE` is emitted per-partition (or with a partition predicate) against just those.

### B69 (lines 737–740) [Per-cell write addressing — addressing vs span]
A genuinely window-free keyed write (one whole-table `MERGE`) is the **exception**, reached only when the cell **provably cannot** be bounded to a partition set (the SCD2 close-out); that unboundedness is itself a **derived per-cell fact, fail-loud** (`MaintenanceUnboundedFootprint` / `MaintenanceScanUnbounded`), **never a default**.

### B70 (lines 740–742) [Per-cell write addressing — addressing vs span]
Partition-scoping is **orthogonal** to the addressing corner: region and keyed writes alike ride the same partition-pruning the plan already computes (§"Partition-local maintenance").

### B71 (lines 744–750) [Per-cell write addressing — user pins]
The override ladder names the write mechanism per cell via `maintenance.cells[].write`. A pin is **validated against the equivalence invariant** for its cell; an addressing that cannot uphold equivalence is **refused with diagnostic `MaintenanceWriteAddressingRefused`, never silently honoured**; a name the target backend cannot execute is refused with `MaintenanceWritePatternUnavailable`. The pin **selects among admissible mechanisms; it never widens the admissible set**.

### B72 (lines 752–758) [Per-cell write addressing — scenarios]
Mixed addressing by which input changed (output declares **both** `timeseries:` and `unique_key:`): the creation-trigger cell (main fact delta) derives a region rewrite (or fold, per the plan matrix); the dimension-change cell derives a keyed column-scoped `MERGE` — available *because* `unique_key` is declared — still **scoped to the partitions the correction touches**, not a whole-table merge. Either may be pinned if the cost model picks wrong.

### B73 (lines 759–763) [Per-cell write addressing — scenarios]
Mixed addressing by trigger (output declares `timeseries:` ± `unique_key`): creation/mutation cells derive keyed merge / fold; the `backfill` cell is pinned `on: backfill, technique: recompute, write: region` → `DELETE`+`INSERT` (a clean region reset). This is **licensed by the fixed-`S` interchangeability rule** (a recompute supersedes and resets what folds wrote).

### B74 (lines 765–771) [The write-pattern set is open]
The write-pattern set will grow (partition/atomic swap such as Delta/Iceberg `REPLACE PARTITION`, copy-on-write vs merge-on-read, `MERGE … WHEN NOT MATCHED BY SOURCE` prune, staged-upsert, incremental MV refresh, backend-specific primitives); the design's durable contract is deliberately **not** the enumeration — **the enumeration is data**.

### B75 (lines 773–781) [The write-pattern set is open]
**The invariant is the admission function, not the enum.** A new pattern is admitted purely by declaring **which contract facts it requires** (identity? partition axis? ordered arrival?) and **discharging the equivalence proof obligation** — `incremental_state(S) == full_refresh(inputs ∈ S)` for the cells it serves. Nothing else moves: grain stays derived, the contract stays the vocabulary, the cost model ranks whatever candidates the rule admits.

### B76 (lines 782–788) [The write-pattern set is open]
The pattern set is **backend-relative** — admission carries backend capability as the fourth factor: the write layer queries the backend's capability registry (`architecture.md`), and a pattern the target cannot execute is **simply not a candidate**. The registry is the home for backend-specific optimisations to be *contributed*, not special-cased in the planner; this keeps a portable project from silently depending on a primitive only one engine has.

### B77 (lines 789–794) [The write-pattern set is open]
The `write:` pin is an **open, fail-loud vocabulary, not a sealed `region|keyed|column|update` enum** — an open name resolved against the registry. An unrecognised pin, or one naming a pattern the target backend cannot provide, is **refused with a diagnostic, never silently downgraded** to a default. The surface admits new pattern names the moment a backend registers them.

### B78 (lines 796–801) [The write-pattern set is open]
Net contract: the enum is a **snapshot of a registry**; the admission rule + equivalence gate + capability factor are the contract. A new write pattern carries its own correctness proof. Backends **execute** registered patterns; they **never author** maintenance-statement text (per §"Statement emission (single owner)" and `architecture.md` maintenance-plan purity).

### B79 (lines 805–808) [Partition-local maintenance (K8 guardrail)]
A cell's per-`(cell × source)` locality verdict is the **partition-locality projection**, whose proof (including the cross-axis predicate requirement) is **owned by `model_properties.md`**; this section owns **only the policy** consuming the verdict.

### B80 (lines 808–810) [Partition-local maintenance (K8 guardrail)]
The emitted maintenance SQL must carry the partition predicate on **both** the scan and the merge/overwrite target — a bound stated only on a non-partition column is one the storage layer cannot prune by.

### B81 (lines 810–812) [Partition-local maintenance (K8 guardrail)]
Under the default `scan_bounds` (`require: partition_local`, `on_violation: error`), a non-local cell **refuses** (`MaintenanceScanUnbounded`) **unless** the source carries `allow_full_scan: true`.

### B82 (lines 812–813) [Partition-local maintenance (K8 guardrail)]
`max_lookback` **additionally refuses** a derived span wider than the operator's stated expectation.

### B83 (line 813) [Partition-local maintenance (K8 guardrail)]
The guardrail **never modifies a clamp**.

### B84 (lines 817–820) [Statement emission (single owner)]
The physical statements a run executes for a cell (region `DELETE`+`INSERT` pair, keyed fold `MERGE`, column-scoped `MERGE`, in-place `UPDATE`, first-run `CREATE TABLE … AS`) are produced by **pure emitter functions in the maintenance layer (`smelt-logical`)** — the statement-level counterpart of "one derivation, many consumers".

### B85 (lines 820–824) [Statement emission (single owner)]
An emitter is a pure function from plain data (target table, region literals, key columns, combiner-rendered set expressions, the compiled/clamped SELECT body, a dialect tag) to an **ordered statement group plus its transactional requirement** — a paired `DELETE`+`INSERT` is **one transaction: a failed `INSERT` must roll back its `DELETE`**.

### B86 (lines 824–828) [Statement emission (single owner)]
Backends *execute* emitted statements (connections, transactions, blocking dispatch) and **never author maintenance-statement text of their own**; dialect differences (e.g. `MERGE … UPDATE SET *` requiring a full-row source projection vs explicit column-list `SET`) live **in the emitters as dialect-keyed variants**, not in backend string construction.

### B87 (lines 830–832) [Statement emission — exclusions]
Deliberate exclusion 1: the reconciliation ledger's DDL/DML is state bookkeeping **owned per dialect by `smelt-state`** — *interleaved transactionally* with an emitted fold statement but **not itself a maintenance statement**.

### B88 (lines 832–837) [Statement emission — exclusions]
Deliberate exclusion 2: the observed-output-delta record and the fingerprint sidecar's **own storage** (table DDL, digest-refresh upsert, GC delete) sit in the same excluded class — warehouse-resident, owned per dialect by `smelt-state` alongside the reconciliation ledger, each interleaved transactionally with the write whose changed-row set or digest it captures but never itself a maintenance statement.

### B89 (lines 837–841) [Statement emission — exclusions]
Exception within the fingerprint-sidecar feature: the sidecar's **diff query** — a derived maintenance-relevant comparison (which source keys count as "changed") — **IS emitter-authored** (`smelt_logical::maintenance::emit::emit_fingerprint_sidecar_diff`), not part of the exclusion.

### B90 (lines 841–842) [Statement emission — exclusions]
Deliberate exclusion 3: non-maintenance SQL (introspection, seed loading, schema-evolution DDL) is **outside this rule**.

### B91 (lines 844–846) [Statement emission (single owner)]
Single ownership makes maintenance SQL *observable*: the **same emitters** serve execution, the conformance equivalence gates, and `smelt explain <model> --show-sql`, so **printed SQL cannot drift from executed SQL**.

### B92 (lines 850–852) [The definition-change trigger]
A model gaining output fields is a trigger of its own kind: the added group's processed-input vector is `∅` over every existing region, and its backfill advances `∅ → current`, **touching only the new group**.

### B93 (lines 852–855) [The definition-change trigger]
The classification of an added field — `SkeletonAdd` / `PureBackfill` / `UpstreamRederive` — is the **definition-change column classification proof, owned by `model_properties.md`**; this section owns only the plan-level policy each classification maps to.

### B94 (lines 857–859) [The definition-change trigger]
`SkeletonAdd` (identity / grouping / dedup / ordering) is a **grain change, refused as a column backfill** (`MaintenanceSkeletonColumnAdded`) — the honest plan is a recompute, effectively a new model.

### B95 (lines 860–862) [The definition-change trigger]
`PureBackfill` lands in the 2×2's targeted-write column as an **in-place `UPDATE`** (no upstream read); `UpstreamRederive` lands there as a **column-scoped `MERGE`**, keyed where the source is keyed, **inheriting each read source's partition-locality verdict unchanged**.

### B96 (lines 863–866) [The definition-change trigger]
Fields added together factor by shared mutation-sensitivity, **one backfill op per group**. The backfill of a newly-added group is **always full-input**, even for a column whose ongoing algebra folds — there is no prior state of that column to fold onto.

### B97 (lines 867–870) [The definition-change trigger — group convergence]
A field co-sensitive with an *existing* group still instantiates at `∅` and forms its own catch-up group; mid-catch-up, a delta folds into the sibling group but is **refused on the new group's unbackfilled regions** (the **never-fold-ahead-of-the-entry** rule). The groups **merge only once** the new group's processed vector equals its sibling's over **every** region.

### B98 (lines 874–875) [The reconciliation ledger]
The plan's bookkeeping is a `(output-region × column-group)` ledger; each entry records the **processed-input vector `S_{i,g}`** of that region-group.

### B99 (lines 875–877) [The reconciliation ledger]
Storage is **graded by algebra**: additive groups record **delta identities** (never-fold-twice needs them); idempotent groups record only a **frontier watermark** (re-folding is harmless).

### B100 (lines 877–879) [The reconciliation ledger]
The ledger has exactly two operations: **fold** — refuse if the delta is already in the entry's processed set, otherwise combine and extend; **recompute-reset** — a region recompute resets every intersecting entry to **exactly the input it read**.

### B101 (lines 879–881) [The reconciliation ledger]
Region↔window attribution is **exact under key temporal locality or explicit footprint tracking**; a delta is attributed to the **unique** ledger region containing its footprint.

### B102 (lines 881–882) [The reconciliation ledger]
Schema evolution is a ledger operation: adding a group **instantiates its entries at `S = ∅`**.

### B103 (lines 886–889) [The graph layer — edges]
A dependency edge is `downstream reads upstream` under the **downstream cell's derived scan clamp**, between two partition axes whose **grain is the declared `timeseries.granularity`** of each node — **never per-edge, never derived from the SQL**; the classifier only *checks* the declaration (e.g. against a `date_trunc` grouping).

### B104 (lines 889–892) [The graph layer — edges]
Clamp margins **ceil outward** to whole partitions; each hop aligns its result outward to the receiving axis's grain. Outward maps are monotone, so **sufficiency composes; narrowing never does** — **widen-never-narrow** is the graph layer's composition law.

### B105 (lines 894–895) [The graph layer — forward propagation]
Runs are driven by **what landed**, per source, as partition intervals on that source's **own axis**; a cron tick is only the poller.

### B106 (lines 895–903) [The graph layer — forward propagation]
Processing nodes in topological order, each node's merged dirt reflects through each outgoing edge — an upstream delta of `[a, b)` dirties downstream `[a − after, b + before)` — accumulating **per-edge dirt** `(model, upstream) → intervals` (keys the trigger cell: the plan cell for that inbound source runs over exactly these regions — recompute for a driving-source delta, column-scoped merge for an enrichment delta) and **per-model dirt** (the union across inbound edges: what consumers see as *their* upstream delta).

### B107 (lines 905–906) [The graph layer — forward propagation]
**Sufficiency**: running exactly the per-edge dirty regions with their cells must leave every model equal to a full refresh; partitions outside the dirty set are **never scheduled**.

### B108 (lines 906–907) [The graph layer — forward propagation]
A delta on a source nothing reads, or an empty delta, propagates nothing.

### B109 (lines 907–909) [The graph layer — forward propagation]
A delta on an **unclocked** source dirties the **whole model** for every mutation-sensitive consumer — **never a silent no-op** (the cell was only admitted under `allow_full_scan`, so the full-table run is a declared cost).

### B110 (lines 911–918) [The graph layer — backward resolution]
Given a target model and period `[s, e)` (aligned outward to the target's grain), walking the ancestor sub-DAG in reverse topological order and applying each edge's clamp **directly** — `[s, e)` requires upstream `[s − before, e + after)` — yields for every ancestor the partition intervals that must exist (data prerequisite for a raw source; build region for a model) plus the **build order** (ancestor models in dependency order, target last). Staging exactly the resolved slices and building bottom-up makes the target period equal a build over complete history. The required slice of an unclocked source is the **whole table**.

### B111 (lines 918–919) [The graph layer — backward resolution]
The two directions are **adjoint, not inverse**: `forward(backward(P)) ⊇ P`.

### B112 (lines 921–928) [The graph layer — observed deltas]
A model edge's propagated delta follows the same landed-delta refinement as a source edge (`sources.md` §"Landed-delta (derived, recorded)"): where a run recorded an **observed output delta** — the changed-row set a conditional write (pruning category 2) actually touched, restricted to comparable columns (per-column change-comparability proof, `model_properties.md`) — that set, projected onto the model's own partition axis, is the edge's delta; **absent a recorded delta the edge falls back to the run's written window**, the coarser and always-correct form (**widen-never-narrow**, same rule as the source hierarchy).

### B113 (lines 928–932) [The graph layer — observed deltas]
The observed-delta record is warehouse-resident (alongside the reconciliation ledger) and is written **in the same backend transaction as the write it records** — a delta visible without its write, or a write without its delta, **breaks propagation soundness** (a downstream consumer would schedule against a delta corresponding to no committed state).

### B114 (lines 932–938) [The graph layer — observed deltas]
**Trust boundary**: an observed delta is trusted because the state is smelt-owned, written only by smelt's own conditional-write execution path (bookkeeping alongside the write, per statement-emission exclusion 2/3, not an emitter-authored maintenance statement), mirroring the trust rule `sources.md` applies to declared world-facts. There is **no out-of-band-edit tripwire in v1** — an external mutation to the target table between runs is **not detected**; this is an **explicit Open Question (§Known Divergences), not a silently-assumed absence**.

### B115 (lines 938–941) [The graph layer — observed deltas]
**Empty and absent are distinct**: an empty recorded delta means the run executed and changed nothing (a real, propagatable fact); an absent record means no delta was ever recorded for that write. A consumer **must not conflate the two**.

### B116 (lines 941–944) [The graph layer — observed deltas]
The observed delta composes with the derived settle bound as named in §"What the composed shape uniquely enables" ("Settle-bound × observed-delta composition"): once both legs are built, a stable upstream chain degenerates to empty-delta no-op propagation with a provable horizon behind it.

### B117 (lines 946–951) [The graph layer — refusals]
The graph **refuses fail-loud** (`MaintenanceGraphUnsupportedNode`) on: (a) a cyclic edge set; (b) a **self-referential** model (a table-graph cycle that is a DAG only when time-unrolled — admissible **in principle iff** its self-clamp is strictly time-backward, with forward dirt running to the frontier and backward resolution reaching the model's basis/checkpoint); (c) a **keyed-grain node without an admitted time axis** (no partition axis for interval dirt — silently treating it as day-axis would be wrong-and-quiet).

### B118 (lines 951–955) [The graph layer — refusals]
A **locality-admitted time-partitioned keyed output is NOT refused**: it is a clocked node whose edges use its declared granularity, and whose outbound dirt is the **key→partition projection** of what its runs changed — **exact under locality routes 1–2, widened backward by `r` plus margins under route 3**.

### B119 (lines 957–963) [The partition grain (`grain: partition`)]
The partition grain is the partition-addressed shape: a complete table with a monotone `partition_column`, kept current by the **recompute-a-region corner** (partition DELETE+INSERT). The machinery in this section is **partition-grain-local**. For a run with run window `[start, end)`, the recompute corner drives four transforms from `model_transforms.md` (first of which is B120).

### B120 (lines 963–965) [The partition grain — execution model]
Transform 1: **partition DELETE** from the output table where `partition_column` falls in the **derived output window** — the run window pushed through the model's declared partition-column relation (`model_transforms.md` §"The output window is derived, never assumed"): identity when the `partition_column` tracks event time (`output window = run window`); skew-inverted when the `partition_column` is derived and skews away from the driving date column (declared by a Form B relation). For a **write-rebasing model** (e.g. session keyed by `session_start_date`, `before = after = 1 day`) the output window for run `[D, D+1)` is `[D−1, D+2)`, so the DELETE must cover **every** partition the INSERT will write, including the prior-day partition the new data reaches — deleting only the run window would strand the skew-reached partition **stale forever** (no later run's window contains it).

# Normative-claim inventory — incremental_models.md lines 957–1325
# Sections: "The partition grain", "The key grain", "Interval versioning", "Interactions"

### C1 (lines 957–959) [The partition grain]
The partition grain (`grain: partition`) is the partition-addressed shape: a complete table with a monotone `partition_column`, kept current by the recompute-a-region corner (partition DELETE+INSERT). Its declared surface lives in §"Partition-grain declaration (`grain: partition`)"; all machinery in this section is partition-grain-**local**.

### C2 (lines 963–968) [Execution model (DuckDB, current)]
A partition-grain run with run window `[start, end)` executes exactly four transforms from `model_transforms.md`, in order: (1) partition DELETE, (2) outer output-clamp, (3) source-filter pushdown, (4) INSERT of the resulting query's output into the output table.

### C3 (lines 965) [Execution model (DuckDB, current)]
The partition DELETE removes rows where `partition_column` falls in the **derived output window** — the run window pushed through the model's declared partition-column relation: identity when `partition_column` tracks event time (`output window = run window`); skew-inverted when `partition_column` is derived and skews away from the driving date column, declared by a Form B relation. For a write-rebasing model with `before = after = 1 day`, run `[D, D+1)` has output window `[D−1, D+2)`.

### C4 (lines 965) [Execution model (DuckDB, current)]
The DELETE must cover **every** partition the INSERT will write, including skew-reached neighbour partitions. Deleting only the run window would strand the skew-reached partition stale forever: no later run's window contains it.

### C5 (lines 966) [Execution model (DuckDB, current)]
The outer output-clamp injects `WHERE partition_column >= out_start AND partition_column < out_end` at the outermost SELECT, constraining the model's output to the same derived output window the DELETE covers.

### C6 (lines 966) [Execution model (DuckDB, current)]
The outer output-clamp is **dropped for the transparent slice**: exactly one timeseries source, zero-margin bound `Bounded(_, 0, 0)`, and no partition-column skew — the per-source pushdown filter already is the output clamp. A genuine lookback margin, a partition-column skew, or more than one timeseries source keeps the outer clamp (scan window and output window are then distinct).

### C7 (lines 966) [Execution model (DuckDB, current)]
Each written partition's **scan** is sized from the derived output window's reach, never the run window's — rewriting a skew-reached neighbour partition from a scan sized for the run window would under-read that partition's own reach.

### C8 (lines 967) [Execution model (DuckDB, current)]
Source-filter pushdown injects a per-source `partition_column` filter on each `smelt.<path>` reference, derived from the model's SQL. Sources without a `timeseries:` declaration are lookups: no bound, read in full.

### C9 (lines 970) [Execution model (DuckDB, current)]
DELETE range and output clamp are derived from **one** window, keeping the contract idempotent for any write-window width: re-running the same `[start, end)` under fixed input converges to the same final state (Constraint: idempotence).

### C10 (lines 972) [Execution model (DuckDB, current)]
The derived output window is a range to be **covered**, not a mandate for one statement: backfill chunking may split it into sequential DELETE+INSERT pairs the same way it splits a wide run window, with each chunk's scan sized from that chunk's own reach.

### C11 (lines 976) [Run window vs partition granularity]
The CLI `[--event-time-start, --event-time-end)` declares a **run window**, not a per-partition invocation. It must be a positive integer multiple of `timeseries.granularity` aligned to granularity boundaries; within that, run-window size and partition-granularity unit are independent.

### C12 (lines 976) [Run window vs partition granularity]
A daily-partitioned model run with a 30-day window is **one** engine query (sources filtered to the union of the run window and each source's pushdown bound; output clamped to the run window) plus **one** partition-aligned DELETE over the 30 partitions and one INSERT. Backfilling 60 days is one `smelt run --event-time-start D --event-time-end D+60d`, not 60 daily invocations. Per-partition equivalence holds regardless of run-window size.

### C13 (lines 978) [Run window vs partition granularity]
The declared `timeseries.granularity` (`g_run`) must be at least as coarse as `g_part`, the granularity actually implied by the `partition_column` projection's truncation/grid transform — derived independently from the model's SQL, not trusted from the declaration. E.g. `partition_column = DATE_TRUNC('day', event_time)` gives `g_part = day`, so declaring `granularity: hour` is rejected.

### C14 (lines 978) [Run window vs partition granularity]
`g_run >= g_part` is checked under the closed enum's increasing-coarseness ordering `hour < day < week < month < quarter < year`; `g_run == g_part` or `g_run` coarser both pass. When `g_part` cannot be derived (opaque projection), the comparison is **skipped** — undecided, not a positive disproof — and only the declared-granularity alignment check applies.

### C15 (lines 978) [Run window vs partition granularity]
This is hard validation: a sub-`g_part` run window is rejected with a message naming the minimum window, never silently widened or coarsened to fit.

### C16 (lines 982–988) [Batch safety classification]
The optimizer rolls the per-source bound map (`BoundResult` per source) into a single **partition-grain-local** batch-safety class per model, meaningful only inside the recompute-a-region shape. The three classes: `FullyBatchSafe` — all timeseries sources `Bounded(_, 0, 0)`, no temporal dependencies, single query for any run window; `BoundedSafe(n)` — all timeseries sources `Bounded` with `n = max(before + after)` > 0, auto-sized chunks (3× context, clamped 7–90 partitions); `PerPartitionOnly` — one or more timeseries sources `Unbounded` (cumulative-across-history), one partition at a time, sequential.

### C17 (lines 990) [Batch safety classification]
`n` for `BoundedSafe` is rendered in the source's partition-column unit and is the same value the source-filter pushdown transform reads.

### C18 (lines 992) [Batch safety classification]
A model with **any** `NotDerivable` source is **refused at planning time**, not assigned a class — diagnostic `MaintenanceReachNotDerivable` (§"Per-cell admission" obligation 4). The diagnostic names the offending construct and the source-map points at the original SQL. There is **no silent downgrade to full-refresh**.

### C19 (lines 994) [Batch safety classification]
When `FullyBatchSafe` causes a single-batch build spanning more than 30 partition periods, smelt warns and recommends `--per-partition` or `--batch-size <n>`. The warning is informational only; either flag suppresses it.

### C20 (lines 998) [First-run and backfill]
A first run (no output table) and a backfill (re-run of a written range) follow the same DELETE+INSERT contract — the DELETE is a no-op when the partition is absent. The planner picks a backfill-chunking shape from the batch-safety class.

### C21 (lines 1000) [First-run and backfill]
First-run bootstrap: a non-self-referential model's first run creates its target directly with `CREATE TABLE ... AS SELECT ...` over the first batch. A **self-referential** model cannot take that path; when the target does not exist, the runtime first materialises an **empty** target table carrying the model's inferred output schema (column names and types, derived the same way any downstream consumer's schema is resolved), then executes every batch — including the first — as ordinary partition DELETE+INSERT. The bootstrap is a one-time structural step keyed only on "does the target exist yet", not a property of the batch-safety class.

### C22 (lines 1002–1006) [First-run and backfill]
Chunking by class: `FullyBatchSafe` — a single DELETE+INSERT pair covers any `[start, end)`, no chunking; `BoundedSafe(n)` — auto-sized sub-ranges (3× context, clamped 7–90 partitions), each sub-range one DELETE+INSERT pair, executed sequentially in temporal order; `PerPartitionOnly` — one partition per iteration, sequential, temporal order, each partition one DELETE+INSERT pair.

### C23 (lines 1008) [First-run and backfill]
When per-partition execution is forced (or `smelt backbuild --per-partition` is requested), batches for `Month`/`Quarter`/`Year` advance by **true calendar units**, landing on month/quarter/year boundaries regardless of month length. `Day` and `Week` use fixed 1-day / 7-day steps.

### C24 (lines 1010) [First-run and backfill]
Output grain may be finer than partition grain: a model whose `partition_column` holds monthly boundaries may emit daily/hourly rows within them; batch-splitting operates on the *partition* grain and writes/reads finer rows in their entirety within each partition batch.

### C25 (lines 1012) [First-run and backfill]
Per-chunk transaction boundary: each chunk's DELETE+INSERT is one backend transaction. INSERT failure rolls back that chunk's DELETE; earlier committed chunks do **not** roll back — partial progress is intentional since each chunk is idempotent.

### C26 (lines 1014) [First-run and backfill]
Failure mode: a run halts at the first failed chunk and exits non-zero. Re-running the same `[start, end)` resumes correctly because every committed chunk is idempotent.

### C27 (lines 1016) [First-run and backfill]
smelt does **not** auto-re-run partitions when data arrives late. Interim mitigations: (1) trail `--event-time-end` behind real-time by the source's known latency; (2) run overlapping ranges (e.g. re-process the last 7 days). Per-column `data_latency:` is a planned mechanism (Known Divergences). A late arrival past the derived clamp is silently excluded from the maintenance run; surfacing it is a model-author + data-quality concern; the mitigations only widen the window a late row can still land in.

### C28 (lines 1020–1027) [Per-partition equivalence]
For every partition `p` in the run window `[run_start, run_end)`, the invariant (verbatim):
```
partition_grain_run(model, [run_start, run_end)).where(partition_column = p)
  == full_refresh(model).where(partition_column = p)
```
This is the partition-grain specialisation of the processed-input equivalence invariant and of the plan's `S`-vector refinement, and is independent of run-window size.

### C29 (lines 1029) [Per-partition equivalence]
Column-locality (partition-grain-local): the equality holds for **local** columns — those depending only on source rows visible within the model's source-filter ranges. A column depending on history outside those ranges (cumulative aggregation, connected-components, backward-fill) is **not equivalent**: it forces its source to `Unbounded` and the model to `PerPartitionOnly`; the run is correct as-of-the-run, not equal to a full refresh over final input.

### C30 (lines 1031) [Per-partition equivalence]
The equality is bit-identical on **deterministic** columns; a column with `contract: plausible` need only be a *plausible full-refresh value*. This leniency never extends to a column that governs *which* rows exist, *where* they are partitioned, or *how* they are deduplicated.

### C31 (lines 1035) [Safety checks]
The optimizer rejects a partition-grain model whose SQL uses constructs that break the partition-DELETE-then-INSERT contract. Each check instantiates one §"Per-cell admission" obligation for the recompute-a-region corner, and each is individually disabled via `safety_overrides.allow_<check>: true` (opt-in, recorded).

### C32 (lines 1039) [Safety checks]
Window functions check: admitted when `OVER (PARTITION BY <keys>)` has `<keys>` a **superset** of `partition_column` (Property: *partition alignment*, scoped over window `OVER`). Also admitted when `PARTITION BY` omits `partition_column` but the `OVER` clause carries a bounded `RANGE BETWEEN INTERVAL '…' PRECEDING` frame with no `UNBOUNDED` bound. `UNBOUNDED PRECEDING`, or an `OVER (...)` with no `PARTITION BY`, is **never** admitted this way. Escape hatch: `safety_overrides.allow_window_functions: true`. Instantiates obligation 4 (*bounded reach*).

### C33 (lines 1040) [Safety checks]
`HAVING` check: admitted when the enclosing scope's own `GROUP BY` key is a **superset** of `partition_column` (partition alignment scoped over `GROUP BY`). Instantiates obligation 4.

### C34 (lines 1041) [Safety checks]
`DISTINCT` check: admitted when `partition_column` is projected in the same scope (partition alignment scoped over the select list). Instantiates obligation 4.

### C35 (lines 1042) [Safety checks]
`LIMIT` check: **never** admitted — a row-count cap never commutes with the partition filter. Fails obligation 4 unconditionally.

### C36 (lines 1043) [Safety checks]
Subqueries (`SELECT ... FROM (SELECT ...)`): rejected unless overridden. A `WITH`-clause CTE is **not** gated by this structural check — only a subquery nested in FROM/JOIN is; CTE bodies flow through bound derivation via the *body-structure classifier* property. Instantiates obligation 4.

### C37 (lines 1044) [Safety checks]
Non-deterministic functions check: admitted only when confined to a payload column with `contract: plausible`. Instantiates obligation 6 (*well-defined groups*).

### C38 (lines 1046) [Safety checks]
All partition-alignment checks are evaluated **per scope**: a `UNION` branch's own `HAVING`/`DISTINCT`/window is judged against that branch's own key set, never inheriting alignment from a sibling branch or the outer query.

### C39 (lines 1048) [Safety checks — payload rule]
A non-deterministic value is admitted only when it flows **exclusively** into a column declared `columns.<c>.contract: plausible` — a payload written once per window and never read back to place, filter, group, or dedup a row.

### C40 (lines 1048) [Safety checks — payload rule]
The taint check enforces three **hard exclusions**, rejecting regardless of the opt-in and naming the offending position: (1) the `event_time_column`/`partition_column` expression; (2) any `unique_key` column; (3) any row-set-membership or grouping position (`WHERE`, `HAVING`, `JOIN … ON`, `DISTINCT`, `GROUP BY`, or a window's `PARTITION BY`/`ORDER BY`/frame). Declaring an excluded column `contract: plausible` is a configuration error.

### C41 (lines 1048) [Safety checks — payload rule]
The run-nondeterministic class (`NOW()`/`CURRENT_*`) is additionally admitted as a **direct** SELECT-list projection even into a column without `contract: plausible`, because compile-time pinning freezes it once per run. The row-nondeterministic class (`RANDOM()`/`UUID()`) still requires the target column declared `plausible`.

### C42 (lines 1048) [Safety checks — payload rule]
The blunt `safety_overrides.allow_nondeterministic` drops the guardrail wholesale and is **discouraged** (not refused).

### C43 (lines 1052) [Event-time outer-visibility]
`event_time_column` must be **accessible** at the outermost SELECT for the outer clamp (`WHERE event_time_column >= start AND event_time_column < end`) to bind correctly. A plain `UNION`/`INTERSECT`/`EXCEPT`, a `UNION ALL` whose branches cannot be proven traceable, or a subquery FROM that does not project `event_time_column`, is rejected with `EventTimeColumnNotVisibleAtOuterSelect` (Error) **before execution**.

### C44 (lines 1054) [Event-time outer-visibility]
A `UNION ALL` is **exempt** when every branch's projection of `event_time_column` traces `Traceable` (Property: *event-time monotonicity trace*, distributed by *set-operation distribution*) back to a real source's own partition column — per-source pushdown then narrows each branch's scan independently. A `StaticSeed` branch is named and rejected; a `NotTraceable` branch conservatively keeps the whole-model outer clamp.

### C45 (lines 1058) [Observing the per-source clamp]
Because lookback is derived, not declared, the derived clamp — the window `partition_col ∈ [run_start − before, run_end + after)` each `smelt.<path>` reference is read under — is surfaced to the author via two surfaces, both using the ISO-8601 duration rendering of the bound.

### C46 (lines 1060) [Observing the per-source clamp]
`smelt explain` (`--json`): the per-cell `source_bounds` map reports, per source, its `source_partition_col` and derived `(before, after)` offsets; with a concrete run window it additionally resolves the scan window `[run_start − before, run_end + after)`.

### C47 (lines 1061) [Observing the per-source clamp]
Editor hover (LSP): hovering a `smelt.<path>` reference in a partition-grain model shows that reference's clamp alongside the existing schema/column readout.

### C48 (lines 1063–1070) [Observing the per-source clamp]
The bound outcomes render distinctly: `Bounded(c, 0, 0)` → "read partition-by-partition; no lookback or lookforward"; `Bounded(c, before, after)` → the window `c ∈ [run_start − before, run_end + after)` with `before`/`after` shown; `Unbounded` → "read across all history (cumulative); forces `PerPartitionOnly`"; lookup (no `timeseries:`) → "read in full; not a pushdown candidate".

### C49 (lines 1072) [Observing the per-source clamp]
A `NotDerivable` source is refused at planning time, so the surfaces show the refusal diagnostic instead of a per-source window.

### C50 (lines 1076) [Functions inside partition-grain bodies]
Function expansion (`expansion.md`) runs **before** every analysis stage: bound derivation, source-filter pushdown, and most batch-safety sub-checks see the expanded CST — a `LAG()` inside a `smelt.define` body and one inlined at the call site are indistinguishable. The outer output-clamp is injected at the outermost expanded query; source-filter pushdown reaches `smelt.<path>` references originating inside a `smelt.define` body. **Exception:** the `OVER`-clause admissibility sub-check scans the outer model SQL before expansion (Known Divergences).

### C51 (lines 1078) [Functions inside partition-grain bodies]
Opaque calls remain black boxes: bound derivation cannot read through `smelt.extern`/built-ins. A partition-grain model whose time-dependence is hidden behind an opaque call is `NotDerivable` and refused, **unless** a bound is provable from the surrounding SQL (a WHERE clause, an explicit RANGE-windowed projection).

### C52 (lines 1082) [Window independence and self-referential models]
Whether windows may be built in parallel or must be sequential in temporal order is the *window-independence / ordered-execution* property, **derived from the model's dependency graph, never declared**.

### C53 (lines 1084) [Window independence and self-referential models]
Window-independent is the default: every window is a pure function of source rows in its own scan range (widened by derived lookback). The entire safe slice the recompute corner admits is window-independent — lookback reaches into *sources*, never the model's own earlier partitions — so a backfill of `[t₀, tₙ)` may split into sub-ranges built in any order, including in parallel.

### C54 (lines 1085) [Window independence and self-referential models]
A **self-referential** partition-grain model (reading its own prior partitions via `smelt.<self>`) is **in scope** and still executes as partition DELETE+INSERT — it stays a partition-addressed table and does **not** become key-grain — but its windows must be built **sequentially in strict temporal order**, and its backfill may not be parallelised or reordered.

### C55 (lines 1085) [Window independence and self-referential models]
A self-edge the planner cannot prove converges partition-by-partition (a self-reference reading *forward* or across all history) is **refused at planning time**, never silently mis-parallelised.

### C56 (lines 1087) [Window independence and self-referential models]
A self-referential partition-grain model is *stateful-ordered* in execution yet keeps the partition-grain *output shape*: partitioned, per-partition-equivalent within each window's input.

### C57 (lines 1089) [Window independence and self-referential models]
Ordered execution composes with the derived output window: when the `partition_column` is derived and a genuine Form B relation — anchored on a **non-self** source — declares skew, a run requesting `[D, D+1)` also rewrites the skew-reached neighbouring partitions, exactly as for a window-independent model. Ordering then applies over the *rebased* partitions: every partition in the rebased range builds strictly sequentially, in temporal order, one partition per batch.

### C58 (lines 1089) [Window independence and self-referential models]
The self-edge itself is **never** a skew anchor — its bounding relation (the backward-bounded read proving the `Ordered` verdict) is a distinct convergence mechanism, not a partition-column skew declaration, even when the self-referenced table's column shares the model's own `partition_column` name.

### C59 (lines 1093) [State ownership]
smelt does not track watermarks, offsets, or run history for partition-grain models — the backend owns computational state (DuckDB: table state + transactions; future Delta/Spark: transaction log + MERGE; future Flink: checkpoints). Optional run-state tracking with gap detection is opt-in via `state.mode: intervals` (`virtual_environments.md`); on-disk layout owned by `run_state.md`.

### C60 (lines 1097) [`partition_column` validation]
Partition-column projection validation is owned by `timeseries.md` §"Constraints & Invariants" rule 1: `partition_column` must appear in the model's output `SELECT` (and in the `GROUP BY` when grouping is present), else `MalformedTimeseries`. The partition-grain rule consumes that guarantee rather than re-checking.

### C61 (lines 1099–1101) [The key grain]
The key grain (`grain: key`) is the key-addressed shape: keyed state, one row per `unique_key`, kept current by the fold-a-delta corner (keyed `merge_into`). Its declared surface is §"Key-grain declaration (`grain: key`)"; the machinery is key-grain-**local**.

### C62 (lines 1103–1107) [The two run shapes]
The run shape is **derived, never declared** — the keyed application of the input-consumption axis, derived from the driving source. **Window-forward**: the FROM clause contains exactly one source whose resolved target declares `timeseries:` (the **driving source**, resolved by the shared driving-fact / anchor proof). Zero clocked sources means snapshot-reconcile; **two or more is `KeyedMultipleDrivingSources`**.

### C63 (lines 1107) [The two run shapes]
Window-forward execution: the run steps over the source partitions covered by `[run_start, run_end)` **in temporal order**; for each partition, source-filter pushdown injects the partition's window onto the driving source's reference, the per-partition delta SELECT executes, and a `merge_into` folds the delta into the target with the per-column combiner map. Non-timeseries sources (lookups/dimensions) are read in full each step. If the output table does not exist at the first step, it is created from that step's delta (`CREATE TABLE AS SELECT`).

### C64 (lines 1108) [The two run shapes]
Snapshot-reconcile: no clocked source. The run re-scans the source whole, computes the per-key aggregation, and `merge_into`s the result — matched keys overwritten, unmatched inserted. A key present in the store but **absent from the incoming scan is retained** unchanged; deletion requires an explicit mechanism (out of scope, §Known Divergences).

### C65 (lines 1110) [The two run shapes]
Out-of-order, parallel, or sliced-backfill window application is admitted **iff the model is order-independent**; otherwise windows must be applied sequentially in temporal order.

### C66 (lines 1112–1116) [Derived execution postures]
Three model-level properties are folded from the column families; each is **derived, surfaced by `smelt explain`, never declared**. Posture 1, **re-run tolerance**: an already-merged window may be blindly re-merged over unchanged input iff every column is idempotent, i.e. **no additive-fold column** (repeated window converges: `GREATEST(x, GREATEST(x, y)) = GREATEST(x, y)`); additive models double-count and must be refused (via the ledger).

### C67 (lines 1117) [Derived execution postures]
Posture 2, **order-independence**: windows may be applied out of order or in parallel iff every column's combiner is order-independent — the extremal/lattice and proven once-write families qualify; the **order-monotone overwrite family does not** (order-independence holds only up to ordering-key ties, which are not statically excludable), so any model with an overwrite column executes windows sequentially in temporal order.

### C68 (lines 1118) [Derived execution postures]
Posture 3, **reprocessing refusal**: a window whose *input changed* since it was merged must not be re-merged for **any** family — an irreversible fold cannot un-see a removed contribution, and an overwrite cannot retract a superseded-by-nothing value.

### C69 (lines 1120–1122) [The transactional merge ledger]
Every **window-forward** keyed model maintains a per-model **ledger** — a small backend table recording each merged window — written **in the same backend transaction** as that window's `merge_into`.

### C70 (lines 1124) [The transactional merge ledger]
For additive-fold models (not re-run tolerant): a run whose window is already ledgered is **refused** (`KeyedReprocessedWindow`) — exactly, not best-effort. Crash resume merges only unledgered windows; a run interrupted at window *k* of *n* resumes correctly by re-running the same range.

### C71 (lines 1125) [The transactional merge ledger]
For re-run-tolerant models: a ledgered window may be re-merged (a no-op on unchanged input); the ledger serves reprocessing detection and `--auto` bookkeeping, **not refusal**.

### C72 (lines 1127) [The transactional merge ledger]
Snapshot-reconcile models keep **no ledger** — each run is a self-contained reconciliation and re-running is always safe. The ledger is backend-resident and transactional with the write it describes; it is a **correctness structure**, distinct from the opt-in run-state observability surface (`run_state.md`).

### C73 (lines 1129–1131) [Admission matrix]
The admission matrix is the key-grain instance of §"Per-cell admission": each cell discharges obligations 2 ("faithful fold") and 3 ("combiner algebra class") for one `(column family × run shape)` pair. Fold families consume **events** (each row contributes exactly once — faithful only under a replayable, retraction-free feed); overwrite families consume **observations** (each row supersedes — faithful only under the snapshot's current-state semantics, never a fold). The matrix is checked **per column**.

### C74 (lines 1131) [Admission matrix]
The faithful-fold obligation binds **fold-contributing sources** — sources whose rows the cumulative combiner actually folds — not every source the model's `FROM` clause names.

### C75 (lines 1133–1139) [Admission matrix]
Matrix cells: additive fold — ✓ window-forward (obligation 2, ledger-enforced), ✗ snapshot-reconcile (re-folding state double-counts, fails obligation 2). Extremal/lattice fold — ✓ window-forward, ✗ snapshot (observer semantics, fails obligation 2). Order-monotone overwrite — ✓ window-forward, ✗ snapshot (observer semantics). Once-write — ✓ window-forward (obligation 2, provenance proof), ✗ snapshot (observer semantics). Plain overwrite — ✗ window-forward (order-dependent over events, fails obligation 3; `KeyedUnknownCombiner` names the `MAX_BY` fix), ✓ snapshot-reconcile (obligation 3, current-snapshot semantics).

### C76 (lines 1141) [Admission matrix]
The three snapshot ✗ cells marked *observer semantics* are not double-count hazards (those families re-merge safely) but **equivalence failures**: `MIN(price)` folded over snapshots computes *min ever observed* vs the current min; `MAX_BY(attr, updated_at)` retains a stale incumbent if the ordering value regresses; `COALESCE`-once-write captures *first observed*. Each is refused with `KeyedSnapshotSourceUnsupportedColumn` rather than admitted silently — obligation 2 **fails closed, never approximated**.

### C77 (lines 1143) [Admission matrix — scope]
A mutable source the model consumes **only** through a covered enrichment cell (an `UpstreamMutation`-triggered column-scoped `MERGE`) is admitted regardless of its own mutation profile: its post-creation mutations are maintained by that separate cell. A source that is **both** a fold input and a mutable enrichment stays refused with `MaintenanceNoAdmissibleTechnique` — admission fails closed rather than approximating which of a source's columns are "safe".

### C78 (lines 1145–1149) [End-state equivalence: the SQL is the oracle]
The key grain upholds the end-state specialisation of the equivalence invariant, and because the body is required to be the aggregation itself, the oracle is the model's **own SQL**, executable for every admitted model. Window-forward form: for any set `S` of processed driving-source partitions and any admitted ordering over `S`, the stored state equals the model SQL evaluated over `source.where(partition ∈ S)`. For overwrite columns this holds **up to ordering-key ties**.

### C79 (lines 1150) [End-state equivalence: the SQL is the oracle]
Snapshot-reconcile form: the stored row for every key **present in the current snapshot** equals the model SQL evaluated over that snapshot. Keys absent from the snapshot are retained — a named divergence from the oracle relation (the stored table is the oracle's rows plus retained departed keys).

### C80 (lines 1152–1154) [No write-eligibility clamp]
There is **no write-eligibility clamp**: a run merges **every** delta row it scans, into whatever key it names, however old that key is. A derivable forward reach is computed and reported (`smelt explain`) but never gates admission and never bounds which keys a run may touch — no scanned input is ever silently dropped.

### C81 (lines 1156–1158) [Key temporal locality]
A keyed model may time-partition its output with a `timeseries:` block (grammar/structural rules: `timeseries.md`; the named columns must be projections of the model, and `event_time_column` may name the partition column itself). Admission requires **key temporal locality**: every stored row a run's deltas can touch lies within a computable **slice** of the output's time axis. Locality is what allows the `merge_into` target scan to be pruned to the slice and downstream consumers to window over the output.

### C82 (lines 1160–1164) [Key temporal locality — preconditions]
Structural preconditions, checked before the routes: (1) the run shape is **window-forward** (snapshot-reconcile establishes no locality); (2) `partition_column` names either a `unique_key` column or a non-key projection in the extremal-fold, order-monotone-overwrite, or once-write family, provably NOT NULL from a key's first stored row (`timeseries.md` validation rules); (3) the block's `granularity` equals the driving source's granularity.

### C83 (lines 1166–1168) [Key temporal locality — routes]
Route 1, **key-embedded**: `partition_column` is a `unique_key` column. A stored row's partition value is its key's own; a delta touches exactly its own partition values. Slice: the run's scan window, widened by the derived lateness/skew margins.

### C84 (lines 1169) [Key temporal locality — routes]
Route 2, **key-determined**: the partition projection is a per-key constant under the once-write provenance proof — a key-derived expression, or a declared functional dependency over a column present non-null on every input row. Slice: the delta's own partition values — exact **regardless of key age**.

### C85 (lines 1170) [Key temporal locality — routes]
Route 3, **recurrence-bounded**: a key-recurrence bound `r` holds — every pair of input rows sharing a key lies within `r` of each other on the event-time axis. `r` is derived from the model's SQL where statically decidable; otherwise declared on the driving source (`sources.md` §"Source YAML shape", `key_recurrence`). Slice: the scan window widened backward by `r`, plus the derived margins.

### C86 (lines 1170) [Key temporal locality — routes]
A **declared** `r` is admitted only **checked**: the run verifies at merge time that no delta row matched (or would duplicate) a stored key outside the slice, and any violation fails the run **transactionally** (`KeyedRecurrenceBoundViolated`). A declaration can bound work; it can never silently drop data.

### C87 (lines 1172) [Key temporal locality]
Pruning is **not** a write clamp: slice pruning is no-op elimination on the merge's **target scan** — rows outside the slice provably cannot match a delta key (routes 1–2) or are checked not to (route 3). Every scanned delta row still merges. General principle: only proofs prune; a declared bound is admitted only checked; no unproven bound ever refuses a write.

### C88 (lines 1174) [Key temporal locality — row movement]
Under routes 1–2 a key's partition value never changes. Under route 3 it may move (an extremal or overwrite partition projection superseded by a late row); the merge updates the stored row **in place, partition value included**, and both old and new values lie within the slice by the bound. Movement does not change the derived postures — an overwrite column still forces sequential temporal order.

### C89 (lines 1176) [Key temporal locality — per-slice equivalence]
With locality established, the invariant is additionally checkable slice-by-slice: for any output slice, the stored rows equal the model SQL evaluated over the source rows within the slice's derived reach — the keyed analogue of the partition grain's per-partition strengthening.

### C90 (lines 1178) [Key temporal locality — output as clocked source]
An admitted block makes the output a clocked, time-partitioned table: downstream partition-grain models receive source-filter pushdown against it, and a downstream keyed model may take it as its clocked driving source — the clock propagates through the DAG instead of stopping at the keyed stage.

### C91 (lines 1178) [Key temporal locality — settle bound]
The output's **settle bound** — how long a written slice may still change — is derived and surfaced by `smelt explain`: under route 1 a slice settles with the source's lateness margin; under route 3 after `r` plus the margins; under route 2 it **never settles** (a late delta may touch an arbitrarily old slice). A re-written slice is *changed input* to downstream consumers, handled by the ordinary staleness machinery (§"Interaction with `--auto` / staleness").

### C92 (lines 1180–1191) [What the composed shape uniquely enables]
Propagation admissibility: a bare keyed node refuses in the graph layer — it has no partition axis to carry interval dirt. A locality-admitted keyed output participates in forward propagation and backward resolution as a clocked node, its edges at the declared `timeseries.granularity`; the composed shape is the only way a keyed stage can sit *inside* a propagation chain rather than terminating it.

### C93 (lines 1192–1197) [What the composed shape uniquely enables]
Exact key→partition dirt projection: under locality routes 1–2 a stored row's partition value is a per-key constant, so a key-level change set projects to **exact** partition intervals — the keys' own partitions, no widening. Under route 3 the projection widens backward by `r` plus the derived margins (**widen-never-narrow**).

### C94 (lines 1198–1203) [What the composed shape uniquely enables]
Slice-bounded no-op write elimination: the conditional write (§"Windowed maintenance and the horizon", category 2) must read stored rows to compare against candidates; on a bare keyed output that read is the whole key space, on a composed output it is bounded by the pruned target slice.

### C95 (lines 1204–1208) [What the composed shape uniquely enables]
Settle-bound × observed-delta composition: consumers skip settled slices unconditionally and skip unsettled slices whose observed delta is empty; a stable upstream chain degenerates to empty-delta no-ops with a provable horizon behind it.

### C96 (lines 1210–1211) [What the composed shape uniquely enables]
The first two bullets (propagation admissibility, dirt projection) bind at the graph layer, the third (no-op write elimination) at statement emission, the fourth (settle × delta) across both; implementation status is recorded in §Known Divergences.

### C97 (lines 1213–1215) [The maintenance boundary]
On the algebraic ladder the keyed families sit on the **direct-monoid rung**: every catalogued combiner folds `(state, delta)` with no inverse and no history re-read. The additive family is additionally a **group** (invertible) — what a future subtract-then-add reprocessing path would exploit; the idempotent families are monoids but not groups (a folded contribution cannot be un-seen), which is why reprocessing is refused for them.

### C98 (lines 1215) [The maintenance boundary]
Rungs 2–4 (decomposed state + presentation view for `AVG`-class aggregates; group-rung retraction; the opt-in bounded-domain multiset for exact holistic aggregates) grow this shape without changing its contract; transforms catalogued in `model_transforms.md`, the `bounded_domain:` budget declaration in `model_properties.md`. Beyond the ladder — general-operator retraction over joins, unbounded non-additive state — is delegated to `refresh: materialized_view`.

### C99 (lines 1217–1219) [Reprocessing]
If a merged window's source data changes, re-running it does not produce correct state for any family. The rule refuses at planning time when it can detect it — the ledger says the window was merged; `--auto` staleness says the input changed — with `KeyedReprocessedWindow` pointing at the two mitigations: `--full-refresh` (truncate-and-rebuild), or a manual cascade rebuild. Subtract-then-add for all-invertible models is a future path.

### C100 (lines 1221–1223) [Ordering ties]
The pairwise combiner for `MAX_BY(value, ordering)`: the delta wins iff `delta.ordering > target.ordering` (strict); **on equality the incumbent (target) wins**. This is deterministic given the processing history but **not order-independent when ties occur across windows** — which is why overwrite columns force sequential execution. Recommended practice: a composite, provably-tie-free ordering expression (e.g. `(updated_at, source_seq)`); the classifier cannot verify uniqueness and does not claim to.

### C101 (lines 1225–1227) [Enrichment joins]
A fact-to-dimension join bringing an enriching event in as a separately-arriving relation is admitted when its per-key contribution is **provably monotone** (join-contribution monotonicity proof): the contribution feeds only extremal, order-monotone, or once-write columns and does not fan into a decrementing aggregate. The maintainability line is monotone-vs-retractable **semantics, not join-vs-union spelling** — the join form is normalised to the same keyed-monoid merge as the union form. Only a genuinely retractable contribution is refused (`KeyedRetractableContribution`).

### C102 (lines 1227) [Enrichment joins]
A **re-scanned existence flag** additionally requires the dimension source declared `append_only` (`sources.md`); extremal milestones are safe regardless of that declaration.

### C103 (lines 1227) [Enrichment joins]
Where a dimension batch's forward reach `H` is **derivable from the model's SQL**, the dimension-driven horizon-bounded MERGE may clamp the enrichment *recompute* to `[event_ts, event_ts + H]` — a scan-side bound that cannot under-cover because it is derived. Where `H` is not derivable, the transform is not licensed and the enrichment evaluates through the ordinary widened scan. No declared value ever truncates a recompute or a write.

### C104 (lines 1229–1231) [Key-grain output shape]
One row per `unique_key`; column names are the projection's `AS` aliases (or source column names). By default there is no `partition_column`, no `event_time_column`, and no `timeseries:` on the model, and downstream consumers see the output as a lookup table read in full each run, identical to any non-timeseries source. With an admitted `timeseries:` block the output is instead a clocked, time-partitioned keyed table — still one row per key — that downstream consumers window over like any clocked source.

### C105 (lines 1233–1235) [Functions inside keyed bodies]
Function expansion runs **before** the classifier: projection reading, GROUP-BY inspection, FROM-clause walking, family classification, and pushdown operate on the expanded CST. A `smelt.define`-resolved call is admitted iff its expanded body produces a catalogued aggregator at the outermost expression position — the pattern functions are admitted exactly this way, with no privileged treatment. Opaque calls (`smelt.extern`, non-inlinable built-ins) in the projection list are rejected via `KeyedUnknownCombiner`.

### C106 (lines 1237–1240) [Interaction with `--auto` / staleness]
Window-forward: stale driving-source windows are re-processed subject to posture — re-run-tolerant models re-step exactly the stale windows (safe by idempotence); additive models refuse re-processing of ledgered windows (`KeyedReprocessedWindow`) and steer to `--full-refresh`. Snapshot-reconcile: the model is treated as **always-stale**; every `--auto` run reconciles.

### C107 (lines 1242–1244) [Interval versioning]
`versioning: interval` is the key grain's history-keeping sub-declaration (SCD2): keyed state plus a validity interval per version. Its declared surface is §"Interval-versioned declaration (`versioning: interval`)"; the machinery is local to `versioning: interval`.

### C108 (lines 1246–1248) [End-state equivalence (interval-keyed)]
The profile upholds the end-state equivalence invariant in its interval-keyed specialisation: the user-visible set of `(key, version, validity interval)` rows equals what a full rebuild would compute from the same set of processed snapshots, **independent of the order in which non-overlapping snapshots were merged**. smelt owns freshness (pull) — the history is correct as of the last `smelt build`.

### C109 (lines 1250) [End-state equivalence (interval-keyed)]
Order-independence holds because validity is anchored to the source's event time, not the run clock: the close-old / open-new combiner reads versions in event order via the **driving-fact / anchor resolution** and **ordered-execution** proofs, so replays and out-of-order windows converge to the same history rather than shifting interval boundaries.

### C110 (lines 1252–1257) [Input consumption is derived from the source]
How new input is discovered is **never declared on the model**; it is derived from the source's shape. **Window-forward**: a source carrying `timeseries:` (update-events / CDC feed) is consumed in `--event-time` run windows applied to the *source's* `partition_column`; only the new tail is read (source-filter pushdown); windows are applied in temporal order (ordered execution) because the combiner consumes versions in event order. **Snapshot-diff**: a mutable snapshot source (no monotone clock) is re-scanned each run and compared against the stored current versions; the end-state contract is identical, only the scan cost differs.

### C111 (lines 1259) [Input consumption is derived from the source]
The choice between the two routes is the mutation-profile world-fact (`sources.md`) feeding the input-delta-discovery proof (`model_properties.md`); moving along this axis never changes the equivalence contract, only what is scanned.

### C112 (lines 1261–1265) [Interval-versioning admission]
Every admission check for this profile is one instance of §"Per-cell admission" for the fold-a-delta corner over a key-grain-plus-interval output. Obligations 1–2 (replayable input / faithful fold): the combiner consumes an update-events/CDC feed (replayable, append-only) **or** a mutable snapshot (re-scanned whole each run) — either discharges the obligation for its own consumption route, **never a hybrid of the two on one model**.

### C113 (lines 1266) [Interval-versioning admission]
Obligation 3 (combiner algebra class): the combiner is the profile's own local machinery, not a catalogued key-grain column family; it is admitted **once per model, not per column**, because every tracked attribute is folded through the same close-old / open-new step.

### C114 (lines 1267) [Interval-versioning admission]
Obligations 4–5 (bounded reach / bounded footprint): window-forward — the reach is the run's event-time window on the driving source; the footprint is the set of keys touched by that window's rows. Snapshot-diff — reach and footprint are the whole snapshot and the whole key space: an **intentional escape hatch** for a source with no monotone clock, not a derivation gap.

### C115 (lines 1268) [Interval-versioning admission]
Obligation 6 (well-defined groups): all tracked attributes plus the validity columns form **one column group**; a version change is a single indivisible event across every tracked column, so there is no sub-model factoring to compute.

### C116 (lines 1272–1279) [Close-old / open-new combiner]
The combiner, per incoming row keyed by natural key: (1) look up the key's current (open) version in the stored table; (2) if no current version exists, **open** a new version — insert with `valid_from` = the incoming event time, `valid_to` = open, `is_current = true`; (3) if a current version exists and a **tracked attribute** differs, **close** the old version (set `valid_to` = the incoming event time, `is_current = false`) and **open** a new one at that boundary; (4) if a current version exists and no tracked attribute differs, do nothing — no spurious version.

### C117 (lines 1281) [Close-old / open-new combiner]
The close and the open share the same boundary timestamp, so intervals abut without gaps or overlaps. The mechanism is emitted as a keyed `merge_into` — matched keys close-and-reopen, unmatched keys open — so history is never re-read wholesale.

### C118 (lines 1283–1285) [Validity columns (smelt-managed)]
`valid_from`, `valid_to`, and `is_current` are **managed by smelt**, not projected by the user's SELECT: the user projects only the natural key and the tracked attributes; smelt appends and maintains the interval columns. The open interval's `valid_to` is either NULL or a far-future sentinel — **undecided** (§Known Divergences). `is_current` is a convenience flag equivalent to "`valid_to` is open" that indexes the current-version lookup the combiner performs every run.

### C119 (lines 1287–1289) [Tracked-attribute selection]
A new version is opened for a key only when a **tracked attribute** changes between the stored current version and the incoming row. By default **every projected non-key column is tracked**. Whether a modeller can mark a column *untracked*, and whether that is derived or declared, is an Open Question; the posture is to derive the key and tracked set from the SQL where unambiguous rather than restate them in a strategy block.

### C120 (lines 1291–1293) [Validity stamped from source event-time]
`valid_from` / `valid_to` boundaries are stamped from the **source's event time** — the update-events feed's event-time column, or the snapshot's as-of timestamp — **never the run clock**. Re-running a window, or backfilling windows out of order, reproduces byte-identical interval boundaries, so end-state equivalence survives replays; a run-clock stamp would break order-independence.

### C121 (lines 1295–1297) [Deletion handling]
A key present in the store but absent from the incoming set is a **retraction**, handled as a **soft-close**: the key's current version is closed (`valid_to` set, `is_current = false`) with no new version opened. The event time used is the run's window boundary for a window-forward feed, or the snapshot's as-of time for snapshot-diff.

### C122 (lines 1297) [Deletion handling]
A hard delete (physically removing the key's rows) is **not** the default. The exact surface for opting into a hard delete, and for *late corrections* to an already-closed interval, remain Open Questions (the retraction question the key grain shares). A CDC feed carrying explicit delete events resolves deletion directly: the delete event is the close signal.

### C123 (lines 1301–1303) [Interactions]
The equivalence invariant, ladder, horizon, and validator-not-chooser are owned above in §Semantics; the plan's per-cell theorem is the `S`-vector refinement of the invariant, and per-cell choice operates strictly inside the validator-not-chooser rule.

### C124 (lines 1304–1307) [Interactions]
Output shape/grain declaration and the refresh trichotomy are owned by `models.md`; the plan validates against them. The **declaration law and litmus rule** (`models.md` §Design) — whether a fact is declared, derived, or implied, and whether a proposed combination earns a new peer shape — are likewise owned there; this spec consumes them.

### C125 (lines 1308–1313) [Interactions]
Input-consumption (`models.md` §"Input-consumption axis") — which input rows are new — is a derived, cross-cutting axis (mutation-profile world-fact → input-delta-discovery proof in `model_properties.md` → re-scan/probe transform in `model_transforms.md`). Moving along it never changes the equivalence contract, only what is scanned. The **default** is windowed (clocked source → window-forward); full scan is the fallback for a clockless snapshot source.

### C126 (lines 1314–1315) [Interactions]
Source postures (`mutation_profile`, lateness, retention, delta identity, unique keys) are declared in `sources.md` and consumed by admission; their runtime tripwires live there.

### C127 (lines 1316–1318) [Interactions]
The technique primitives (`merge_into`, DELETE+INSERT, column-scoped merge, targeted backfill) are catalogued in `model_transforms.md`; the outer output clamp is the subquery wrap over the model's output schema defined there.

# Claims inventory — incremental_models.md lines 1320–1624 (## Design, ## Constraints & Invariants)

### D1 (lines 1322–1328) [Design] (design-decision)
Strategy *content* of the refresh enum is derived per `(group × trigger)` cell — one model is simultaneously append-driven, merge-driven, and recompute-driven at different cells — while *shape* and *grain* stay declared-and-checked. Normative source: `docs/research/20260705-refresh-as-maintenance-plan/01-framework.md` §10, §13.

### D2 (lines 1326–1328) [Design] (rejected-alternative)
Deriving *shape* (not just strategy content) was rejected because it reintroduces the silent contract swap the declaration law exists to prevent — a projection refactor could flip downstream consumption semantics with no diagnostic.

### D3 (lines 1330–1333) [Design] (design-decision)
Column factoring is by **mutation-sensitivity**, not syntactic provenance: a column reading a second input's *immutable-at-creation* value must not inherit that input's mutation-sensitivity, or the plan degenerates and targeted cells are lost. This is what makes the append-only declaration on a source load-bearing (01 §5).

### D4 (lines 1335–1338) [Design] (design-decision)
Dirt keys trigger cells **per edge**; a dirty set merged per model was rejected because it would erase which repair runs where — two sources landing in one tick genuinely drive different techniques over different regions of the same table (`10-dependency-propagation.md` §3; ratified P4).

### D5 (lines 1340–1343) [Design] (design-decision)
**Widen-never-narrow**: every approximation in the plan and graph widens (partial-day clamps ceil outward, coarse grains align outward, whole-partition dirt over-runs, an unclocked delta dirties everything). Widening costs compute; narrowing costs correctness silently. Guardrails (K8) exist so widenings are *visible* costs, refused by default when unbounded.

### D6 (lines 1345–1348) [Design] (design-decision)
Grain is **declared** (`timeseries.granularity`), consistent with the shape anchor: deriving it from a `date_trunc` projection was rejected because propagation grain governs downstream scheduling and a refactor would silently change scheduling semantics; the declaration is checked instead (ratified P3).

### D7 (lines 1350–1354) [Design] (design-decision)
Forward reflection and backward resolution are **one edge object** run in opposite directions (the scan/footprint duality of 01 §5 lifted to the graph); keeping them one object makes the test-build story (backward) automatically consistent with the scheduling story (forward). The adjointness containment is the honest statement of their relationship (`10` §2).

### D8 (lines 1356–1365) [Design] (design-decision)
Offline cost measurement is first-class (`smelt bakeoff`): because per-cell technique choice is contract-preserving at fixed `S`, smelt may measure alternative physical plans over real data offline and pin the cheapest — a capability per-query optimisers structurally lack (01 §11).

### D9 (lines 1358–1362) [Design] (design-decision)
Bakeoff measurement is **real, not simulated**: each candidate technique executes the project's actual `execute_project` pipeline against a representative window of the project's own data, redirected to a disposable scratch schema.

### D10 (lines 1362–1365) [Design] (design-decision)
Pinning is deliberately a **human act, never automatic**: `--pin` only emits the winning choice as YAML for review; applying it is a separate explicit step, and the applied pin remains subject to the same admission proof as any other override.

### D11 (lines 1367–1372) [Design] (rejected-alternative)
An earlier cut splitting the contract into "per-partition equivalence" (partition grain) and "end-state equivalence" (key grain), one per output shape, was rejected as miscast: order/set-determinacy falls out of the **single** invariant for *every* shape, and per-partition equivalence is a *strengthening* of that one invariant, not a peer.

### D12 (lines 1372–1377) [Design] (design-decision)
The real physical-transform axis is how a write **addresses rows** — partition-addressed (identity-free whole-partition rewrite) vs key-addressed (identity-requiring `merge_into`, reaching stored rows by key outside the input window) — and addressing is a property of *a write*, not of *a model*. Declared shape facts (clock, identity) fix which addressings are **available**; each `(column-group × trigger × changed-input)` cell derives its own addressing via the available-addressings rule.

### D13 (lines 1377–1383) [Design] (design-decision)
SCD2 is the proof that addressing is intrinsic to the *write*, not the source clock: its close-out write escapes the input time-window (keyed regardless of source clock) while the same model's creation cell is region-addressed. `grain: partition`'s old "addressed by whole-partition rewrite" half is derived per-cell; only "a stored row is one row of a complete clocked table" stays declared (`models.md` §Design). Full derivation: `docs/research/20260716-relation-contract-and-per-cell-addressing.md`.

### D14 (lines 1385–1391) [Design] (design-decision)
Within a cell the two mechanisms stay **binary**: region-overwrite vs keyed-merge is the write-scope corner; the concrete pattern realizing the corner comes from an **open registry**, so the mechanism set grows without the corner distinction changing. Key temporal locality does not change addressing — a keyed write is still a keyed `merge_into`; locality adds a proof about *where* addressed rows can live, licensing target pruning, a time-partitioned keyed output, and per-slice equivalence.

### D15 (lines 1391–1394) [Design] (rejected-alternative)
Promoting key temporal locality to a **third addressing pole** was rejected: it would suggest a different write primitive and identity requirement where there is none, and would misplace a per-model derived/declared fact as a shape property (`docs/research/20260705-keyed-time-superset.md`).

### D16 (lines 1396–1406) [Design] (design-decision)
The partition and key axes **compose**; exclusivity is the recurring error. The composed shape is deliberately first-class (propagation through keyed stages, exact dirt projection, slice-bounded write suppression pay best there). Reviewers must treat one-or-the-other ("partitioned or keyed") phrasing anywhere in the corpus as a defect against the composed-shape sections, not a stylistic nit.

### D17 (lines 1408–1414) [Design] (design-decision)
**Scope maps** name the per-input dispatch: without the name the run shape reads as a property of the *model*, hiding that different inputs changing engage different targeted recomputes (fact delta folds forward; dimension delta probes and horizon-merges; definition diff backfills columns; self-edge forces ordering). The name gives per-input world-fact verdicts and future multi-clock driving-source work a stable home (`docs/research/20260705-keyed-time-superset.md` §5).

### D18 (lines 1416–1422) [Design] (design-decision)
**Windowed by default; full scan is the surfaced fallback.** Treating full-table recomputation as baseline and windowing as per-shape optimisation was rejected as inverting the real economics: a clocked model can always be maintained over a bounded scan window; only clock absence forces a wider read. Join optimisation is pushed to the engine over a safe widened scan rather than smelt hand-computing minimal deltas.

### D19 (lines 1421–1422) [Design] (design-decision)
Output addressing (partition vs key) is **orthogonal** to scan windowing: a key-addressed model windows its scan yet writes back by key.

### D20 (lines 1424–1432) [Design] (design-decision)
**The horizon is derived, not declared** (from the model's reach); a declared horizon is admitted only as a *ceiling* that warns when the derived value would exceed it. Trusting a declared horizon was rejected because an under-estimate silently corrupts the clamp, dropping rows still within the model's reach. Softening (widening beyond derived reach) is possible later; the safe default is derive-for-correctness, consistent with derive-else-declare (`models.md` §Design).

### D21 (lines 1427–1430) [Design] (design-decision)
Because the derived clamp *is* the model's SQL, a late arrival beyond the horizon is **silently excluded rather than diagnosed** — surfacing lateness is a model-author + data-quality-check concern, not a maintenance guarantee.

### D22 (lines 1434–1436) [Design] (rejected-alternative)
Auto-selecting or silently downgrading the declared shape was rejected — it reproduces dbt's `strategy:` footgun where the effective contract is invisible. The declared shape is authoritative; the machinery only proves or refuses it (**validator, never chooser**).

### D23 (lines 1438–1451) [Design] (design-decision)
Capability placement is **definitional, not consumer-counted**: a capability whose verdict is stateable without naming a shape profile lives in a capability spec (`model_properties.md` / `model_transforms.md`); one meaningful only inside a profile lives in that profile's section (or `materialized_view.md`). Every capability gets exactly one home (what lets `smelt:validate` catch drift), with no mechanical ≥N-consumer rule — building before a second consumer exists is fine.

### D24 (lines 1446–1451) [Design] (design-decision)
The invariant and ladder live in the **shared** sections because every shape profile cites them as its contract; keeping them in one profile's section would force siblings to reach into it. The key grain (§"The key grain (`grain: key`)") remains the reference implementation of the key-addressed maintenance path (retraction, reprocessing, presentation-purity) with its column-family catalogue.

### D25 (lines 1453) [Design → Rejected alternatives] (rejected-alternative)
A `strategy:` sub-knob was rejected — dbt's invisible-contract footgun.

### D26 (lines 1453–1455) [Design → Rejected alternatives] (rejected-alternative)
A new `smelt-maintenance` crate was rejected — the derivation needs the tightest coupling to the sibling classifiers; the module boundary is kept extraction-mechanical instead (`08-code-placement.md` §2.1).

### D27 (lines 1455–1457) [Design → Rejected alternatives] (rejected-alternative)
Qualifying the output clamp to a resolved inner alias was rejected — it answers a question the output clamp must never ask (`03-design-forks.md` F1).

### D28 (lines 1458–1460) [Design → Rejected alternatives] (rejected-alternative)
Per-edge grain declarations were rejected — two declarations can disagree; resolved by the derived label + check-only assertion (§"Grain is a derived label").

### D29 (lines 1460–1461) [Design → Rejected alternatives] (rejected-alternative)
A *declared model-wide* addressing token was rejected — the per-cell plan already knows better (§"Addressing is per-cell", `docs/research/20260716-relation-contract-and-per-cell-addressing.md`).

### D30 (lines 1462–1463) [Design → Rejected alternatives] (rejected-alternative)
A **closed** write-pattern enum baked into the surface was rejected — it bakes today's engines in (§"The write-pattern set is open").

### D31 (lines 1463–1465) [Design → Rejected alternatives] (design-decision)
Deeper rationale for the Design section is recorded in `docs/research/20260705-refresh-as-maintenance-plan/` (parts 01–10), with ratification records in 09 §1 and 10 §11.

### D32 (lines 1467–1469) [Partition-grain design] (design-decision)
The partition-grain design section carries only partition-grain-**specific** rationale; each shared property/transform's rationale lives in its owning spec, and the derive-strategy/declare-grain rationale lives in `models.md` §Design.

### D33 (lines 1471) [Partition-grain design] (design-decision)
**Logical SQL is pure; the framework injects the time filter.** A model body never contains `is_incremental()` or conditional full-vs-incremental branching; the same SQL is both descriptions; the framework injects the outer clamp and drives pushdown. The trade-off — partition-grain models must accept the framework's per-model filter shape — is policed by the batch-safety analysis.

### D34 (lines 1471) [Partition-grain design] (rejected-alternative)
Jinja-style `is_incremental()` branching (dbt) was rejected because it splits one model into two implicit ones that drift.

### D35 (lines 1473) [Partition-grain design] (design-decision)
**DELETE+INSERT over partition columns, not MERGE, for v1**: DuckDB's strategy is `DeleteInsert`; DELETE+INSERT is idempotent under fixed input and aligns with the partition-column safety analysis.

### D36 (lines 1473) [Partition-grain design] (rejected-alternative)
MERGE was rejected as the v1 default because it requires a `unique_key` (not every model has one) and carries cross-engine subtleties; it stays in the `IncrementalStrategy` enum for backends that opt in.

### D37 (lines 1475) [Partition-grain design] (design-decision)
The batch-safety taxonomy is three-class — `FullyBatchSafe` / `BoundedSafe(n)` / `PerPartitionOnly` — and is partition-grain-local because it is meaningful only for this execution shape.

### D38 (lines 1475) [Partition-grain design] (rejected-alternative)
A binary safe/unsafe flag was rejected (too many real workloads are bounded-safe and need auto-chunking); a continuous safety score was rejected (the user-facing decision is qualitative and maps directly to three backend-execution shapes).

### D39 (lines 1477) [Partition-grain design] (design-decision)
**Lookback is derived from the model's SQL**, computed by the shared bound/reach derivation (including inlined `smelt.define` bodies), not a `lookback_days:` YAML annotation (which would let declaration and logic drift). Trade-off: a model with implicit time logic refuses partition-grain eligibility and must be rewritten into a derivable form. Because deriving removes the confirming artifact, the derived clamp is made **observable** (Semantics §"Observing the per-source clamp") as the deliberate counterpart. Deeper rationale: `docs/research/20260521-incremental-as-planner-rule.md`.

### D40 (lines 1479) [Partition-grain design] (design-decision)
**smelt does not own state — scoped to the partition grain.** Watermarks, run history, and offsets live in the backend; owning a watermark store was rejected as a v1 requirement (duplicates engine state, opens a sync-correctness window). Optional run-state tracking is an opt-in extension.

### D41 (lines 1479) [Partition-grain design] (design-decision)
The state doctrine's one deliberate exception is `grain: key`'s transactional merge ledger — backend-resident, written in the *same transaction* as the window's merge, so it cannot drift and does not reopen the sync-correctness window. Consequence: a backend may only select a physical strategy preserving the declared shape's invariants, so the partition-grain `Append` strategy is unreachable until gated on ledger-verified unwritten windows (`docs/research/20260705-keyed-collapse-application.md` D7) — unguarded append-only writes cannot detect a re-run.

### D42 (lines 1481) [Partition-grain design] (design-decision)
**Non-determinism is opted in per column** (`columns.<c>.contract: plausible`) — acceptable-to-vary is a value judgement only the author holds, so it is **declared** (the one place the derive-don't-declare default correctly yields) — and confined by the shared taint-flow proof that the tolerance did not leak into the deterministic skeleton. Derivation: `docs/research/20260703-model-updates.md` §9.2.

### D43 (lines 1481) [Partition-grain design] (rejected-alternative)
A whole-model `allow_nondeterministic` boolean was rejected as the primary mechanism because it drops the guardrail keeping non-determinism out of the skeleton roles.

### D44 (lines 1485) [Key-grain design] (design-decision)
**One keyed mode; the column family is the pattern.** Running-aggregate, latest-value, and milestone patterns share output shape, invariant (end-state equivalence), transform (`merge_into` via the one windowed driver), and key derivation; they differ only in per-column combiner algebra, every consequence of which is derivable from the SQL — by the litmus rule, derived-never-declared, so they must not multiply the refresh enum. Full derivation: `docs/research/20260705-unified-keyed-refresh.md`; decision record: `docs/research/20260705-keyed-collapse-application.md`.

### D45 (lines 1485) [Key-grain design] (rejected-alternative)
Splitting the keyed patterns into peer modes was rejected for a second decisive reason: combiner intent is **per column, not per model** — one table mixes an additive fold, an overwrite, and two extremal milestones, a shape no per-pattern mode can express without materialising the same keyed state several times.

### D46 (lines 1487) [Key-grain design] (design-decision)
**The SQL is the oracle**: the body must be the aggregation itself so `full_refresh(model SQL)` is an executable correctness oracle for every admitted model. The plain-overwrite family (`ANY_VALUE`) exists to give the snapshot posture an honest aggregated spelling under this rule.

### D47 (lines 1487) [Key-grain design] (rejected-alternative)
A bare-projection surface with mode-imposed dedup was rejected: its full refresh is not one row per key, so the equivalence invariant would have no executable oracle and the mode would add semantics the SQL does not carry (`docs/research/20260705-model-refresh-review.md` §1.1).

### D48 (lines 1489) [Key-grain design] (design-decision)
**Derive `unique_key` and combiners from the SQL, not frontmatter**: the `GROUP BY` names the key; each projection names its aggregator; the combiner is a fixed lookup. A config block restating them re-introduces metadata-vs-SQL drift (`docs/research/20260521-incremental-as-planner-rule.md`). If it is in the SQL, it is not also in YAML.

### D49 (lines 1491) [Key-grain design] (rejected-alternative)
A horizon-clamped merge (write-eligibility clamp: only keys newer than `run_start − H` eligible) was rejected: it silently drops *scanned* inputs — the one silent-data-loss point in the maintained family — and is not needed for correctness since merge work is proportional to delta size. What it would buy (settled-key GC, a work bound) is deferred optimisation and must arrive as a package with late-fact accounting (`docs/research/20260705-keyed-collapse-application.md` D6).

### D50 (lines 1491) [Key-grain design] (design-decision)
Slice pruning under key temporal locality is *not* a write clamp: it removes provably-unmatchable rows from the merge's **read** side — or, on the declared route, checks the bound transactionally — while every scanned delta row still merges. Narrow principle: **only proofs prune**; a declared bound is admitted only checked; no unproven bound ever refuses a write.

### D51 (lines 1493) [Key-grain design] (design-decision)
**Time-partitioned keyed output is locality-gated, not a new mode.** The (key, time)-addressed cell absorbs shapes that fell between the modes (bounded-window event-grain dedupe, per-(key, period) aggregates, the clock-sink problem where a keyed stage strips the timeseries property from the DAG). The gate exists because without locality the merge target is the whole key space and an output clock would promise a partition structure the writes do not respect; the declared route is runtime-checked because an over-optimistic recurrence bound would re-import the silent truncation the no-clamp rule prevents (`docs/research/20260705-model-refresh-review.md` §3.2). Full derivation: `docs/research/20260705-keyed-time-superset.md`.

### D52 (lines 1493) [Key-grain design] (rejected-alternative)
A peer mode for the time-partitioned keyed cell was rejected: it shares the key grain's invariant, oracle, driver, ledger, and column families, differing by one derived/declared world-fact — by the litmus rule that earns a **gate**, not a peer. The partition grain remains the honest peer for keyless/multiset bodies.

### D53 (lines 1495) [Key-grain design] (design-decision)
**The ledger is the deliberate exception to "smelt does not own state"** — it has neither defect of the rejected watermark store: backend-resident and written in the same transaction as the merge it describes, so it cannot drift. Without it, additive-fold models cannot detect a double-counting re-run and any mid-run crash forces a full rebuild — an unacceptable operational cliff for `SUM`/`COUNT` combiners.

### D54 (lines 1497) [Key-grain design] (rejected-alternative)
**Observer semantics are refused, not smuggled**: folding state observations (a mutable snapshot) into `MIN`/`MAX`/once-write columns yields min-ever / first-observed values no full refresh can reproduce — a genuinely different contract (a history observer). Admitting it silently would put two contracts behind one mode (the dbt-`strategy:` failure). The refused cells name the observer contract as the future opt-in path.

### D55 (lines 1499) [Key-grain design] (design-decision)
**Ties: honest boundary, not fake proof.** Incumbent-wins plus mandatory sequential execution makes overwrite columns deterministic-given-history without claiming an order-independence no static analysis can prove.

### D56 (lines 1499) [Key-grain design] (rejected-alternative)
A last-processed combiner (no ordering column, order-dependent for *all* rows) was rejected outright; the snapshot posture's plain-overwrite family serves that need where well-defined (one row per key per scan).

### D57 (lines 1501) [Key-grain design] (rejected-alternative)
**No `safety_overrides:` on the key grain.** The partition grain offers per-check overrides because its rejections guard partial-correctness properties a modeller may knowingly waive; every keyed rejection guards the equivalence invariant itself — a bypass would produce silently order-dependent or double-counted state impossible to debug. The escape is to remodel or move to `refresh: materialized_view`.

### D58 (lines 1503) [Key-grain design] (design-decision)
**One windowed executor, shared**: the window-forward step loop is the windowed-keyed-maintenance driver (`model_transforms.md`), parameterised by `(classifier, merge-SQL builder)`. A per-pattern copy of the loop was rejected as four-way drift risk; a consequence is the mode inherits the driver's granularity support.

### D59 (lines 1507) [Interval-versioning design] (design-decision)
`versioning: interval` is **a sub-declaration of `grain: key`, not a third grain**: row addressing is still by key; the interval is structure *within* the key. Only the local combiner and extra validity columns changed vs the former `refresh: versioned` peer — derived machinery by the litmus rule, not grounds for a new enum value.

### D60 (lines 1509) [Interval-versioning design] (design-decision)
Interval versioning is **a smelt-owned pattern, distinct from engine-owned SCD**: smelt owns the combiner (close-old / open-new) and validates the profile against derived properties. An *engine-maintained* SCD2 is not a variant of this profile — it is hand-written SCD2 SQL declared `refresh: materialized_view` with the engine's IVM runtime as maintainer; different modes with different freshness owners, not this profile plus a maintainer flag (`docs/research/20260703-model-updates.md` §17.8).

### D61 (lines 1511) [Interval-versioning design] (design-decision)
**The combiner stays local; the driver and `merge_into` are referenced**: close-old / open-new is meaningful only inside this profile so lives here in full; keyed `merge_into`, the windowed-keyed-maintenance driver, and source-filter pushdown are general capabilities referenced by name, not re-specified.

### D62 (lines 1513) [Interval-versioning design] (design-decision)
Following the key-grain posture, the natural key and tracked attributes should be **derived from the SQL** and the declared key rather than restated in a strategy block wherever unambiguous; the precise derive-vs-declare line for change-tracking columns is an Open Question.

### D63 (lines 1519–1522) [Constraints — contract/plan/graph layer] (constraint)
The **equivalence invariant** holds for every non-`full` model and on every ladder rung; a transform that cannot preserve it for a given model is refused, never applied approximately. Order/set-determinacy is a corollary of it for **every** shape (partition grain included); per-partition equivalence is a *strengthening*, not a separate contract.

### D64 (lines 1523–1531) [Constraints — contract/plan/graph layer] (constraint)
**Write addressing is per-cell, not per-model**: region-addressed writes (identity-free) rewrite whole partitions; key-addressed writes (identity-requiring) `merge_into` by key and may write outside the input time-window. Which a cell uses is derived by the **available-addressings rule** — `available = declared contract facts × trigger/changed-input needs × equivalence invariant × backend capability` — over the open write-pattern registry. Declared shape facts (clock, identity) fix availability; some writes are intrinsically keyed regardless of source clock (SCD2's retroactive close-out).

### D65 (lines 1531–1535) [Constraints — contract/plan/graph layer] (constraint)
A keyed write on a clocked output is still **partition-scoped** to the touched partitions unless it provably cannot be. Key temporal locality, where established, refines keyed addressing with a derived slice bound (target-scan pruning, per-slice equivalence) **without changing the addressing corner**.

### D66 (lines 1536–1541) [Constraints — contract/plan/graph layer] (constraint)
**The write-pattern set is an open registry, not a closed enum**: new patterns are admitted by declaring their required contract facts and discharging the equivalence proof obligation; the `write:` pin is an open, fail-loud name resolved against the registry; a pattern the target backend cannot execute is not a candidate (`architecture.md` capability registry). The stable contract is the admission function + equivalence gate, never the enumeration.

### D67 (lines 1542–1543) [Constraints — contract/plan/graph layer] (constraint)
Maintenance is **windowed by default** where the model is clocked; a full scan is a surfaced fallback, never the silent baseline. Always `scan window ⊇ write window`.

### D68 (lines 1544–1547) [Constraints — contract/plan/graph layer] (constraint)
The **horizon is derived** from the model's reach; a declared horizon is a warning ceiling only and never relaxes the clamp. Late arrivals beyond the horizon are silently excluded; surfacing them is a model-author + data-check concern, not a maintenance guarantee.

### D69 (lines 1548–1550) [Constraints — contract/plan/graph layer] (constraint)
**One home per capability and per rule**: the invariant, ladder, composition contract, and the plan are owned in this spec; properties in `model_properties.md`, transforms in `model_transforms.md`, the declaration law and litmus rule in `models.md`. No spec re-specifies another's.

### D70 (lines 1551–1553) [Constraints — contract/plan/graph layer] (constraint)
**Proofs are fail-closed** (owned in `model_properties.md`, relied on here): an undecidable construct rejects; a declared escape hatch may only *widen* eligibility, never substitute for a proof's default reject.

### D71 (lines 1554–1559) [Constraints — contract/plan/graph layer] (constraint)
The declared `refresh:` value plus the shape-defining facts (clock `timeseries:`, identity `unique_key:`) are the **only shape surface**; the `grain` label is a derived check-only assertion; physical write addressing is derived per cell (steerable only via the validated `write:` pin); input-consumption is derived from the source. No `strategy:` sub-knob.

### D72 (lines 1558–1559) [Constraints — contract/plan/graph layer] (constraint)
The machinery **validates, never chooses** the shape or the addressing; a fallback to full refresh is a surfaced diagnostic, never an automatic switch.

### D73 (lines 1560–1562) [Constraints — contract/plan/graph layer] (constraint)
**The plan is pure data, derived by pure functions, in one place** (`smelt-logical`); consumers — diagnostics, planner application, runtime lowering, the graph layer — never re-derive it. (Also recorded as an invariant in `architecture.md`.)

### D74 (lines 1563–1566) [Constraints — contract/plan/graph layer] (constraint)
**Maintenance statements have one author**: every maintenance statement a run executes is the output of a pure emitter in the maintenance layer; backends execute, never author. Printed (`--show-sql`), gate-verified, and executed SQL are the same emitters' output by construction.

### D75 (lines 1567–1568) [Constraints — contract/plan/graph layer] (constraint)
**Never fold a delta already reflected in the state**: every fold consults the ledger; every region recompute resets the entries it overwrote; no path may merge a window twice.

### D76 (lines 1569–1570) [Constraints — contract/plan/graph layer] (constraint)
**Write window = output window, per cell**: the DELETE/merge target and the output clamp range over the same output-axis column and the same window, by construction.

### D77 (lines 1571–1573) [Constraints — contract/plan/graph layer] (constraint)
**Only proofs prune**: a declared bound is admitted only checked; a guardrail (`scan_bounds`, `horizon_ceiling`) may refuse but never modifies a clamp; no unproven bound drops a scanned input.

### D78 (lines 1574–1577) [Constraints — contract/plan/graph layer] (constraint)
**Fail-loud, fail-closed**: every admission failure, non-local scan, skeleton-position add, and unsupported graph node is a named diagnostic; nothing degrades to a silent fallback. The graph layer never silently under-runs: unrepresentable dirt widens to whole-model, never to nothing.

### D79 (lines 1578–1579) [Constraints — contract/plan/graph layer] (constraint)
**Widen-never-narrow** is the composition law of every interval operation (clamp ceiling, grain alignment, footprint reflection, backward widening).

### D80 (lines 1580–1581) [Constraints — contract/plan/graph layer] (out-of-scope)
Content-aware delta pruning is deliberately out of scope (an engine/CDF concern).

### D81 (lines 1580–1582) [Constraints — contract/plan/graph layer] (out-of-scope)
File-level write-amplification minimisation is deliberately out of scope (the engine's job — the plan guarantees the partition bound).

### D82 (lines 1582) [Constraints — contract/plan/graph layer] (out-of-scope)
Cross-*project* propagation is deliberately out of scope (project isolation, `architecture.md`).

### D83 (lines 1586) [Partition-grain constraints] (constraint)
**Logical model is pure SQL**: no `is_incremental()`, no macros, no conditional branches; the framework injects the time filter.

### D84 (lines 1587) [Partition-grain constraints] (constraint)
`timeseries:` is **required** for `grain: partition`: a model with `grain: partition` and no `timeseries:` block is a hard error at workspace load (`models.md` §"Constraint violations").

### D85 (lines 1588) [Partition-grain constraints] (constraint)
**Strategy is not on the model**: frontmatter declares `unique_key`; the backend chooses `DeleteInsert`/`Merge`/etc. for the recompute corner's execution.

### D86 (lines 1589) [Partition-grain constraints] (constraint)
**smelt does not manage computational state — a partition-grain-scoped doctrine**: watermarks, offsets, and run-history live in the backend. The one deliberate exception across the refresh axis is `grain: key`'s transactional merge ledger (backend-resident, transactional-with-the-merge). A backend may select only a physical strategy preserving the declared shape's invariants; the partition-grain `Append` strategy is unreachable until gated on ledger-verified unwritten windows.

### D87 (lines 1590) [Partition-grain constraints] (constraint)
**Output-filter injection is per-model; source-filter pushdown is per-reference**: the outer clamp is applied once at the outermost SELECT; pushdown filters apply per `smelt.<path>` reference in the expanded body.

### D88 (lines 1591) [Partition-grain constraints] (constraint)
**Per-partition equivalence with full refresh, up to full-refresh non-determinism**: for every partition `p` in the run window, the output `where(partition_column = p)` equals the full-refresh output for `p` on all local, deterministic columns; a `columns.<c>.contract: plausible` column need only be a plausible full-refresh value; globally-dependent columns are not equivalent.

### D89 (lines 1592) [Partition-grain constraints] (constraint)
**Idempotence under fixed input**: re-running the same run window on unchanged sources converges to the same output table state.

### D90 (lines 1593) [Partition-grain constraints] (constraint)
**Granularity is closed under partition arithmetic**: a run window must align to whole granularity units; partial-unit ranges are rejected. The declared granularity must be at least as coarse as the granularity derived from the `partition_column` projection's own truncation transform (`g_run >= g_part`); a declared granularity finer than the derived partition grid is rejected.

### D91 (lines 1594) [Partition-grain constraints] (constraint)
**Safety-check overrides are explicit**: a `safety_overrides` entry names the specific check it bypasses; there is no global disable.

### D92 (lines 1595) [Partition-grain constraints] (constraint)
**No silent downgrade to full-refresh**: a model the safety classifier rejects, or whose bound derivation is `NotDerivable`, is refused at planning time with a diagnostic — never a silent fall back to full-table execution.

### D93 (lines 1596) [Partition-grain constraints] (constraint)
`event_time_column` must be accessible at the outermost SELECT, unless every UNION ALL branch traces `Traceable`; otherwise `EventTimeColumnNotVisibleAtOuterSelect` (Error) fires at the diagnostic gate.

### D94 (lines 1597) [Partition-grain constraints] (constraint)
**Non-determinism stays in the payload**: non-deterministic SQL is admitted only when its value flows exclusively into a `columns.<c>.contract: plausible` column (except the run-nondeterministic class as a direct projection); it must never reach `event_time_column`, `partition_column`, a `unique_key` column, or any membership/grouping position. Declaring an excluded column `plausible` is a configuration error.

### D95 (lines 1601) [Key-grain constraints] (constraint)
Opt-in is `refresh: incremental` + `grain: key` (storage implied `table`); `unique_key` is required and must restate the `GROUP BY`. No config block; `safety_overrides:` is a hard error (partition-grain only).

### D96 (lines 1602) [Key-grain constraints] (constraint)
A `timeseries:` block is admitted **iff key temporal locality is established**; otherwise refused (`KeyedForbidsTimeseries`).

### D97 (lines 1603) [Key-grain constraints] (constraint)
The body is an aggregated `GROUP BY` query; `unique_key` is derived from `GROUP BY`; every non-key projection classifies into exactly one column family. The combiner is a fixed lookup; authors never declare combiners.

### D98 (lines 1604) [Key-grain constraints] (constraint)
**The catalogue is closed and the classifier is fail-closed**: unrecognised aggregators, composite expressions, unproven once-write columns, and retractable contributions are refused — never approximated, never silently downgraded.

### D99 (lines 1605) [Key-grain constraints] (constraint)
**End-state equivalence holds with the model's own SQL as the oracle**, with exactly two named carve-outs: retained departed keys under snapshot-reconcile, and ordering-key ties on overwrite columns.

### D100 (lines 1606) [Key-grain constraints] (constraint)
**No write-eligibility clamp**: a run merges every delta row it scans; no scanned input is silently dropped. Target-scan slice pruning under established key temporal locality is no-op elimination (or a transactionally-checked declared bound), never a write clamp. Any future clamp or settled-key GC must ship together with late-fact accounting.

### D101 (lines 1607) [Key-grain constraints] (constraint)
**The run shape is derived from the driving source** (clocked ⇒ window-forward; unclocked ⇒ snapshot-reconcile) and surfaced by `smelt explain`; it is never declared.

### D102 (lines 1608) [Key-grain constraints] (constraint)
**The admission matrix is enforced per column**: fold and once-write families require a clocked (replayable) driving source; the plain-overwrite family requires the snapshot posture.

### D103 (lines 1609) [Key-grain constraints] (constraint)
**Window-forward models maintain the transactional merge ledger**, written atomically with each window's merge. Additive-fold models must refuse a ledgered window's re-run; re-run-tolerant models may re-merge. Snapshot-reconcile models keep no ledger.

### D104 (lines 1610) [Key-grain constraints] (constraint)
**Ordering and parallelism follow the derived postures**: out-of-order/parallel/sliced backfill only for order-independent models; overwrite columns force sequential temporal order.

### D105 (lines 1611) [Key-grain constraints] (constraint)
**Reprocessing changed input is refused for every family** when detected; the mitigation is `--full-refresh` (or a manual cascade rebuild).

### D106 (lines 1612) [Key-grain constraints] (constraint)
**Exactly one clocked driving source under window-forward**: zero clocked sources selects snapshot-reconcile; two or more is refused.

### D107 (lines 1613) [Key-grain constraints] (constraint)
Without an admitted `timeseries:` block the output has no `partition_column` and downstream consumers treat the keyed table as a lookup; with one, the output is a clocked, time-partitioned keyed table.

### D108 (lines 1614) [Key-grain constraints] (constraint)
**The windowed step loop is the shared driver**, not a per-pattern copy (`model_transforms.md`).

### D109 (lines 1615) [Key-grain constraints] (constraint)
**Key temporal locality is established only by the three named routes** (key-embedded, key-determined, recurrence-bounded). Derived routes prune by proof; the declared route prunes only under the transactional runtime check (`KeyedRecurrenceBoundViolated`). A violated declaration fails the run; it never silently drops.

### D110 (lines 1619) [Interval-versioning constraints] (constraint)
`versioning: interval` is admitted **only on `grain: key`**; no `materialized_view` restatement; the opt-in implies `table` storage (inherited from the key grain).

### D111 (lines 1620) [Interval-versioning constraints] (constraint)
**No `timeseries:` block on the model itself together with `versioning: interval`** — keyed + interval output, not a partitioned build. Window-forward consumption of a `timeseries:` *source* is derived and in-bounds.

### D112 (lines 1621) [Interval-versioning constraints] (constraint)
**Validity intervals are non-overlapping per key**: at most one open (`is_current`) version per key at any time; closed intervals abut at shared boundaries with no gaps.

### D113 (lines 1622) [Interval-versioning constraints] (constraint)
**Validity is stamped from source event-time, never the run clock** — this is what makes the profile order-independent and replay-safe.

### D114 (lines 1623) [Interval-versioning constraints] (constraint)
**End-state equivalent and order-independent**: merging non-overlapping snapshots in any order converges to the same version history.

# Claims E — incremental_models.md §Known Divergences / Open Questions (lines 1625–2326)

### E1 (lines 1629–1640) [contract]
**Entry:** "The grain-demotion has landed for the top-level surface (one narrow gap remaining)."
**Verdict:** MIXED
**Evidence:** Landed part: top-level `unique_key:` parses (.sql frontmatter and smelt.yml overrides), `refresh: incremental` admitted on facts alone, written `grain:` validated against `derive_grain(...)` erroring on mismatch.
**Keep-content:** Narrow gap: a `grain: key` model with no top-level `unique_key:` (identity from body `GROUP BY`) is checked against the derived key only at plan derivation (`smelt-db::queries::maintenance`), not at frontmatter validation — and only when a top-level `unique_key:` also exists to check against; a bare `grain: key` model with neither declaration is unchecked (cross-ref `models.md` §Known Divergences).

### E2 (lines 1641–1659) [contract]
**Entry:** "The open write-pattern registry, the `maintenance.cells[].write` pin, and both write-addressing diagnostics are built; the equivalence-invariant factor the registry consults is still the structural contract-facts check only."
**Verdict:** MIXED
**Evidence:** Landed part cites `smelt_logical::maintenance::WRITE_PATTERN_REGISTRY`, `resolve_write_pin` (factors 1, 2, 4), `MaintenanceWritePatternUnavailable` fail-loud refusal, open-string parse in `smelt_core::config::MaintenanceCellConfig`, and `supports_column_scoped_merge` migrated to `BackendCapabilities`; design in `docs/research/20260716-relation-contract-and-per-cell-addressing.md`.
**Keep-content:** The third available-addressings factor (a per-cell equivalence proof beyond the pattern's declared required facts, e.g. threading P3 column-comparability or a suppression-specific proof) is a caller-supplied hook (`resolve_write_pin`'s `cell_can_uphold_equivalence` closure) that today always accepts; deepening it is later work tracked alongside this entry.

### E3 (lines 1660–1681) [contract]
**Entry:** "An inadmissible write-*variant* pin (`technique`/`prefer: suppress`/`unconditional`) has no pre-execution diagnostic gate, unlike `cells[].write`."
**Verdict:** MIXED
**Evidence:** Landed part: `cells[].write` pin validated pre-run; `smelt explain` correctly propagates `ChoiceRefusal` for the P2-decidable sub-case (`technique: suppress` over a `WholeRow`-identity cell, since `resolve_write_suppression` short-circuits on row identity).
**Keep-content:** An inadmissible write-variant pin (forcing `suppress` on a cell whose P2/P3 proof refuses) is never checked pre-execution — the resolver silently falls back to full region recompute instead of refusing the run up front; `explain` also misses the P3-only-inadmissible case (Key-identity cell with an incomparable compared column) since it has no `sql`/`JoinContext` to redo the P3 walk. Extending the pre-execution gate to this pin dimension is tracked in `docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase G1.

### E4 (lines 1682–1729) [contract]
**Entry:** "Observed-delta recording is built for the change-suppressed column-scoped MERGE family …; its key→partition projection into forward propagation is built for a composed model edge, and both are surfaced by `smelt explain`; backward resolution and the keyed-fold/staged-candidate write families' own recording are not."
**Verdict:** MIXED
**Evidence:** Landed part: recording into a warehouse-resident table in the same transaction as the write (DuckDB-scoped), route-aware key→partition projection (routes 1–2 exact, route 3 widened by `r` plus margins), `smelt explain` `observed-delta recording:` and `observed-delta projection:` lines.
**Keep-content:** Live gaps: reading the real warehouse-resident delta table live during `smelt run --since-upstream` is not wired into the CLI path (projection exercised only via a directly-supplied lookup); backward resolution consumes no recorded delta (every ancestor requirement is still the full clamp-derived slice); keyed-fold and staged-candidate families don't record (no recording line printed); explain has no live "is this slice's recorded delta actually empty" leg for the settle-bound × observed-delta composition. Tracked by `docs/plans/20260715-composed-axes-conditional-maintenance.md`; external-source fingerprint-sidecar lifecycle normative in `sources.md` §"The fingerprint sidecar".

### E5 (lines 1730–1743) [contract]
**Entry:** "No execution technique keys off a maintained-model creation cell."
**Verdict:** MIXED
**Evidence:** Landed part: §"Upstream model edges" otherwise live — `smelt explain` and `build_forward_graph` share `derive_model_maintenance_plan_with_edges`; underivable upstream clock is a `MaintenanceReachNotDerivable` refusal; `--source <address>` accepts an upstream maintained model as delta origin.
**Keep-content:** Execution-side gap: `execute.rs`'s technique resolution excludes model refs entirely, so a maintained-model creation cell drives forward propagation and explain but no per-cell execution technique keys off it (propagated region materialized by the ordinary incremental run loop). Tracked in `docs/plans/20260710-web-analytics-maintenance-demo.md`.

### E6 (lines 1744–1813) [contract]
**Entry:** "The plan has three live consumers: diagnostics, `smelt explain`, and one execution technique."
**Verdict:** MIXED
**Evidence:** Landed part cites `derive_maintenance_plan` full per-cell admission, the `maintenance_plan` Salsa query, two diagnostics folded into `file_diagnostics()`, `resolve_cell_technique` + `execute_column_scoped_merge`, both run loops dispatching column-scoped MERGE (tests `column_scoped_merge_e2e`, `keyed_run_loop_dispatches_column_scoped_merge_through_execute_project`, `yes_corner_clamps_the_merge_to_the_horizon_and_leaves_the_rest_untouched`).
**Keep-content:** Live gaps: the horizon-clamped `PartitionLocal::Yes` corner is not reachable from any real workspace (trigger-list construction only emits `Trigger::UpstreamMutation` for unclocked sources; clocked mutable-source scan-bound derivation deferred — `real_fixture_daily_events_status_would_admit_partition_local_yes_cell` pins readiness); nothing distinguishes "a mutation genuinely happened" from re-derivation — dispatch fires every run, change-aware triggering being `smelt run --since-upstream`'s job; `defaults.prefer`/`cells[].prefer` soft-bias ladder and `scan_bounds.on_violation: warn` parse but are not consumed (every refusal is an Error); cost model between two admissible techniques unbuilt; `AppendOnly` sources get no `UpstreamMutation` cell (mutation-sensitivity real but no post-creation mutation to trigger on). Refs: `docs/research/20260705-refresh-as-maintenance-plan/08-code-placement.md` §2.8; `docs/plans/20260707-maintenance-plan-impl.md`.

### E7 (lines 1814–1838) [contract]
**Entry:** "The keyed run loop's suppression and technique-selection dispatch is now covered generatively, not only by one hand-built fixture; the underlying provenance gap that makes every dispatched merge a structural no-op is unchanged."
**Verdict:** MIXED
**Evidence:** Landed part: generative coverage via `crates/smelt-cli/tests/maintenance_conformance/gate.rs` legs `keyed_enriched_recipe_admits_suppressed_column_scoped_merge` and `keyed_enriched_pool_upholds_equivalence_with_zero_write_redelivery`.
**Keep-content:** Live gap: `smelt_logical::maintenance::grouping::collect_column_refs` misreads an aggregate's function-name token as an ambiguous unqualified column once 2+ FROM sources are joined; the resulting fail-closed `degenerate_whole_model` collapse is what admits the cell at all, so every keyed-path dispatched merge is a `WriteSuppression::Suppressed` no-op by construction — no fixture can drive the keyed dispatch through a genuinely changed merged value (the non-keyed sibling test does assert a changed value; the keyed path has no equivalent). Tracked by `docs/plans/20260720-prod-w10-keyed-mutable-admission.md`.

### E8 (lines 1839–1890) [contract]
**Entry:** "Statement emission is single-owner for the region-recompute, keyed-fold, and column-scoped-MERGE families, and both the conformance gate and `--show-sql` are wired to prove/print it."
**Verdict:** MIXED
**Evidence:** Landed part cites `emit_delete_insert`/`emit_keyed_fold`/`emit_create_table_as`/`emit_column_scoped_merge` in `crates/smelt-logical/src/maintenance/emit.rs`, `statement_parity.rs` per-family legs plus the structural `no_maintenance_statement_authoring_outside_the_emitter` gate, conformance HOLDS legs, and `--show-sql` calling the same emitters.
**Keep-content:** Live remainders: `emit_in_place_update` has no production consumer (legs only in tracer tests); the `Grade::Additive` fold's MERGE-inside-ledger-transaction interior (`Backend::fold_ledger_delta`) is not observable at `execute_statement_group`, so its parity leg uses a self-contained idempotent fixture rather than a real Additive model; `Backend::delete_partitions`/`insert_overwrite` still hand-author DELETE/INSERT-OVERWRITE for the production-unreachable `IncrementalStrategy::InsertOverwrite` (dead code, allowlisted in the structural gate) — deleting or emitter-routing it is follow-up outside `docs/plans/20260710-emit-unification.md`.

### E9 (lines 1891–1913) [contract]
**Entry:** "Four of the seven maintenance-plan proofs are unbuilt"
**Verdict:** MIXED
**Evidence:** Landed part: per-column mutation-sensitivity/provenance, skeleton-role extraction, and the grain-alignment check are built as leaf classifiers (`grouping.rs`, `skeleton.rs`, `granularity.rs`), failing closed on CTE/set-op/derived-table/ambiguous shapes.
**Keep-content:** Live gaps: footprint reflection, partition-locality projection, faithful-fold conditions, and definition-change column classification are unbuilt and hand-supplied in the tracer; column-group-scoped dirt coarsens to whole-partition (safe, over-running); hour granularity is declared surface but propagation is day-ordinal (sub-day deferred); the grain-alignment check only checks the declaration (widen-never-narrow, `MaintenanceGranularityMismatch`) — graph edges still take the declaration directly (P3 stands). Refs: `model_properties.md` §Surface "Derived proofs" not-yet rows; `docs/plans/20260707-maintenance-plan-impl.md` MP5/MP6/MP14; `09-spec-readiness.md` §2.

### E10 (lines 1914–1934) [contract]
**Entry:** "The ledger has two storage substrates, one per grading."
**Verdict:** MIXED
**Evidence:** Landed part: `smelt_state::reconciliation` JSON store (keying, two gradings, combine + recompute-reset ops), region recompute writing recompute-reset entries, and the warehouse-resident per-delta ledger (`smelt_state::ddl_duckdb`, `Backend::fold_ledger_delta`, `KeyedReprocessedWindow` on repeat delta).
**Keep-content:** Live gap: the DuckDB-dialect DDL/DML is the only ledger substrate implemented; an additive-graded cell on a non-DuckDB backend fails loudly (`UnsupportedFeature`) — a Spark-dialect ledger builder is unbuilt.

### E11 (lines 1935–1980) [contract]
**Entry:** "A bare keyed-grain hop still refuses in the graph; a locality-admitted composed node no longer does."
**Verdict:** MIXED
**Evidence:** Landed part: locality-admitted composed nodes classified by declared granularity and contributing edges (`build_forward_graph`, `refuse_keyed_nodes`); route-aware inbound dirt projection (`locality_margin_days`); `key_recurrence` threaded into the graph layer via `build_key_recurrences`; bare keyed `--source` origin refuses fail-loud even edge-less; `--include-upstreams` walks through composed nodes.
**Keep-content:** Live gaps: bare `grain: key` nodes with no admitted locality still refuse (`MaintenanceGraphUnsupportedNode`, P7/P8); time-unrolled self-edges designed but unbuilt; no key-level dirt representation exists — intervals are the graph's only currency (`10-dependency-propagation.md` §6, S12); the real `examples/web_analytics` workspace is not fully `--since-upstream`-compatible end to end (`silver.sessions_chained` self-referential and `silver.device_user_edges` bare-keyed-with-readers each refuse the whole-workspace graph; no `--select` scoping).

### E12 (lines 1981–1993) [contract]
**Entry:** "Delta detection for `--since-upstream` is explicit, not automatic, for v1."
**Verdict:** LIVE
**Evidence:** The caller must supply each source's landed delta (`--source … --landed …`); no persisted "last propagated through" watermark or automatic diffing exists, and the graph layer consumes neither `smelt_state::landed_deltas` nor `change_feed` offset/snapshot delta detection.
**Keep-content:** v1 is explicit-only: the runner supplies landed deltas on the command line and the graph reflects exactly those intervals; automatic watermark-diffed `--since-upstream` is a possible future extension once a persisted per-source watermark lands in `smelt-state` (`landed_deltas` is built but unconsumed by the graph layer; `change_feed` offset detection/snapshot diffing unbuilt).

### E13 (lines 1994–1995) [contract]
**Entry:** "Straddle attribution without locality"
**Verdict:** LIVE
**Evidence:** A per-key footprint chaining across history is scoped out of the ledger's v1.
**Keep-content:** The ledger's v1 is locality-or-explicit-footprint only; straddle attribution without locality is scoped out (01 §8's own caveat).

### E14 (lines 1996–2000) [contract]
**Entry:** "The refresh-axis cut has landed."
**Verdict:** MIXED
**Evidence:** Landed part: `RefreshStrategy` (`crates/smelt-core/src/config.rs`) accepts only `full`/`incremental`/`materialized_view`; removed names (`batched`/`keyed`/`cumulative`/`versioned`) hard-error with fix-its.
**Keep-content:** A proposed `on_column_add: backfill | leave_null | recompute` policy knob is noted but not yet surface.

### E15 (lines 2001–2007) [contract]
**Entry:** "Windowed-by-default and the derived horizon are contract, partially built."
**Verdict:** MIXED
**Evidence:** Landed part: per-source reach (`derive_model_bounds`) and the `horizon_ceiling:` declaration with compile-time warning are surfaced.
**Keep-content:** Live gaps: a model-wide derived-horizon proof composing every source's reach into one number is under construction, as is the model-author lateness-flag pattern's data-quality check. Tracked by `docs/plans/20260704-model-updates.md`.

### E16 (lines 2008–2118) [contract]
**Entry:** "Key temporal locality: all three routes and their slice-pruned merge (route 3's checked) are built, the admitted slice and derived settle bound are folded into `smelt-db`'s own plan-derivation surface, and `smelt explain` prints the route/slice/settle bound; the broader per-input scope-map explain surface (§"Scope maps") is specified but unbuilt."
**Verdict:** MIXED
**Evidence:** Landed part: all three routes built (route 2's refusal of MIN/MAX-derived columns, route 3's checked probe `emit_recurrence_bound_probe` covered by statement_parity, `execute_project`-driven route-3 fixture via web-analytics `events_deduped`), slice/settle bound on `MaintenancePlan` and printed by explain; slice-bounded write suppression realized.
**Keep-content:** Live gaps: (a) the per-input scope-map explain surface is specified but unbuilt; (b) route 2's declared-FD sub-route unreachable for an arbitrary non-clock-derived dimension column — the NOT-NULL derivation (`partition_column_provably_not_null`) recognises only driving-clock-derived shapes; extending it (or a dedicated non-null declaration) is unbuilt; (c) a runnable end-to-end route-2 fixture needs the once-write classifier family (`docs/plans/20260705-keyed-collapse.md`); (d) route 2's `IN (SELECT DISTINCT …)` slice predicate is unexercised against a real backend due to a genuine DuckDB MERGE binder limitation (`BindMerge … LOGICAL_GET but got FILTER`, confirmed v1.4.4/v1.5.4) — merges run with `slice: None`; lifting needs a rewrite (e.g. pre-materialized semi-join) or a fixed DuckDB, tracked by `docs/plans/20260715-composed-axes-conditional-maintenance.md`; (e) the settle-bound × observed-delta composition remains unbuilt (same plan); (f) `smelt-db` plan derivation admits routes only where it can determine the driving source's granularity (runtime always can); (g) declared-vs-derived precedence (derived first) and order-independent key-set comparison are implementation choices where spec text underdetermines.

### E17 (lines 2119–2125) [contract]
**Entry:** "`grain: key_per_partition` derives no plan yet."
**Verdict:** LIVE
**Evidence:** The value parses but maintenance-plan derivation has no trajectory/backfill machinery; a `refresh: incremental` model declaring it refuses fail-loud (`MaintenanceUnsupportedGrain`).
**Keep-content:** `grain: key_per_partition` parses and validates but refuses at plan derivation with `MaintenanceUnsupportedGrain` (no cells, no executor); full trajectory support (locality routes, emitted plan, graph admission) tracked by `docs/plans/20260715-composed-axes-conditional-maintenance.md`.

### E18 (lines 2126–2186) [contract]
**Entry:** "Conditional maintenance technique: column-scoped and keyed-fold MERGE, plus a merge-less keyed realisation; the region DELETE+INSERT family and the whole-row merge-less realisation remain unbuilt."
**Verdict:** MIXED
**Evidence:** Landed part: change-suppressed matched arms for `ColumnScopedMerge` and keyed-fold MERGE (fail-closed over P2/P3), `emit_staged_candidate_conditional` + `resolve_keyed_write_mechanism` capability-flag choice, live keyed-loop wiring, and the delta-restricted model-edge region recompute (`append_model_edge_cells` P1 verdict, `resolve_recompute_restriction`, `emit_delete_insert_delta_restricted`, `execute_delete_insert_with_delta_restriction`, dispatched in `execute.rs` for DuckDB targets with explain/--dry-run sharing the same derivation).
**Keep-content:** Live gaps: `smelt explain --show-sql` always renders the unconditional matched arm, never the suppressed form the live run executes (reporting not wired to `resolve_write_suppression`); the region DELETE+INSERT family has no conditional variant (unchanged rows rewritten); the whole-row (keyless, `EXCEPT ALL`-both-ways) staged-candidate realisation does not exist; no `write:` pin over the keyed MERGE/staged-candidate choice; observed deltas recorded only on a maintained-model edge's conditional write; delta-restriction admission does not yet consume an external `mutable_snapshot` source's fingerprint-sidecar delta as a driving-source delta; non-DuckDB targets keep the widened-scan recompute (observed-delta read is DuckDB-only). Refs: `docs/research/20260715-conditional-maintenance-without-cdf.md`; `docs/plans/20260715-composed-axes-conditional-maintenance.md`.

### E19 (lines 2187–2234) [contract]
**Entry:** "The conditional variant now enters the override ladder; `smelt bakeoff` is landed."
**Verdict:** MIXED
**Evidence:** Landed part: `resolve_write_variant` structural steady-state-prefers/first-build-doesn't rule, asserted end-to-end (`first_build_posture_and_steady_state_preference_resolve_bit_identical_state`), consulted by `resolve_live_column_scoped_cell`, printed by explain; `suppress`/`unconditional` values in the `prefer:`/`technique:` ladder with pin-never-bypasses-admission semantics; `smelt bakeoff` landed (`docs/plans/20260719-prod-w7-bakeoff.md`).
**Keep-content:** Live gaps: the keyed-fold suppression consumer (`smelt-runtime::cumulative`) still honours `Suppressed` unconditionally — the ladder rule doesn't reach it; no real fixture derives a `ColumnScopedMerge`/`KeyedFold` cell under a first-build/backfill trigger (`derive_backfill` always emits `DeleteInsert`; `Trigger::ColumnAdded` needs a `ModelDiff` no live caller supplies), so the branch is proven only at resolver level with a hand-built cell; bakeoff measures technique-family cost only, not the write-suppression dimension — today's no-override default is the structural rule, not measured. Open question: whether a future cost model needs region-level change-ratio statistics from prior observed deltas.

### E20 (lines 2235–2245) [contract]
**Entry:** "User docs describe the trichotomy + grain surface; the plan's own CLI surface is now partly covered."
**Verdict:** MIXED
**Evidence:** Landed part: docs-site pages describe `refresh: full | incremental | materialized_view` and the grain trichotomy; `cli.md` documents `--since-upstream`/`--include-upstreams`, explain's report + `--show-sql`, bakeoff; `smelt-yml.md` documents the `maintenance:` block, override wiring, and `--pin` workflow.
**Keep-content:** The entry's own headline says the CLI surface is only "partly" covered — the unnamed remainder of the plan's CLI surface is still undocumented (the entry does not enumerate what's missing; a redraft should either name the residue or drop the qualifier after verification).

### E21 (lines 2246–2255) [contract]
**Entry:** "A group merged across two mutable inputs has no group-merge-provenance policy."
**Verdict:** LIVE
**Evidence:** Per-cell admission checks obligations 4/5 identically whether a group's `mutation_sensitivity` came from one input or several; a stricter multi-input policy is undecided and unbuilt.
**Keep-content:** A partition-aligned multi-input mutable merge (e.g. `orders.amount * fx_rates.rate`) is admitted as targeted `ColumnScopedMerge` like a single-input case; a stricter "partition-local ≠ foldable" policy forcing region recompute when provenance spans multiple mutation-sensitive inputs is undecided and unbuilt; pinned by `maintenance_coverage_matrix.rs::ex12_multi_input_merge_degenerates_to_recompute`.

### E22 (lines 2256–2271) [contract]
**Entry:** "The trigger-list builder's `explicitly_mutable` scoping misses `change_feed`-declared sources entirely, not just clocked ones."
**Verdict:** LIVE
**Evidence:** `derive_model_maintenance_plan` only constructs `UpstreamMutation` for unclocked sources literally declaring `mutation_profile: mutable_snapshot`; `change_feed` maps to `MutableSnapshot` for admission but fails the literal check, so change_feed sources never get a mutation cell.
**Keep-content:** A `change_feed` source (clocked or not) never gets an `UpstreamMutation` cell constructed ("no cell to even refuse"), same gap family as append-only enrichment sources; pinned by `coverage_matrix_gaps.rs::ex08_unclocked_change_feed_dimension_scan_unbounded`; even when the posture IS threaded through directly (`ex14`, `ex26`), only full-input re-derivation is admitted — no live fold machinery consumes a change feed's delta shape yet.

### E23 (lines 2272–2282) [contract]
**Entry:** "`INTERSECT`/`EXCEPT` are unclassified set operations."
**Verdict:** LIVE
**Evidence:** Set-op distribution classifies `UNION ALL` only; INTERSECT/EXCEPT compositions collapse to whole-model mutation-sensitivity, so every admitted cell is `DeleteInsert` region recompute.
**Keep-content:** INTERSECT/EXCEPT fall through to the whole-model collapse (pinned by `coverage_matrix_gaps.rs::ex41_ex42_intersect_no_payload_column_still_delete_insert`); a future distribution proof would need per-arm-cardinality reasoning (a row's fate depends on both arms simultaneously) before any targeted technique could be admitted. Cross-ref `model_properties.md` §Known Divergences.

### E24 (line 2286) [partition]
**Entry:** "The mode value is cut and the sub-block is retired."
**Verdict:** LANDED
**Evidence:** `refresh: batched` hard-errors with fix-it (`crates/smelt-core/src/config.rs`; delivered by `docs/plans/20260707-maintenance-plan-impl.md`); the `batched:` frontmatter sub-block is refused with a `YamlParseError` naming each key's replacement (`crates/smelt-core/src/metadata.rs`); the `smelt migrate` non-existence is recorded as a deliberate decision (fix-it prints replacement YAML instead). Delivered by `docs/plans/20260719-prod-w8-composed-axes-followups.md`.
**Keep-content:** none

### E25 (line 2287) [partition]
**Entry:** "A row-shaped partition-grain model's MERGE-dedup key has no `.sql` frontmatter home."
**Verdict:** MIXED
**Evidence:** Landed part: the workaround exists — the `smelt.yml` model override `models.<name>.batched.unique_key` (separate parsing path, untouched by the sub-block retirement) feeds `decide_column_merge_dispatch`'s `model_declares_unique_key` gate.
**Keep-content:** Open question: whether the MERGE-dedup concept deserves its own top-level `.sql` frontmatter spelling distinct from identity-conferring `unique_key:` (which makes an output key-shaped, impossible for a row-shaped body — `KeyedRequiresGroupBy`); today it lives only in the smelt.yml override. Tracked by `docs/plans/20260719-prod-w8-composed-axes-followups.md`.

### E26 (line 2288) [partition]
**Entry:** "`nondeterministic_columns` predates `columns.<c>.contract`."
**Verdict:** LIVE
**Evidence:** Two spellings of the same mechanism coexist: `columns.<c>.contract: plausible` is the only surviving `.sql` frontmatter spelling, but the `smelt.yml` model override's `batched.nondeterministic_columns` sub-block remains a separate, still-parsing spelling.
**Keep-content:** The dual-surface state persists: `.sql` frontmatter uses `columns.<c>.contract` only (key owned by `models.md` §"`columns:`"; semantics by this spec), while `smelt.yml`'s `batched.nondeterministic_columns` still parses as a separate spelling (per-column `contract:` is `.sql`-frontmatter-only, `smelt-yml.md` §"Layer split").

### E27 (line 2289) [partition]
**Entry:** "One non-hot classification call site still reads the outer SQL body."
**Verdict:** LIVE
**Evidence:** `derive_model_source_bounds` (bound-`NotDerivable` refusal gate) classifies on outer `model.sql`; a lookback only inside a function body with no outer Form B filter would behave differently (none exists in repo).
**Keep-content:** The bound-`NotDerivable` gate classifies on the outer SQL; the sole divergent case is a lookback living only inside a function body with no outer Form B filter. Tracked in `docs/plans/20260530-thread-fn-registry-classification.md`.

### E28 (line 2290) [partition]
**Entry:** "Window-function batch-safety check runs on unexpanded outer SQL."
**Verdict:** LIVE
**Evidence:** `find_inadmissible_over` scans the outer model SQL before function expansion; an `OVER` inside a `smelt.define` body is invisible.
**Keep-content:** An `OVER` clause inside a `smelt.define` body escapes the batch-safety check. Tracked in `docs/plans/20260530-thread-fn-registry-classification.md`.

### E29 (line 2291) [partition]
**Entry:** "Per-source clamp observability partly emitted."
**Verdict:** MIXED
**Evidence:** Landed part: `smelt explain --json` reports `source_partition_col` and `(before, after)` offsets.
**Keep-content:** Live gaps: `--json` does not resolve the run-relative scan window even when a run window is supplied; the editor-hover readout is not implemented (LSP hover is type/column/ref oriented). Both specified ahead of a plan.

### E30 (line 2292) [partition]
**Entry:** "Per-column `data_latency` not implemented."
**Verdict:** LIVE
**Evidence:** Late-arriving-data automation is deferred.
**Keep-content:** `data_latency` per column is unimplemented; the two interim mitigations in Semantics §"First-run and backfill" are the only options.

### E31 (line 2293) [partition]
**Entry:** "Non-deterministic row-set-membership or grouping is out of scope."
**Verdict:** LIVE
**Evidence:** Always rejected regardless of `columns.<c>.contract`; reconciling frozen-per-window membership against a full refresh needs its own design.
**Keep-content:** Non-deterministic row-set-membership/grouping is always rejected regardless of column contract; a frozen-per-window-membership design is needed before admitting it (research §9.1a).

### E32 (line 2294) [partition]
**Entry:** "CTE-only `event_time_column` references not yet detected."
**Verdict:** LIVE
**Evidence:** Constraint 11 enforced for direct-subquery FROM clauses and set ops; a CTE alias not projecting `event_time_column` is uncaught and fails at DuckDB execution.
**Keep-content:** A CTE alias that fails to project `event_time_column` escapes constraint 11's check and only fails at execution. Tracked in `docs/plans/20260616-smelt-feedback-fixes.md`.

### E33 (line 2295) [partition]
**Entry:** "Three execution paths in `crates/smelt-cli/src/main.rs`."
**Verdict:** LIVE
**Evidence:** CLI dispatch is tri-modal (legacy, optimizer+batched, batched-only) though unified around `PartitionGrainConfig`; should converge.
**Keep-content:** The CLI's tri-modal dispatch should converge into one path. (Note for verification pass: memory records the CLI incremental path as deleted 2026-07-06, so this entry may be stale — verify against `main.rs`.)

### E34 (line 2296) [partition]
**Entry:** "Schema evolution is unspecified."
**Verdict:** LIVE
**Evidence:** A `partition_column` rename or output schema change has no defined handling.
**Keep-content:** Schema evolution (partition-column rename, output schema change) has no defined handling for partition-grain models.

### E35 (line 2297) [partition]
**Entry:** "`smelt.metric()` interaction."
**Verdict:** LIVE
**Evidence:** Metric expansion vs time-filter injection interaction not fully spelled out for partition-grain models consuming metrics.
**Keep-content:** The metric-expansion × time-filter-injection interaction for partition-grain models is unspecified.

### E36 (line 2298) [partition]
**Entry:** "Generator-emitted partition-grain models are landed."
**Verdict:** MIXED
**Evidence:** Landed part: generator-emitted `ModelDef`s (meta_language.md) may carry partition-grain frontmatter subject to every rule on equal terms.
**Keep-content:** Per-`ModelDef` overrides are not part of the closed field set in v1. Tracked in `docs/plans/20260509-meta-language-overall.md`.

### E37 (line 2299) [partition]
**Entry:** "Diagnostic code ownership."
**Verdict:** LIVE
**Evidence:** Not a landed/unbuilt claim — a standing normative ownership rule (this spec owns semantics; `diagnostics.md` is the catalogue; the two must agree).
**Keep-content:** The ownership split (this spec = semantics of its diagnostic codes; `diagnostics.md` = cross-feature catalogue of severity + canonical trigger; must agree) should survive somewhere, though it arguably belongs in the spec body/References rather than Known Divergences.

### E38 (line 2300) [partition]
**Entry:** "`g_run >= g_part` auto-coarsening is not implemented."
**Verdict:** LIVE
**Evidence:** A sub-`g_part` run window is a hard rejection; auto-coarsening (or reject-with-suggestion) is a deferred enhancement.
**Keep-content:** Sub-`g_part` run windows hard-reject today (fail-closed chosen first); auto-coarsening the run window or suggesting a corrected value is a deferred future enhancement.

### E39 (line 2301) [partition]
**Entry:** "Monotone-integer `partition_column` is recognised by the trace but not yet driven end to end."
**Verdict:** MIXED
**Evidence:** Landed part: the event-time monotonicity trace and per-source bound/reach derivation admit a monotone integer key (constant `batch_id ± n` shift derives lookback like an INTERVAL shift).
**Keep-content:** Live gap: run-window/backfill-chunking and per-source scan-filter injection are date-typed (`run_start`/`run_end` are ISO dates), so an integer-partitioned model gets no end-to-end run; `smelt explain --json` clamp rendering is temporal-only. Tracked in `docs/plans/20260704-model-updates-l4-batched.md`.

### E40 (line 2305) [key]
**Entry:** "The pre-cut surface is removed."
**Verdict:** LANDED
**Evidence:** `refresh: keyed`/`cumulative` hard-error with fix-its (`crates/smelt-core/src/config.rs`; delivered by `docs/plans/20260707-maintenance-plan-impl.md`); the grain-conflict form is inexpressible; the retired `KeyedForbidsPartitionGrain` diagnostic's surviving case is covered by `PartitionGrainRequiresRefreshIncremental` (a strict superset).
**Keep-content:** none

### E41 (line 2306) [key]
**Entry:** "The classifier covers only the direct-monoid families."
**Verdict:** MIXED
**Evidence:** Landed part: the classifier seed (`rules/cumulative.rs`), windowed-keyed-maintenance driver, and per-window `merge_into` execution admit the additive-fold and extremal/lattice-fold families.
**Keep-content:** The classifier union (overwrite, once-write, plain-overwrite families) and the run-shape/posture derivation distinguishing window-forward from snapshot-reconcile are unbuilt. Decision record: `docs/research/20260705-keyed-collapse-application.md`; tracking plan: `docs/plans/20260705-keyed-collapse.md`.

### E42 (line 2307) [key]
**Entry:** "The transactional merge ledger is built on DuckDB only."
**Verdict:** MIXED
**Evidence:** Landed part: per-delta ledger table folded in-transaction (`Backend::fold_ledger_delta`, `smelt_state::ddl_duckdb`), repeat delta refuses with `KeyedReprocessedWindow`, idempotent-only cells never create the table. (Substantially duplicates E10.)
**Keep-content:** DuckDB is the only ledger substrate; an additive-graded cell on another backend fails loudly (`UnsupportedFeature`). Consider merging with E10 in the redraft.

### E43 (line 2308) [key]
**Entry:** "The snapshot-reconcile executor is unbuilt."
**Verdict:** LIVE
**Evidence:** An unclocked keyed model (zero timeseries-tagged sources) refuses fail-loud with `KeyedSnapshotPostureUnsupported`.
**Keep-content:** Until the snapshot-reconcile executor lands, unclocked keyed models refuse with `KeyedSnapshotPostureUnsupported` naming the delivering plan — a not-yet-supported refusal, not a model error.

### E44 (line 2309) [key]
**Entry:** "The time-partitioned keyed output's admission, downstream pushdown, downstream keyed driving-source selection, and the `smelt explain` settle-bound surface are all wired."
**Verdict:** LANDED
**Evidence:** All named pieces described as built/wired: the single fail-closed locality gate in plan derivation (`KeyedForbidsTimeseries` refusal naming all three routes), pure per-route settle-bound derivation threaded onto `MaintenancePlan` and printed by explain, downstream partition-grain pushdown and keyed driving-source resolution treating the composed output like a declared source. Design derivation: `docs/research/20260705-keyed-time-superset.md`. No gap recorded in this entry itself (gaps live in E16/E45).
**Keep-content:** none

### E45 (line 2310) [key]
**Entry:** "Locality open questions."
**Verdict:** LIVE
**Evidence:** Three explicitly open questions.
**Keep-content:** Open: (1) whether a derived recurrence bound can license slice pruning under snapshot-reconcile (v1: window-forward only); (2) relaxing the granularity-equality precondition (daily driver, weekly output partitions); (3) slice-scoped deletion (`NOT MATCHED BY SOURCE` over a provably complete slice) — interacts with the key-deletion divergence (E51).

### E46 (line 2311) [key]
**Entry:** "The pattern functions (`smelt.latest`, `smelt.once`, `smelt.current`) are unshipped"
**Verdict:** LIVE
**Evidence:** Unshipped; built-ins-vs-template-files decision unmade; canonical once-write spelling fixed alongside them.
**Keep-content:** The three pattern functions are unshipped and their built-in-vs-template-file form undecided; the canonical once-write spelling will be fixed with them. Tracked in the keyed-collapse plan.

### E47 (line 2312) [key]
**Entry:** "Driver granularity is `day`/`week` only"
**Verdict:** LIVE
**Evidence:** `maintenance_driver.rs::driving_steps` refuses other granularities.
**Keep-content:** The shared driver refuses granularities other than day/week — a property inherited by every consumer; widening is driver work, not profile work.

### E48 (line 2313) [key]
**Entry:** "`--auto` staleness fidelity"
**Verdict:** LIVE
**Evidence:** Exact changed-window fidelity for all-invertible models needs the group rung's delta-history mechanism; v1 is conservative.
**Keep-content:** `--auto` staleness for all-invertible models is conservative in v1; "exactly the changed windows" needs the group-rung delta-history mechanism. Carried from the cumulative-era list.

### E49 (line 2314) [key]
**Entry:** "Self-referential keyed models"
**Verdict:** LIVE
**Evidence:** Rejected — self-reference is not an admissible input; an input/state-distinction design would be needed.
**Keep-content:** Self-referential keyed models (`state += delta − decay`) are rejected; admitting them needs an explicit input/state-distinction design. Carried.

### E50 (line 2315) [key]
**Entry:** "Run-pinning alignment"
**Verdict:** LIVE
**Evidence:** `NOW()`/`CURRENT_*` rejected outright rather than compile-time-pinned as the partition grain does.
**Keep-content:** Adopting the partition grain's compile-time pinning transform for `NOW()`/`CURRENT_*` in keyed models is a deferred alignment; today they are rejected outright. Carried.

### E51 (line 2316) [key]
**Entry:** "Key deletion is unresolved beyond retention."
**Verdict:** LIVE
**Evidence:** Snapshot-reconcile retains departed keys; window-forward has no delete signal short of a change feed with delete events.
**Keep-content:** Key deletion is unresolved: tombstones, opt-in hard delete, and the observer contract for refused matrix cells are deferred in the decision record (§5 there).

### E52 (line 2317) [key]
**Entry:** "Rungs 2–4 are specified ahead of this profile's use of them"
**Verdict:** LIVE
**Evidence:** AVG-via-decomposed-state, group-rung retraction, bounded-domain multiset mechanisms live in `model_transforms.md`/`model_properties.md` but are not wired into keyed columns.
**Keep-content:** Rungs 2–4 mechanisms are specified elsewhere; wiring them into keyed columns is future composition work.

### E53 (line 2321) [interval]
**Entry:** "Not implemented — `versioning:` does not parse."
**Verdict:** LIVE
**Evidence:** No `versioning:` frontmatter key exists, so `versioning: interval` fails deserialization; the classifier, close-old/open-new maintenance, and validity-column management are assigned to (not evidenced as shipped by) `docs/plans/20260707-maintenance-plan-impl.md`.
**Keep-content:** Interval versioning is entirely unbuilt: `versioning:` does not parse (cross-ref `models.md` §Known Divergences); classifier, close-old/open-new maintenance via `merge_into`, and validity-column management are the delivering plan's scope. (Verification note: the "delivered by" phrasing is ambiguous — read as the delivering plan, since the headline says "Not implemented".)

### E54 (line 2322) [interval]
**Entry:** "Validity-column surface is unsettled."
**Verdict:** LIVE
**Evidence:** Names/types of `valid_from`/`valid_to`/`is_current`, NULL vs sentinel open interval, configurability all open.
**Keep-content:** Validity-column naming/typing, open-interval representation (NULL vs far-future sentinel), and configurability are Open Questions to settle when the profile is built.

### E55 (line 2323) [interval]
**Entry:** "Tracked-attribute selection is unsettled."
**Verdict:** LIVE
**Evidence:** All projected non-key columns vs a declared subset; how to mark a column untracked — undecided.
**Keep-content:** Tracked-attribute selection is undecided; preference is deriving from SQL over a strategy block, but the exact line is open.

### E56 (line 2324) [interval]
**Entry:** "Late corrections to a closed interval."
**Verdict:** LIVE
**Evidence:** Deletion is settled as soft-close, but corrections to an already-closed interval and any opt-in hard-delete surface need their own design.
**Keep-content:** How a correction to an already-closed interval is applied (and any opt-in hard delete) needs its own design — the same retraction question the key grain shares (§"Reprocessing"; `docs/research/20260703-model-updates.md` §18.2).

### E57 (line 2325) [interval]
**Entry:** "Umbrella subsumption."
**Verdict:** LIVE
**Evidence:** A settled design decision about an unbuilt profile (standalone classifier, not shared execution machinery with the key grain), not shipped work.
**Keep-content:** The interval-versioning profile is settled as standalone (its own classifier), composing shared capabilities by name but owning its combiner — consistent with the narrow-composable-rules posture (`docs/research/20260522-cumulative-as-its-own-rule.md`). This decision should survive (possibly promoted into the spec body when the profile is built).

# Claims inventory — Part F: ## Future Extensions + ## References (lines 2327–2588)

Source: /home/andrew/smelt-sql/.claude/worktrees/incremental3/docs/specs/incremental_models.md

## Future Extensions (2327–2428)

### F1 (lines 2327–2331) [Section preamble — non-surface status]
Blanket status claim covering the whole section: these are ideas for widening the plan's admission space beyond what is decided; **nothing here is surface** — no `maintenance:` field, diagnostic, or technique in this section may be relied on until it graduates into §Surface/§Semantics via its own spec diff and plan. This preamble is the "not decided / may not be relied on" statement for every idea below and must survive verbatim in force.

### F2 (lines 2333–2356) [Row-local column derivation]
Motivating case: a column that is a pure function of other columns in the same row (truncated date, normalized GUID, case-folded string). The **added**-column case is explicitly NOT a new idea — it is already the spec'd `PureBackfill` verdict (§"The definition-change trigger"; classification proof in `model_properties.md` §"Definition-change column classification"), tracked as unbuilt in §Known Divergences, needing only an implementation of `classify_definition_change`. The **open extension is the changed-column case** (sub-bullet 2343–2356): redefining an existing column's expression has no plan-level treatment today (falls to full recompute); the extension would apply the same per-column-provenance test to admit a targeted in-place `UPDATE`, and would need its own trigger (distinct from the additive-only definition-change trigger), its own fail-closed diagnostic naming, and a decision on ledger composition (redefinition invalidates the ledger's provenance identity for the group despite no upstream delta).

### F3 (lines 2358–2369) [Automatic, watermark-diffed `--since-upstream`]
Motivating case: today `--since-upstream` requires explicit per-source `--source`/`--landed` flags (§CLI, §Known Divergences). Extension: persist a per-source "last propagated through" watermark in `smelt-state`, diff it against the source's current `covered_intervals`, so a bare `--since-upstream` discovers its own delta. Explicit scope carve-outs that must survive: (a) this still does not solve a raw never-modeled source's freshness — no `covered_intervals` exists; live backend source-freshness querying is out of scope (no such capability in `smelt-backend*` today; sources declare posture in `sources.md`, are not polled); (b) explicit and automatic forms are not exclusive — the automatic form computes the same `--landed` intervals, layering on top without changing the graph layer or §CLI surface.

### F4 (lines 2371–2373) [Conditional maintenance without a change feed — umbrella]
Three composable mechanisms (M1–M3), sourced from `docs/research/20260715-conditional-maintenance-without-cdf.md` with tracking plan `docs/plans/20260715-composed-axes-conditional-maintenance.md`.

### F4a (lines 2374–2378) [M1 — change-suppressed writes]
Emitted MERGE gains an `IS DISTINCT FROM` matched-arm predicate (merge-less backends: staged-candidate conditional DELETE+INSERT), so unchanged regions write zero rows and redelivery storms become no-ops. Status claim: **built** for the column-scoped and keyed-fold write families (`model_transforms.md` §Known Divergences "Change-suppressed MERGE").

### F4b (lines 2379–2398) [M2 — delta-restricted enrichment compute]
Where the row skeleton is provably owned by the driving source alone (payload-only 1:1 enrichment joins), expensive joins run only over rows whose enrichment inputs changed — delta-join algebra licensed by the skeleton-source-closure proof (`model_properties.md` §"Skeleton-source closure", P1) plus an exact input delta. Status claims: the proof, the transform, and the `referential_integrity` world-fact (`sources.md`, consumed by the proof's row-preservation conjunct for inner joins) are all **built** and reach a maintained-model edge's own driving-source recompute; the restriction licence **also extends to an `UpstreamMutation` cell driven by an external `mutable_snapshot` source** — same proof and same gate (`smelt_logical::maintenance::choice::resolve_recompute_restriction`) admit the cell when the enrichment join closes and M3's fingerprint-sidecar synthesized changed-key set is non-empty for the touched region (renamed dimension row → point lookup, not full-reach scan). Explicit gap: wiring into a live run's own trigger/technique dispatch (`crates/smelt-runtime/src/execute.rs` regular incremental batch loop) is separate follow-on work; today the mechanism is proven against a real fixture and real backend directly — the same "build it, then wire live dispatch" split as M3's sidecar halves.

### F4c (lines 2399–2419) [M3 — derived change feeds]
Snapshot-diff on both boundaries: a fingerprint sidecar (lifecycle: `sources.md` §"The fingerprint sidecar") synthesizes a change feed for an external `mutable_snapshot` source; the conditional write's changed-row set is recorded as the model's **observed output delta**, making every maintained model a change-feed-postured upstream for free. On a composed (key + time) output the observed delta projects to exact partition dirt (§"What the composed shape uniquely enables") — what makes M3 propagatable through the interval-based graph without keyed dirt-sets. Status claims: output-delta half (recording + key→partition projection) **built** for the change-suppressed column-scoped MERGE family (§Known Divergences "Observed-delta recording is built…"); fingerprint-sidecar half **built for DuckDB** (table DDL, digest-refresh upsert, emitter-authored diff query — `sources.md` §Known Divergences) as a standalone independently-tested capability; **non-DuckDB target fails loudly**. Invalidation is live: stored row identity stamp (digest-construction version, P4 projection identity, hash of consuming model's SQL) checked against freshly computed on every diff; any mismatch degrades that partition to whole-table delta (same as absent sidecar), logged loudly, never silently trusted or skipped (`sources.md` §"The fingerprint sidecar" — "Invalidation"). Explicit gap: wiring the synthesized changed-key set into a live run's trigger/technique selection is separate follow-on work.

### F5 (lines 2420–2428) [M1–M3 graduation requirements]
Each mechanism needs its own spec diff before it is surface. Cross-spec status ledger that must survive: P1–P4 proofs in `model_properties.md` (P1–P4 landed; P4 = fingerprint projection, §"Fingerprint projection"); T1–T5 transforms in `model_transforms.md` (T1/T2/T3 landed as catalogue rows; T5 observed-output-delta recording is specified in this spec's graph-layer section, not as a catalogue row; T4 fingerprint sidecar build + diff query is built for DuckDB); referential-integrity world-fact (landed) and landed-delta refinement (landed) in `sources.md`; capability flags in `multi_backend.md`; persistence-fingerprint stance to be reconciled with `output_fingerprint.md`.

## References (2430–2588)

### F6 (lines 2432–2543) [References — the contract, plan, and graph layer]
Categories present: **Code, Tests, User docs, Plans (history), Research, Related specs**. Prose-bearing claims that must survive (the Tests entry is by far the largest prose block in the whole References section, 2444–2514):
- Tests prose (2444–2453): the smelt-logical tracer tests are "the regression floor for chains, fan-out, diamonds, granularity mapping, and adjointness"; `maintenance_propagation_adjoint.rs` is the dedicated home for the `forward(backward(P)) ⊇ P` law; smelt-runtime tracer tests are the DuckDB equivalence oracles + real-workspace propagation-graph assembly; `since_upstream.rs` includes the sufficiency-vs-full-refresh equivalence check; `include_upstreams.rs` covers two-hop resolved-slices-suffice equivalence and an unclocked-ancestor-resolves-to-whole-table case.
- Tests prose (2454–2465): `crates/smelt-maintenance-testkit` is dev-only, `publish = false`, the Link-C in-process harness, with named components (recipe.rs, schedule_gen.rs, s_tracker.rs, oracle_modes.rs, oracle.rs), wired as a dev-dependency of smelt-cli. `maintenance_conformance` is the standing generative equivalence gate: deterministic-seeded typed `ModelRecipe` sample (append-only partition-grain, fact+mutable-dimension, `grain: key`, generated 2-3 node DAGs), staged, classified through the real derivation, driven through `execute_project` against real DuckDB, asserted equal to a full-refresh oracle after every step under adversarial append/lateness/mutation/redelivery/definition-change schedules; `SMELT_CONFORMANCE_CASES` scales depth.
- Tests prose (2465–2470): the composed (`grain: key` + `timeseries:`) recipe family exercises all three key-temporal-locality routes — key-embedded via `execute_project`; key-determined and declared-recurrence-bounded (with in-bound redeliveries) driven directly against real DuckDB, the same workaround `locality_route3_recurrence_check.rs` uses — asserting whole-table and per-slice equivalence after every step, gated by its own admission-rate floor (`SMELT_CONFORMANCE_COMPOSED_CASES`).
- Tests prose (2470–2480): the generated model-edge enrichment recipe family (one closure-admissible `LEFT JOIN` + two closure-failing siblings: bare inner join, membership predicate over an enrichment column) drives delta-restricted-vs-widened-scan both ways over the same fixed `S`; its own P1 verdict, derived through the real per-cell derivation (not asserted), gates which cases run; end states must be bit-identical. A second case drives a fully-suppressed conditional write through real observed-delta recording and asserts the cascade: zero rows written, present-and-empty recorded delta, zero regions scheduled downstream, end state still equal to full-refresh oracle; both gated by their own admission-rate floor.
- Tests prose (2480–2484): key-determined merge mechanics (write-once partition, additive fold) exercised against real DuckDB, but the slice-pruned target scan is NOT — the driver omits the slice predicate because DuckDB's `MERGE` binder refuses the predicate shape (§"Key temporal locality", the `BindMerge` divergence).
- Tests prose (2484–2489): a `pinned` module reproduces every construct × posture cell and named hazard schedule as deterministic always-reproducible cases (never proptest-drawn alone); a `registry` module tracks named divergences with a staleness report (never-firing entries reported, never failed — same governance pattern as `known_unknowns.rs`).
- Tests prose (2489–2496): `property_discovery/` probe modules cover constructs the recipe generator lacks vocabulary for (self-referential models, `UNION ALL`, `LEFT JOIN`, correlated `EXISTS`, stacked window frames, cross-source column-name collision, mutable source aggregated directly) and remain **disposable research probes** (`.claude/scripts/property-experimental-gate.sh`). `crates/smelt-cli/tests/incremental/` is narrower still: drives a backend's incremental strategy with a hand-supplied filter, proving the strategy executes correctly once handed one, independent of filter derivation.
- Tests prose (2496–2514): `maintenance_plan_conformance::coverage_matrix_is_inhabited` is the standing inventory gate over the research example-catalogue coverage matrix (plus one added `INTERSECT`/`EXCEPT` row): matrix encoded as data; every inhabited (construct × source-property) cell accounted for by exactly one of two explicit disjoint lists — `CLAIMED` (grounded executable test proves HOLDS-or-refuses) or `KNOWN_GAPS` (named, not silently omitted); adding an unmatched cell fails by construction (additive-only). `CLAIMED` currently lifts 9 catalogue ids (EX-02, EX-08, EX-12, EX-14, EX-18, EX-24, EX-26, EX-27, EX-35, plus the added EX-41/EX-42 row); remaining ~100 inhabited cells named individually in `KNOWN_GAPS` (most "plausibly covered by an existing pinned hazard case or `G-*`/`SC-*` probe, not re-verified against this exact catalogue id" — cross-referencing is itself unbuilt; a few, like EX-25's LAG/LEAD footprint reflection and EX-29's as-of-run-contract gating, need production investigation not yet done). Both lists are per-cell, never per-row, so one cell can be lifted at a time.
- User-docs prose (2515–2519): the listed pages "describe the trichotomy + grain surface"; `cli.md` additionally documents `--since-upstream`, `--include-upstreams`, and `smelt explain`'s cell/clamp/ledger report with `--show-sql`; `smelt-yml.md` documents the `maintenance:` block.
- Plans prose (2520–2522): `20260704-model-updates-fundamentals.md` is "the L1+L2 substrate"; `20260705-property-discovery-loop.md` is "the empirical engine".
- Research prose (2523–2528): the 20260715 conditional-maintenance doc is the source of the pruning taxonomy's no-op write-elimination category and the composed-shape composition points; the 20260716 relation-contract doc covers the shared Relation Contract, grain-as-derived-label, per-cell write addressing, and the open write-pattern registry that this spec's §"Per-cell write addressing" and §"The declared shape axis" encode.
- Related-specs prose (2529–2542): this is stated to be "one list for the whole spec"; each spec carries a parenthetical scope note (e.g. `model_properties.md` = the derived proofs; `model_transforms.md` = the physical mechanisms; `models.md` = refresh axis / declared shape facts / Relation Contract / three-state declaration law / input-consumption axis / litmus rule; `expansion.md` "runs before every analysis stage here"; `materialized_view.md` = "where beyond-the-ladder shapes and hand-written SCD2 go"; `multi_backend.md` = backend capability flags a strategy checks). The parenthetical scope notes are claims.

### F7 (lines 2544–2567) [References — the partition grain]
Categories present: **Code, Tests, User docs, Plans (history), Research, Legacy reference**. Prose-bearing claims:
- Code annotations naming specific symbols per file (e.g. `inject_time_filter`, `delete_and_insert_transactional` "(per-chunk transaction boundary)", `BackendCapabilities::supports_merge`; incremental.rs note "in `smelt-logical`; `smelt-planner` re-exports").
- Tests (2555) names "the per-partition full-refresh-equivalence harness" alongside the file references.
- Plans prose (2557–2561): `20260322-incremental-model-support.md` — "comprehensive plan; many phases still open"; `20260704-model-updates.md` — "the mode-vertical master this spec re-cuts as a composition"; `20260707-maintenance-plan-impl.md` — "lands the target frontmatter surface and diagnostics".
- Research prose (2562–2566): per-doc scope notes ("design direction this spec absorbs"; "batched eligibility audit; §9.2 non-determinism derivation"; "the maintenance-framework design"; "the shape-profile demotion and per-cell admission this spec composes").
- Legacy prose (2567): `docs/DESIGN.md` §"Incremental Table Builds" — **"superseded for current behavior; useful for design rationale"**.

### F8 (lines 2569–2575) [References — the key grain]
Categories present: **Code, Tests, User docs, Plans (history), Research** (no Legacy). Prose-bearing claims:
- Code annotations (2571): `cumulative.rs` is "the built classifier seed — combiner lookup, GROUP-BY key derivation, driving-source resolution"; `maintenance_driver.rs` hosts the windowed-keyed-maintenance driver / `WindowedKeyedRule`.
- Tests (2572) names "the keyed end-state-equivalence harness".
- User-docs prose (2573): `materializations.md` "(to be replaced by a keyed-models guide with per-pattern recipes)" — a forward-looking claim; `incremental-models.md` §"The composed shape (key + time)" documents the composed form and its three locality routes; `deduplication.md` is the worked tutorial — a redelivery-prone feed deduplicated by a keyed extremal fold under a declared recurrence bound, contrasted against the partition-grain `QUALIFY`-window workaround from the preceding tutorial page.
- Plans/Research prose (2574–2575): per-doc scope notes, notably `20260523-cumulative-aggregate.md` = "the built seed", `20260705-keyed-collapse-application.md` = "the decision record this spec encodes", `20260704-monotone-join-maintenance.md` = "the monotone-vs-retractable boundary".

### F9 (lines 2577–2588) [References — interval versioning]
Categories present: **Code, Research, Plans (history)** only (no Tests / User docs / Related specs / Legacy). Prose-bearing claims:
- Code prose (2579): `RefreshStrategy` — **"no `grain`/`versioning` surface yet"**; "on build, the classifier under `crates/smelt-logical/src/rules/` and the maintenance path under `crates/smelt-runtime/`" (forward-looking placement claim).
- Research prose (2580–2584): per-doc scope notes — `20260703-model-updates.md` Part 17 (user surface; naming) and Part 19 (input-consumption axis); `20260522-cumulative-as-its-own-rule.md` = the sibling-rule sketches (`scd2`, `latest_value`, `accumulating_snapshot`).
- Plans prose (2585–2587): `20260707-maintenance-plan-impl.md` "lands the target frontmatter surface (`grain`/`versioning`) and diagnostics".
