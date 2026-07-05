# A unified `keyed` refresh mode, with patterns as functions

**Status:** research (decision-oriented)
**Date:** 2026-07-05
**Owners:** andrew (analysis drafted by Claude at andrew's request)
**Related:**
- [`20260705-model-refresh-review.md`](20260705-model-refresh-review.md) — the fresh-context review whose findings §1.1, §3.1, §3.2 and §6.1 this note resolves structurally.
- [`20260704-maintenance-fundamentals.md`](20260704-maintenance-fundamentals.md) — the framework this note takes as given (properties / transforms / composition).
- Specs affected if adopted: `models.md`, `model_maintenance.md`, `cumulative_aggregate.md`, `latest_value_models.md`, `versioned_models.md`, `accumulating_snapshot.md`, `functions.md`.

## The question

Could `cumulative`, `latest_value`, `versioned`, and `accumulating_snapshot` be
collapsed into **one** refresh mode — not as they are, but such that a single
"merge + keyed" refresh type covers all their use cases? And if so, can the
individual patterns be reintroduced as **smelt functions** whose signature and
implementation encode the pattern, rather than as enum values?

Short answer: yes for three of the four, and the framework's own rules almost
force it; `versioned` is the one genuinely hard case, and the function layer is
exactly the right place to bring it (and the others) back as named, reusable,
proof-gated patterns.

## 1. The three-way collapse is clean

Strip `cumulative`, `latest_value`, and `accumulating_snapshot` to their
composition tables and they are the same machine:

- **Output shape** — keyed, one row per key (identical).
- **Invariant** — end-state equivalence (identical).
- **Transform** — keyed `merge_into` sequenced by the one
  windowed-keyed-maintenance driver (identical; the specs already insist all
  keyed modes share the driver rather than copying it).
- **Key** — derived from `GROUP BY` (identical, once `latest_value` adopts the
  SQL-carried `MAX_BY` shape — which the review's §1.1 argues it needs anyway to
  make the equivalence invariant an executable oracle).

The *only* difference is the per-column **combiner family**, and every
consequence of that difference is a **derived** fact, not a contract:

| Difference | `cumulative` | `accumulating_snapshot` | `latest_value` | What it really is |
|---|---|---|---|---|
| Combiner allowlist | monoids incl. `SUM`/`COUNT` | idempotent monotone folds | `MAX_BY` | derived per column from the SQL |
| Overlap / re-run tolerance | no (non-idempotent) | yes | yes | derived from idempotence of the combiner set |
| Reprocessing story | group rung or refuse | always safe | always safe | derived from invertibility |
| Horizon clamp | none (all keys forever hot) | required, bounded `H` | none | derived from forward reach |

Now apply the project's own litmus rule (`models.md` §Design): *"changes only how
deltas are discovered or how much is scanned → derived, never declared."* Every
row of that table is scan/execution posture under an unchanged contract. By
smelt's own law, these are not three modes. The current six-value enum conflates
the axis `model_maintenance.md` names as load-bearing — **output addressing** —
with derived combiner families. The collapsed enum

```
refresh: full | batched | keyed | materialized_view      (± versioned, §3)
```

answers exactly "how is output addressed for update, and who owns freshness",
which is the axis the framework already declared to be the real one. The
dbt-strategy-footgun rationale for peer modes does not apply among the three:
that footgun was one YAML name silently swapping *contracts*; here the contract
(end-state equivalence, keyed shape) is identical across all three, and what
varies with the SQL is visible *in* the SQL.

### 1.1 The decisive use case: mixed combiner families in one table

Combiner intent is **per column, not per model**. A real entity-summary table
wants all three families at once:

```sql
---
refresh: keyed
---
SELECT
    order_id,
    MIN(event_ts)                        AS placed_at,      -- milestone (acc-snapshot style)
    MAX_BY(status, event_ts)             AS current_status, -- overwrite (latest_value style)
    SUM(item_count)                      AS total_items,    -- running fold (cumulative style)
    MAX(shipped_ts)                      AS shipped_at      -- milestone
FROM smelt.order_events
GROUP BY order_id
```

Under the peer taxonomy this model is **inexpressible** — no mode's allowlist
covers it — and the answer is "split into three keyed models and join",
materialising the same keyed state three times over. Under a unified
`refresh: keyed`, the classifier gives **per-column verdicts** and derives the
model's execution posture from the weakest column (here: `SUM` present ⇒
non-idempotent ⇒ run-ledger / no blind re-run; were every column idempotent, the
model would be overlap-tolerant). Any entity-lifecycle summary hits this shape,
which makes it common enough to be decisive on its own.

