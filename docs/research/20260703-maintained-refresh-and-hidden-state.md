# Maintained refresh and hidden state

**Status:** research (positioning / decision-framing, not normative)
**Date started:** 2026-07-03
**Owners:** andrew
**Related:**
- Spec: [`docs/specs/cumulative_aggregate.md`](../specs/cumulative_aggregate.md) — the `{direct, smelt-maintained}` corner of the design space this doc opens up.
- Spec: [`docs/specs/incremental_models.md`](../specs/incremental_models.md) — the window-forward sibling; the *other* maintenance camp.
- Spec: [`docs/specs/multi_backend.md`](../specs/multi_backend.md) — the capability matrix this doc extends with `supports_native_ivm` / `supports_retraction`.
- Spec: [`docs/specs/models.md`](../specs/models.md) §"Refresh axis" — where `cumulative`, and any generalization of it, lives.
- Research: [`docs/research/20260701-expanding-incremental-eligibility.md`](20260701-expanding-incremental-eligibility.md) — the audit of the window-forward camp; §7.1 (two camps), §7.2 (external validation), §7.3 (monotone/linear theory), §11.3 (ordered / self-referential).
- Research: [`docs/research/20260522-cumulative-as-its-own-rule.md`](20260522-cumulative-as-its-own-rule.md) — why cumulative is its own rule; the sibling-rule sketches (`scd2`, `latest_value`, `accumulating_snapshot`).

---

## Why this document exists

smelt has two ways to keep a stored model up to date across runs, and they are not
variations of one mechanism — they are the two camps the whole field splits into
([eligibility research §7.1](20260701-expanding-incremental-eligibility.md)):

- **Window-forward over a monotone event-time** — smelt's `incremental`. Read the
  next time window, assume the source is append-only so earlier windows are
  settled, `DELETE+INSERT` that window's partitions. Simple, needs no
  change-tracking metadata, but pays the monotonicity-primitive price
  ([`incremental_models.md`](../specs/incremental_models.md) §"Event-time
  monotonicity trace") and only covers the monotone/linear operator slice.
- **Change-tracking / delta-diffing** — Databricks Enzyme, Snowflake Dynamic
  Tables, BigQuery MVs, Feldera/DBSP (and, beyond the warehouses, Materialize
  and Flink). Detect *which rows changed* and propagate the
  delta into maintained state. No monotone column needed; covers joins, `DISTINCT`,
  non-additive aggregates — far more of SQL — but needs a **stateful runtime that
  keeps maintenance state the user never selects**.

smelt sits squarely in the first camp for `incremental`. Its **one foothold in the
second camp is `refresh: cumulative`**: cumulative does not rebuild a window, it
keeps target state and merges per-partition deltas into it. That is a *maintained
view*, not a window rebuild.

This document asks whether a single **maintained-relation** abstraction —
cumulative generalized along two axes — can give smelt *both* camps' trade-offs in
one system: emulated on a plain engine like DuckDB, and **delegated to the engine's
native incremental-view maintenance (IVM) on a platform like Databricks**. It is a
positioning doc. It defines the design space, lands one load-bearing algebraic
boundary, recommends an ontology (§7), and leaves the normative work to a future
spec. Nothing here changes behaviour.

The governing observation, developed below: **what the "smart engines" do is keep
hidden maintenance state behind a clean logical relation, and smelt can do the same
thing itself with a `(state table + presentation view)` pair.** Native IVM and
smelt-emulated maintenance are then the *same logical object with two maintainers*,
which is exactly smelt's stated logical-spec / physical-execution separation.

---

## Part 1 — The two axes

Cumulative today is a single point. Two orthogonal choices open it into a space.

### Axis I — state representation: *direct* vs *hidden*

- **Direct state.** The value smelt stores *is* the value the user selects. Merging
  two partitions' `SUM` gives a `SUM`; the stored column is the answer. This is
  today's cumulative ([`cumulative_aggregate.md`](../specs/cumulative_aggregate.md)
  §"Aggregator allowlist"): the combiner's output is directly presentable, so no
  indirection is needed.
