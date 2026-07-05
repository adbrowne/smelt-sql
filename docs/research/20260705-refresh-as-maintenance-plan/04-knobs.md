# 04 — Knobs: the user-facing configuration surface

- **Date**: 2026-07-06
- **Status**: research (part 4 of [the refresh-as-maintenance-plan series](README.md))
- **Depends on**: [01-framework.md](01-framework.md) (the per-cell plan, the theorem, skeleton/payload), [02-loop-findings.md](02-loop-findings.md) (what smelt actually executes today), [05-source-properties.md](05-source-properties.md) (the source-side declarations several knobs consume)

This part proposes the complete configuration surface a user touches once maintenance is a
per-`(column-group × input)` plan. Every proposal is written as a **diff against the surface that
exists today** (`models.md`, `batched_models.md`, `keyed_models.md`, `sources.md`,
`model_properties.md` §"Model-scoped declarations") — nothing here is invented on a blank page.

## Governing principles (restated, then applied)

Four rules from the framework govern every knob below; each knob entry says which rule it answers to.

1. **Derive-else-declare.** Anything statically derivable from the SQL is derived; a declaration is
   admitted only for world-facts smelt cannot compute (`models.md` §Design). Andrew's standing
   preference sharpens this: time-window properties (lookback, batch safety) are derived from the
   model's SQL/functions, never declared in YAML where they can drift.
2. **Declared things are checked assertions, never silent selectors.** A declaration is validated
   against the derived plan and errors on mismatch (`01-framework.md` §10 — shape stays declared
   *because* it is checkable; strategy content moves to derived *because* leaving it declared makes
   the enum lie).
3. **Every relaxation names the guarantee it trades** (`01-framework.md` §6). A knob that weakens a
   contract must say which column loses which guarantee; unlabeled looseness is the dbt
   `strategy:` failure this design exists to avoid.
4. **Only proofs prune; a declared bound is admitted only checked** (`model_maintenance.md`
   §"Windowed maintenance and the horizon"). Declarations that can only *widen* a scan are safe to
   trust; declarations that *narrow* one are admitted only with a fail-loud runtime check (the
   `key_recurrence` precedent, `sources.md` §Design).

Each knob below states: the proposed surface, its **status** (declared / derived / override), the
default, what it trades or validates, and the motivating section or ledger cell.

---

## K1 — The refresh surface: trichotomy + declared shape/grain

**Status:** declared (checked against the derived plan). **Motivation:** `01-framework.md` §10;
`models.md` §"Refresh axis".

Today's five-value enum (`full` / `batched` / `keyed` / `versioned` / `materialized_view`) collapses
to the honest trichotomy, with the shape/grain content of the old modes surviving as explicit,
checked declarations:

```yaml
---
refresh: incremental          # full (default) | incremental | materialized_view
grain: partition              # partition | key | key_per_partition   (required for incremental)
unique_key: [order_id]        # required when grain is key-addressed (grain: key | key_per_partition)
timeseries:                   # required when grain: partition; admitted on grain: key
  event_time_column: order_date    #   only under key temporal locality (unchanged from keyed_models.md)
  partition_column: order_date
  granularity: day
---
```

- `refresh: full` and `refresh: materialized_view` are unchanged (freshness-owner trichotomy — the
  part of the old enum the paper keeps).
- `refresh: incremental` replaces `batched`/`keyed`/`versioned` as the *declared* commitment. What
  those modes actually pinned down — output shape and grain — is now said directly:
  - `grain: partition` ≈ today's `batched`: partition-addressed output, no row identity needed
    (identity is optional exactly as `batched.unique_key` is optional today).
  - `grain: key` ≈ today's `keyed`: key-addressed end-state, `unique_key` required (declared, not
    silently derived from `GROUP BY` — see the validation note below).
  - `grain: key_per_partition` — the trajectory grain (`01-framework.md` §7): one stored row per
    `(key, partition)`. Naming it makes the §7 grain distinction (end-state vs trajectory) a
    declared, reviewable fact instead of an accident of the SQL. Under late data this grain's
    forward footprint is unbounded, so it is admissible only with the `backfill:` discipline in K5
    or a declared lateness truncation.
  - `versioned` becomes `grain: key` + `versioning: interval` (a sub-declaration, since SCD2 is
    key-grain plus a close-out write) — deliberately *not* a fourth grain: the row-addressing is
    still by key; the interval is payload+skeleton structure within the key.
