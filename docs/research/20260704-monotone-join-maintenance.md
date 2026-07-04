# Monotone join enrichment: smelt-driven MERGE without native IVM

**Status:** research (decision-oriented)
**Date:** 2026-07-04
**Owners:** andrew
**Related:**
- Spec: [`docs/specs/accumulating_snapshot.md`](../specs/accumulating_snapshot.md) — the keyed once-write enrichment mode this note proposes to widen; today it *rejects* the join spelling (`AccumulatingSnapshotJoinExpressedEnrichment`) and requires a keyed union.
- Spec: [`docs/specs/materialized_view.md`](../specs/materialized_view.md) — engine-owned native IVM, the current answer for the join case; this note argues it should be the *fallback* for the retractable slice, not the primary path.
- Spec: [`docs/specs/batched_models.md`](../specs/batched_models.md) — the driving-fact resolution (window-only-the-fact, full-scan the dimension) and `source_bounds` machinery this reuses.
- Research: [`docs/research/20260703-model-updates.md`](20260703-model-updates.md) — §13 (maintained camp), §14.2–14.3 (the monoid/group ladder and the bilinear-join slice), §17.6 (source mutation profile as the one non-derivable world-fact), §19 (input-consumption axis).

## Why this note exists

`accumulating_snapshot` is the smelt-owned mode for *retroactive enrichment*: a
past row (an event) gains a milestone (`converted_at`) when a later fact (a
conversion) arrives, up to a bounded forward horizon `H`. Its spec draws the
maintainability line at the **surface**: it *rejects* the natural spelling —
`events LEFT JOIN conversions` — with `AccumulatingSnapshotJoinExpressedEnrichment`
and requires the modeller to rewrite the enrichment as a keyed **union** over
one driving stream, on the stated grounds that a join is "the bilinear operator
smelt cannot self-maintain" (`accumulating_snapshot.md` §Design "Model it as a
keyed union").

This note argues that line is drawn one notch too conservatively. The join
smelt genuinely cannot self-maintain is the *retractable* one; a **monotone**
enrichment join — the conversion-flag / milestone-fill case, which is the
common one — is semantically identical to the union form and is
`merge_into`-maintainable on a plain engine with **no native IVM**. The
recommendation is to move the classifier boundary from *join-vs-union syntax*
to *monotone-vs-retractable semantics*, and to reframe native IVM
(`materialized_view`) from the enrichment-join *answer* to the retractable-only
*fallback*.

The framing matters beyond ergonomics. Trusting native IVM on truly large
tables is exactly where smelt is supposed to earn its existence: an engine's
incremental-view runtime is opaque and can strand you in a full-table rebuild
when something changes, with no partial-recovery escape hatch. A smelt-driven
MERGE over plain tables always has that escape hatch. So "do the join without
IVM" is not a convenience — it is the differentiated capability.

## 1. The observation: the conversion-flag join is monotone

The motivating enrichment:

```sql
SELECT
    e.event_id,
    e.event_ts,
    EXISTS (
        SELECT 1 FROM smelt.sources.bronze.conversions c
        WHERE c.event_id = e.event_id
          AND c.conv_ts >= e.event_ts
          AND c.conv_ts <  e.event_ts + INTERVAL '30' DAY
    ) AS converted
FROM smelt.silver.events e
```

`converted` only ever transitions **false → true**. A `converted_at =
MIN(conv_ts)` milestone only ever transitions **NULL → set**. Neither can move
backward as more conversion rows arrive. That is precisely the **once-write /
idempotent-monoid** property `accumulating_snapshot` already runs on
(`accumulating_snapshot.md` §"The maintenance boundary": `LEAST`/`GREATEST`/
`COALESCE`/`MAX_BY` are commutative, associative, idempotent monoids with
identity `NULL`). Monotonicity is what makes the enrichment maintainable by a
per-key `merge_into` that touches only the keys a conversion reached this run —
no partition rebuild, no retraction, no engine IVM.

## 2. Join vs union is a surface choice, not a semantic one

The spec's own union rewrite of the same query —

```sql
SELECT event_id, MIN(event_ts) AS occurred_at, MAX(conv_ts) AS converted_at
FROM ( <events> UNION ALL <conversions> )
WHERE conv_ts IS NULL OR conv_ts BETWEEN event_ts AND event_ts + INTERVAL '30 days'
GROUP BY event_id
```

— computes the **same relation** as the semi-join form. Both funnel the event
and its later conversion through one key (`event_id`) and combine their
contributions with an idempotent monoid. The union spelling makes the monoid
*syntactically visible* (one `GROUP BY`, one aggregator per milestone); the join
spelling hides it inside `EXISTS` / `LEFT JOIN`. But the object being maintained
is identical. Rejecting one spelling and mandating the other is a legibility
tax, not a correctness boundary — and it is a tax on the spelling users reach
for first.

