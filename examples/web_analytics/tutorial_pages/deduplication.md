# Deduplication without the workaround

The [previous page](late-data.md) built `silver.events_parsed`'s dedup the
only way a plain partition-addressed table allows: a `QUALIFY
ROW_NUMBER()` window, refused until a `safety_overrides` comment vouches
for it. That refusal was correct — smelt genuinely cannot see from the SQL
alone that a redelivered duplicate stays inside one partition. But the
override is also a debt: it's an assertion the analyzer takes on faith,
sitting apart from anything that checks it at runtime.

There's a second shape that removes the debt instead of excusing it. This
page builds it: the same dedup, with no window function, no override, and
a runtime check instead of a comment.

This stage is a complete project at
[`tutorial_stages/06_composed_dedupe/`](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics/tutorial_stages/06_composed_dedupe).

## "Partitioned or keyed" is a category error

Every incremental model so far in this series declared one shape-defining
fact: a `timeseries:` clock, which makes the output **partition-addressed**
— a complete table, rebuilt region by region. The [key-grain patterns
reference](../../reference/cumulative-aggregate.md) covers the other
single fact: a declared identity, which makes the output
**key-addressed** — one merged row per key.

These two facts are not alternatives. They're independent: a model can
declare a clock, an identity, both, or neither. Declaring **both** is a
first-class shape of its own, not "keyed with an optimization" — one row
per key, *and* that row lives in a fixed time partition. It's what this
page's model does.

## The redelivery contract, stated as data

The composed shape needs one more fact to admit a `timeseries:` block on a
keyed output: proof, or a checked declaration, that every duplicate
delivery of one key stays within a bounded window of itself on the event
axis — "key temporal locality." Here, that fact isn't provable from the
model's SQL (the duplicates arrive out of order, with nothing in the
query bounding how late a repeat can land), so it's declared where the
feed's contract actually lives: on the source.

<!-- smelt-include: tutorial_stages/06_composed_dedupe/models/sources/raw/events.yml -->

`key_recurrence` says: any two rows sharing an `event_id` are within 3
days of each other on `event_date`. Unlike the `safety_overrides` comment
on the previous page, this isn't taken on faith — smelt checks it every
run. A row that violates the bound fails the run transactionally
(`KeyedRecurrenceBoundViolated`); the target table is left exactly as it
was, never partially written.

## The model

`silver.events_deduped` declares both facts — `unique_key:` for the
identity (`grain: key` is the resulting derived label), `timeseries:` for
the clock — over the same source:

<!-- smelt-include: tutorial_stages/06_composed_dedupe/models/silver/events_deduped.sql -->

There is no `QUALIFY`, no `ROW_NUMBER()`, and no `safety_overrides` block.
Dedup instead falls out of the merge itself: every non-key column folds
through `MIN`. A redelivered duplicate is identical to the original except
its arrival time — which isn't selected — so `MIN` over any other column
converges to the same value no matter which copy of the row a run
happens to see. There's nothing left to order.

## What smelt derives

Ask smelt what it makes of this model:

```bash
smelt explain silver.events_deduped
```

<!-- smelt-generate: @cwd=tutorial_stages/06_composed_dedupe @render=text explain silver.events_deduped -->

Two things the previous page's refusal didn't have:

- **`Refusals: (none)`.** Compare this to `silver.events_parsed`'s bare
  `QUALIFY` version, which `smelt explain`/`--dry-run` refuses outright.
  Nothing here needs excusing.
- **A `Key temporal locality` block**, naming the established route —
  `route 3 (recurrence-bounded, declared key_recurrence)` — and the
  derived **settle bound**: how long a written slice can still change
  before it's final. Route 3's bound is the declared window plus margins;
  it's what a downstream consumer would wait for before treating a slice
  as done.

## Running it

Where the QUALIFY version's `--dry-run` exits non-zero with a refusal,
this model's runs clean:

<!-- smelt-generate: @cwd=tutorial_stages/06_composed_dedupe @render=text run --select silver.events_deduped --event-time-start 2026-04-10 --event-time-end 2026-04-11 --dry-run -->

## The statements smelt emits

The two shapes don't just differ in what's declared — they differ in what
runs. A plain partition-grain model always emits `DELETE`+`INSERT`
(the [first page](first-model.md)); this one emits a keyed `MERGE` for an
ordinary incremental run, and only falls back to `DELETE`+`INSERT` for an
explicit backfill of a whole region:

<!-- smelt-generate: @cwd=tutorial_stages/06_composed_dedupe explain silver.events_deduped --show-sql --json --period 2026-04-10..2026-04-11 -->

The `MERGE`'s `WHEN MATCHED` clause is exactly the `MIN` fold from the
model's SELECT, rendered as `LEAST(target.c, delta.c)` per column — the
same combiner lookup the [key-grain patterns
reference](../../reference/cumulative-aggregate.md) documents, just now
also feeding a time-partitioned output. The locality route above is what
*licenses* pruning the merge's target read to the derived slice instead of
scanning every stored key — on DuckDB the predicate isn't emitted onto
this `MERGE` yet (a backend binder limitation, not a correctness gap:
every delta row merges correctly either way). Every scanned delta row
still merges regardless — locality prunes the target read, it never
narrows which delta rows get written.

## What declaring both facts buys

A key-addressed model with no clock is a dead end for the rest of the
pipeline: nothing downstream can window over it, and it can't sit inside
a chain of partition-by-partition runs. Declaring the clock alongside the
identity changes that — this table is now a clocked source in its own
right, exactly like `bronze.raw_events`, and a keyed stage can sit
*inside* a propagation chain instead of terminating it. A change to one
key's rows also now has a bounded home downstream instead of an unbounded
one: under routes 1 and 2 (partition value fixed per key) a key-level
change projects to its exact own partition; under route 3 — this model's
route — it projects to the touched partitions widened backward by the
declared recurrence window, still a bounded interval rather than a scan
of the whole table.

A third capability — skipping the merge's write entirely when a key's
folded state didn't actually change — needs to read the stored rows to
compare against, and only a bounded target scan makes that comparison
cheap enough to be worth doing. That machinery isn't wired up yet; the
locality this page establishes is what will make it affordable when it
lands.

If you've built this in other tools: dbt and SQLMesh both treat "dedup a
key" and "partition a table" as separate concerns you reach for
independently — there's no single model shape where declaring an identity
*and* a clock together changes what the engine is willing to prove about
either one. Here they compose, and the composition is what removes the
override.

Continue to [Sessions and the cross-midnight backfill](sessions.md), which
picks the story back up with `silver.events_parsed` — sessions build on
the plain partition-grain shape this series introduces first; the
composed shape above is this pipeline's dedup stage, not a replacement
for every downstream model.
