---
feature: definition_deltas
status: experimental
last_reviewed: 2026-08-12
owners: [andrew]
---

# Definition Deltas

> **What this is.** The normative spec for what happens when a maintained model's **own SQL
> changes**: how the change is classified, how the stored table is migrated to match the new
> definition without a full rebuild, and the plan-and-approve workflow that gates the migration.
> Out of scope, with their own homes: changes in a model's *input data* and the maintenance
> machinery that folds them (`incremental_models.md`); the per-column change-classification proof
> itself (`model_properties.md` §"Definition-change column classification"); the partition-grain
> and key-grain shape profiles (`incremental_shapes.md`); column-level DDL classification and
> `ALTER TABLE` strategy for declared-schema changes (`schema_evolution.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (behaviour + tracking link) or §References → Plans (history).

## Overview

*This section is a non-normative primer. The normative statements live in §Surface, §Semantics,
and §Constraints & Invariants — on any conflict, those win.*

An incremental model's stored table is kept current by folding in **deltas** — descriptions of
what changed since the table was last brought up to date. A delta comes in exactly two kinds:

- a **data delta** — rows changed in one of the model's inputs (new orders arrived, a customer's
  tier was corrected). `incremental_models.md` owns how data deltas are typed, planned, and
  folded.
- a **definition delta** — the model's own SQL changed (a column was added, an expression was
  rewritten, a filter was tightened). This spec owns those.

The two kinds share one algebra. Both are folded into the stored table under the same
bookkeeping (the **frontier** — the record of which deltas each part of the table has already
absorbed, `incremental_models.md` §"The frontier") and both answer to the same correctness
promise (the **equivalence invariant**, `incremental_models.md` §"The equivalence invariant").
For a definition delta the promise reads: after the migration — or mid-migration, for every
region already caught up — the stored table equals what a full refresh of the **new** definition
would produce over the inputs processed so far. The old definition drops out of the correctness
statement entirely; it only ever helps *derive* a cheaper route to that end state.

What distinguishes the two kinds is their **workflow**, not their semantics. Data deltas are
routine and bounded, so they fold automatically — that is the whole point of maintenance. A
definition delta is rare and can be destructive (it may rewrite or delete stored rows), so its
derived migration plan is **presented and approved** before anything runs: `smelt migrate`
prints the plan, an operator approves it, and only then does `--apply` execute it. Nothing
destructive ever runs unapproved.

A worked example, using the running warehouse of `incremental_models.md`. `order_facts` is a
keyed, time-partitioned model over `orders` joined to `customers`. Its author makes one
commit: adds a `net_amount` column computed as `amount - discount` (both already stored
columns), and reformats the `revenue` expression while in there.

```
$ smelt migrate order_facts

definition delta for order_facts (3 column groups affected):

  net_amount        backfill in place    ALTER TABLE + UPDATE from stored columns
                    regions: all (2024-01-01 .. 2026-08-12), no upstream read
                    cost: one pass over stored rows
  revenue, ...      eclipsed             formatting-only change — provably no output change
  (skeleton)        unchanged

plan hash: 4f2a91c6   approve and execute with: smelt migrate order_facts --apply
```

Had the author instead changed the `GROUP BY` — altering which rows exist — the plan would
refuse the in-place route and present a rebuild as the only honest migration. Had they only
reformatted the SQL, the plan would be empty ("eclipsed — nothing to do") and a CI job watching
`smelt migrate --json` would pass without ceremony.

## Surface

### Detection

smelt records, per model, the definition it last maintained the stored table under. A mismatch
between that recorded definition and the model's current SQL is a **definition delta**. Detection
is passive — editing a model never triggers work by itself; the delta is reported the next time
the model is planned, explained, or run. Between detection and approval, `smelt run` on a model
with a pending non-eclipsed definition delta **refuses to fold data deltas** rather than
silently maintaining a table whose definition no longer matches its contents; once a migration
plan is approved and applying, data deltas fold under the rules of §"Mid-migration data
folds".

### `smelt migrate`

```
smelt migrate <model>            # derive and print the migration plan; records its hash
smelt migrate <model> --apply    # execute the most recently printed plan
smelt migrate <model> --json     # machine-readable plan + exit-code contract (CI mode)
```

- **Plan.** `smelt migrate <model>` derives the migration plan (§"The migration plan") and prints
  it: per column group, the verdict, the chosen technique, the regions touched, and a cost class
  — or "eclipsed: nothing to do". The plan carries a **plan hash** over its content.
- **Approve and apply.** `--apply` executes the plan whose hash was recorded by the most recent
  plan step. If the model's SQL, its inputs' declared facts, or anything else that feeds the plan
  has changed since — so the freshly derived plan's hash no longer matches the recorded one —
  `--apply` refuses and prints the new plan instead. Approval is therefore always approval of the
  exact statements that will run, never of a stale description.
- **Resume.** An interrupted `--apply` resumes on re-invocation: the frontier records which
  regions each affected column group has caught up (§"Frontier semantics"), and re-applying the
  same approved plan continues from there.
- **CI mode.** `--json` plus the exit-code contract makes the pending-migration state visible to
  CI: exit 0 when there is no definition delta or the delta is eclipsed-only; a distinct non-zero
  exit when a non-trivial migration is derived but unapproved. "The deploy changes what this
  table means" becomes a checkable pipeline state; formatting-only changes pass silently.

### `smelt rebuild`

`smelt rebuild --event-time-start <ISO-8601> --event-time-end <ISO-8601> [selectors]` re-runs a
model and its upstreams over a time range — an ordinary ranged re-run using each model's normal
maintenance plan, batch-safety-aware chunking included (`incremental_shapes.md` §"First-run and
backfill"). It is a *data-side* verb: it re-processes inputs under the current definition and has
nothing to do with definition-delta migration. The two verbs are deliberately disjoint.

### Diagnostics

Catalogued in `diagnostics.md`; semantics owned here.

| Code | Fires when |
|---|---|
| `MaintenanceSkeletonChanged` | A definition delta adds or changes a field in a skeleton position — identity, grouping, dedup, or ordering — so the change alters which rows exist or the model's grain. Refused as an in-place migration; the honest plan is a rebuild (§"Skeleton changes are a new relation"). |

## Semantics

### The two delta kinds, one algebra

A **delta** is either a data delta (a typed change in an input relation — the delta-signature
lattice of `incremental_models.md` §"Delta signatures") or a definition delta (a typed change in
the model's own function). The maintenance plan already indexes its cells by trigger, and the
definition-change trigger is one of the four (`incremental_models.md` §"The plan matrix");
this spec is that trigger's semantics.

### The verdict per column group

A definition delta is classified **per output column group** (columns that change together —
`incremental_models.md` §"The plan matrix"). The per-column classification proof is owned by
`model_properties.md`; this spec owns the four verdicts it produces and the plan policy each
maps to:

- **eclipsed** — the change provably does not alter stored output: formatting, reordering,
  provably-equal rewrites. The induced delta is *empty*, so the correct migration is nothing.
  An eclipsed-only definition delta updates the recorded definition and touches no data.
- **backfill in place** — a new or changed column computable from columns already stored in the
  same table, with no upstream read: an `ALTER TABLE … ADD COLUMN` (for an added column) plus an
  `UPDATE` from stored columns.
- **re-derive** — a column that must be recomputed from the model's inputs, column-scoped: a
  column-addressed write over just the affected group, inheriting each read source's
  partition-locality verdict unchanged (`incremental_models.md` §"Partition-local maintenance
  (the K8 guardrail)").
- **skeleton change** — the change alters which rows exist or the model's grain (§"Skeleton
  changes are a new relation").

Fields added together factor by shared mutation-sensitivity, one migration operation per group.
A newly-affected group's catch-up is always **full-input** for its regions, even for a folding
column — there is no prior state for the new definition to fold onto.

### Frontier semantics

The frontier already has the vocabulary a definition delta needs: a definition delta sets the
affected column groups' processed-input record to **"nothing yet" over every existing region**.
Catch-up then advances region by region under the same fold rules and the same never-fold-ahead
discipline that governs any other delta (`incremental_models.md` §"The frontier"), until the
affected groups' processed record equals their siblings' over every region — at which point
the groups converge and ordinary maintenance resumes. Two consequences:

- **Migration is resumable for free.** An interrupted `--apply` left the frontier honest about
  which regions each group caught up; re-applying continues from there.
- **Migration and maintenance cannot interleave incorrectly.** The never-fold-ahead rule is the
  same rule; there is no second bookkeeping system to drift from the first.

**The catch-up unit** is the frontier realisation's own addressing: output regions where the
model carries a partition axis; the whole table as a single region for a bare keyed output —
catch-up is then all-or-nothing per column group, still resumable *across* groups; a
snapshot-reconcile model, which ordinarily keeps no frontier (`incremental_shapes.md` §"The
transactional frontier write (merge ledger)"), materialises a frontier record for the
migration's duration — the one case where a frontier exists only while a definition delta is
pending.

### Mid-migration data folds

Data deltas keep flowing while an approved migration catches up, under four rules:

1. **No fold under the new definition dispatches before the plan's schema statements have
   run.** An added column physically exists only once the approved plan's `ALTER` has executed
   (§"The atomicity rule"); until then the model is in the detection phase's refusal posture.
2. **A creation delta writes whole rows computed under the new definition**, into caught-up
   and pending regions alike; in a pending region the affected group's frontier record stays
   at "nothing yet", so that region's later catch-up recompute supersedes and resets whatever
   the creation fold wrote there (`incremental_models.md` §"The frontier", recompute-reset) —
   sound, and it keeps sibling freshness live everywhere.
3. **A mutation delta scoped to unaffected sibling groups folds only where the shape admits a
   column-scoped write** (a declared `unique_key` — `incremental_models.md` §"Per-cell write
   addressing"). On a region-addressed shape, whose only write rewrites all columns of a
   region, a fold into a pending region is instead taken as that region's catch-up: a full
   region recompute under the new definition, advancing the data and definition frontiers
   together — cheaper than stalling, and never a partial write.
4. **Never fold ahead.** No fold advances an affected group's record over a region ahead of
   that region's catch-up; the discipline is the same one every delta obeys.

### The oracle

After migration the stored table must equal the **new** definition's full refresh over the
processed inputs:

```
incremental_state(S) == full_refresh(new definition | input ∈ S)
```

Mid-catch-up the oracle holds per `(region × column group)` — the frontier's own grain, since
a region caught up for one group but not another matches neither definition row-wise:

- for every pair the frontier records as **caught up**, the stored values of that group's
  columns within that region equal the new definition's full refresh over the processed
  inputs, restricted to that region and those columns;
- for every **pending** pair, the stored values are unchanged from the pre-migration state —
  the old definition's invariant continues to hold over that pair's recorded inputs, updated
  only by the folds §"Mid-migration data folds" admits. A migration may never scramble what
  it has not yet caught up.

After migration the two quantifiers collapse to the whole-table statement. This is the
equivalence invariant of `incremental_models.md` §"The equivalence invariant" with the model's
function updated; nothing about the invariant's form, its order-independence corollary, or its
conformance gating changes. The generative conformance suite covers this by staging a
definition edit mid-history — the same harness, one more step kind.

### The migration plan

For a detected definition delta, smelt derives a **migration plan**: per affected column group,
the verdict, the **technique** that realises it, the regions touched, and a coarse cost class.
Techniques come from the definition-delta technique catalogue — the fold family for this delta
kind — alongside the always-available full-refresh baseline (the verdict each serves in
parentheses):

- self-derived column add, and self-derived column rewrite — an `UPDATE` from stored columns
  (backfill in place);
- column rename — zero rows touched (backfill in place);
- upstream key-matched pull-through (re-derive);
- join-enrichment backfills — an `UPDATE … FROM` shape and a per-reference scalar-subquery
  shape, offered together where both admit (re-derive);
- predicate-tighten `DELETE` (re-derive, row-removing);
- filter-loosen and time-horizon difference `INSERT`s (re-derive, row-adding);
- union-branch `INSERT` and discriminated-branch `DELETE` (re-derive, per branch);
- aggregate-column and window-column backfills (re-derive at unchanged grain);
- column drop (always destructive; sequenced last).

Write mechanisms are shared with the data side — shadow-build-and-swap and diff-then-patch
(`incremental_models.md` §"Per-cell write addressing") are the same registered patterns here —
and each technique's write passes through the **same available-addressings rule** unchanged:
on a shape with no declared identity, a re-derive group's realisation is a region recompute
under the new definition (whole rows, region-addressed), never a column-scoped write the
shape cannot address.

The plan is **options, not choices**: where several techniques admit for one group, the plan
presents the candidates with their cost class and safety posture (for example, the two
join-enrichment shapes differ in whether a wrongly-declared uniqueness fact fails loudly or picks
an arbitrary match — the plan says so), and the operator's approval selects. Admission is
per-group and fail-closed exactly as for data-delta cells (`incremental_models.md` §"Per-cell
admission"): a group no technique admits for falls back to the full-refresh baseline, presented
as such, never silently.

**Destructive legs are verified, and verified visibly.** A plan leg that drops rows, drops a
column, or swaps a table surfaces its verification probes — row-count and fingerprint checks run
before the swap or drop is committed — as part of the presented plan, so approval covers the
checks as well as the writes.

### Plan-and-approve

Data deltas fold automatically; definition deltas never do. The gate is a property of the delta
kind's **workflow**, not a second machinery: both kinds read the same plan data, the same
frontier, the same emitters. Concretely:

- Nothing destructive runs unapproved. An eclipsed-only delta is the one case with nothing to
  approve; it completes on the next plan step.
- Approval is of a plan hash (§Surface "`smelt migrate`"), so what was approved is exactly what
  runs.
- Every statement a migration executes is the output of the same single-owner emitters as any
  maintenance statement (`incremental_models.md` §"Statement emission (single owner)"): backends
  execute, never author, and the printed plan's SQL cannot drift from the executed SQL.

### Skeleton changes are a new relation

A change that alters which rows exist or the model's grain — a field added to or changed in an
identity, grouping, dedup, or ordering position — is honestly a **new relation**, not a
migration of the old one. It is refused as an in-place migration
(`MaintenanceSkeletonChanged`), and the plan presents the rebuild as the only route. There
is deliberately no "best-effort" in-place path: a skeleton change invalidates the premise that
stored rows correspond to the new definition's rows.

### Interaction with the contract lattice

A definition delta touches **all** regions — including partitions a contract point has promised
never to revisit. The plan-and-approve gate is where that conflict is resolved, explicitly:

- **Frozen horizon.** Where a model declares `frozen_horizon: H`
  (`incremental_models.md` §"The contract lattice"), a migration whose catch-up would write into
  the frozen band surfaces the conflict on the presented plan. Approval is then either explicit
  consent to cross the horizon for this migration, or the plan clamps its catch-up to the
  horizon and says so — leaving the frozen band on the old definition, a state the plan names
  rather than hides.
- **Deferral.** A deferral-licensed run skip (`deferral: D`) never silently defers
  definition-delta catch-up: skip licensing applies to data-delta lag only, and catch-up
  progress is reported on the migration plan, not absorbed into the ambient scheduler.

### The atomicity rule

A backfill-in-place group's physical column and its backfilled values are created by the **same
statement group** as the schema migration adding the column — never a separately-dispatched
write that could observe the column added but not yet backfilled. On a backend with
transactional DDL, a group failure leaves neither the column nor the values, and the next apply
retries the whole group. There is no window in which the deployed schema outruns the column's
real values.

### Downstream of a migration

A migration rewrites regions of a table other models read, and those rewrites propagate
through the ordinary graph layer (`incremental_models.md` §"The graph layer"), never a side
channel: each applied plan leg's written regions (or its recorded observed output delta, where
the write path records one) are the model's landed delta for its consumers, exactly as any
maintenance run's writes are. Two consequences: an **eclipsed-only** definition delta writes
nothing and therefore propagates nothing — a formatting-only deploy provably costs downstream
nothing; and a re-derive leg over all regions dirties consumers over all regions — a real cost
the presented plan's cost class makes visible before approval.

### Boundary with `schema_evolution.md`

`schema_evolution.md` owns column-level DDL classification (safe vs unsafe), the `ALTER TABLE`
strategy, and the stored-schema format for *declared-schema* changes. This spec owns the
migration of a **maintained model's stored data** across a change in its defining SQL. Where
both apply — an added column on an incremental model is both a schema change and a definition
delta — the definition-delta path governs, because only it carries the frontier bookkeeping and
the plan-and-approve gate. (`schema_evolution.md`'s `strategy: full_refresh` escape currently
bypasses that gate — a recorded divergence, §Known Divergences.)

### What stays data-side

Retraction handling and change-feed consumption are data-delta concerns and remain future work
on that side (`incremental_models.md` §Future Extensions); nothing in this spec depends on them.
A definition delta never expresses "rows were deleted upstream" — that is a data delta of a
shape the data side does not yet type.

## Design

**One algebra, not a bolt-on migration engine.** Definition changes, the live column-add
handling, and no-op-change detection were three unrelated mechanisms occupying one territory.
Recasting a definition change as a delta — empty for eclipsed changes, region-complete for the
rest — unifies them under the frontier and oracle that already exist, so migration inherits
resumability, never-fold-ahead, statement single-ownership, and conformance gating rather than
re-implementing each. The rejected alternative — a standalone migration subsystem with its own
bookkeeping — would have duplicated the frontier and inevitably drifted from it.
(`docs/research/20260811-delta-signatures-and-definition-deltas.md` §3.)

**Plan-and-approve, not auto-fold.** Data deltas fold automatically because they are routine,
bounded, and oracle-checked. Definition deltas are rare and can destroy data, so their workflow
is terraform-shaped: derive, present, approve, apply. Auto-applying was rejected — an eclipsed
misclassification or an unwanted destructive leg must have a human between derivation and
execution. The gate lives in the workflow, not the semantics, so the two delta kinds still share
one plan representation. (`docs/research/20260811-delta-signatures-and-definition-deltas.md` §4.)

**Approval is a stored plan hash.** A `--yes`-style flag on the invocation was rejected: it
approves whatever plan happens to be derived at apply time, which may not be the plan the
operator read. Hashing the plan and refusing on drift makes approval refer to exact statements,
and is what makes the CI exit-code contract honest — "unapproved migration pending" is a durable
state, not a race. (`docs/research/20260811-delta-signatures-and-definition-deltas.md` §7.)

**The plan hash covers the plan data structure, not only rendered SQL.** The hash is taken over
the plan data structure the emitters consume — verdicts, techniques, and the input facts they
were derived from (source declarations, backend capabilities) — not merely the rendered statement
text. Hashing only rendered SQL was rejected: two structurally different derivations can render
identical statement text incidentally (for example, the same `UPDATE` shape justified by
different declared uniqueness facts), and a hash that cannot distinguish them would let approval
bind to the wrong justification. The hash excludes facts that routine data arrival changes: region
*enumeration* is resolved at apply time from the frontier's own record ("nothing yet" over every
existing region, including regions that appeared after the plan was printed), not fixed at plan
time — otherwise `--apply` would be unreachable on any actively-loading warehouse, refusing on
every new partition. The printed region range is illustrative freshness, not hashed content. This
refines the plan-hash decision above rather than introducing a second one.
(`docs/research/20260811-delta-signatures-and-definition-deltas.md` §7.)

**The skeleton-change diagnostic is one code, not a split add/changed pair.**
`MaintenanceSkeletonColumnAdded` is renamed to `MaintenanceSkeletonChanged`, covering a
skeleton-position field that is added or changed alike. Both trigger the identical refusal and
the identical remediation (a rebuild is the only honest plan — §"Skeleton changes are a new
relation"), so a split pair would carry two codes for one decision path. This matches how every
other `Maintenance*` code names the refused condition rather than the trigger that produced it
(`MaintenanceGranularityMismatch`, `MaintenanceWriteAddressingRefused`) — none of them split by
which edit produced the violation. Splitting was rejected because a CI or LSP consumer matching
on the code would have to handle both identically anyway, doubling the catalogue surface for no
behavioural distinction.

**Two verbs, disjoint by construction.** Ranged data-side re-runs (`smelt rebuild`) and
definition-delta migration (`smelt migrate`) share no name and no flags. A single overloaded
verb was rejected: the two operations differ in destructiveness, approval posture, and what
"done" means, and a name collision between them was itself a defect this spec removes.
(`docs/research/20260811-delta-signatures-and-definition-deltas.md` §4, §7.)

**Skeleton changes refuse rather than degrade.** A best-effort in-place migration across a grain
change was rejected: no per-column technique can make stored rows correspond to a definition
that changes which rows exist. Refusing with the rebuild named keeps the fail-loud discipline
(`incremental_models.md` §"Validator, not chooser").

## Constraints & Invariants

- **The oracle is the new definition's full refresh over the processed inputs** — the same
  equivalence invariant, with the model's function updated. Mid-catch-up, it holds per region
  already caught up.
- **Nothing destructive runs unapproved.** Only an eclipsed-only delta completes without an
  approval step.
- **One frontier.** Definition-delta catch-up uses the same frontier and the same
  never-fold-ahead rule as data-delta maintenance; no second bookkeeping system exists.
- **One statement author.** Migration statements are emitted by the same single-owner emitters
  as maintenance statements (`incremental_models.md` §"Statement emission (single owner)");
  backends execute, never author.
- **Skeleton changes are never migrated in place.**
- **Admission is fail-closed.** A group no technique admits for is presented as a full-refresh
  leg, never silently widened or silently skipped.
- **The conformance gate covers definition edits**: the generative equivalence suite stages
  definition edits mid-history and asserts the new-definition oracle after every step.

## Known Divergences / Open Questions

Live gaps between this spec and the implementation as of `last_reviewed`.

- **The definition-delta synthesis layer is unwired.** The classification and emission machinery
  (`crates/smelt-logical/src/backbuild/` — diff factoring, per-group verdicts, the technique
  catalogue, script assembly) exists and is fully tested, but nothing outside its own crate
  calls it: no CLI verb reaches it, no plan derivation consumes it. Tracked:
  `docs/research/20260811-delta-signatures-and-definition-deltas.md` §6 step 2 (no
  implementation plan yet).
- **`smelt migrate` does not exist**, and the ranged-rebuild verb ships under the name
  `smelt backbuild` rather than `smelt rebuild`. The live handling of definition changes is a
  narrower third mechanism covering **column additions only** (the definition-change trigger in
  the maintenance driver); a changed column's redefinition falls to a full recompute. Same
  tracking as above.
- **The atomicity rule is conditional in practice.** A model whose
  `schema_evolution: strategy: full_refresh` frontmatter skips the migration gate falls back to
  a standalone `UPDATE` for backfill-in-place fields — the non-atomic two-step §"The atomicity
  rule" forbids — and that path is also the only one exercised on a backend without
  transactional DDL. Neither case has a repair path today. Tracked:
  `docs/plans/20260809-sensitivity-precision.md`. The
  `schema_evolution.md` full-refresh escape bypassing the gate is the divergence §"Boundary with
  `schema_evolution.md`" names; the unification should subsume it, not inherit it.
- **The conformance harness has no definition-edit step kind yet** — the oracle extension in
  §"The oracle" is specified ahead of the harness work.
- **No approval store exists.** The plan-hash persistence §Surface requires, hashing the plan
  data structure per §Design "The plan hash covers the plan data structure, not only rendered
  SQL", is unbuilt. Tracked:
  `docs/outcomes/20260815-definition-delta-migrate/outcome.md` phase 3.
- **The diagnostic code is not yet renamed in the implementation.** §Diagnostics and §Design name
  `MaintenanceSkeletonChanged`; the shipped `DiagnosticCode` variant, its `smelt-db` mapping, and
  the LSP code string still read `MaintenanceSkeletonColumnAdded`, reflecting the live mechanism's
  add-only derivation. The rename is a diagnostic-API change and needs its own sweep across
  sibling specs (`model_transforms.md`, `model_properties.md`, `incremental_models.md`,
  `schema_evolution.md`, `diagnostics.md`) and code. Tracked:
  `docs/outcomes/20260815-definition-delta-migrate/outcome.md` phase 7.

## Future Extensions

- **Row-local derivation for a changed column** already falls out of the re-derive verdict where
  the new expression reads only stored columns (the backfill-in-place verdict covers it); the
  open extension is admitting it for expressions over columns whose *own* groups are
  mid-catch-up, which needs an ordering over group catch-ups.
- **Eclipse-detection breadth.** The eclipsed verdict is only as good as the provably-equal
  rewrite recognizer; widening it (algebraic identities, join reorderings) is pure win — every
  widening turns a presented migration into a silent pass — but each widening must be a proof,
  never a heuristic.

## References

- **Code**: `crates/smelt-logical/src/backbuild/{mod,diff,classify,emit,requalify}.rs` (the
  synthesis layer: diff factoring, verdicts, technique catalogue, script emission);
  `crates/smelt-logical/src/analysis/definition_change.rs` and
  `crates/smelt-logical/src/maintenance/skeleton.rs` (the live column-add classification and
  skeleton refusal); `crates/smelt-runtime/src/maintenance_driver.rs` (the live
  definition-change trigger path).
- **Tests**: `crates/smelt-logical/src/backbuild/` module tests;
  `crates/smelt-logical/tests/maintenance_skeleton.rs`;
  `crates/smelt-logical/tests/maintenance_tracer_evolution.rs`;
  `crates/smelt-runtime/tests/tracer_evolution.rs`;
  `crates/smelt-cli/tests/targeted_column_backfill.rs`.
- **User docs**: none yet — the docs-site page for migration lands with the wiring plan.
- **Plans (history)**: `docs/plans/20260809-sensitivity-precision.md` (atomicity-gap tracking).
- **Research**: `docs/research/20260802-backbuild-synthesis.md` (the technique catalogue and its
  correctness oracle); `docs/research/20260811-delta-signatures-and-definition-deltas.md` (the
  unification and the plan-and-approve workflow).
- **Related specs**: `incremental_models.md` (deltas, frontier, plan, emitters, lattice);
  `incremental_shapes.md` (the shape profiles migrations run against); `model_properties.md`
  (the per-column classification proof); `schema_evolution.md` (declared-schema DDL);
  `diagnostics.md` (code catalogue); `cli.md` (verb surface).
