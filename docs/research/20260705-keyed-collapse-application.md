# Applying the keyed collapse: decisions, resolved questions, and the change list

**Status:** research (application / decision record — the bridge from the two
2026-07-05 research docs to spec edits and a delivery plan)
**Date:** 2026-07-05
**Owners:** andrew (drafted by Claude at andrew's request; reviewed by an
independent subagent against the current specs before commit — see §7)
**Applies:** [`20260705-unified-keyed-refresh.md`](20260705-unified-keyed-refresh.md)
(the collapse + patterns-as-functions proposal) and the accepted findings of
[`20260705-model-refresh-review.md`](20260705-model-refresh-review.md).
**Feeds:** spec edits under `docs/specs/` (via `/smelt:spec`) and a re-registered
sub-plan under `docs/plans/` (via `/smelt:plan`). Nothing in this note is
implementation; it fixes the decisions the spec diffs will encode.

---

## 0. What is being applied

1. Collapse `cumulative`, `latest_value`, and `accumulating_snapshot` into one
   refresh mode, **`refresh: keyed`**.
2. Keep `versioned` as a peer mode for now; adopt the grow-only-set /
   neighbour-local-presentation framing as its intended architecture.
3. Reintroduce the collapsed patterns as **proof-gated smelt functions**
   (`smelt.latest`, `smelt.once` now; `smelt.versions` later).
4. Land the supporting review findings the collapse depends on: the honest
   per-mode invariant statement, the column-family × source-shape matrix, the
   transactional run ledger, and the `materialized_view` shape correction.

Everything below is stated as **D-numbered decisions** (each: the question, the
resolution, the rationale, and where it lands), followed by the concrete spec
change list, the plan re-registration, and the deliberately-deferred list.

---

## 1. Surface decisions

### D1. The mode is named `keyed`

**Question.** `keyed` / `merged` / `state` / `entity`?
**Resolution.** `refresh: keyed`.
**Rationale.** The refresh enum's job (post-collapse) is to name the
output-addressing + freshness-owner axis; `keyed` is the framework's own word
for the addressing half (`model_maintenance.md` "key-addressed"). `merged`
names a mechanism, `state`/`entity` name neither axis.
**Lands in.** `models.md` §"Refresh axis"; `RefreshStrategy` enum.

The v1 enum becomes:

```
refresh: full | batched | keyed | versioned | materialized_view
```

(`versioned` stays per D14; it remains unbuilt and its sub-plan continues.)

### D2. `refresh: cumulative` is removed, not aliased

**Question.** Alias `cumulative → keyed` or remove?
**Resolution.** Remove. Declaring `refresh: cumulative` becomes a hard config
error whose message says exactly *"`refresh: cumulative` is now
`refresh: keyed`"*.
**Rationale.** Pre-1.0, no-back-compat doctrine; an alias is a second name for
one contract, which the peer-enum design exists to avoid. The error message is
the migration tool.
**Lands in.** `smelt-core/src/config.rs` deserializer; `models.md`.

### D3. Frontmatter and body shape of a keyed model — one surface change, named

**Resolution.** `refresh: keyed` is the entire opt-in (storage implied
`table`), same one-line posture as cumulative today. It forbids `timeseries:`
and `batched:` blocks on the model (output is keyed; consumption windowing is
derived from the source) and requires no config block. **The body must be an
aggregated `GROUP BY` query**: `unique_key` is derived from `GROUP BY`
(`KeyedRequiresGroupBy` when absent), and every non-key projection must
classify into a column family (D5).

This is carried over unchanged from `cumulative` and `accumulating_snapshot`,
but it is a **deliberate surface change for the latest-value pattern**:
`latest_value_models.md`'s bare-projection form
(`SELECT customer_id, tier, region FROM …`, key inferred, dedup imposed by the
mode) is **dropped**. The latest-value pattern is written as an aggregation —
`MAX_BY(attr, ordering) … GROUP BY key` under window-forward, or
`ANY_VALUE(attr) … GROUP BY key` under snapshot-reconcile (D5's plain-overwrite
family) — with `smelt.latest` / `smelt.current` as the intent-naming sugar
(D13).
**Rationale.** The bare-projection form is exactly the "mode adds semantics the
SQL does not have" defect of review §1.1: its full refresh is not one row per
key, so the equivalence invariant has no executable oracle. Requiring the
aggregated spelling makes the SQL its own oracle for every keyed model — the
property the whole collapse leans on (D15).
**Lands in.** `keyed_models.md` (new spec, D17); `latest_value_models.md`
retirement notes the surface change explicitly.

### D4. Diagnostic codes are renamed to the `Keyed*` family

