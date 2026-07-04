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
  existence flag; a `LEFT JOIN` to a dimension feeding only monoid milestones
  (`MIN`/`MAX`/`MIN_BY`/`MAX_BY`/first-non-null). The defining property is that a
  new row folds in from `(current state, new row)` alone — the combiner needs
  **no inverse**, so no prior row is ever re-read. `merge_into` suffices.
- **Retractable contribution → needs a group/retraction → delegate.** A
  `COUNT(conversions)` per event that must *decrement* when a conversion is
  removed; a value that must be *un-seen* because the element it was folded from
  was corrected or deleted. Un-seeing a contribution needs the underlying
  multiset (the group rung) — the slice properly routed to engine-native IVM via
  `refresh: materialized_view`.

### 3.1 Two senses of "monotone" — the value may switch

"Monotone" above is easy to over-read as "the reported value never changes."
That is one case but not the boundary. Two distinct properties hide under the
word, and **both** are MERGE-maintainable with no source re-read:

- **Value-monotone.** The reported value only ever moves one way: `NULL → set`,
  never revised (`MIN`, `MAX`, `COALESCE`/first-non-null, an `EXISTS` flag).
- **Order-monotone (a semilattice fold).** The *merge state* advances
  monotonically along an **ordering key** while the *reported value rides along
  and may switch repeatedly* (`MAX_BY(value, conv_ts)` — the last-update-in-window
  case). The value flips A → B → C as later conversions land; what never happens
  is un-seeing an element.

The real discriminant is therefore **needs-no-inverse (semilattice fold) vs
needs-an-inverse (group)** — *not* "the value is stable." A milestone that
transitions set → different-set is still safe when the new value **supersedes**
via the monoid's own order (`MAX_BY` keeps the larger `conv_ts`); it is unsafe
only when the transition **replaces a previously-folded element that is now
wrong** (a correction or a deletion), which is what actually needs the multiset.

**Two constraints keep the order-monotone case broadly usable** (and they are
reasonable ones to ask of a modeller):

- **The ordering key is a data attribute, not arrival order.** `MAX_BY` must be
  ordered by a *logical timestamp on the dimension side* (`conv_ts`), never by
  processing/ingestion order. Ordered by a data column the combiner is
  commutative — replaying the same rows in any order yields the same answer, which
  is exactly what §7.1's idempotent re-scan mode relies on. Ordered by arrival it
  is **not** commutative, silently breaking re-scan (and leaving delta mode
  fragile).
- **The dimension is append-only** (§5). With no deletes, the current winner can
  never be pulled out from under the fold.

The classifier's job is therefore to **prove the join's contribution folds
without an inverse** (value-monotone *or* order-monotone under a data ordering
key), not to reject all joins or all value-switching.
`AccumulatingSnapshotJoinExpressedEnrichment` should fire only when the join
feeds a genuinely retractable milestone or fans out into an aggregate that must
decrement — not merely because the enrichment was written with a `JOIN` keyword,
and not merely because the value changes more than once.

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
  the fact (`events`), full-scan the dimension (`conversions`). This is the
  *event-driven / initialisation* read pattern; the conversion-driven trigger
  inverts it, reading the already-materialised target in place of the fact (§7).
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
proof that a semi-join / dimension-join feeds only inverse-free monoid milestones
(value- or order-monotone; §3.1) and does not fan out into a decrementing
aggregate. This is a cousin of the existing
structural monotonicity primitive (`trace_event_time`,
`crates/smelt-logical/src/analysis/monotonicity.rs`) — monotonicity of a *join
contribution* rather than of an *event-time transform* — and should live beside
it.

## 5. The mutation-profile dependency

Monotonicity of the enrichment rests on one world-fact smelt cannot derive
(§17.6): the dimension's **mutation profile**. If a conversion row can be
*deleted* from the (no-CDF) conversion table, then:

- A **value-monotone** `MIN`/`MAX`/first-non-null milestone is still safe —
  losing a row cannot un-fill an already-set column (the monoid has no inverse,
  so it simply never retracts).