### 1.2 What unification dissolves for free

- **The cumulative/accumulating_snapshot overlap zone** (review §3.1 —
  `SELECT key, MIN(x), MAX(y) … GROUP BY key` is admissible under both today,
  with the choice under-specified) stops existing.
- **The horizon clamp's invariant-weakening** (review §3.2). In a unified mode
  the clamp is no longer load-bearing: `merge_into` reaches keys wherever they
  live (as `cumulative` already does), so the default is *no write clamp* and no
  silently dropped scanned rows. `H` demotes to a derived work-bound plus an
  optional, explicitly-declared late-fact policy (with the review's late-fact
  accounting), rather than a mode-defining completeness contract.
- **The multi-driving-source rejection** (review §6.1) needs solving once, not
  four times.

## 2. How a `keyed` model runs: the clocked / unclocked split is real, and derived

A single mode does **not** mean a single run shape. Within `refresh: keyed`
there are two execution postures, and the distinction is exactly the
input-consumption axis the framework already owns (`models.md`
§"Input-consumption axis") — **derived from the driving source, never declared**:

| Driving source | Run shape | CLI surface | Ordering / resume |
|---|---|---|---|
| **Clocked** (`timeseries:` on the source) | **window-forward**: `--event-time` run window on the *source's* partition column; the driver steps covered windows in temporal order; per-window delta SELECT → `merge_into` | `--event-time-start/-end` required, exactly as `cumulative` today | sequential in temporal order when any column is non-idempotent or order-dependent; out-of-order/parallel/backfill-in-slices admitted when every column is an idempotent commutative fold |
| **Unclocked** (mutable snapshot source) | **snapshot-diff**: re-scan the source whole, diff against stored state, upsert the changed keys | no run window (the flags are meaningless; `--auto` staleness semantics per the open question in `models.md`) | each run is a self-contained reconciliation; re-running is always safe |

Two compatibility constraints fall out, and they are the place the
event-vs-state distinction from the review (§2.1) must be enforced:

1. **Fold-family columns require delta-shaped input.** A `SUM`/`COUNT` column
   consumes *events* (each row folded exactly once). Over an unclocked snapshot
   source there is no delta stream — successive re-scans would re-fold state and
   double-count — so a `keyed` model containing any non-idempotent fold column
   **requires a clocked (replayable) driving source** and is refused over a bare
   snapshot. (Idempotent folds — `MIN`/`MAX`/`MAX_BY` — are safe under
   re-scan-and-merge and are admitted in both postures.)
2. **Overwrite-family columns accept both.** `MAX_BY`-shaped columns consume
   either an event stream (window-forward) or state observations
   (snapshot-diff) with the same end-state contract.

So the mode × discovery matrix the review asked for becomes a **column-family ×
source-shape** matrix inside one mode — smaller, and enforced per column by the
same classifier that derives the combiners. This is strictly more precise than
the current arrangement, where the constraint is smeared across four mode specs
and partially enforced by an incidental diagnostic
(`CumulativeNoDrivingSource`).

The run-shape split also keeps the operational surfaces honest: `smelt explain`
reports the derived posture (window-forward + ordered + ledger-required vs
snapshot-reconcile + idempotent), which is the same observability posture the
batched per-source clamp already takes — derive, then surface.

## 3. `versioned`: the one hard case

