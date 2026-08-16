# Backbuild Synthesis

You changed a model. Maybe you fixed a bug in one column's expression, renamed a
field, added an enrichment join, or extended the history window. The table built
from that model is 10 TB. What happens next?

In most transformation frameworks — and in smelt, before backbuild synthesis —
the answer is polar. Either the change happens to fit a narrow special case
(for example, appending a purely additive column to an
[incremental model](incremental-models.md)), or the whole table is rebuilt from
scratch. A one-line fix to one column recomputes ten terabytes.

Between those poles sits a large class of edits whose effect on the deployed
table is reachable by a **targeted script** — an `ALTER`, a column-scoped
`UPDATE`, a predicate-scoped `DELETE` or `INSERT` — far cheaper than
recomputing the table. Backbuild synthesis finds those scripts automatically:
it emits one only when fail-closed structural conditions hold, and every
technique is verified against a full-rebuild oracle in smelt's conformance
suite.

!!! note "Naming: two things called “backbuild”"

    [`smelt rebuild`](../reference/cli.md#smelt-rebuild) is the CLI command
    that re-runs a model (and its upstreams) over a date range — reprocessing
    *data* under an unchanged definition. **Backbuild synthesis**, this page, is
    about *definition* changes: deriving a migration script from the diff
    between two versions of a model's SQL. The two share a goal (avoid
    recomputing what you don't have to) but operate on different inputs.

!!! warning "Availability"

    There is no CLI command for this yet — today you cannot invoke synthesis on
    your own project. The classifier, script emitters, and every technique on
    this page are complete and verified against a DuckDB oracle in the
    conformance suite
    (`crates/smelt-logical/tests/backbuild_conformance.rs`); what remains is
    the surface on top: sourcing the "before" definition, and choosing and
    executing an option. Emitted scripts are DuckDB dialect.

## The idea in one example

Say this model is deployed as a table with millions of rows:

<!-- backbuild-example(intro): before -->
```sql
-- before
SELECT id, amount, rate, amount AS amount_usd FROM orders
```

You spot the bug: `amount_usd` was never converted. You fix it:

<!-- backbuild-example(intro): after -->
```sql
-- after
SELECT id, amount, rate, amount * rate AS amount_usd FROM orders
```

A full rebuild recomputes every column of every row. Backbuild synthesis diffs
the two definitions, sees that exactly one output column's expression changed,
checks that the new expression is computable from columns the table *already
stores* (`amount` and `rate` pass through from the input unchanged), and emits:

<!-- backbuild-example(intro): script -->
```sql
UPDATE t SET amount_usd = amount * rate
```

One column written, zero upstream reads, siblings untouched. The conformance
suite asserts the result is row-for-row identical to a fresh rebuild — and that
`id`, `amount`, and `rate` are byte-identical before and after.

Throughout this page, `t` stands for the model's deployed table.

## How it works

Given the **before** and **after** definitions (plus a few declared physical
facts, like which upstream columns form a unique key), backbuild synthesis:

1. **Factors the diff into atomic changes.** The SELECT list is diffed per
   output column (added / dropped / changed / unchanged); the WHERE clause is
   diffed as a set of top-level `AND` conjuncts; the FROM/JOIN tree, GROUP BY,
   and set operations are compared structurally. Formatting and comment changes
   compare equal — a pure reformat is a no-op.

2. **Enumerates every provable technique per atom** — an *option set*, not a
   single verdict. A changed expression derivable both from stored columns and
   from an upstream read yields both scripts. `FullRefresh`
   (`CREATE OR REPLACE TABLE t AS <after>`) is always in the model's option
   set, so targeted scripts always have the rebuild to be compared against.
   Each option carries the metadata a cost decision needs: write scope,
   whether it reads upstream, statement count, and whether it is safe to
   re-run.

3. **Refuses, by name, anything it cannot prove.** There is no silent
   fallback. A refusal states which atom failed and why — and where a small
   model edit or declaration would fix it, the message says which one. If any
   atom has no admissible technique, no partial script is offered: full
   refresh remains the model's only option, with the refusals explaining why.

### The guarantee

Every emitted script comes with the same equivalence promise:

> If the deployed table is up to date with its inputs under the old definition,
> then after the script runs, the table equals what a full rebuild under the
> new definition would produce — the same multiset of rows, columns matched by
> name and type.

The precondition matters. Scripts that read only the deployed table (renames,
stored-column updates, filter-tightening deletes) stay coherent even against a
stale table. Scripts that read upstream (pull-throughs, join enrichments,
difference inserts) bake in *current* upstream state — run them against a stale
table and the touched columns reflect fresh data while untouched siblings
don't. The conformance suite includes a stale-input case that demonstrates
this edge rather than just stating it.

## A tour of what smelt can migrate

Each example below is a real case from the conformance suite: the before/after
SQL and the emitted script are verified against a DuckDB oracle — build the
table from *before*, apply the script, and assert multiset equality with a
fresh build from *after*.

### Rename a column

<!-- backbuild-example(rename): before -->
<!-- backbuild-example(rename): after -->
```sql
-- before
SELECT id, amount FROM orders
-- after
SELECT id, amount AS total FROM orders
```

Detected as a dropped column and an added column with an identical expression —
a rename, not a drop-plus-add:

<!-- backbuild-example(rename): script -->
```sql
ALTER TABLE t RENAME COLUMN amount TO total
```

Zero rows touched. If two dropped columns had identical expressions the match
would be ambiguous, and smelt refuses rather than guessing.

### Add a column computed from stored columns

<!-- backbuild-example(add_stored): before -->
<!-- backbuild-example(add_stored): after -->
```sql
-- before
SELECT id, price, qty FROM orders
-- after
SELECT id, price, qty, price * qty AS total FROM orders
```

Every input of the new expression is already stored, so no upstream read is
needed:

<!-- backbuild-example(add_stored): script -->
```sql
ALTER TABLE t ADD COLUMN total INTEGER;
UPDATE t SET total = price * qty;
```

(The added column's type comes from smelt's type inference over the new
expression.)

### Fix a column's logic in place

The example from the top of the page — and the highest-value case in practice:
"fix a bug in one column of a huge table."

<!-- backbuild-example(fix_in_place): before -->
<!-- backbuild-example(fix_in_place): after -->
```sql
-- before
SELECT id, amount, rate, amount AS amount_usd FROM orders
-- after
SELECT id, amount, rate, amount * rate AS amount_usd FROM orders
```

<!-- backbuild-example(fix_in_place): script -->
```sql
UPDATE t SET amount_usd = amount * rate
```

The proof obligation is subtle: the new expression is defined over the model's
*inputs*, so every input needs a **stored representative** — an unchanged, bare
pull-through in the model's own output, matched by both qualifier and source
column, never merely by a coinciding output name. A changed column is never a
representative of anything (otherwise a mutual swap like
`x AS a, y AS b` → `y AS a, x AS b` would emit self-invalidating updates), so
if the fixed expression depends on a sibling that changed in the same edit,
the atom refuses with a message naming that sibling.

### Pull a column through from an upstream

<!-- backbuild-example(pullthrough): before -->
<!-- backbuild-example(pullthrough): after -->
```sql
-- before
SELECT o.order_id AS order_id, o.customer AS customer FROM orders o
-- after
SELECT o.order_id AS order_id, o.customer AS customer, o.discount AS discount FROM orders o
```

The new column comes from an upstream already in the FROM tree. Because the
model stores a 1:1 pull-through of that upstream's
[declared `unique_key`](../reference/sources-yml.md) (`order_id`), each stored
row can be addressed and enriched:

<!-- backbuild-example(pullthrough): script -->
```sql
ALTER TABLE t ADD COLUMN discount INTEGER;
UPDATE t SET discount = u.discount FROM orders u WHERE t.order_id = u.order_id;
```

Rows the model's WHERE clause filtered out simply never match — the join
touches only rows the table already has.

### Add an aggregate at the model's own grain

<!-- backbuild-example(aggregate): before -->
<!-- backbuild-example(aggregate): after -->
```sql
-- before
SELECT o.customer_id AS customer_id, count(*) AS n
FROM orders o WHERE o.qty > 0 GROUP BY o.customer_id
-- after
SELECT o.customer_id AS customer_id, count(*) AS n, SUM(o.qty) AS total_qty
FROM orders o WHERE o.qty > 0 GROUP BY o.customer_id
```

The new column is a recognized aggregate call, the `GROUP BY` grain is
unchanged, and the grouping key (`customer_id`) is a stored, `NOT NULL`
pull-through — so each stored group can be re-derived and matched by key:

<!-- backbuild-example(aggregate): script -->
```sql
ALTER TABLE t ADD COLUMN total_qty HUGEINT;
UPDATE t SET total_qty = s.total_qty
FROM (
  SELECT o.customer_id AS customer_id, SUM(o.qty) AS total_qty
  FROM orders o WHERE o.qty > 0 GROUP BY o.customer_id
) s
WHERE t.customer_id = s.customer_id;
```

The re-aggregation carries the model's `WHERE` clause verbatim — a bare
`SELECT <keys>, <agg> FROM <upstream> GROUP BY <keys>` would over-count rows
the model's own filter drops. There is no insert arm: a key group missing
from `t` is one the model's own row-set already proves cannot exist, so the
backfill only ever updates matched keys.

### Enrich via a new LEFT JOIN

<!-- backbuild-example(left_join_enrich): before -->
<!-- backbuild-example(left_join_enrich): after -->
```sql
-- before
SELECT o.order_id AS order_id, o.customer_id AS customer_id FROM orders o
-- after
SELECT o.order_id AS order_id, o.customer_id AS customer_id,
       c.customer_name AS customer_name
FROM orders o LEFT JOIN customers c ON o.customer_id = c.customer_id
```

Before emitting anything, smelt checks the added join **cannot change the row
set**: it is a LEFT JOIN (never removes rows), the join key has a declared
`unique_key` on the dimension side, the key columns are provably `NOT NULL`,
and nothing outside the added columns references the new alias. With that
established, a bare pull-through admits *two* independently verified scripts:

<!-- backbuild-example(left_join_enrich): script -->
```sql
-- option 1: update-from
ALTER TABLE t ADD COLUMN customer_name TEXT;
UPDATE t SET customer_name = c.customer_name
FROM customers c WHERE t.customer_id = c.customer_id;

-- option 2: scalar subquery
ALTER TABLE t ADD COLUMN customer_name TEXT;
UPDATE t SET customer_name =
  (SELECT c.customer_name FROM customers c WHERE t.customer_id = c.customer_id);
```

In both, an unmatched row keeps the `NULL` it got from `ALTER ADD` — exactly
LEFT-JOIN semantics. The shapes differ under a *wrong* uniqueness declaration:
`UPDATE … FROM` silently picks one duplicate match, while the scalar subquery
errors loudly — a free runtime uniqueness probe.

Wrap the dimension column in an expression —
`COALESCE(c.customer_name, 'none') AS customer_label` — and only the subquery
shape survives, with the substitution applied **per column reference**, not
around the whole expression:

<!-- backbuild-example(left_join_wrapped): script -->
```sql
ALTER TABLE t ADD COLUMN customer_label TEXT;
UPDATE t SET customer_label =
  COALESCE((SELECT c.customer_name FROM customers c
            WHERE t.customer_id = c.customer_id), 'none')
```

Wrapping the whole expression in one subquery would be wrong: for an unmatched
row the subquery returns zero rows, the whole scalar becomes `NULL`, and the
`COALESCE` never fires. Per-reference substitution makes each dimension
reference evaluate to `NULL` exactly as LEFT-JOIN null-extension would. The
oracle test pins this: an unmatched order ends up `'none'`, not `NULL`.

### Chain multiple joins

<!-- backbuild-example(chain_joins): before -->
<!-- backbuild-example(chain_joins): after -->
```sql
-- before
SELECT o.order_id AS order_id, o.dim1_id AS dim1_id FROM orders o
-- after
SELECT o.order_id AS order_id, o.dim1_id AS dim1_id,
       d1.region_id AS region_id, d2.region_name AS region_name
FROM orders o LEFT JOIN dim1 d1 ON o.dim1_id = d1.dim1_id
              LEFT JOIN dim2 d2 ON d1.region_id = d2.region_id
```

The second join keys on a column the *first* join provides. That is fine —
provided `region_id` is stored as a bare pull-through in the added output, so
it exists by the time step two runs. The emitted script backfills `dim1`'s
columns first, then `dim2`'s, in dependency order. Bareness is load-bearing: a
wrapped carrier like `COALESCE(d1.region_id, 0) AS region_id` would store `0`
where a rebuild has `NULL`, and the second join would then hit dimension rows
the rebuild's `NULL` key misses — so that shape refuses.

### Add a window column

<!-- backbuild-example(window): before -->
<!-- backbuild-example(window): after -->
```sql
-- before
SELECT o.order_id AS order_id, o.status AS status, o.amount AS amount FROM orders o
-- after
SELECT o.order_id AS order_id, o.status AS status, o.amount AS amount,
       ROW_NUMBER() OVER (PARTITION BY status ORDER BY order_id) AS rn
FROM orders o
```

The window's `PARTITION BY` and `ORDER BY` reference only stored, bare
columns, and the model declares a `NOT NULL` row identity (`order_id`). smelt
backfills it with a **self-read**: the source subquery reads the deployed
table itself, not the upstream, so the window computes over exactly the rows
`t` already has — matching a rebuild by construction, even when the model's
own `WHERE` has filtered rows out along the way.

<!-- backbuild-example(window): script -->
```sql
ALTER TABLE t ADD COLUMN rn BIGINT;
UPDATE t SET rn = s.rn
FROM (
  SELECT order_id, ROW_NUMBER() OVER (PARTITION BY status ORDER BY order_id) AS rn
  FROM t
) s
WHERE t.order_id = s.order_id;
```

An `OVER` clause with no `ORDER BY` refuses outright: within a partition, a
rank-family function's row order is whatever the engine happens to produce —
different each run, and never provably the same draw a rebuild would take. A
window whose `PARTITION BY`/`ORDER BY`/arguments reach outside the model's own
stored columns refuses too, the same uniform-representative rule every other
backfill uses — add the missing column to the `SELECT` list to make it
backfillable.

### Tighten a filter

<!-- backbuild-example(tighten): before -->
<!-- backbuild-example(tighten): after -->
```sql
-- before
SELECT id, status FROM orders
-- after
SELECT id, status FROM orders WHERE status = 'active'
```

The added conjunct is evaluable over stored columns, so the difference is a
pure delete — no upstream read at all:

<!-- backbuild-example(tighten): script -->
```sql
DELETE FROM t WHERE (status = 'active') IS NOT TRUE
```

Note `IS NOT TRUE`, not `NOT`. SQL's `WHERE p` keeps rows where `p` is TRUE;
rows where the predicate is `NULL` are dropped too. Given
`amount > 0` over rows with amounts `10`, `-5`, and `NULL`, a rebuild keeps
only the first — and `DELETE … WHERE (amount > 0) IS NOT TRUE` removes both
the negative *and* the `NULL` row, where a bare
`DELETE … WHERE NOT (amount > 0)` would wrongly keep the `NULL` row. The
conformance suite pins exactly this case.

### Extend the history window

The classic "backfill more history":

<!-- backbuild-example(extend_history): before -->
<!-- backbuild-example(extend_history): after -->
```sql
-- before
SELECT ts, amount FROM events WHERE ts >= '2025-01-01'
-- after
SELECT ts, amount FROM events WHERE ts >= '2024-01-01'
```

The difference — rows the old predicate excluded and the new one admits — is
inserted from the after-definition:

<!-- backbuild-example(extend_history): script -->
```sql
INSERT INTO t (amount, ts)
SELECT amount, ts
FROM (SELECT ts, amount FROM events WHERE ts >= '2024-01-01') AS __backbuild_diff
WHERE (ts >= '2025-01-01') IS NOT TRUE
```

Classification requires a provable widening: same column, same comparison
operator, and a literal that strictly widens the range. `ts >= X` → `ts >= Y`
with `Y > X` is a *narrowing* and refuses (an INSERT can't remove rows); mixed
operators like `>` → `>=` refuse rather than trusting literal arithmetic at
the boundary.

When the model has a declared row identity (its
[`unique_key`](../reference/smelt-yml.md#incremental-configuration)) whose
columns are provably `NOT NULL`, the insert also carries an anti-join guard
(`AND NOT EXISTS (SELECT 1 FROM t WHERE t.id = __backbuild_diff.id)`) making
it safe to re-run. Without one, the option is honestly flagged one-shot
(`rerun_safe: false`) rather than refused — the risk is stated, not hidden.

### Loosen or reshape a filter

Removing a conjunct works the same way in reverse — the previously-excluded
slice is inserted from the after-definition. A change that both adds and
removes conjuncts can compose the two — delete the newly-excluded rows, insert
the newly-admitted ones — but only under strict conditions: one removed
conjunct, touching columns disjoint from every added conjunct's (when they
overlap, the delete and the insert would interact, so only the provable
range-widening shape above is admitted). A predicate rewritten in a way that
doesn't factor into added and removed top-level `AND` conjuncts refuses — the
conjunct algebra is deliberately syntactic, not a general implication prover.

### Add a UNION ALL branch

<!-- backbuild-example(union_add): before -->
<!-- backbuild-example(union_add): after -->
```sql
-- before
SELECT id, 'a' AS kind FROM events_a
-- after
SELECT id, 'a' AS kind FROM events_a
UNION ALL
SELECT id, 'b' AS kind FROM events_b
```

`UNION ALL` is additive, so the new branch is exactly the delta:

<!-- backbuild-example(union_add): script -->
```sql
INSERT INTO t (id, kind)
SELECT id, kind FROM (SELECT id, 'b' AS kind FROM events_b) AS __backbuild_branch
```

Plain `UNION` deduplicates across branches, so it refuses. One more proof rides
along: `UNION ALL` binds columns *positionally*, while this INSERT is
name-based — so the added branch's declared column names must match the first
branch's order exactly. A branch declaring `SELECT kind, id` against a first
branch declaring `SELECT id, kind` would silently swap values under a rebuild's
positional binding, and refuses here by name.

### Remove a UNION ALL branch

<!-- backbuild-example(union_remove): before -->
<!-- backbuild-example(union_remove): after -->
```sql
-- before
SELECT id, 'a' AS src FROM events_a
UNION ALL
SELECT id, 'b' AS src FROM events_b
-- after
SELECT id, 'a' AS src FROM events_a
```

A removed branch needs a **discriminator**: a column that is a distinct
literal constant in every branch of the before-definition (here, `src`). With
one, the removed branch's own constant becomes an equality delete:

<!-- backbuild-example(union_remove): script -->
```sql
DELETE FROM t WHERE src = 'b'
```

This is an equality predicate, not the `IS NOT TRUE` complement form
[Tighten a filter](#tighten-a-filter) uses — the discriminator is proven to be
a non-NULL literal that lands on exactly the removed branch's rows, so there's
no NULL-evaluation case to guard against. That proof matters even when the
removed branch's other columns happen to coincide with a surviving branch's —
two branches unioning the same `id` values under different `src` constants
still delete only the rows the removed branch actually contributed, because
the predicate keys on the discriminator, not the payload.

Without a discriminator, branch removal refuses with an actionable nudge:

> no provenance predicate distinguishes the removed branch's rows in the
> stored table — add a constant discriminator column (e.g. a literal `AS src`
> value distinct per branch) to make branch removal targetable

The same proof also refuses when a candidate column isn't a constant in every
branch (a non-literal expression in even one branch means the predicate can't
be proven to hold everywhere it needs to), or when two branches share the same
constant (the resulting predicate would delete a surviving branch's rows too).
Plain `UNION` refuses here for the same reason it does on the add side.

### Several changes at once

Atomic changes compose. Rename a column, add a derived one, tighten the
filter, and drop an unrelated column, all in a single edit:

<!-- backbuild-example(composite): before -->
<!-- backbuild-example(composite): after -->
```sql
-- before
SELECT id, price, extra, qty FROM orders
-- after
SELECT id, price AS unit_price, price AS list_price, qty FROM orders WHERE qty > 0
```

The assembled script (asserted verbatim in the conformance suite):

<!-- backbuild-example(composite): script -->
```sql
ALTER TABLE t RENAME COLUMN price TO list_price;
ALTER TABLE t ADD COLUMN unit_price INTEGER;
UPDATE t SET unit_price = list_price;
DELETE FROM t WHERE (qty > 0) IS NOT TRUE;
ALTER TABLE t DROP COLUMN extra;
```

Statements run in a fixed dependency order — renames first (so later
expressions reference final names), then each added column's `ALTER ADD` with
its backfill, deletes, remaining column updates (so they touch fewer rows),
inserts, and dropped columns **last**, strictly after every statement that
might still read them. Notice `unit_price`'s update reads `list_price`, the
rename's *target* name — that is why renames go first. And note the
composition rule: a targeted script is
offered only when **every** atom in the edit has at least one admissible
technique. One unprovable atom means full refresh is the only option, with the
refusal naming the culprit — partial migration is never offered.

Dropping a column discards data irreversibly, so backbuild only ever
*sequences* the `ALTER ... DROP COLUMN` statement into the right place in the
script — it never decides on its own whether the drop is allowed to run.
Column removal stays an explicit opt-in at run time (see [schema
evolution](schema-evolution.md)), applied independently of this ordering.

## When smelt refuses

Refusals are first-class output, not error noise. Three flavors:

**Structural refusals** — changes that are effectively a different model. A
grain change (GROUP BY keys added or removed, `DISTINCT` toggled, dedup
ordering changed) refuses: no in-place script can merge or split stored rows
the way a rebuild would. So does a join-multiplicity change (`INNER` ↔ `LEFT`,
an edited join condition): whether it changes the row set depends on the data,
not the definitions.

**Conservatism refusals** — things a smarter analysis might admit someday, kept
fail-closed today: a changed CTE, a `SELECT *` or
[spread expression](../meta-language/lists.md) in the select list, a top-level
`OR` in a diffed WHERE clause, volatile or unrecognised functions in an added
or changed expression (a volatile backfill could never match a rebuild),
expression changes under `DISTINCT` or `LIMIT`, and an added window column
whose `OVER` clause has no `ORDER BY` — an underdetermined draw within a
partition can never be proven equal to a rebuild's own draw.

**Actionable refusals** — the most useful kind: the missing fact is something
you can supply. Real messages from the classifier:

> the added join's key column 'region_id' has no stored bare representative in
> the model's own output — a join keyed on a column the model does not store is
> unaddressable; **add it to the SELECT list to make this backfillable**

> upstream 'c' has no declared unique_key — an equality backfill needs an
> addressable identity

> depends on an unqualified column reference — the FROM-tree alias it reads
> from cannot be determined; **qualify it (e.g. `o.col`)**

A small edit to the model — storing a join key, qualifying a column — often
converts a full rebuild into a column-scoped update.

## Why you can trust the scripts

Every SQL snippet on this page — before, after, and emitted script — is
generated by smelt's own `definition_diff` → `derive_backbuild_options` →
`assemble` pipeline from the shown before/after definitions, byte-compared
against this page's own text, and oracle-verified against a real DuckDB by a
standing test suite (`crates/smelt-logical/tests/backbuild_docs.rs`); the
page cannot drift from what smelt actually emits.

- **Oracle-verified equivalence.** Every technique is tested the hard way:
  build the table from the before-definition, run the emitted script, build a
  fresh table from the after-definition over the same inputs, and assert the
  two are equal as multisets (both directions of `EXCEPT ALL`, plus column
  names and types). Every option a case admits is verified independently.
- **Fail-closed proofs.** Key-addressed techniques require join keys provably
  `NOT NULL` — SQL `UNIQUE` admits NULLs, and an equality join silently skips
  a NULL-keyed row, so declared uniqueness alone is not enough.
- **Three-valued logic throughout.** Complements are always `IS NOT TRUE`
  (see [Tighten a filter](#tighten-a-filter)), authored in one place per
  statement family, never ad hoc.
- **Honest idempotence.** `UPDATE`-family steps are naturally idempotent for
  deterministic expressions. `INSERT`-family steps carry an identity anti-join
  guard where a usable row identity exists; where it doesn't, the option is
  flagged `rerun_safe: false` instead of pretending.

## Current scope

- Applies to **any table-materialized model**, not only incremental/maintained
  ones — a plain full-refresh table is the biggest win, since today it rebuilds
  on any edit.
- Emitted scripts are **DuckDB dialect**; other backends need dialect
  variants. Note the win is also engine-dependent: on a copy-on-write
  warehouse format a column-scoped `UPDATE` still rewrites every touched file
  — what it saves there is the upstream scans and joins, not the write.
- smelt **enumerates options; it does not yet choose** between a targeted
  script and full refresh — a cost model over the recorded option metadata is
  the planned chooser.
- Dropped columns are sequenced into the script (`ALTER ... DROP COLUMN`,
  always last), but whether a drop is *allowed to run at all* stays owned by
  [schema evolution](schema-evolution.md)'s `--allow-column-removal` opt-in —
  backbuild only orders the statement, never gates it.
- Not yet classified: refs repointed to a different upstream. These refuse
  with named reasons today. (A changed *cast* is not a type change — it is a
  changed expression, handled above; a bare type change with no expression
  change has no trigger in a definition diff at all.)

## Related pages

- [Incremental Models](incremental-models.md) — maintenance of *data* changes
  under an unchanged definition, including the `smelt rebuild` range rebuild.
- [Schema Evolution](schema-evolution.md) — physical schema change
  classification and DDL capability per backend.
- [Incremental Equivalence](../concepts/incremental-equivalence.md) — the same
  "equal to a full rebuild" contract, applied to incremental maintenance.