## 3. The real boundary is monotone vs retractable

The maintainability boundary that *is* real (from `20260703-model-updates.md`
§14.2–14.3, the monoid-vs-group rung):

- **Monotone contribution → smelt-MERGE-maintainable, no IVM.** A semi-join
  existence flag; a `LEFT JOIN` to a dimension feeding only once-write
  milestones (`MIN`/`MAX`/`MIN_BY`/`MAX_BY`/first-non-null). The join's
  per-key contribution only ever fills a column NULL → set. `merge_into`
  suffices.
- **Retractable contribution → needs a group/retraction → delegate.** A value
  that can be *revised* (a corrected conversion value overwriting an earlier
  one); a `COUNT(conversions)` per event that must *decrement* when a
  conversion is removed; any milestone that can transition set → different-set.
  Un-seeing a contribution needs the underlying multiset (the group rung) — the
  slice properly routed to engine-native IVM via `refresh: materialized_view`.

The classifier's job should therefore be to **prove the join's contribution is
monotone**, not to reject all joins. `AccumulatingSnapshotJoinExpressedEnrichment`
should fire only when the join feeds a *retractable* milestone or fans out into
an aggregate that must decrement — not merely because the enrichment was written
with a `JOIN` keyword.

## 4. What admitting the join form takes (and what already exists)

Most of the machinery is built:

- **Milestone monoid classification** — the once-write / idempotent-monoid
  allowlist and combiner-derivation already exist
  (`accumulating_snapshot.md` §"Milestone combiner allowlist"). A monotone join
  feeds the *same* combiners; the delta is recognising the join/`EXISTS`
  spelling as producing a monotone per-key contribution.
- **Driving-fact resolution** — batched already resolves a single driving fact
  via alias-scoped leaf resolution and windows only that input while
  full-scanning every other join input (`batched_models.md` §"Event-time
  monotonicity trace consumers"). The enrichment join wants exactly this: window
  the fact (`events`), full-scan the dimension (`conversions`).
- **Forward horizon `H`** — derived from the `conv_ts BETWEEN event_ts AND
  event_ts + INTERVAL '30 days'` predicate by the `after_secs` forward-reach
  walk (`accumulating_snapshot.md` §"The attribution horizon"; landed by the
  model-updates Group-B B2 phase). Unchanged whether the enrichment is a join or
  a union.
- **Source-declared functional dependency** — the once-write prover already
  admits a `COALESCE` milestone when the source declares `key → column`
  (`accumulating_snapshot.md` §"Classifier checks"). The same declaration
  mechanism carries the append-only assertion §5 needs.

What is genuinely new is a **monotonicity classifier for the join operator**: a
proof that a semi-join / dimension-join feeds only once-write milestones and
does not fan out into a decrementing aggregate. This is a cousin of the existing
structural monotonicity primitive (`trace_event_time`,
`crates/smelt-logical/src/analysis/monotonicity.rs`) — monotonicity of a *join
contribution* rather than of an *event-time transform* — and should live beside
it.

## 5. The mutation-profile dependency

Monotonicity of the enrichment rests on one world-fact smelt cannot derive
(§17.6): the dimension's **mutation profile**. If a conversion row can be
*deleted* from the (no-CDF) conversion table, then:

- A `MIN`/`MAX`/first-non-null **milestone** is still safe — losing a row cannot
  un-fill an already-set column (the monoid has no inverse, so it simply never
  retracts).
- A **re-scanned existence flag** (`EXISTS`) is *not* safe — re-evaluating the
  semi-join after a delete would flip `converted` true → false, breaking
  monotonicity.

So admissibility is: **the dimension is declared append-only** (deletes never
happen) → both the milestone and the existence-flag forms are monotone; **the
dimension is mutable** → only the extremal-milestone form is monotone, and the
existence-flag form must either be modelled as a milestone (`converted_at =
MIN(conv_ts)`, `converted := converted_at IS NOT NULL` downstream) or delegated.
The append-only assertion rides on the *source* (`timeseries.md` source
declaration), shared by every consumer — the same place §17.6 already routes the
mutation-profile world-fact.

## 6. The escape hatch is the point

Why draw the line at monotone-MERGE at all, rather than sending every enrichment
join to `materialized_view` on a capable engine? Because native IVM is a
different *operational* bet, and on large tables it is the wrong one:

- **smelt-driven MERGE** keeps maintenance state in **plain tables**. Any
  partition range can be dropped and rebuilt, inspected, or backed out. A
  mis-derived horizon or a bad conversion batch is recovered with a scoped
  `--full-refresh`. The state is legible and the escape hatch is always present.
- **Native IVM** keeps **opaque engine-managed state** and owns freshness
  continuously (`materialized_view.md` §"Freshness owner"). When the query or an
  upstream changes in a way the runtime cannot incrementalise, the fallback is a
  full-table rebuild the engine schedules on its own terms — precisely the
  "stuck, no partial recovery" failure that makes IVM untrustworthy at scale.

The current specs treat native IVM as the *answer* for the enrichment join and
smelt-driven maintenance as the thing that "cannot self-maintain a join." This
note inverts the default: **smelt-driven monotone MERGE is the primary path for
the enrichment join; native IVM is the fallback for the genuinely retractable
slice** — which is the smaller, rarer case, and the one where giving up the
escape hatch is an acceptable trade because the alternative is not maintainable
at all.

## 7. Recommendation

- **Relax `AccumulatingSnapshotJoinExpressedEnrichment` from a syntactic to a
  semantic check.** Admit the join / semi-join spelling of an enrichment when
  the join's per-key contribution is provably monotone (feeds only once-write
  milestones; no decrementing fan-out). Reject only the retractable case, and
  point it at `materialized_view` or DAG composition as today.