`versioned` differs in **output shape** — multiple rows per key
(`(key, version)` with a validity interval) — so by the litmus rule it
legitimately earns a peer name. That is the conservative resolution: collapse
the three, keep `versioned`.

But there is a more useful framing. Versioned history is a **pure function of
the set of change observations per key**: sort a key's observations by event
time, collapse consecutive duplicates, and derive intervals. As SQL over the
full input:

```sql
SELECT key, attrs,
       event_ts                                                   AS valid_from,
       LEAD(event_ts) OVER (PARTITION BY key ORDER BY event_ts)   AS valid_to,
       LEAD(event_ts) OVER (PARTITION BY key ORDER BY event_ts) IS NULL AS is_current
FROM deduped_change_events
```

That query's full refresh *is* the SCD2 history — which repairs the review's
§1.1 finding (the mode currently adds semantics, including whole columns, that
the SQL does not carry, leaving the equivalence invariant without an executable
oracle). And it is maintainable inside unified `keyed`:

- the per-key **state** is a grow-only set of change events — set union, an
  idempotent commutative monoid, the friendliest fold on the ladder;
- the interval columns are a **presentation** over that state, and the
  presentation is **neighbour-local**: inserting an (even out-of-order) event
  touches only its two temporal neighbours' `valid_to`/`is_current`, so the
  merge writes ≤ 2 stored rows per incoming event.

This framing also answers `versioned_models.md`'s hardest open question — late
corrections to a closed interval — trivially: insert the correction into the
set; the presentation recomputes the local intervals. Nothing is rewritten
"in place"; replay safety is preserved by construction.

The cost is that the classifier needs one new proof: *grow-only-set state +
neighbour-local window presentation is maintainable*. That is a narrow,
one-shape proof (a `LEAD`/`LAG` over `PARTITION BY key ORDER BY event_ts` on an
append-monotone input), not general window-function IVM. §4 shows where that
proof naturally lives.

Two honest limits:

- **Snapshot input still needs a diff front-end.** Over a mutable snapshot
  source, the change *events* must first be manufactured by diffing the scan
  against stored state (hash-diff is the standard cost reduction — review
  §5.5). That is input-consumption machinery in the runtime, not something a
  pure function can express; it applies identically whether versioned is a mode
  or a pattern.
- **Consecutive-duplicate collapse** must live in the presentation (compare
  with `LAG`), not at merge time, for order-independence to hold. The canonical
  body must encode this; hand-rolled variants that diff-at-merge are
  order-dependent and should fail the proof.

## 4. Reintroducing the patterns as smelt functions

The question's second half: rather than enum values, encode each pattern as a
**smelt function** whose signature and implementation carry the semantics.
This is not just viable — the existing function architecture is unusually
well-shaped for it.

### 4.1 Why the machinery already fits

- **Expansion runs before every analysis stage** (`expansion.md`; restated in
  the batched and cumulative specs). A `smelt.define`-resolved call is
  indistinguishable from hand-written SQL by the time the classifier runs, and
  the mode specs already admit define-wrapped aggregators "on the same terms as
  a hand-written allowlisted call". So a pattern function needs **no privileged
  treatment**: the classifier keeps seeing core SQL, and the function is
  admitted iff its expansion proves. Fail-closed is preserved with zero new
  trust surface.
- **The fragment-sort vocabulary already has the right sorts** (`functions.md`
  §Surface): `AggExpr<T>` for aggregate-position patterns, `TableExpr` /
  `TableExpr<{…}>` for table-shaped patterns, and the models-as-functions
  equivalence for invoking them in FROM position.

### 4.2 The pattern library

**Aggregate-level patterns** — thin, immediately buildable:

```sql
smelt.define latest(value AggExpr-position Expr<T>, ordering Expr<Ordered>) -> T AS
  MAX_BY(value, ordering)

smelt.define once(value Expr<T>) -> T AS          -- first-non-null, once-write
  COALESCE-first-non-null over the group          -- (canonical spelling)
```