- **Hidden (decomposed) state.** smelt stores an *intermediate* the combiner is
  closed over, plus a **presentation map** from that intermediate to the
  user-facing value. A mean is stored as `(sum, count)` and presented as
  `sum / count`. The user never selects `sum` or `count`; they select `mean`.

Hidden state is exactly the trick the delta engines use — Enzyme's row-tracking,
Dynamic Tables' change streams, Materialize's arrangements are all maintenance
state the user's `SELECT` never sees. The insight of this doc is that **smelt can
keep that state itself**, in an ordinary table, and expose the user-facing value
through a view:

```
state table   device_id, user_id, _sum_amount, _count_amount   ← smelt merges into this
presentation  CREATE VIEW … SELECT device_id, user_id,
              _sum_amount / _count_amount AS avg_amount FROM <state table>   ← user selects this
```

### Axis II — maintainer: *smelt-driven* vs *engine-native IVM*

- **smelt-driven.** smelt emits the per-partition delta `SELECT` and the
  `merge_into` loop (today's cumulative execution model,
  [`cumulative_aggregate.md`](../specs/cumulative_aggregate.md) §"Execution
  model"). For hidden state it additionally emits the presentation view. This works
  on **any** backend that has `merge_into` and views — DuckDB, plain Spark.
- **Engine-native IVM.** smelt emits a native maintained object — a Databricks
  materialized view / Enzyme-managed table, a Snowflake Dynamic Table — and lets
  the engine keep the hidden state *and* the presentation. smelt supplies the
  logical specification; the engine's differential runtime does the maintenance.
  Only available on backends whose capability matrix advertises it. Note
  `models.md` already carries a `materialized_view` mode on the **storage**
  (materialization) axis, described as a "backend-managed persistent view" — the
  natural physical home for this maintainer.

### The matrix

|  | **smelt-driven** | **engine-native IVM** |
|---|---|---|
| **direct state** | `cumulative` *today* — `SUM/COUNT/MIN/MAX/BOOL_*/BIT_*` | native MV over an additive aggregate (redundant with smelt-driven, but free) |
| **hidden, append-only (monoid)** | `AVG`, variance, HLL-approx-distinct via `(state table + view)` | native MV; engine keeps the sketch |
| **hidden, retraction (group)** | reversible aggregates + delete/reprocess via a stored, invertible delta | the full delta camp: joins, `DISTINCT`, non-additive aggregates — anything the engine can maintain |

Today's `cumulative` is the top-left corner. The whole rest of the matrix is
reachable, and — the key finding of Part 4 — the reachability boundary is
**algebraic**, not backend-specific.

---

## Part 2 — The unifying logical contract

All four corners uphold **one** contract, and it is cumulative's contract
generalized:

> **Maintained-relation equivalence.** The *user-visible* value of the model
> equals what a full refresh would compute over the set of inputs processed so far.

Cumulative states this as cross-partition equivalence
([`cumulative_aggregate.md`](../specs/cumulative_aggregate.md)
§"Cross-partition equivalence"): the end state after merging a *set* of source
partitions equals a full refresh restricted to that set, independent of merge
order. Generalizing changes only two words:

- "end state" → "**user-visible** value" — because with hidden state the stored
  columns are no longer the answer; the *view over them* is. The contract is
  asserted against what the user selects, not what smelt stores.
- The contract becomes **backend-uniform**. Whether smelt keeps `(sum, count)` in a
  DuckDB table or Databricks maintains the mean natively, the user sees the same
  logical relation with the same equivalence guarantee. Hidden state and the
  maintainer are *implementation detail beneath the contract*.

This reframes the whole design space as **one contract, four physical
realizations** — the textbook logical/physical split smelt is built on
(stated most crisply in [`multi_backend.md`](../specs/multi_backend.md) §Design;
`architecture.md` gives the compiler pipeline that realizes it). It also means the contract does *not* mention monotone
event-time: the maintained-relation camp sidesteps camp-1's monotonicity price
entirely (see §5), because it tracks *what changed* rather than *assuming what is
settled*.

---

## Part 3 — What hidden state collapses

Three entries currently sitting in cumulative's §"Known Divergences / Open
Questions" are **the same mechanism** — hidden state — seen three times:

| Cumulative Known Divergence | What it needs | = hidden state? |
|---|---|---|
| **`AVG` rewrite** ("classifier refuses `AVG`; a future plan may rewrite to `SUM/COUNT`") | store `(sum, count)`, present `sum/count` | **yes** — decomposed monoid + presentation map |
| **Reprocessing via delta history** ("store per-partition deltas for reversible aggregators, enabling subtract-then-add") | store enough to *invert* a partition's contribution | **yes** — the stored delta *is* hidden group state |
| **`--auto` staleness fidelity** ("exactly the stale partitions … needs the delta-history mechanism") | the same per-partition delta history | **yes** — same store |

Three deferred features, one enabling abstraction. That is strong evidence hidden
state is a real organizing idea and not a speculative flourish: the cumulative spec
already *reached for it three times* without naming it. Naming it once, as an axis,
subsumes all three.

It also connects to the eligibility audit. `MIN/MAX` are supported append-only by
Snowflake but require retraction state in Flink
([eligibility §7.2 obs. 2](20260701-expanding-incremental-eligibility.md)); the
monoid/group frame of Part 4 *names why* (they are a monoid but not a group). And
the ordered / self-referential slice
([eligibility §11.3](20260701-expanding-incremental-eligibility.md)) is exactly the
maintained-relation camp: cumulative and self-referential incremental are the two
shapes that read computed cross-window state, i.e. that keep maintenance state.

---

## Part 4 — The algebra is the eligibility boundary

The reason to put algebra in a positioning doc: it draws the *exact* line between
what each corner of the matrix can express, with no hand-waving. Every combiner in
play is an operation on stored state; its algebraic structure decides what is
maintainable.

### 4.1 Monoid = append-only maintainable

A per-key aggregate is maintainable by merging partition deltas iff its combiner
forms a **commutative monoid**: an associative, commutative binary operation `⊕`
with an identity. Associativity + commutativity are precisely
cumulative's order-independence contract; identity is the empty partition.

- **Direct monoid** (stored value presentable as-is): `SUM (+, 0)`, `COUNT (+, 0)`,
  `MIN (min, +∞)`, `MAX (max, −∞)`, `BOOL_AND/OR`, `BIT_AND/OR/XOR`. This is
  today's allowlist — and it is exactly the closed set of *directly presentable*
  commutative monoids over scalar columns. (The spec itself asserts only
  commutativity + associativity of the combiner; the identity element — the empty
  partition — is implicit there and made explicit here.)
- **Decomposed monoid** (needs a presentation map `π`): the *state* is a monoid
  element in a richer space; the user value is `π(state)`.
  - `AVG` → state `(sum, count)` under componentwise `+`; `π = sum/count`.
  - variance / stddev → state `(count, sum, sum_of_squares)` (or a numerically
    stable Welford triple) under componentwise merge; `π` = the closed form.
  - approximate `COUNT(DISTINCT)` → state = an HLL/sketch register vector under
    register-wise `max`; `π` = the cardinality estimate. (Exact `COUNT(DISTINCT)`
    is *not* a bounded monoid — its state is the full set — which is why every
    delta engine treats exact distinct specially.)

The whole append-only half of the design space is "which commutative monoids can we
store and present." Direct is the subset where `π` is the identity. **Decomposed
state is the entire content of the `AVG`/variance/approx-distinct unlock** — and it
needs no engine support beyond a table and a view.

### 4.2 Group = retraction / delete / reprocess

Append-only monoids cannot *remove* a contribution. The moment inputs can change —
late-arriving corrections, a reprocessed partition, a true source delete — the
combiner must be **invertible**: a commutative **group** (a monoid with an inverse
`⊖`).

- `SUM`, `COUNT`, `BIT_XOR` are groups — `x ⊕ y ⊖ y = x`. These are precisely
  cumulative's "reversible aggregators" ([`cumulative_aggregate.md`](../specs/cumulative_aggregate.md)
  §"Reprocessing semantics"). Subtract-then-add reprocessing works because they
  form a group.