- **Add a join-contribution monotonicity classifier** beside the existing
  event-time monotonicity primitive; reuse driving-fact resolution to window the
  fact and full-scan the dimension.
- **Gate monotonicity on a source-declared append-only mutation profile** for
  re-scanned existence flags; extremal milestones are safe regardless.
- **Reframe the maintained-family docs** so native IVM is the retractable-slice
  fallback, not the enrichment-join answer — and state the escape-hatch-vs-opaque-state
  rationale explicitly in `accumulating_snapshot.md` §Design and
  `materialized_view.md` §Design.

## Open questions

- **How much monotonicity is statically provable vs needs a declaration?** The
  clean cases — a semi-join feeding a boolean, a `LEFT JOIN` to a `SELECT
  DISTINCT`-style dimension feeding `MIN`/`MAX` — are structurally recognisable.
  Fan-out joins whose aggregate happens to be monotone (a `MAX` over many matched
  rows) are provable; ones whose aggregate must decrement are not. Where is the
  static/declared boundary?
- **1:1-after-dedup recognition.** When the dimension is dedup'd inline (`JOIN
  (SELECT DISTINCT event_id …)`) the join is 1:1 and cannot fan out. Is that
  recognised structurally, or does it lean on declared join cardinality (the
  `joins:` cardinality reuse question, `20260703-model-updates.md` §18.2)?
- **Does the join form share the union form's executor?** Both should compile to
  the one windowed-keyed-maintenance driver (`accumulating_snapshot.md` §Design
  "One windowed executor"); the join spelling is a front-end normalisation to the
  keyed-monoid-merge, not a second execution path.

## Adjacent threads (surfaced in the same discussion, tracked elsewhere)

- **Per-upstream-trigger run model (no watermark).** A daily cron knows which
  dates changed per upstream (often the last day or two); propagating that
  changed-date set through the derived per-source bound map
  (`before_secs`/`after_secs`, and `H`) yields the downstream partition run-set —
  `run_partitions(M) = { p : [p − after, p + before] ∩ changed_dates(U) ≠ ∅ }`.
  The bound map is already computed; the propagating orchestrator is unspecced.
  Belongs with `20260703-model-updates.md` §18.3 ("One orchestrator signal across
  camps"). No watermark store required — the changed-date set is a run *input*,
  not persisted state.
- **Cross-cutting ceiling guardrail.** The declare-as-assertion ceiling of
  §17.6 (bound a derived cost-driver, warn/error on exceed, never change
  execution) is not batched-specific: it bounds the batched lookback, the
  `accumulating_snapshot` horizon `H`, and keyed-mode state cardinality alike.
  It belongs in `models.md` §Design as one cross-cutting assertion, not per-mode.

## References

- **Related specs**: `accumulating_snapshot.md`, `materialized_view.md`,
  `batched_models.md`, `cumulative_aggregate.md`, `timeseries.md`, `models.md`.
- **Research**: `20260703-model-updates.md` (§13, §14.2–14.3, §17.6, §19);
  `20260522-cumulative-as-its-own-rule.md` (sibling-rule sketches);
  `20260521-incremental-as-planner-rule.md` (derive-from-SQL principle).
- **Code**: `crates/smelt-logical/src/analysis/monotonicity.rs`
  (`trace_event_time` — the event-time primitive this note's join-contribution
  classifier is a cousin of); `crates/smelt-logical/src/analysis/source_bounds.rs`
  (`after_secs` forward-reach the horizon consumes);
  `crates/smelt-backend/src/lib.rs` (`merge_into`).