A latest-value model is then:

```sql
---
refresh: keyed
---
SELECT customer_id,
       smelt.latest(tier,   updated_at) AS tier,
       smelt.latest(region, updated_at) AS region
FROM smelt.customer_updates
GROUP BY customer_id
```

The *name at the call site* carries the intent (per column — finer-grained than
a mode name ever was); the *body* carries the proven semantics; the classifier
sees `MAX_BY` after expansion and derives the order-monotone verdict as usual.
Note `smelt.once` does **not** bypass the once-write provenance proof — the
expanded `COALESCE` still needs the key-derived or declared-FD licence at the
call site. Functions name patterns; proofs still gate them.

**Table-level patterns** — where `versioned` comes back:

```sql
FROM smelt.versions(
  input      => smelt.customer_change_events,   -- TableExpr
  key        => customer_id,
  event_time => updated_at
)
```

with the canonical dedup + `LEAD` body from §3 as the implementation, returning
the projected attributes plus `valid_from`/`valid_to`/`is_current` as **ordinary
output columns of the function**. This is a quiet but significant repair: the
"smelt-managed validity columns" — the one place in the family where stored
columns appear from nowhere — become ordinary projected columns of a library
function, and the model's SQL once again fully describes its output.

Other candidates once the shape exists: `smelt.dedup_latest(input, key,
ordering)` (the cross-partition dedup ingest pattern, review §6.5), and a
milestone-set helper for funnel tables.

### 4.3 What the function layer buys beyond sugar

1. **Intent cross-checking returns, better than before.** The main cost of
   collapsing the enum was losing the declared-mode-vs-SQL cross-check.
   Pattern functions restore it *per column and per table expression*, in the
   SQL itself, next to the thing they describe — strictly better placement than
   frontmatter. (The previously-floated `asserts: pattern:` frontmatter key
   becomes unnecessary.)
2. **Correctness by construction for the fiddly shapes.** Nobody hand-writes
   the dedup-and-`LEAD` dance correctly the fifth time. One canonical, tested
   body — validated against the equivalence oracle like any other maintained
   shape — replaces four spec-prose descriptions of interval bookkeeping.
3. **The custom-combiner registry falls out for free.**
   `cumulative_aggregate.md` §Design rejected letting authors register custom
   combiners because it needed a registry surface plus trust. Under
   pattern-functions there is no registry and no trust: a user-defined pattern
   function is maintainable **iff its expansion classifies** — the same
   fail-closed gate as the standard library, with zero privileged names..
4. **Functions become the unit of proof modularity.** Today every proof runs
   over the fully expanded model. A pattern function invites the obvious
   optimisation: derive the body's verdict once at the definition (the
   grow-only-set + neighbour-local proof for `versions`; order-monotone for
   `latest`), cache it, and compose verdicts at call sites instead of
   re-deriving over the expanded tree. This must stay *derived-and-cached*
   (never asserted-and-trusted) to preserve fail-closed — it is a performance
   architecture, not a trust architecture. It is also exactly the shape a
   third-party pattern ecosystem needs.

### 4.4 What has to land first (honest dependency list)

- **`TableExpr`-parameter invocation in FROM position** is parsed but deferred
  (`models.md` §"Named parameters parsed but deferred"). `smelt.versions`
  needs it live. The aggregate-level patterns (`latest`, `once`) do **not** —
  they are expression-position and work with today's expansion.
- **Column-set polymorphism.** `versions` must carry "the rest of the columns"
  (the tracked attributes) through its body. v1 `smelt.define` is monomorphic
  and non-variadic; `smelt.as_struct` / `Expr<Struct<{…, ..r}>>` row
  polymorphism is partially landed and explicitly deferred. Until then,
  `versions` is either arity-specialised (unsatisfying) or waits for the struct
  work. This is the long pole for the table-level patterns.