- `MIN`, `MAX`, `BOOL_*`, `BIT_AND/OR` are monoids **but not groups** — you cannot
  un-see a maximum without rescanning. This is the exact fault line
  [eligibility §7.2 obs. 2](20260701-expanding-incremental-eligibility.md) noticed
  empirically: Snowflake supports `MIN/MAX` *append-only* (monoid is enough to add)
  while Flink keeps *retraction state* for them (a non-group needs the raw multiset
  to handle a delete). Same fact, now named.

So the state-representation axis has **three** rungs, not two:

```
direct monoid ⊂ decomposed monoid (append-only) ⊂ group (retraction)
   SUM,MIN…        + AVG, variance, HLL              + delete/reprocess for the invertible ones
```

### 4.3 The boundary is where smelt-driven stops and native IVM begins

smelt-driven maintenance (a `merge_into` loop, optionally with a presentation view)
can realize **any commutative monoid it can store, and retraction for the group
subset**. That already covers `AVG`, variance, approximate distinct, and reversible
reprocessing — a large, clean, *derivable-from-SQL* class, on DuckDB, with no engine
IVM at all.

What it **cannot** self-maintain is the part of the delta camp that needs
general retraction over arbitrary operators: incremental **joins** (bilinear —
DBSP's product rule), `DISTINCT` and exact `COUNT(DISTINCT)` (unbounded state),
non-additive aggregates like `MEDIAN`/`PERCENTILE` (all-rows state). Those are
maintainable, but only by a runtime that keeps per-operator differential state —
which is what native IVM *is*. That is the honest boundary between the two
maintainer columns: **smelt emulates the monoid/group aggregate slice; native IVM
adds the general-operator slice smelt cannot keep state for.**

---

## Part 5 — Emulation vs delegation

### 5.1 `(state table + view)` on DuckDB *is* what Enzyme does natively

The presentation-view mechanism is not a DuckDB workaround — it is a *portable
reimplementation of the engine trick*. Enzyme keeps hidden row-tracking state and
serves a clean logical MV; smelt keeps hidden `(sum, count)` state and serves a
clean logical view. Same logical object, two maintainers. This is why the same
`refresh` declaration can compile to either without changing the user's mental
model or the equivalence contract (§2):

- **DuckDB / plain Spark** — smelt maintains: state table + `merge_into` loop +
  presentation view. Capability required: the `merge_into` primitive
  (`supports_merge`) + views (both already present).
- **Databricks** — the engine maintains: `CREATE MATERIALIZED VIEW …` and Enzyme's
  runtime keeps state + presentation. Capability required: `supports_native_ivm`.

### 5.2 Capability model

This slots into the existing `multi_backend.md` capability matrix, which already
carries a "native materialized view" notion (§Semantics "Required lowerings": "No
backend today emits a native materialized view: DuckDB and both Spark profiles
take the table fallback … a real one would be a Databricks-only capability").
Two flags — named after the matrix's existing `supports_*` convention — express
the space:

- **`supports_native_ivm`** — the backend can maintain a declared query as a
  native incremental view. `true` → delegate; `false` → smelt-driven `(state
  table + view)` fallback. This is the standard `multi_backend.md`
  **lower-don't-reject** posture ([§Design "Lower, don't
  reject"](../specs/multi_backend.md)): a missing capability is a lowering
  obligation, not a user-facing error.
- **`supports_retraction`** — whether the maintainer can invert contributions
  (delete / reprocess). smelt-driven sets this `true` **only for the group
  subset** (§4.2); native IVM sets it `true` generally. Drives whether a
  reprocess is accepted or refused-with-`--full-refresh` (cumulative's current
  v1 policy).

The user-facing surface stays a single `refresh:` declaration; direct vs
decomposed vs native is **derived** (SQL shape + capability), never declared — the
derive-don't-declare posture cumulative already takes for `unique_key` and
aggregators ([`cumulative_aggregate.md`](../specs/cumulative_aggregate.md)
§Design), extended here to state representation, and the one
[eligibility §11.3a](20260701-expanding-incremental-eligibility.md) argues for
ordering. `AVG` in the SQL ⟹ decomposed; `has_native_ivm` ⟹ delegate; otherwise
smelt-driven.

### 5.3 The two hazards emulation introduces

Delegation inherits the engine's correctness; **emulation is smelt's to get right**,
and hidden state adds two concerns the direct case never had:

1. **Presentation-view consistency under partial merge.** Cumulative merges N
   partitions as N transactions and tolerates partial progress
   ([`cumulative_aggregate.md`](../specs/cumulative_aggregate.md) §"Execution
   model"). A view over the state table is then always well-defined *as a function
   of current state* — `sum/count` of a half-merged state is the mean of what has
   been merged. That is the same partial-progress semantics cumulative already
   documents, lifted through `π`. The requirement is only that `π` be a pure
   function of a single consistent snapshot of the state row — which rules out a
   `π` that reads other rows or other tables.
2. **Atomic state/view swap on schema change.** Adding a decomposed aggregator
   changes the state table's shape *and* the view. The pair must move together, and
   a full rebuild of decomposed state requires rescanning source history — the same
   backfill limitation cumulative's §"Known Divergences" (schema evolution) already
   names, now also touching the view definition. Emulation must treat
   `(state table, view)` as one atomically-swapped unit.

---

## Part 6 — Relationship to the two existing docs

This doc is the **stateful/maintained camp's** positioning; the eligibility
research is the **window-forward camp's** audit. They partition the field between
them:

- [eligibility research](20260701-expanding-incremental-eligibility.md) works
  camp 1 exhaustively (what makes a `DELETE+INSERT` window-rebuild ≡ full). Its
  §7.1 names both camps; §7.3 grounds the monotone/linear frontier; §11.3 hands the
  ordered / self-referential / cumulative slice *to this camp*. The open question
  it leaves for us explicitly — "where should the window-cluster classifier hand
  off to cumulative … route automatically or reject-and-suggest?" (§Open questions,
  "Cumulative vs. incremental boundary") — is a boundary this doc's abstraction is
  meant to make principled: a cross-partition `UNBOUNDED PRECEDING` running total
  *is* a maintained relation, and should route to the maintained camp.
- [`cumulative_aggregate.md`](../specs/cumulative_aggregate.md) is the
  `{direct, smelt-maintained}` origin; this doc is the space it sits in. The
  [20260522 rationale](20260522-cumulative-as-its-own-rule.md) already sketches the
  *sibling* rules (`scd2`, `latest_value`, `accumulating_snapshot`) as "stateful
  merge with history" — those are further points in the *same* maintained-relation
  space, differing in unique-key/validity semantics rather than in the two axes
  here. The ontology question (§7) is partly: do the two axes and the sibling rules
  share one umbrella?

---

## Part 7 — Ontology recommendation

The brief was to let the analysis pick the spine. It does.

**Recommendation: introduce a `maintained` refresh concept as the umbrella, and
make `cumulative` its first named member — the `{direct, smelt-maintained,
monoid}` instance. Do *not* stretch the word "cumulative" to cover the whole
space.**

The argument, in order of weight:

1. **The two axes are orthogonal to "it is an aggregate."** State representation
   (direct/decomposed/group) and maintainer (smelt/native) are properties of *how
   state is kept*, not of *what the query computes*. `scd2`, `latest_value`, and
   `accumulating_snapshot`
   ([20260522 §"Sibling rules"](20260522-cumulative-as-its-own-rule.md)) are
   maintained relations that keep hidden state and defer/emulate identically, yet
   are not aggregates at all. An umbrella keyed on "maintained relation with hidden
   state and an equivalence contract" holds all of them; a generalized
   "cumulative" would have to mean "any maintained relation," at which point the
   name actively misleads (a slowly-changing dimension is not cumulative anything).

2. **The contract generalizes cleanly; the *name* does not.** §2 shows the
   equivalence contract lifts verbatim to the whole space. The word "cumulative"
   describes *one* combiner behaviour (running accumulation), so using it as the
   umbrella severs the tight name↔contract fit smelt values elsewhere
   (`models.md`'s insistence that each axis value names one contract). Keeping
   `cumulative` = the additive-aggregate member preserves that fit.

3. **It matches the rule-composition posture already chosen.**
   [20260522](20260522-cumulative-as-its-own-rule.md) explicitly prefers "narrow,
   composable rules … separate sibling rules per pattern … compose better than one
   generic MERGE rule with enough knobs." An umbrella-with-members *is* that
   posture; generalize-cumulative is the "one rule with enough knobs" it rejected.

4. **It leaves the DuckDB-emulation / native-IVM choice where it belongs — in
   physical execution, invisible to the surface.** Under the umbrella framing the
   maintainer axis is a `multi_backend` lowering decision (§5.2), not a new
   user-facing refresh value. Generalize-cumulative would tempt a
   `cumulative: { native: true }` knob — surfacing physical execution, exactly the
   metadata-vs-SQL drift both predecessor docs fought to avoid.

**What this recommendation is *not*:** it is not a proposal to rename or restructure
`cumulative` now. Cumulative ships as-is. The umbrella is a *conceptual* home
introduced when the first sibling (or the first hidden-state member) is specified;
until then it is a documented direction. The concrete near-term surface implication
is only that `models.md` §"Refresh axis" should describe `cumulative` as *one
member of a maintained family*, leaving room for `maintained` / siblings, rather
than as a one-off peer of `incremental`.

**The runner-up (generalize `cumulative`) is worth stating** so the rejection is
legible: it has the smallest surface (no new concept) and would be right *if* the
space were only "more aggregates." It fails because the space is also non-aggregate
maintained relations (the siblings) and a physical maintainer axis — two things the
word cannot absorb without becoming a misnomer.

---

## Part 8 — Open questions and boundaries

- **Where is the maintained ↔ window-forward routing decision made?** A
  cross-partition running total can be *written* as an `incremental` model with an
  `UNBOUNDED PRECEDING` frame or as a maintained aggregate. The eligibility
  research routes the former to per-partition/full recompute and flags the handoff
  as open. Should the classifier *route* such a shape to the maintained camp
  automatically, or reject-and-suggest? (Shared open question with the eligibility
  audit.)

- **How much decomposition is derivable vs. needs a registry?** `AVG →
  (sum,count)` and variance are mechanical rewrites. Approximate distinct needs a
  *chosen* sketch (HLL precision). Is the decomposed-monoid set a closed,
  hard-coded rewrite table (like the current combiner lookup), or an extensible
  registry? The closed-table answer matches cumulative's "fixed allowlist, not a
  registry" stance; revisit only when a concrete sketch motivator appears.

- **Presentation-view purity.** §5.3 requires `π` to be a pure function of a single
  state row. Is that guaranteed by construction from the decomposition rewrite, or
  does it need a classifier check (reject a `π` that references another table /
  window / row)?

- **Retraction without a monotone event-time.** The maintained camp does *not*
  need monotone event-time (§2). But smelt-driven retraction still needs to know
  *which* prior contribution to invert — i.e. it needs change-tracking on the
  source (a delta history / side table). What is the minimum source-side machinery
  smelt must keep for group-state retraction on DuckDB, and is it worth it vs.
  simply delegating retraction to native IVM only?

- **Native-IVM delegation scope and negotiation.** Which engines expose a usable
  IVM surface (Databricks Enzyme; Snowflake Dynamic Tables; anything else)? How
  does smelt negotiate the eligibility gap — the engine may reject a query smelt
  offered as a native MV (Enzyme's `MATERIALIZED_VIEW_NOT_INCREMENTALIZABLE`). Does
  smelt fall back to smelt-driven maintenance (only possible for the monoid/group
  slice) or to full-refresh? This is a `multi_backend` conformance question.

- **Downstream pushdown is unchanged.** A maintained relation has a unique key and
  no partition column, so downstream consumers treat it as a lookup exactly as
  cumulative output is treated today
  ([`cumulative_aggregate.md`](../specs/cumulative_aggregate.md) §"Output shape").
  Hidden state does not change this — the *view* is the lookup; the state table is
  never a dependency target. Worth stating explicitly in any future spec so nobody
  tries to push a filter into the state table.

- **Does the umbrella subsume the sibling rules, or sit beside them?** §7 places
  `scd2`/`latest_value`/`accumulating_snapshot` in the same space, but they were
  sketched as *separate rules*. Is `maintained` an abstract contract the sibling
  rules each *implement* (shared execution, per-rule classifier), or a peer refresh
  value? Settling this is the first job of the umbrella's own spec.

---

## Part 9 — Does the umbrella reach up to `incremental`?

Part 7 recommends a `maintained` umbrella over `cumulative` and its siblings. The
natural follow-on: should there be a *higher* umbrella spanning `maintained` **and**
`incremental`? The answer splits on what "umbrella" means — a conceptual/contract
lid is a small win; a structural selector is a re-introduction of the dbt footgun.

### 9.1 The shared parent contract is real

Incremental and the maintained family share one contract, worth stating once:

> **Processed-input equivalence.** A non-`full` refresh produces the same result as
> a full refresh restricted to the inputs it has processed.

Incremental's **per-partition equivalence**
([`incremental_models.md`](../specs/incremental_models.md)) and maintained's
**end-state equivalence** (§2) are two *specializations*: one slices the output by
`partition_column`, the other asserts equality of the whole keyed end-state. Beneath
the contract they also share machinery — `--event-time` run windows (cumulative
already reuses incremental's flags,
[`cumulative_aggregate.md`](../specs/cumulative_aggregate.md) §CLI), `--auto`
staleness, the derive-from-SQL posture, and `multi_backend` capability lowering.
Naming the parent contract and the shared machinery once, with two clearly-distinct
children beneath it, is a clean documentation improvement.

It also resolves a genuine terminology collision. **The industry calls the
*maintained* camp "incremental view maintenance."** So smelt's `incremental`
(window-forward) and "incremental" in the Enzyme/Snowflake/Materialize literature
name *opposite* camps. A parent term lets the docs say "both are incremental in the
broad sense; here are the two shapes," defusing a confusion a flat taxonomy would
otherwise inherit.

### 9.2 A structural umbrella re-creates the dbt footgun

If the umbrella instead means making `incremental` and the maintained family
**siblings under one selector with a strategy knob** (`refresh: incremental` +
`strategy: window | merge`), it walks directly back into what the cumulative spec
already rejected, verbatim:

> *"dbt conflates the two under `materialized='incremental'` and dispatches by
> `incremental_strategy`. This is the single most common source of confusion in dbt
> because the `strategy:` knob silently changes the equivalence contract — same
> frontmatter, different invariants."* —
> [`cumulative_aggregate.md`](../specs/cumulative_aggregate.md) §Design

The two children differ in exactly the things that are *not* knobs:

| | `incremental` | maintained (`cumulative`, …) |
|---|---|---|
| output shape | partitioned, **has** `partition_column` | keyed lookup, **no** partition column |
| equivalence | per-partition slice | end-state |
| execution | window-independent (parallel, §11.2) | ordered (sequential, reads own state, §11.3) |
| camp (§7.1) | window-forward; needs monotone event-time | change-tracking; sidesteps monotonicity |

A selector that makes those feel like variants of one strategy **subordinates the
single most load-bearing line in the space** — stateless/window-independent vs
stateful/ordered — which
[eligibility §11](20260701-expanding-incremental-eligibility.md) was careful to keep
sharp. That is a strictly worse taxonomy even though it looks tidier.

### 9.3 The actual improvement: statefulness as the spine, not the selector

The upgrade the question surfaces is not the lid — it is promoting **statelessness
to the *named reason* the children differ**, rather than presenting `incremental`
and `cumulative` as two arbitrary peers on a flat enum. The refresh axis reads
better as:

```
full                                  — recompute everything
processed-input-equivalent            — (conceptual umbrella; §9.1)
  ├─ stateless / window-independent    → incremental  (per-partition, partitioned, parallel)
  └─ stateful  / maintained            → cumulative + siblings (end-state, lookup, ordered)
```

One caveat keeps this honest and stops it hardening into a rigid two-level selector:
window-independence is a **derived** property, not a declared one
([eligibility §11.3a](20260701-expanding-incremental-eligibility.md)), and it
*leaks* across the split — a **self-referential incremental** model is
stateful-ordered yet still executes as partition `DELETE+INSERT`, not `merge_into`
(the contrast [eligibility §11.3](20260701-expanding-incremental-eligibility.md)
draws for cumulative; the self-referential shape is named there as its
"incremental cousin"). So
statefulness explains the *split* but must not *become* the selector; users still
write `incremental:` / `refresh: cumulative`, and ordering stays derived.

### 9.4 Recommendation

- **Adopt the conceptual umbrella.** State **processed-input equivalence** as the
  shared parent contract, specialized to per-partition (incremental) and end-state
  (maintained); note the shared machinery; and call out the "incremental" ↔ IVM
  terminology collision explicitly so the docs resolve it rather than inherit it.
- **Reject any structural/selector umbrella.** No `refresh: incremental` +
  `strategy:` knob, no single refresh value spanning both — it silently varies the
  equivalence contract, the precise dbt failure the cumulative spec was built to
  avoid.
- **`models.md` §"Refresh axis" phrasing nudge.** Present `incremental` and
  `cumulative` as *processed-input-equivalent children distinguished by
  statefulness*, not as flat peers of each other and of `full`. The value is ~90%
  in naming the shared contract and fixing the terminology, and actively negative in
  any shared selector.

---

## References

- **Specs**: `cumulative_aggregate.md`, `incremental_models.md`, `multi_backend.md`
  (§Design "Lower, don't reject"; capability matrix), `models.md` (§"Refresh
  axis"; `materialized_view` on the storage axis), `architecture.md` (compiler
  pipeline; backend primitives).
- **Research**: `20260701-expanding-incremental-eligibility.md` (§7.1 two camps,
  §7.2 external validation, §7.3 monotone/linear theory, §11.3 ordered slice,
  §Open questions "Cumulative vs. incremental boundary");
  `20260522-cumulative-as-its-own-rule.md` (why cumulative is its own rule; sibling
  rules); `20260521-incremental-as-planner-rule.md` (derive-from-SQL principle).
- **External** (via eligibility §7 citations): Databricks Enzyme
  (`MATERIALIZED_VIEW_NOT_INCREMENTALIZABLE`), Snowflake Dynamic Tables, BigQuery
  MVs, Materialize, Feldera/DBSP (Budiu et al., VLDB 2023) for the linear/bilinear
  cost taxonomy and the Z-set retraction model; CALM theorem (Hellerstein 2010;
  Ameloot–Neven–Van den Bussche 2011/2013) for monotone = coordination-free.