- **Strategy content is gone from the surface.** There is no per-model knob that says
  "DELETE+INSERT" or "merge" or "fold". The planner derives the technique per
  `(column-group × input)` cell and `smelt explain` reports it (K4).

**Validation (rule 2).** The declared grain is checked against the derived plan:
- `grain: partition` with a cell that *requires* key-addressing (e.g. a late-conversion targeted
  merge in its only admissible plan) → error naming the cell and the two candidate grains.
- `unique_key` not actually unique at the declared grain → the existing keyed derivation cross-check,
  plus the optional runtime uniqueness probe ([05](05-source-properties.md) §P3).
- `unique_key` containing a non-deterministic or payload column → error (OQ1 resolution: skeleton
  positions must be deterministic).
- For key-grain aggregated bodies, a declared `unique_key` that differs from the `GROUP BY` column
  set → error. Keeping the declaration (rather than pure derivation as `keyed_models.md` does
  today) is deliberate: identity is the one thing every technique writing the table must agree on
  (`01-framework.md` §10), so it should be visible in review even when derivable. This is a checked
  restatement, which the derive-else-declare rule tolerates exactly when the check is total.

**Migration for an existing `refresh: batched` model** is mechanical and lossless:

```yaml
# before                          # after
refresh: batched                  refresh: incremental
timeseries: {...}                 grain: partition
batched:                          timeseries: {...}
  unique_key: [order_id]          unique_key: [order_id]        # optional, as before
  nondeterministic_columns: [..]  columns: { inserted_at: { contract: plausible } }   # K3
  safety_overrides: {...}         safety_overrides: {...}       # unchanged, still partition-grain-only
```

`refresh: keyed` → `refresh: incremental` + `grain: key` + explicit `unique_key`. The project is
pre-1.0 with no compatibility constraints, so this is a hard cut with a `smelt migrate` assist, not
a compat shim.

**Contentious point, flagged:** an alternative keeps `refresh: batched|keyed|versioned` as *sugar*
for the grain declarations (each name pinning a grain, all strategy content still derived). That
preserves familiar names at the cost of implying the old strategy semantics. This proposal removes
the names to make the semantic change unmissable; the sugar variant is the fallback if migration
friction matters more.

---

## K2 — `maintenance:` — per-cell technique override

**Status:** override (derived by default; frontmatter pins a cell). **Motivation:** OQ3 resolution
(`01-framework.md`): sensible defaults, frontmatter override, offline measurement as the real
selector.

The planner picks each cell's technique from defaults (delta size vs region size, backend merge
support). The override exists so a bake-off result (K6) — or an operator who knows the workload —
can pin a cell. Proposed surface:

```yaml
maintenance:
  defaults:
    prefer: recompute            # recompute | fold | auto   (auto = cost-model default)
  cells:
    - columns: [converted]       # the column-group, named by member columns
      on: smelt.sources.conversions   # the input whose delta this cell handles
      technique: fold            # fold | recompute | rederive_columns
    - columns: [converted]
      on: backfill               # the reserved trigger name for explicit region recompute
      technique: recompute
```

Addressing scheme: a cell is `(columns, on)` where `columns` names any member of a derived column
group (the planner resolves it to the whole group and errors if the listed columns span two groups
— that would silently re-partition the plan) and `on` is either a source address or the reserved
trigger `backfill`. This mirrors the plan matrix in `01-framework.md` §2 exactly, so `smelt explain`
output and the override surface use the same coordinates.

**Validation (rules 2, 4).** An override naming a technique outside the cell's *admissible* set is
an error citing the refusing proof (e.g. `fold` on a non-invertible combiner over a retractable
source → the observer-semantics refusal). An override can only choose among proven-interchangeable
techniques — by the §4 theorem this changes cost and freshness, never contract. That theorem is
what makes this knob safe to expose at all.

**Default posture:** the block is absent from almost every model. It exists for the measured-pin
workflow, not as a routine tuning surface. Keep it deliberately minimal: no per-cell cost hints, no
scheduling knobs — those belong to the cost model and the bake-off, not frontmatter.

---

## K3 — Per-column contracts: `nondeterministic_columns` generalized

**Status:** declared (checked, with DAG propagation). **Motivation:** `01-framework.md` §6; OQ1
resolution; `batched_models.md` §"Non-determinism and the payload rule".