**Resolution.** The mode-local diagnostics become `KeyedRequiresGroupBy`,
`KeyedForbidsTimeseries`, `KeyedForbidsBatched`, `KeyedUnknownCombiner`,
`KeyedGroupByContainsPartitionColumn`, `KeyedForbidsWindowFunctions`,
`KeyedForbidsNondeterministic`, `KeyedSqlNotParseable` (renaming
`CumulativeSqlNotParseable`, which exists in `diagnostics.md`'s catalogue),
and `KeyedMultipleDrivingSources`; plus the new `KeyedOnceWriteUnproven`
(replacing `AccumulatingSnapshotCorrectableMilestone`),
`KeyedRetractableContribution` (replacing
`AccumulatingSnapshotRetractableEnrichment`), and
`KeyedSnapshotSourceUnsupportedColumn` (D9).

Two codes do **not** carry over unchanged:

- **`CumulativeNoDrivingSource` is retired, not renamed.** Under D8 an
  unclocked model is a legitimate snapshot-reconcile posture, not an error;
  the cases that genuinely need a clock are refused per column by
  `KeyedSnapshotSourceUnsupportedColumn`. Interim (until the reconcile
  executor ships, §4 phase 5): an unclocked keyed model is refused fail-loud
  with a not-yet-supported diagnostic (`KeyedSnapshotPostureUnsupported`)
  naming the phase that delivers it — never silently treated as an error of
  the model.
- **`AccumulatingSnapshotUnboundedHorizon` is retired** (D6).

The `Cumulative*` and `AccumulatingSnapshot*` code families are otherwise
retired with their triggers preserved under the new names.
**Lands in.** `keyed_models.md` (owner), `diagnostics.md` (catalogue).

---

## 2. Semantics decisions

### D5. The v1 combiner catalogue: five column families

The classifier classifies each non-key projection into exactly one **column
family**; the family determines the cross-window combiner and every derived
posture. The v1 catalogue:

| Family | Per-key aggregators | Cross-window combiner | Idempotent (re-run safe) | Order-independent | Invertible | Postures admitted | Extra licence |
|---|---|---|---|---|---|---|---|
| **additive fold** | `SUM`, `COUNT`, `BIT_XOR` | `SUM` / `+` / `BIT_XOR` | no | yes | yes | window-forward only | run ledger enforcement (D7) |
| **extremal / lattice fold** | `MIN`, `MAX`, `BOOL_AND`, `BOOL_OR`, `BIT_AND`, `BIT_OR` | `LEAST`/`GREATEST`/same | yes | yes | no | window-forward only (D9) | — |
| **order-monotone overwrite** | `MAX_BY`, `MIN_BY` | max/min-by-ordering | yes | up to ordering-key ties (D11) | no | window-forward only (D9) | — |
| **once-write** | `COALESCE`-first-non-null | `COALESCE(target, delta)` | yes | yes (given the proof) | no | window-forward only (D9) | once-write provenance proof (key-derived or declared FD), unchanged from the accumulating-snapshot spec |
| **plain overwrite** *(new)* | `ANY_VALUE` | incoming wins | yes | n/a (one row per key per scan) | no | **snapshot-reconcile only** | — |

The plain-overwrite family is the one addition relative to the union of the
three retired allowlists, added to give the snapshot posture an honest
spelling (H1/H2 of the §7 review): over a snapshot whose key is the `GROUP BY`
key, each scan carries one row per key, so `ANY_VALUE(attr)` is deterministic
and its full refresh over the current snapshot *is* the current row — the
executable oracle D15 requires. Under window-forward it is order-dependent and
refused (`KeyedUnknownCombiner` names `MAX_BY` + ordering column as the fix).
Everything else is the exact union of the three modes' allowlists. Composite
expressions over aggregates remain rejected. The join-contribution
monotonicity check and its side conditions carry over unchanged — including
the requirement that a **re-scanned existence-flag dimension be declared
append-only** (`sources.md`), which licenses that enrichment shape exactly as
in `accumulating_snapshot.md` classifier check 4.

The model's derived posture is a fold over its columns' families, as **three
distinct properties** (previously conflated):

- **Re-run tolerance** (may an already-merged window be blindly re-merged over
  *unchanged* input?) ⇔ every column idempotent, i.e. no additive-fold column.
  Non-tolerant models rely on the ledger (D7) to refuse the re-run.
- **Out-of-order / parallel / sliced backfill** ⇔ every column
  order-independent — the extremal/lattice and once-write families qualify;
  the `MAX_BY` family does **not** (its order-independence holds only up to
  unprovable ordering-key ties, D11), so any model with an overwrite column
  executes windows sequentially in temporal order. Re-running a `MAX_BY`
  window over unchanged input remains safe (equal ordering keys ⇒ incumbent
  wins ⇒ no-op).
- **Reprocessing** (re-running a window whose *input changed*) is refused for
  **every** family when detected, exactly as cumulative today — an
  irreversible fold cannot un-see a removed contribution, and even the
  overwrite family cannot retract a superseded-by-nothing value. Detection is
  the ledger (window previously merged) plus `--auto` staleness (input
  changed); mitigation is `--full-refresh`. The group-rung subtract-then-add
  for all-invertible models stays deferred, unchanged.

**Rationale.** The families are exactly the discriminants
(`model_properties.md`) the ladder already reads. `AVG` remains out (rung-2
decomposed state is unchanged deferred work and now benefits every family at
once).
**Lands in.** `keyed_models.md` §Surface (the catalogue) and §Semantics (the
posture derivation).

### D6. No write-eligibility clamp; the *required-bounded-H* rule and declared-H write bound are dropped; derived-reach transforms survive

**Question.** Accumulating-snapshot required a bounded forward horizon `H`,
refused unbounded (`AccumulatingSnapshotUnboundedHorizon`), and clamped merge
*eligibility* to keys with event time `≥ run_start − H`; cumulative had no
clamp. Which survives?
**Resolution.** **No write-eligibility clamp.** `merge_into` reaches every key
its delta names, wherever it lives (cumulative's posture today). The
unbounded-horizon refusal is retired; a derivable forward reach `H` is still
computed and **reported** (`smelt explain`) but never gates admission and
never bounds which keys a run may touch. The hot-key cap (a guard against a
mis-derived horizon) is retired with the requirement that motivated it.

Two catalogued transforms read `H` and are **re-scoped, not dropped**:

- **Dimension-driven horizon-bounded MERGE** (built, `model_transforms.md`)
  survives with its licence narrowed to a **derived** `H` only — it is a
  scan/recompute bound (clamp the enrichment recompute to
  `[conv_ts − H, conv_ts]`), and a *derived* reach cannot under-cover by
  construction. The *declared-on-source* `H` licence for this transform is
  dropped: an under-declared lateness would silently truncate the recompute,
  the exact failure `model_maintenance.md`'s derived-horizon rule names.
  Where `H` is not derivable from the SQL, the transform is simply not
  licensed and the enrichment evaluates against the fact via the ordinary
  widened scan. (`source_lateness` keeps its existing role as a scan-widening
  term; it no longer feeds any write- or recompute-truncating bound.)
- **Horizon settled-delay / tail-rewrite** (unbuilt) remains catalogued
  unchanged — it is batched-side forward-reach machinery and already tracks
  the *derived* horizon.

**Rationale.** The eligibility clamp was the one place the family silently
dropped *scanned* inputs (review §3.2) and it weakened the invariant per-key.
It was never needed for correctness — merge work is proportional to delta
size. What it bought (settled-key reasoning, hot-state GC, a work bound) is
deferred optimisation, not v1 semantics; if a clamp or GC is ever introduced
it must come as a package with late-fact accounting (review §5.4).
**Consequence.** The unified invariant statement is clean: a keyed run merges
*every* delta row it scans. No completeness carve-out needed.
**Lands in.** `keyed_models.md`; `model_maintenance.md` (the horizon section
keeps the scan-side derivation; write-clamp language is scoped to batched);
`model_transforms.md` (both rows re-scoped as above).

### D7. The transactional run ledger: maintained for every window-forward keyed model; correctness-critical for additive folds

**Question.** Review §4.1: reprocessing detection and crash recovery for
`SUM`/`COUNT` need state that today is opt-in or absent.
**Resolution.** Every **window-forward** keyed model maintains a **per-model
ledger** — a small backend table recording each merged window — written **in
the same backend transaction** as that window's `merge_into`. Its enforcement
role differs by posture:

- **Additive-fold models** (not re-run tolerant): a run covering a ledgered
  window is **refused** exactly (not best-effort); crash resume merges only
  unledgered windows. This is correctness-critical.
- **Re-run-tolerant models**: a ledgered window may be re-merged (no-op on
  unchanged input); the ledger's role is reprocessing detection (D5) and
  `--auto` bookkeeping, not refusal.

Snapshot-reconcile models keep no ledger (each run is a self-contained
reconciliation). The ledger lives in the target's backend/schema alongside the
model (naming and layout settled in the spec phase, coordinated with
`run_state.md` — which remains the opt-in *observability* surface; the ledger
is a *correctness* structure and is not optional where required).

This decision **supersedes the blanket "smelt does not manage computational
state" constraint** for the keyed mode: `batched_models.md` Constraint 4 and
its §Design rationale ("owning a watermark store … duplicates engine state and
opens a sync-correctness window") are re-worded to scope the doctrine to
batched and to name the keyed ledger as the deliberate exception — the ledger
is backend-resident and transactional-with-the-merge, so the sync-correctness
window the rationale feared does not exist. Relatedly, review §4.2 is decided
here: a backend may only select physical strategies that preserve the declared
mode's invariants, which makes batched's `Append` strategy unreachable until
it is gated on ledger-verified unwritten windows.
**Lands in.** `keyed_models.md` §Semantics; `model_transforms.md` (new
catalogued row: "transactional merge ledger — licensed by: window-forward
keyed consumption; enforcement required by any non-idempotent combiner");
`batched_models.md` (Constraint 4 rewording + strategy-choice constraint);
`run_state.md` (relationship note); backend trait gains a
merge-with-ledger transactional entry point.

### D8. Run shape is derived from the driving source: window-forward vs snapshot-reconcile

**Resolution.** Two postures, derived, never declared:

- **Window-forward** (driving source carries `timeseries:`): identical to
  cumulative's CLI and step loop today — `--event-time-start/-end` required,
  windows stepped in temporal order; out-of-order / parallel / sliced backfill
  is additionally admitted only for order-independent models (D5). Exactly one
  clocked driving source, resolved by the shared anchor proof, unchanged
  (D12).
- **Snapshot-reconcile** (no clocked source): the run re-scans the source
  whole and upserts per key — admitted column families per D9 (in v1:
  plain overwrite only). `--event-time` flags are a **hard error** ("model has
  no clocked driving source; run without event-time flags"). `--auto` treats
  the model as always-stale. **Key deletion:** a key absent from the incoming
  scan is **retained** (v1; tombstones/eviction deferred) — this is a named,
  documented divergence from the current-snapshot oracle, carried into D15's
  invariant statement rather than left implicit.

Until the reconcile executor ships (§4 phase 5), an unclocked keyed model is
refused with the fail-loud not-yet diagnostic named in D4.
**Lands in.** `keyed_models.md` §Semantics + §CLI; resolves the
"snapshot-diff mechanics under-specified" open question in `models.md` for the
keyed mode's scope.

### D9. The column-family × source-shape admission matrix

**Question.** Which column families are admissible under which posture? (The
event-vs-state gap, review §2.1 — including the two subtle cases this
application work surfaced: even *idempotent* folds are wrong over snapshots,
and so is `MAX_BY`.)
**Resolution.**

| Column family | clocked source (window-forward) | unclocked mutable snapshot (reconcile) |
|---|---|---|
| additive fold | ✓ (ledger, D7) | ✗ — refolding state double-counts |
| extremal / lattice fold | ✓ | ✗ in v1 — observer semantics, see below |
| order-monotone overwrite (`MAX_BY`) | ✓ | ✗ in v1 — observer semantics, see below |
| once-write | ✓ (provenance proof) | ✗ in v1 — observer semantics, see below |
| plain overwrite (`ANY_VALUE`) | ✗ — order-dependent under events | ✓ |

The three "✗ in v1" snapshot cells are not double-count hazards — those
families are safe to re-merge — they are **equivalence** failures: `MIN(price)`
folded over successive snapshots computes *min ever observed*;
`MAX_BY(attr, updated_at)` retains a stale incumbent forever if a mutation
regresses the ordering value (nothing in `sources.md`'s `mutable` profile
guarantees ordering monotonicity under in-place update); `COALESCE`-once-write
captures *first observed*, unrecoverable from the current snapshot. In every
case a full refresh of the SQL over the current snapshot disagrees with the
maintained state. That is observer semantics (review §1.2) — potentially
useful, but a different contract, and admitting it silently would put two
contracts behind one mode. v1 refuses these cells with
`KeyedSnapshotSourceUnsupportedColumn`, whose message names the
observer-contract design as the future opt-in path.
**Rationale.** This makes the review's event-vs-state distinction an enforced,
per-column fact rather than prose, and it keeps `refresh: keyed`'s invariant
executable in both postures: under window-forward the oracle is the SQL over
the replayable input; under snapshot-reconcile the one admitted family (plain
overwrite) makes the stored row equal the SQL over the *current* snapshot for
every key present in it (retained deleted keys carved out per D8/D15).
**Lands in.** `keyed_models.md` §Semantics (normative matrix); `models.md`
input-consumption axis gains a sentence noting per-column family compatibility
is mode-checked.

### D10. Mutation-profile interaction stays as today, tightened later

**Resolution.** v1 admission reads clock presence (as cumulative does):
clocked ⇒ window-forward; unclocked ⇒ snapshot-reconcile. A declared
`mutation_profile: mutable` on a *clocked* source does not change admission in
v1 (reprocessing detection via the ledger + `--auto` staleness is the answer
to in-place changes); `change_feed` continues to flow through input-delta
discovery as built. Tightening (e.g. requiring `append_only`/`change_feed` for
additive folds) is deferred until the mutation-profile declaration changes
verdicts generally (`models.md` Known Divergences already tracks that gap).

### D11. `MAX_BY` ordering ties: incumbent-wins, documented, sequential

**Question.** `latest_value`'s open tie-break question, inherited by the
overwrite family.
**Resolution.** The pairwise combiner is: delta wins iff
`delta.ordering > target.ordering` (strict); on equality the **incumbent
(target) wins**. This is deterministic given processing history but **not
order-independent when ties occur across windows**; since the classifier
cannot prove ordering-key uniqueness, the consequence is drawn in D5: models
with an overwrite column are never admitted to out-of-order or parallel
execution — sequential temporal order makes "deterministic given processing
history" a real guarantee rather than a race. The documented recommendation
remains a composite, unique ordering expression (e.g.
`(updated_at, source_seq)`), which restores order-independence in practice
without smelt pretending to prove it.

The **last-processed combiner is dropped entirely.** Under window-forward, an
overwrite column requires an ordering column (no ordering column ⇒ the
projection does not classify ⇒ `KeyedUnknownCombiner`). Under
snapshot-reconcile the equivalent need is served by the plain-overwrite family
(D5), where "last scan wins" is well-defined because each scan carries at most
one row per key.
**Rationale.** Every alternative either fakes a guarantee (no static proof of
uniqueness exists) or refuses the whole family for a corner case. Naming the
exact boundary of order-independence — and pinning execution to sequential
order wherever ties could bite — is the honest option.
**Lands in.** `keyed_models.md`; retires `latest_value_models.md`'s two open
questions (ties; definition-of-latest).

### D12. Multi-driving-source stays exactly-one in the collapse; union admission is the first follow-on

**Resolution.** The collapse itself does not change anchor cardinality:
exactly one clocked source under window-forward,
`KeyedMultipleDrivingSources` as today. Admitting a `UNION ALL` of same-clock,
same-granularity sources as *one* anchor (review §6.1) is registered as the
first post-collapse enhancement — it now needs solving once instead of four
times, which is the collapse paying for itself, but bundling it would grow the
diff for no structural reason.

### D13. Pattern functions: `smelt.latest`, `smelt.once`, `smelt.current` ship with the mode

**Resolution.** Three aggregate-position pattern functions ship in the same
delivery as the collapsed classifier, defined with the existing
`smelt.define` machinery (expression-position, no new fragment sorts needed):

- `smelt.latest(value, ordering)` → expands to `MAX_BY(value, ordering)`.
- `smelt.once(value)` → expands to the once-write `COALESCE`-first-non-null
  canonical spelling (exact body fixed in the spec phase alongside the
  combiner catalogue's canonical forms).
- `smelt.current(value)` → expands to `ANY_VALUE(value)` (the plain-overwrite
  family; snapshot-reconcile posture).

All are ordinary transparent functions: the classifier sees the expansion and
gates it with the same proofs as hand-written SQL (`smelt.once` still requires
the once-write provenance licence; `smelt.current` is still refused under
window-forward). Whether they ship as project-template files or as built-in
`smelt.`-namespace functions is settled in the spec phase; the *default* is
built-in (they are the vocabulary the docs teach), provided the built-in
function registry can host transparent bodies — if it cannot yet, they ship as
documented template snippets first.
**Rationale.** They are the intent-naming layer that makes the collapse
teachable, and they cost nothing new.
**Lands in.** `functions.md` (if built-in) or docs-site recipes; the keyed
user-doc page teaches the patterns through them.

### D14. `versioned` stays a peer; its spec adopts the grow-only framing; `smelt.versions` waits

**Resolution.** As argued in the proposal doc: `versioned`'s output shape
passes the litmus test, so it keeps its enum value and its (unbuilt) sub-plan.
Its spec's §Design gains the grow-only-event-set + neighbour-local-presentation
architecture as the intended implementation, which is also the recorded answer
to the late-corrections open question. `smelt.versions` (the table-function
form) is explicitly sequenced behind `TableExpr` FROM-invocation and struct
row-polymorphism; its landing is the trigger to *revisit* folding `versioned`
into `keyed`, not a commitment to do so.

### D15. The invariant restatement lands with the collapse

**Resolution.** `model_maintenance.md`'s invariant section is edited to:
(a) state the invariant over an abstract *processed-input set* with the
partition-set form as the clocked specialisation (fixing the type error for
unclocked sources); (b) add the replayability split — full equivalence for
replayable inputs, and an explicit statement that v1 admits **only**
combinations whose oracle is executable (which D9's matrix guarantees), naming
the observer/prefix-consistency contract as the designed-but-unshipped third
column for the refused cells; (c) name the **two carve-outs** the executable
oracles carry: snapshot-reconcile equivalence is per-key for keys present in
the current snapshot (absent keys are retained, D8), and overwrite-column
equivalence is up to ordering-key ties (D11). The `latest_value`-style "mode
adds semantics" problem disappears structurally: every admitted keyed model's
oracle is its own SQL (D3). `versioned` gets an interim honest sentence (its
oracle is the canonical change-set SQL of the grow-only framing) pending its
build.
**Lands in.** `model_maintenance.md`; `models.md` refresh-axis table.

### D16. `materialized_view` shape wording is corrected in the same pass

**Resolution.** Output shape changes from "keyed" to **engine-defined** in
`materialized_view.md` and the `models.md` refresh table (cheap, purely
textual, and blocking-free). Allowing a consumer-facing `timeseries:`
declaration on it (so downstream pushdown works against partitioned engine
views) is *accepted as direction* but deferred — it needs pushdown wiring and
its own small design (which outputs may carry a consumer clock, review §6.4).

---

## 3. Spec change list (the concrete edit set)

| File | Change |
|---|---|
| **`keyed_models.md`** (new) | The mode spec: composition table; frontmatter + body surface (D3); combiner catalogue + three-property posture derivation (D5); no-clamp semantics (D6); ledger (D7); run shapes + CLI + retained-keys rule (D8); admission matrix (D9); tie-break rule (D11); diagnostics table (D4); Design section carrying forward the load-bearing rationale of the three retired specs (derive-from-SQL, no safety_overrides, one driver); Known Divergences carrying the retired specs' still-open items (§5). |
| **`cumulative_aggregate.md`** | Retired; its classifier/execution content is the seed of `keyed_models.md` (it is the only built mode); its §Design rationale and Known Divergences are carried forward, not lost (deletion preferred over a tombstone; specs are timeless). |
| **`latest_value_models.md`**, **`accumulating_snapshot.md`** | Retired (both unbuilt); content merged per D5/D9/D11; the latest-value surface change named in D3; open questions resolved here or carried into `keyed_models.md`/§5. |
| **`versioned_models.md`** | Stays; §Design gains the grow-only-set framing (D14); late-corrections open question re-pointed at it. |
| **`models.md`** | Refresh-axis table rewritten to the D1 enum; constraint-violation table updated (`keyed` rows replace three mode rows); litmus rule gains the fourth clause ("names a reusable combiner/table shape without changing contract or shape → a function, not a mode"); input-consumption section cross-references the D9 matrix; Known Divergences updated — of today's three non-parsing values, `latest_value`/`accumulating_snapshot` cease to exist, `versioned` still does not parse, and `cumulative` (which parses today) is renamed per D2. |
| **`model_maintenance.md`** | Invariant restatement + carve-outs (D15); horizon section re-scoped (D6); composition-contract examples re-pointed at `keyed_models.md`. |
| **`model_transforms.md`** | New row: transactional merge ledger (D7); dimension-driven horizon MERGE licence narrowed to derived `H` (D6); settled-delay/tail-rewrite row confirmed derived-only (D6); the windowed-keyed-maintenance driver's consumer list updated (one keyed mode + versioned prospective); eviction/settled-key GC row re-tagged deferred-with-late-fact-accounting (D6). |
| **`model_properties.md`** | No structural change; consumer notes updated (the discriminants now feed one keyed classifier); once-write proof text re-pointed. |
| **`batched_models.md`** | Constraint 4 ("smelt does not manage computational state") re-scoped to batched with the keyed ledger named as the deliberate exception; backend strategy choice constrained to invariant-preserving strategies (`Append` unreachable until ledger-gated) (D7). |
| **`run_state.md`** | Relationship note: run-state intervals = opt-in observability; the keyed ledger = required correctness structure; neither substitutes for the other (D7). |
| **`materialized_view.md`** | Shape wording (D16). |
| **`multi_backend.md`** | "Smelt-driven keyed modes (`cumulative`, `versioned`, `latest_value`)" enumeration and the `supports_retraction` cross-reference re-pointed at `keyed`/`versioned` and `keyed_models.md`. |
| **`cli.md`**, **`data_catalog.md`** | The pinned JSON enum `"refresh": "full" \| "cumulative"` updated to the D1 enum. |
| **`smelt_yml.md`** | The normative `refresh: cumulative` mention updated (D2). |
| **`diagnostics.md`** | Code family rename + retirements (D4). |
| **`functions.md`** / docs-site | `smelt.latest` / `smelt.once` / `smelt.current` (D13). |
| **docs-site** | One "keyed models" guide replacing the three planned mode pages; pattern-recipe sections (running totals; latest-value; milestones/funnel; mixed lifecycle table — the collapse's headline example). |

## 4. Plan re-registration

- **Deregister** `20260704-model-updates-l4-latest-value.md` and
  `20260704-accumulating-snapshot.md` as independent verticals; their still-relevant
  local content (once-write proof consumer wiring, overwrite execution) moves into
  the new sub-plan.
- **Re-cut** `20260704-model-updates-l4-cumulative.md` as
  `2026xxxx-keyed-collapse.md` with phases roughly: (1) spec edits per §3;
  (2) enum + rename + config errors (D1/D2/D4), including the interim
  unclocked-model refusal; (3) classifier union + three-property posture
  derivation + admission matrix (D5/D9, window-forward only); (4)
  transactional ledger + refuse/resume semantics (D7); (5) snapshot-reconcile
  executor for the plain-overwrite family (D8/D9), retiring the interim
  refusal; (6) pattern functions + docs (D13). Phases 1–4 change or generalise
  the shipped cumulative path and must keep its equivalence harness green
  throughout; phase 5 is the first genuinely new executor.
- **`l4-versioned`** proceeds, updated to compose against `keyed_models.md`'s
  driver text and to adopt D14's framing.
- **`l4-materialized-view`** absorbs D16's wording fix.
- The master plan's registry is updated accordingly (conservative roll-up
  rules unchanged).

## 5. Deliberately deferred (recorded, not lost)

| Item | Why deferred | Trigger to revisit |
|---|---|---|
| Union-of-same-clock-streams as one anchor (D12) | scope control on the collapse diff | first post-collapse enhancement |
| Observer/prefix-consistency contract (folds, `MAX_BY`, and once-write over snapshots — D9's ✗ cells) | a genuinely different contract; needs its own invariant text | a real user hits a D9 ✗-cell with a legitimate min-ever/first-observed need |
| Per-key targeted recompute (review §5.2) | new transform; not needed for the collapse | reprocessing/erasure demand on non-invertible columns |
| Subtract-then-add reprocessing (group rung) | unchanged deferred status | as before |
| Settled-key GC / any write-eligibility clamp + late-fact accounting | D6 removed the need; must land as a package | keyed state size becomes a real operational problem |
| **Self-referential keyed** (`state += delta − decay`, the model joining its own target) | carried from `cumulative_aggregate.md` Known Divergences; still rejected in v1 — under D4/D8 the rejection re-anchors on the self-reference/anchor analysis rather than the retired no-driving-source code | an explicit input/state-distinction design |
| **`--auto` staleness fidelity** ("exactly the changed windows" for all-invertible models) | carried from cumulative; needs delta history | group-rung work |
| **Day/week granularity restriction** of the shared windowed driver | carried from cumulative; a property of the driver, inherited by all its consumers | widening the driver's step arithmetic |
| **Run-pinning alignment** (`NOW`/`CURRENT_*` compile-time pinning for keyed, as batched has) | carried from accumulating_snapshot; conservative reject holds | adopting the pinning transform beyond batched |
| `smelt.versions` + folding `versioned` into `keyed` | blocked on `TableExpr` invocation + struct row polymorphism | those features landing (D14) |
| Consumer-facing `timeseries:` on `materialized_view` / versioned outputs | needs pushdown wiring + small design | first partitioned engine-maintained view in anger |
| Mutation-profile-driven tightening of D9/D10 | mutation profile barely changes verdicts today | the declaration becoming verdict-bearing generally |
| Self-emitted change feeds (review §5.3) | architecture-level; orthogonal to the collapse | its own research note |
| **Property-typed function signatures** (§8) | post-v1 type-system extension; needs the pattern functions to exist first | `smelt.versions` design; or the first third-party pattern function |

## 6. Risks

- **The ledger (D7) is the only decision that adds a new runtime structure.**
  It is also the one that changes shipped `cumulative` behaviour (blind re-run
  of a `SUM` window goes from "sometimes undetected double-count" to
  "refused"). That is a strictly-safer change, but the equivalence harness and
  the CLI integration tests must grow ledger-path coverage before the rename
  phase ships, or the collapse inherits an untested correctness structure.
- **D9 narrows nothing that ships today** (cumulative is window-forward only)
  but refuses snapshot-source models that `latest_value`'s spec *promised* —
  including, per H2/D11, the `MAX_BY`-over-snapshot form that spec implied
  would work. The refusal is v1-honest (the executor doesn't exist anyway);
  the D8/D9 snapshot-reconcile phase keeps the promise through the
  plain-overwrite family, and the observer-contract row in §5 records the
  intentionally-unserved remainder.
- **D3 is a real surface break** for anyone modelling on the unshipped
  latest_value spec: the bare-projection form is gone. Named here so the spec
  retirement says it out loud rather than burying it in a diff.
- **Teachability** remains the collapse's soft cost; the docs-site recipe page
  and `smelt explain`'s per-column family readout are the mitigations and
  should be treated as part of the delivery, not follow-up polish.

## 7. Review record

This document was reviewed pre-commit by an independent subagent against the
ten current specs, the two source research docs, the plans directory, and
`RefreshStrategy` in `crates/smelt-core/src/config.rs`. Material findings —
all folded in above: the latest_value bare-projection surface contradiction
(now D3's named surface change + D5's plain-overwrite family); the
`MAX_BY`-over-snapshot observer failure (now a ✗ cell in D9); the
re-run/reprocessing conflation and the posture-property split (D5); the two
horizon-dependent transforms D6 originally left unaddressed; the ledger's
collision with `batched_models.md`'s state doctrine and the `Append` decision
(D7); retired-vs-renamed diagnostics including `CumulativeNoDrivingSource` and
`CumulativeSqlNotParseable` (D4); four missed spec touchpoints (`cli.md`,
`data_catalog.md`, `smelt_yml.md`, `multi_backend.md` — §3); the carried open
questions from the retired specs (§5); the parallel-execution/tie-break
contradiction (D5/D11); the retained-keys oracle carve-out (D8/D15); and the
`models.md` parse-set arithmetic in §3.

---

## 8. Extension for later: property-typed function signatures (express the proofs in the type system)

*(Appended after the review; a direction to keep, not a decision. Recorded
here because it changes how the pattern functions of D13/D14 should
eventually state their requirements.)*

**The idea.** The proof layer already produces named verdicts — a column is
`Monotone`, a source is append-only, a value is per-key-constant (FD), a
combiner is a monoid / idempotent / invertible. Today those verdicts are
consumed *globally*: the mode classifier runs over the fully expanded model
and refuses the whole model when a proof fails, and the error surfaces at the
model level, after expansion, far from the argument that caused it. The
extension: give the verdicts a **type-level vocabulary** and let function
signatures *demand* them as argument constraints, so the same failure becomes
a local, argument-positioned type error — in the LSP, at the call site, before
any classifier runs.

Sketch, using `smelt.versions` (the motivating case — today its failure mode
would be a whole-model keyed-classifier refusal after expansion):

```
smelt.define versions(
    input       TableExpr<{..r}> & AppendOnly & Clocked,
    key         Expr<K>,
    event_time  Expr<Monotone<Timestamp>>
) -> TableExpr<{..r, valid_from Timestamp, valid_to Timestamp?, is_current Bool}>
```

Passing a mutable snapshot source produces
*"argument `input`: expected an append-only clocked table, got
`smelt.orders_snapshot` (`mutation_profile: mutable`)"* at the argument span —
instead of a post-expansion `Keyed*` refusal pointing at generated SQL.
Similarly `smelt.latest(value, ordering Expr<Ordered>)` already almost states
its requirement; the extension would let it (or a stricter variant) demand
`Expr<Monotone<…>>` where the maintenance proof needs it.

**Why this is more than ergonomics.**

1. **It is the missing surface for proof modularity.** The unified-keyed doc
   (§4.3) proposed deriving a pattern function's verdict once at the
   definition and composing at call sites. Signature constraints are what make
   that sound rather than cached: the body is checked once *under the
   assumption* that its parameters satisfy the constraints; each call site
   discharges the assumptions for its arguments. Assume/guarantee at the
   function boundary — the standard refinement-type discharge split — instead
   of re-deriving over the expanded tree.
2. **The escape hatches get a principled shape.** A declared widening
   (`assert_monotonic`, declared FD) becomes an explicit, visible *cast* to
   the property type at a specific expression — same only-widen rule,
   expressed where the reader can see it, instead of a frontmatter key acting
   at a distance.
3. **Third-party pattern functions get a contract language.** The
   proof-gated-extensibility story (a user pattern is maintainable iff its
   expansion classifies) currently gives library authors no way to *state*
   requirements; constraints let them, and let the checker reject misuse at
   the boundary rather than deep inside an expansion.
4. **Precedent says the load-bearing case works.** Flink SQL makes event-time
   a column-level *type* attribute (a rowtime attribute) — precisely the
   `Monotone`/clocked property — and it is the single property doing the most
   work in this family.

**Why it plausibly fits the existing machinery.** The constraint slot already
exists in narrow form: `functions.md` permits `<T: Constraint>` (e.g.
`Ordered`) on built-ins and `smelt.extern`; the extension is (a) a closed
vocabulary of *property* constraints drawn from the existing proof inventory
(`Monotone`, `Clocked`, `AppendOnly`, `PerKeyConstant<K>`, perhaps
`UniqueOrdering`), and (b) admitting them on `smelt.define` parameters. The
checker discharges each constraint by calling the *existing* analysis
(the monotonicity trace discharges `Monotone`; the mutation profile discharges
`AppendOnly`; the FD verdict discharges `PerKeyConstant`) — no new proof
engine, just routing existing verdicts through the type checker at argument
boundaries. Gradual-typing tiers apply naturally: an undischargeable
constraint on a Tier-1 call degrades to the global classifier path (or
fails closed), consistent with `gradual_typing.md`.

**Honest limits.**

- **Fragment-shaped properties only.** Whole-model *flow* properties —
  nondeterminism taint, event-time outer-visibility, run/partition granularity
  alignment, the model-wide horizon — are not argument-local and stay with the
  global classifier. The type layer narrows the classifier's job; it does not
  replace it.
- **Propagation is the real cost.** Each property needs propagation rules
  through the operators between the source declaration and the argument
  (which is exactly what the monotonicity trace already is, for one
  property). The vocabulary must stay closed and small, or the propagation
  matrix sprawls.
- **Sequencing.** This is post-v1-collapse work: it needs the pattern
  functions to exist (D13), and its first forcing customer is
  `smelt.versions` (D14), whose signature is the natural pilot. Landing the
  vocabulary before a second property-demanding function exists would be
  speculative; landing it with `smelt.versions` is the right moment.

**If adopted**, the spec homes are: `types.md` (the property-constraint
vocabulary), `functions.md` (constraints on `smelt.define` parameters + the
assume/guarantee checking rule), `model_properties.md` (each property names
the proof that discharges it), and `gradual_typing.md` (tier behaviour).
Registered in §5 as deferred with `smelt.versions` as the trigger.