- A **re-scanned existence flag** (`EXISTS`) is *not* safe — re-evaluating the
  semi-join after a delete would flip `converted` true → false, breaking
  monotonicity.
- An **order-monotone `MAX_BY`** (§3.1, the last-update-in-window case) is safe
  against deleting *any row except the current winner*, but **not** against
  deleting the currently-winning row: with the top `conv_ts` gone, "latest" must
  fall back to the second-latest, which needs the multiset the fold discarded.
  So `MAX_BY` is the case where the deletion gate bites most specifically — it is
  fatal only for the one row whose contribution is currently reported.

So admissibility is: **the dimension is declared append-only** (deletes never
happen) → the value-monotone milestone, the order-monotone `MAX_BY`, and the
existence-flag forms are all maintainable; **the dimension is mutable** → only
the value-monotone extremal milestone survives (losing any row cannot un-fill
it), while the `MAX_BY` and existence-flag forms must either be re-modelled as a
value-monotone milestone (`converted_at = MIN(conv_ts)`, `converted :=
converted_at IS NOT NULL` downstream) or delegated.
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

§7 completes this with the missing *cost* argument: the conversion-driven
rewrite makes smelt-driven MERGE not merely recoverable but cheaper than a full
fact scan, so the escape hatch is no longer bought at a performance premium.

## 7. The conversion-driven update: the target is the fact replica

§4's read pattern — window the fact `events`, full-scan the dimension
`conversions` — is the *initialisation* pattern: what an **event-driven**
trigger does when it must birth new target rows. It is the wrong pattern, and an
unnecessary full-fact cost, for the **conversion-driven** trigger. The
accumulating_snapshot target already materialises `(event_id, occurred_at,
converted_at)` for every in-scope event — a narrowed, keyed replica of exactly
the fact columns the enrichment consumes: the join key, the event-time the
horizon predicate ranges over, and the current milestone the monoid folds into.
So a new conversions batch merges straight into the target, and **`events` is
never read on a conversion trigger.**

The read set is therefore *upstream-conditional* — the one new object the
executor needs over batched, where the target is write-only:

| Trigger (upstream that advanced) | Reads | Writes | Never reads |
|---|---|---|---|
| New events `[e0, e1]` | events window; conversion window `[e0, e1 + H]` to seed | INSERT target rows | events history, target history |
| New conversions `[c0, c1]` | conversions window; **target slice `occurred_at ∈ [c0 − H, c1]`** | UPDATE via monoid MERGE | **`events` entirely** |

Both sides are bounded, so maintenance is genuinely incremental. The
conversion-trigger target read is **horizon-bounded**: a conversion at `conv_ts`
can only match an event with `occurred_at ∈ [conv_ts − H, conv_ts]`, so the
MERGE touches a window of the target, not the whole accumulated table.

The MERGE, pre-aggregating the conversion batch per key so it stays 1:1 on the
target side (legal *because* the milestone monoid is commutative / associative /
idempotent — the §1 property reused to make the physical MERGE valid):

```sql
MERGE INTO target t
USING (
    SELECT event_id, MIN(conv_ts) AS conv_ts
    FROM conversions_window
    GROUP BY event_id
) c
  ON  t.event_id = c.event_id
  AND c.conv_ts >= t.occurred_at
  AND c.conv_ts <  t.occurred_at + INTERVAL H
WHEN MATCHED AND (t.converted_at IS NULL OR c.conv_ts < t.converted_at)
  THEN UPDATE SET converted_at = c.conv_ts, converted = TRUE
```

### 7.1 Two maintenance modes: delta-driven vs idempotent re-scan

The out-of-order case — a conversion arriving before its event is materialised —
has two admissible resolutions, **both preserving the invariant that the full
fact `events` is never re-read**:

- **Delta-driven symmetric probe.** Use the per-run changed set on each side:
  event-insert also probes the conversion window `[occurred, occurred + H]`;
  conversion-insert merges the horizon-bounded target slice. Every (event,
  conversion) pair is caught on whichever side arrives *second*, against the
  other side's already-materialised replica (target for conversions, the
  conversion window for events). Minimal read, but depends on change-tracking
  (CDF or a watermark) to know the deltas.
- **Idempotent window re-scan (no dimension CDF).** Re-scan the *entire*
  conversion window for the affected period and merge unconditionally. Safe
  **because the monoid is idempotent**: re-folding a conversion already applied
  (`LEAST`/`MIN`) converges to the same value, so double-application is harmless.
  This drops the change-feed dependency on the dimension altogether *and*
  dissolves the out-of-order problem for free — an early conversion is picked up
  whenever its event's window is (re)scanned. The cost is re-reading the bounded
  conversion window each run rather than only its delta.

The two are operating points on a **CDF-dependency-vs-re-read-cost** trade,
selectable per model. Idempotency is what makes the CDF-free point admissible at
all — the same property that lets the "no watermark required" run model
(Adjacent threads) treat the changed-date window as a run *input* rather than
persisted per-row delta state.

## 8. The same rewrite is a targeted column backfill on model change

The target-as-fact-replica identity is not specific to a recurring conversion
trigger. It applies to a one-off trigger the maintained family does not yet
exploit: **a change to the model's own definition.** Today, editing a model's
SQL to add a column forces a full `--full-refresh` rebuild. But the target
already holds `(key, event_time, existing columns)`, so an **additive** diff can
be backfilled in place — reading the target, and at most a bounded dimension
window, never a full fact rebuild:

| Model diff | Backfill read plan | Cost |
|---|---|---|
| Add **computed column** over columns already in the target (`gross = net + tax`) | in-place `UPDATE target SET newcol = <expr over existing columns>` | O(target), no source read |
| Add **monotone join / enrichment column** (a new once-write milestone from a dimension) | merge dimension → target over the horizon window — *identical to §7's conversion path* | O(dim window + target slice), **no fact read** |
| Add a column needing a **projected-away fact column** | keyed re-read of that one fact column → merge | O(fact) for one column, not a semantic rebuild |
| **Change / retract** existing column semantics | full rebuild or delegate (retractable — §3) | — |

The dividing line is the one the whole note is built on: monotone-vs-retractable,
plus "is the input already in the target." A diff that only **adds** columns
derivable from `{existing target columns} ∪ {a monotone dimension contribution}`
is a **targeted backfill, not a full rebuild**, and it reuses §7's read-set
machinery verbatim — "the model definition changed" is just another trigger
whose read plan is *target + bounded dimension window, never the full fact.*

This is the schema-evolution face of §6's escape-hatch argument. A full rebuild
on every model change is exactly the opaque, all-or-nothing cost that makes large
maintained tables painful to evolve; a targeted, column-scoped backfill keeps the
change legible and recoverable — the same reason smelt-driven MERGE beats native
IVM for the recurring case makes it beat a full rebuild for the schema-change one.

## 9. Recommendation

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
- **Make the executor's read plan upstream-conditional and mode-selectable.**
  The event-driven trigger initialises from the fact window; the conversion-driven
  trigger merges into the horizon-bounded target slice without reading the fact
  (§7); the dimension read is either delta-driven or an idempotent window re-scan
  (§7.1). Treat targeted column backfill (§8) as a first-class trigger on this
  same executor.

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
- **Where is the static/declared line for an "additive-only" diff?** Recognising
  that a model edit only *adds* columns derivable from existing target columns
  plus a monotone dimension contribution — versus one that silently changes an
  existing column's semantics — is a structural diff over the model's column set
  and dependency graph (§8). How much is provable from the SQL diff alone, and
  how much needs a declared migration intent (as the mutation profile §5 is
  declared)?
- **Delta-driven vs re-scan: a per-model knob or a derived choice?** §7.1's two
  modes trade a CDF dependency against a bounded re-read. Is the mode declared,
  or derived from whether the dimension source exposes a change feed?

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