Today's `batched.nondeterministic_columns` list becomes a per-column `contract:` in the existing
`columns:` metadata map (`models.md` §"`columns:` — column metadata"), since it is not
batched-specific:

```yaml
columns:
  inserted_at:
    contract: plausible          # exact (default) | plausible
  event_id: {}                   # exact, skeleton — nothing to declare
```

- `exact` (default): held to the `S`-indexed equivalence invariant.
- `plausible`: the payload relaxation — non-determinism admitted, bit-identity traded for
  plausibility. The trade is named in the declaration itself (rule 3).
- `as-of-run` is **reserved and rejected** (OQ2 resolution: deferred, tracked, not driving the
  design). The parser recognises the value and errors with "not yet admitted", so the lattice slot
  is visible without being buildable.

**What is not a knob:** skeleton-position enforcement. A `plausible` column reaching any skeleton
position — `unique_key`, partition/grain column, `JOIN … ON`, `WHERE`, `GROUP BY`, dedup/ordering
key, window-bound expression — fails loud, *including downstream*: consumer-side propagation (a
payload column of `M` consumed in a skeleton position of `N` is an error at `N` naming `M`'s
declaration) is automatic and unconditional. There is deliberately no
`allow_payload_in_skeleton:` escape hatch; the OQ1 resolution bars it outright, and the sanctioned
escape is a stable derivation (hash of skeleton columns).

---

## K4 — Settle/freshness: derived and reported, almost nothing declared

**Status:** derived (reported via `smelt explain` / the ledger); one declaration lives on the
source. **Motivation:** `01-framework.md` §6 (the two-dimensional ledger); §4 (`S`-indexed
freshness).