- **The grow-only-set / neighbour-local proof** (§3) for whichever layer hosts
  versioned — needed whether versioned stays a mode or becomes a function; the
  function route just means the proof runs over one canonical body instead of
  arbitrary user SQL first.
- **Diagnostic provenance through expansion** already exists (`expansion.md`
  frames); pattern functions lean on it harder — a classifier rejection inside
  an expanded pattern body must point at the call site with the function frame,
  which the frame machinery is built for.

### 4.5 The taxonomy end-state

- **Modes (declared; the selector):** `full` · `batched` · `keyed` ·
  `materialized_view` — the output-addressing + freshness-owner axis, nothing
  else. (`versioned` optionally survives as a fifth value during transition;
  see §5.)
- **Patterns (library functions; named intent, proof-gated):** `smelt.latest`,
  `smelt.once`, `smelt.versions`, `smelt.dedup_latest`, … — user-extensible on
  equal terms.
- **Everything else derived**, exactly per the three-state law: per-column
  combiners, idempotence, invertibility, horizon/work-bounds, run shape
  (window-forward vs snapshot-diff), ordering, ledger requirement.
- The litmus rule gains a fourth clause: *"names a reusable combiner/table
  shape without changing contract or output shape → a **function**, not a
  mode."*

## 5. Costs, and what keeps them acceptable

- **One bigger classifier** instead of four small ones — but the four were
  already sharing the driver, the discriminants, and the anchor resolution;
  what merges is the surface dispatch, while total verdict logic shrinks
  (per-column families replace four overlapping allowlists).
- **Teachability.** `refresh: latest_value` is instantly graspable;
  `refresh: keyed` + `smelt.latest` needs a recipe page. Mitigations: the
  pattern names survive as function names (visible in the SQL, hoverable in the
  LSP, documented as recipes), and `smelt explain` names the derived per-column
  families. Names that don't change contracts belong in the library and the
  docs, not the enum.
- **A silently-wrong-intent model** (`SUM` where `smelt.latest` was meant) is
  legal under one mode. This is the residual risk after the mitigations above;
  it is the same class of risk as any other SQL bug, it is visible in the SQL,
  and it is qualitatively different from the dbt footgun (an invisible YAML
  contract swap).
- **Migration** is cheap *now*: only `cumulative` is built; the
  `latest_value` / `versioned` / `accumulating_snapshot` classifiers exist as
  specs and L4 sub-plans but have not landed. After they ship as four modes,
  this re-cut becomes a breaking rename plus classifier merge.

## 6. Recommendation

1. **Collapse `cumulative`, `latest_value`, `accumulating_snapshot` into
   `refresh: keyed`.** Rewrite the three specs as: one `keyed_models.md` (the
   mode: composition table, per-column combiner families, run-shape derivation
   per §2, the column-family × source-shape compatibility matrix), demoting the
   current mode names to *pattern* vocabulary.
2. **Keep `versioned` as a peer for now** (its output shape passes the litmus
   test) — but write the grow-only-set + neighbour-local-presentation framing
   into its spec's §Design as the intended internal architecture, since it is
   also the best answer to the late-corrections open question and it keeps the
   door open to folding versioned into `keyed` when the table-function
   prerequisites (§4.4) land.
3. **Ship the aggregate-level pattern functions (`smelt.latest`, `smelt.once`)
   with the unified mode** — they need nothing that doesn't exist, and they
   carry the intent-naming that makes the collapse teachable.
4. **Sequence `smelt.versions` behind the `TableExpr`-invocation and struct
   row-polymorphism work**, and treat its landing as the trigger to revisit
   folding `versioned` into `keyed`.
5. **Update the litmus rule** in `models.md` with the function clause (§4.5),
   so the next pattern proposal routes to the library instead of the enum by
   rule rather than by taste.

The timing argument is the sharpest one: the specs and sub-plans for the three
unshipped keyed modes exist, but their classifiers do not. Every week of
mode-vertical implementation raises the price of this re-cut; today it is
mostly a spec exercise plus one classifier that was going to be written anyway.
