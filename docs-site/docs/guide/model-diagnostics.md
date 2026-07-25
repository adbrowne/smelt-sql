# Model Diagnostics

The diagnostics page is a full-screen view of everything smelt knows and derives about a
single model: its columns and configuration, the full set of properties smelt has proven
about it, its relation contract, and — for incremental models — the maintenance plan smelt
would use to keep it up to date, including the SQL every candidate technique would emit.

It answers questions the graph view's [model detail panel](web-ui.md#model-detail-panel)
doesn't have room for, in particular "why did smelt pick this technique and not that one?"
and "what would the SQL look like if it had picked something else?"

## Opening the diagnostics page

From the [dependency graph](web-ui.md#dependency-graph), click a model to open its detail
panel, then click **Open Diagnostics**. The diagnostics page replaces the graph view; use
**Close** to return.

## Overview

The top of the page repeats the model's identity, materialization, tags, owner, incremental
configuration, and column list — the same information as the detail panel, plus the model's
full SQL text in a read-only, syntax-highlighted viewer.

## Properties

The properties section lists everything smelt has derived about the model's rows and columns:

- **Grain** — the column set(s) that uniquely identify a row, when smelt can prove one.
- **Row identity** — whether rows are identified by a key or only comparable as a whole row,
  plus any columns where a claimed key has been proven to mismatch actual row identity.
- **Functional dependencies** — key → column relationships smelt has derived from the model's
  SQL.
- **Per-column determinism, comparability, and algebraic discriminants** — a table listing,
  for every output column, whether it's deterministic, how it compares across runs, and its
  algebraic shape (monoid, invertible, decomposable, monotone) where relevant to incremental
  maintenance.
- **Set-op and fan-out indicators** — badges flagging a `UNION`/`INTERSECT`/`EXCEPT` barrier
  or a fan-out join, both of which constrain which maintenance techniques apply.
- **Literal columns** — columns smelt has proven hold a constant value.
- **Source bounds** — for each upstream source, whether smelt has derived a bounded scan
  range, an unbounded scan, or neither.

## Relation contract

Below the properties, the page shows the model's own relation contract (its event-time clock,
identity columns, and derived grain) alongside the same contract for every inbound edge — each
source or upstream model the plan depends on. This is the same contract shape maintenance
planning uses to decide which techniques a cell can admit; see
[Incremental Models](incremental-models.md) for what each field means.

## Maintenance plan and technique previews

For an incremental model, the page renders one block per maintenance cell (mirroring
`smelt explain`'s report): the cell's trigger, corner, and row identity, followed by a
**technique picker** — one button per technique smelt knows how to render for this cell,
including techniques the plan didn't pick.

Selecting a technique shows the SQL statements it would emit for this cell, plus a badge
explaining whether that technique is actually usable here:

- **Admitted** — the technique the maintenance plan actually resolved for this cell; its SQL
  is exactly what a real run executes.
- **Interchangeable Alternative** — proven sound for this cell (it would produce the same
  resulting state as the admitted technique) but not the one the plan picked. Region recompute
  always carries this badge when it isn't itself the admitted technique, since it's always a
  safe fallback.
- **Not Applicable**, with a reason — the technique's structural preconditions aren't met for
  this cell (for example, previewing a keyed-fold technique on a cell with no row key). The SQL
  is still shown for illustration, but it is not a real option for this cell and must not be
  treated as one.

Only one technique per cell is ever marked Admitted. A technique preview — of any verdict — is
display-only: selecting it does not run anything, and it has no effect on which technique a
real run will actually use. To change what a real run does, use the model's
`maintenance:` frontmatter, as described in
[Incremental Models](incremental-models.md#steering-prefer--technique).

## Remove comments

A single "Remove comments" toggle at the top of the page applies to every SQL viewer on the
page at once — the model's own SQL and every technique preview's statements — stripping `--`
and `/* */` comments while leaving the surrounding formatting untouched. This is purely a
display filter; it doesn't change what SQL a run would execute.

## Further reading

- [Web UI](web-ui.md) for the graph view and the rest of the UI's features.
- [Incremental Models](incremental-models.md) for the maintenance plan, technique
  interchangeability, and how to steer which technique a real run picks.
- [`smelt explain`](../reference/smelt-explain.md) for the command-line equivalent, including
  the `--technique` flag this page's technique picker mirrors.