The per-column settle bound is **derived**: watermark-relative by construction (a column folding a
source with 7-day reach settles when that source's watermark passes `event_ts + 7d`). The model
frontmatter declares nothing. What the user gets:

- `smelt explain <model>` prints the plan matrix — per column group × input: technique, read/write
  scope, equivalence contract, settle condition — extending the existing per-source-clamp
  observability surface (`batched_models.md` §"Observing the per-source clamp").
- An *absolute* settle statement ("settled 9 days after event time") appears only when the source
  carries a declared lateness bound ([05](05-source-properties.md) §P2). No lateness declaration →
  the honest watermark-relative form only. This is rule 3 applied to freshness: a fixed number
  without a declared bound is exactly the unlabeled looseness §6 forbids.
- A possible future `freshness_target:` (alerting when a column's reflected `Sᵢ` lags) is
  operational monitoring, not maintenance semantics — out of scope here, noted so nobody wedges it
  into `maintenance:`.

---

## K5 — Backfill semantics: cascade policy, horizon, lateness handling

**Status:** mixed — one new declared knob (`backfill:`), the rest existing derived machinery.
**Motivation:** ledger cell G-08 (the silent trajectory-staleness trap); `model_maintenance.md`
§"Windowed maintenance and the horizon".

**`backfill: cascade | local`** — admitted only where it matters, derived where it can be:

```yaml
backfill: cascade      # default wherever the model has a cross-partition forward dependency
```

- For a model with a **self-edge or trajectory grain** (G-08's running balance; any
  `window_independence` = `Ordered` model), a backfill of partition `p` leaves every partition
  `> p` stale unless re-run in order. The *fact* that a cascade is required is derived (from the
  self-edge / grain), so the knob's role is narrow: `cascade` (default — the runtime schedules the
  downstream re-derivation automatically) vs `local` (explicitly accept the G-08 trap: rebuild only
  the named window, downstream partitions knowingly stale — a rule-3 named trade for operators
  doing staged repairs). A model with no cross-partition dependency ignores the knob (declaring it
  is a warning, not an error — it asserts nothing false, it is merely inert).
- **Not** a general dependency-cascade system: it covers the model's *own* downstream partitions,
  not downstream models (that is `smelt backbuild`'s existing job).

**Horizon and lateness** — unchanged, restated for completeness: the horizon stays derived with
`horizon_ceiling:` as a warning-only ceiling (`model_properties.md`); beyond-horizon late rows stay
a model-author + data-quality concern. The re-stamp-vs-drop choice stays a **modelling pattern**
(fold the late row into the current partition carrying a lateness flag), not a maintenance knob —
adding a `late_rows: restamp|drop` knob was considered and rejected: it would make the maintenance
layer silently rewrite event time, a skeleton mutation, violating rule 2.

---

## K6 — The bake-off surface (CLI, not frontmatter)

**Status:** CLI command + pinned override output. **Motivation:** `01-framework.md` §11 — offline
whole-workload measurement as a first-class smelt advantage.

```
smelt bakeoff <model> [--cells <col>@<source>,...] \
      --replay <schedule.yml | --from-history N-runs> \
      [--target <name>]
```

- For each named cell (default: every cell with ≥2 admissible techniques), materialise each
  admissible technique's plan over the same replayed run schedule against a scratch schema, and
  measure: wall-clock, rows read/written, backend cost proxies, and (safety net, not the point)
  an `EXCEPT ALL` equivalence check between the variants — which the §4 theorem predicts is empty
  on skeleton columns.
- Report: per cell × technique, measured cost per run and extrapolated cost over a declared horizon
  ("over a year at this cadence"). The run-schedule replay substrate is the property-discovery
  harness productionized ([08-code-placement.md](08-code-placement.md)).
- **Committing a choice**: `smelt bakeoff --pin` emits the winning `maintenance:` block (K2) as a
  frontmatter patch for the user to review and commit. The pin is an ordinary K2 override —
  reviewable, versioned, and re-validated on every compile (so a model edit that changes the cell's
  admissible set fails loud rather than silently keeping a stale pin).

---

## K7 — Escape hatches (existing; unchanged, listed for completeness)

- `--full-refresh` — truncate-and-rebuild, the universal ground-truth reset (already the keyed
  mitigation for `KeyedReprocessedWindow`). Under the generalized ledger it additionally resets
  every ledger entry it overwrites (`01-framework.md` OQ4 design).
- `--event-time-start/--event-time-end` — the per-run window (unchanged; `cli.md`). An explicit
  re-run of a processed window *is* the region recompute, as every G-cell exercised.
- `safety_overrides:` — retained, **partition-grain only** (unchanged rationale from
  `keyed_models.md` §Design: keyed rejections guard the invariant itself, nothing safe to waive).
- `timeseries.assert_monotonic`, `functional_dependencies:`, `bounded_domain:` — the existing
  proof-widening declarations (`model_properties.md`), untouched by this proposal.

---

## Summary table

| Knob | Surface | Status | Default | Validated by / trades |
|---|---|---|---|---|
| Refresh trichotomy | `refresh: full\|incremental\|materialized_view` | declared | `full` | freshness-owner is a real commitment; unchanged |
| Output grain | `grain: partition\|key\|key_per_partition` (+ `versioning: interval`) | declared, checked | — (required for incremental) | checked against derived plan; mismatch = error (K1) |
| Row identity | `unique_key: [..]` | declared, checked | absent (partition grain) / required (key grains) | uniqueness cross-check + probe; deterministic-columns rule |
| Technique per cell | `maintenance.cells[]` | **override** (derived default) | absent (cost-model default) | admissibility check; theorem guarantees contract-preservation |
| Column contract | `columns.<c>.contract: exact\|plausible` | declared | `exact` | skeleton-position + DAG propagation fail-loud; trades bit-identity → plausibility |
| Settle bounds | *(none — `smelt explain` output)* | derived | — | absolute form requires source lateness declaration |
| Backfill cascade | `backfill: cascade\|local` | declared (need is derived) | `cascade` where self-edge/trajectory | `local` = named staleness trade (G-08) |
| Horizon ceiling | `horizon_ceiling:` | declared, warning-only | absent | never relaxes the derived clamp (existing) |
| Bake-off | `smelt bakeoff` (+ `--pin` → K2 block) | CLI | — | measured; pin re-validated per compile |
| Full refresh | `--full-refresh` | CLI | — | resets ledger regions it overwrites |
| Safety overrides | `safety_overrides:` | declared | all false | partition-grain only (existing) |

## Open questions this part leaves

- Whether `batched`/`keyed` survive as sugar names for the grain declarations (K1, flagged).
- The `maintenance.defaults.prefer` granularity — per-model may be too coarse and per-cell too
  noisy; the bake-off experience should decide.
- Where the `columns.<c>.contract` key collides with future column-level `tests:` — same map, needs
  one grammar owner (`models.md` §"`columns:`").
