# Research: Optional virtual data environments for smelt — a SQLMesh study and a type-system-backed proposal

**Date**: 2026-06-01
**Topic**: Whether and how smelt should support SQLMesh-style *virtual data environments* (VDEs) — cheap, isolated dev/staging environments that reuse production physical tables for unchanged models — as an **opt-in** capability that does not require a state store for projects that don't want one.
**Motivating ask** (Andrew): support virtual environments like SQLMesh; make it optional (opt-in per model); don't force a state store on every smelt project. Educate on SQLMesh, then propose a plan for smelt. Treat as a possible major change; ignore the current implementation for the most part. Use smelt's parser and type system to *eclipse* what SQLMesh can do.
**Related**: `docs/research/20260529-dbt-incremental-adapter.md` (state section — the stateless↔stateful spectrum this paper sits inside), `docs/specs/architecture.md` (single-CST pipeline, type system, planner `Transformation` values), `docs/specs/incremental_models.md` (interval/state ownership, batch safety), `docs/specs/schema_evolution.md`, `docs/specs/expansion.md` (function expansion before analysis), `docs/specs/types.md`. Memory: the state model needs a per-project/per-model stateless-vs-stateful choice plus opt-in environments.

---

## 0. TL;DR

SQLMesh's virtual environments reduce to **two computed judgements**:

1. a **fingerprint** per model variant, which decides *can this environment point at an existing physical table instead of building a new one?*; and
2. a **breaking / non-breaking** categorization of each change, which decides *how far down the DAG must a backfill cascade?*

SQLMesh computes both with SQLGlot as a **syntactic AST edit-script**. It is deliberately conservative: the only change it will call non-breaking is *adding a new projection* — "we can safely assume that the change is breaking if the output edit script contains any operation other than Insert or Keep" ([Tobiko: detecting breaking changes](https://www.tobikodata.com/blog/automatically-detecting-breaking-changes-in-sql-queries)). Per-column lineage that would relax this is described as future work.

smelt has, at exactly that layer, three things SQLGlot lacks:

- a **single typed CST** with a byte-identity printer (`architecture.md` §"Identity properties"),
- a **real type system** in `smelt-db` (full type inference, the DuckDB-oracle property tests) plus the *foundations* of column-origin tracking (`RowExtension`/`InputConstraint` carry per-column type provenance today — though leaf-scoped and best-effort, not yet the cross-model column-consumption graph the equivalence relation ultimately needs; see §4(b)), and
- **logical/physical separation** — the model body is the pure *what*; materialization, incrementality, and engine are the *how* the planner decides.

That lets smelt replace SQLMesh's *syntactic guess* with a **typed, provable equivalence relation**: reuse a physical table when the change cannot alter any column a downstream model actually consumes — proven from types and column lineage, not guessed from tree edits. This is the eclipse. **Two honesty caveats up front**, expanded in §4(b) and §5.5: (1) the *full* relation needs a cross-model column-lineage analyser that smelt does not have yet (it has the type-inference and per-column-provenance scaffolding to build it on, not the finished article); and (2) the eclipse is real and provable *over the transparent, deterministic, typed core* — but several signals that core relies on (`deterministic` defaults to `false` and is author-declared; collation/decimal-precision/nullability are not yet tracked by the type system) mean an **un-annotated** pipeline lands closer to parity than eclipse until that coverage is built. The eclipse is the destination and it is reachable; it is not free on day one. Everything else (a state store, environment-suffixed schemas, "virtual update" promotion) is plumbing smelt can adopt as an **opt-in layer** that stateless projects never pay for.

The recommendation: lead with the provable-semantics fingerprint as the headline differentiator, ship it behind an opt-in `state:` posture, default the state store to an embedded `.smelt/` file so the on-ramp needs no new infrastructure, and stage delivery so the smallest experiment (a semantic-equivalence oracle over two model versions) proves the core claim before any environment machinery is built.

---

## 1. Why this paper, and the constraints it must honour

smelt today is deliberately **state-light**. `incremental_models.md` §"State ownership" is explicit: "smelt does not track watermarks, offsets, or run history… The backend owns computational state." The design rationale rejects a watermark store as a v1 requirement because it "duplicates state the backend already tracks, opens a sync-correctness window… and locks adoption."

That is the right default and must stay the default. But it forecloses a class of capability that SQLMesh users value highly — **cheap, safe, isolated environments**: spin up a dev copy of the whole warehouse in seconds, preview a change against real data, promote to prod with zero recompute. Delivering that requires *remembering what was built*, which is exactly the state SQLMesh keeps and dbt refuses to.

So the constraints from the ask are load-bearing and shape every decision below:

- **Optional.** A smelt project that never opts in behaves exactly as today: no state store, warehouse-is-the-state, no new failure surface.
- **Opt-in per model (and per project).** Within a project that enables environments, an individual model can stay stateless. This is the per-model granularity recorded in the state-model direction.
- **No mandatory state store.** When opted in, the state store should default to something with *zero new infrastructure* (an embedded file in `.smelt/`), and only escalate to a separate OLTP database when the user asks for it.

This paper sits inside the broader **stateless↔stateful spectrum** (§3); VDEs are the far, most-stateful end of it. The point of "optional/opt-in" is that a project moves along the spectrum only as far as its needs require.

---

## 2. SQLMesh, explained

### 2.1 The headline feature: virtual data environments

A SQLMesh **environment** is "an isolated namespace that allows you to test and preview your changes" ([environments](https://sqlmesh.readthedocs.io/en/stable/concepts/environments/)). There is one `prod` and any number of dev environments. Isolation is achieved by **schema-name suffixing**: model `db.model_a` is materialized as `db__my_dev.model_a` in environment `my_dev`, while `prod` keeps the bare name.

The thing that makes environments *cheap* — the whole point — is that **an environment is a collection of references to model snapshots, not a copy of data**. When a model's fingerprint matches an existing snapshot, the new environment's view simply points at the **same underlying physical table**. "Any computation that was done in this environment can be safely reused in other environments." A dev environment over an unchanged DAG costs essentially nothing: it is a set of views over the **shared physical-table layer** — and a precise point worth keeping straight is that `prod` does *not* own the physical tables. SQLMesh keeps a virtual layer (per-environment view schemas like `db__dev`) over a separate physical layer (e.g. `sqlmesh__db`); `prod` is itself just another set of views over that shared physical layer, with no privileged tables of its own. Promotion is therefore symmetric: it is a view repoint in the virtual layer, not a copy from a dev table into a prod-owned one.

**State-store isolation is weak in SQLMesh, and smelt should design for stronger isolation.** In SQLMesh, all environments — `prod` and every dev — share a *single* state database. There is no first-class way to give a developer's environment its own state boundary: a dev plan/apply reads and writes the same state store prod uses, so the blast radius of a buggy local run or a careless credential includes prod's recorded state. smelt should treat the access boundary as a first-class part of the state-store design (§6.2): a dev environment can be granted **read access to prod's state** (so it can *fork* — discover existing snapshots and reuse their physical tables) while being **denied write access** to prod's state. The complementary direction is also a user choice — prod may be configured *not* to read dev-created physical tables, so a dev build can never pollute a prod promotion. smelt does not need to mandate any particular policy; it needs the state-store schema and the resolver to *support* per-environment read/write scoping so end users can choose. This likely only becomes enforceable with the `oltp` store (§6.2), where the substrate has real per-role credentials; the embedded file store is single-writer/local and cannot enforce it. Captured as an open question (§7) so the state-store schema doesn't foreclose it.

Two further cost levers:

- **Time-bounded dev data.** Dev environments carry start/end dates so you can build only a recent slice of history.
- **Gap-only compute.** SQLMesh "only computes data gaps that have been directly caused by the changes" — unchanged upstreams are reused; only the changed subgraph is built.

### 2.2 The mechanism underneath: snapshots and fingerprints

A **snapshot** is "a record of a model at a given time… everything needed to evaluate the model and render its query" — the model's query, the macros active at capture, global variables, and the data intervals available for that snapshot ([snapshots](https://sqlmesh.readthedocs.io/en/stable/concepts/architecture/snapshots/)).

Each snapshot has a **fingerprint** "derived from [its] model." The fingerprint is what decides table reuse: "This fingerprint allows SQLMesh to detect if a given model variant exists in other environments or if it's a brand new variant." Critically, "Because SQLMesh can understand SQL with SQLGlot, it can generate fingerprints such that superficial changes to a model, such as applying formatting… will not return a new fingerprint." (SQLMesh splits this into a `data_hash` over things that affect output and a `metadata_hash` over things that don't, so a comment or owner change can be a *metadata-only* change.) The fingerprint also folds in **parent fingerprints** (`parent_data_hash` / `parent_metadata_hash`): an upstream change propagates into every descendant's fingerprint, which is the mechanism by which a breaking change cascades. smelt's analogue cascades through the dependency graph too, but draws the cascade boundary with column lineage rather than a transitive hash (§5.2) — the comparison in §5/§9 is against this parent-hash cascade, not just the single-model hash.

Each distinct fingerprint gets **its own physical table**; environments hold pointers. This is what makes reverts fast (the old table still exists until garbage-collected) and promotion free.

### 2.3 plan / apply

A **plan** "is a set of changes that summarizes the difference between the local state of a project and the state of a target environment" ([plans](https://sqlmesh.readthedocs.io/en/stable/concepts/plans/)). Creating a plan: diff local model definitions against the environment's recorded state, find added/removed/modified models, find downstream models affected, and compute the date range needing backfill.

**Apply** either *backfills* (runs the new versions' logic over the required range to populate new physical tables) or — when the needed tables already exist from a dev build — performs a **virtual update**: "only references to new model versions need to be updated," zero runtime cost. The dev→prod promotion of a fully-built change is a virtual update: you pay the compute once, in dev, then promotion is a pointer swap.

### 2.4 Change categories — and exactly how conservative the heuristic is

This is the crux for the comparison, so it's worth being precise. SQLMesh categorizes each *directly-modified* model:

- **Breaking** — the change functionally affects downstream consumers; the model **and all its downstream dependencies** are backfilled. (e.g. a `WHERE` that now filters rows downstream relied on.)
- **Non-breaking** — the model is backfilled but **downstream is not** (e.g. adding a column nobody downstream reads yet).
- **Metadata-only** — no backfill (owner, comment, audit annotations).
- **Forward-only** — physical tables from previous versions are **reused**; no backfill, but the change can't be cleanly reverted. Used when a full rebuild is infeasible.

How is this decided automatically? Via SQLGlot's **semantic diff**: an edit-script of AST operations (`Insert`, `Remove`, `Move`, `Update`, `Keep`). And the rule is blunt: *"Since the only type of change that will be categorized as non-breaking is addition of a new projection, we can safely assume that the change is breaking if the output edit script contains any operation other than Insert or Keep"* ([Tobiko](https://www.tobikodata.com/blog/automatically-detecting-breaking-changes-in-sql-queries)).

So in practice:

- Add a projection → non-breaking. **Everything else** → breaking, conservatively.
- `SELECT *` is handled by expanding `*` to explicit column references first (requires schema knowledge) so a star-select downstream isn't silently mis-categorized.
- **Per-column lineage** — "categorize changes per each impacted downstream model individually," so that removing a column nobody downstream reads stops being breaking — is described as *future* work, not current behaviour.

The takeaway: SQLMesh's reuse decision is **syntactic and conservative**. It rebuilds in many cases where the *output is provably identical for every consumer*, simply because the edit-script contained an `Update` or `Move`. This is the gap smelt's type system can close (§5).

### 2.5 The state store

VDEs are impossible without durable state. SQLMesh's **state database** stores: model versions (queries, loaded intervals, dependencies), snapshots, the environments and which model versions are promoted into each, auto-restatement schedules, and SQLMesh/SQLGlot version metadata. "The state database is how SQLMesh 'remembers' what it's done before so it can compute a minimum set of operations… instead of rebuilding everything every time" ([state](https://sqlmesh.readthedocs.io/en/stable/concepts/state/)).

Operational facts that matter for smelt's "optional" framing:

- The workload is **OLTP, not analytical**. SQLMesh warns that using the warehouse (OLAP) for state "is supported for proof-of-concept projects but is not suitable for production and **will** lead to poor performance and consistency." Recommended hosts: Postgres / other OLTP, Tobiko Cloud, or DuckDB locally.
- **State loss = forced full rebuilds** and loss of the minimal-change machinery. State must be **backed up**; importing state carries explicit "back it up first" warnings.
- It is, in the words of the dbt-adapter research, "the new desync surface" — the price of the capability.

This is precisely why smelt should make it opt-in: the state store is operationally load-bearing the moment it exists, and a stateless project should never carry it.

### 2.6 Model kinds (for completeness)

SQLMesh's kinds map roughly onto smelt materializations + the incremental rule: `FULL` ≈ `table` full refresh; `VIEW` ≈ `view`; `EMBEDDED` ≈ ephemeral/CTE inlining; `SEED` ≈ seed; `INCREMENTAL_BY_TIME_RANGE` ≈ smelt's time-partitioned incremental rule; `INCREMENTAL_BY_UNIQUE_KEY` ≈ a MERGE/upsert strategy (smelt's `cumulative_aggregate` is adjacent); `SCD_TYPE_2` has no smelt analogue yet. The relevant detail for *this* paper: **non-idempotent kinds** (`INCREMENTAL_BY_UNIQUE_KEY`, `SCD_TYPE_2`, self-referential) "cannot backfill limited date ranges… they either fully refresh or produce preview-only data." smelt's batch-safety taxonomy (`FullyBatchSafe` / `BoundedSafe(n)` / `PerPartitionOnly`) already expresses this distinction *as a proof*, which interacts with environments in §6.4.

### 2.7 What use cases does this actually serve?

| Use case | What SQLMesh gives | How much it relies on state/VDEs |
|---|---|---|
| **Cheap dev environment** | spin up `db__dev` as views over prod tables; build only the changed subgraph, over a recent slice | entirely — this *is* the VDE |
| **Safe preview / data diff before merge** | build the change in dev, diff dev vs prod outputs | VDE + fingerprints |
| **Blue-green prod promotion** | virtual update = pointer swap after dev build | VDE + state |
| **Precise backfill / restatement of a range** | interval ledger knows exactly what's built; restate a window | interval state (not strictly VDE) |
| **CI on every PR** | ephemeral environment per PR, auto-categorized changes | VDE + fingerprints |
| **Fast revert** | old physical table still referenced until GC | snapshots + state |

The first five are the value. Notice the middle two (precise backfill, CI) lean on the **interval ledger** more than on environments — which is why §3 treats state as a spectrum rather than an on/off switch.

---

## 3. The state spectrum: where VDEs sit

From `20260529-dbt-incremental-adapter.md` and the recorded direction, smelt's state model is best understood as a spectrum, chosen **per project and per model**:

```
 stateless                         stateful                       virtual environments
 (dbt-style)                       (interval ledger)              (SQLMesh-style)
 "warehouse is the state"          durable per-model intervals    intervals + fingerprints
                                                                   + env→table map
 ───────────────────────────────────────────────────────────────────────────────────►
 cheap, robust, no store           precise backfill, plan/diff    cheap isolated envs,
 silent gaps possible              must back up the ledger         virtual promotion
                                                                   biggest desync surface
```

Three observations that drive the proposal:

1. **VDEs require the full stateful posture *plus* fingerprints.** You cannot have cheap environments without (a) remembering what physical tables exist (snapshots) and (b) a fingerprint to decide reuse. So "opt into environments" implies "opt into a state store for the models that participate."

2. **The opt-in boundary is the model, not just the project.** A project can run environments while a specific high-churn or externally-fed model stays stateless (never snapshot-reused). "Stateless" here means smelt keeps no fingerprint/snapshot for it — *not* that it is always fully rebuilt: a stateless incremental model still runs incrementally against the backend's own state (warehouse-is-the-state), it just never participates in the cheap fingerprint-based table reuse. This matches the per-model granularity in the recorded direction and avoids forcing a posture on models where it doesn't pay.

3. **smelt already owns the *concept* of the stateful end.** `smelt-state` exists (`RunManifest`, `IntervalStore`, `FileStore`; `architecture.md` crate table) and the runtime already does interval-store updates, so the *interval-ledger seam* — the thing "precise backfill/restatement" needs — is already cut into the architecture. The caveat is that the current `smelt-state` crate is an early, lightly-tested prototype: it should be treated as evidence the seam is viable, **not** as a foundation to preserve. This work is free to evolve it substantially or rebuild it outright, and the state-store schema this paper implies (snapshots, fingerprints, env→table map, per-environment access scoping) is a clean-sheet design question — `run_state.md` (§7, OQ10) is where that schema gets settled. So the asset is the *seam and the design*, not the existing code.

**Design stance:** model state posture as an explicit, three-valued choice with a stateless default.

```yaml
# smelt.yml  (project default)
state:
  mode: stateless            # stateless | intervals | environments
  # store: { kind: embedded, path: .smelt/state.db }   # only when mode != stateless
```

```sql
---
# per-model override in frontmatter
state: environments          # this model participates in VDEs / snapshot reuse
---
```

`stateless` = today's behaviour, no store. `intervals` = durable interval ledger (precise backfill/restatement, gap detection) but no environments. `environments` = intervals + fingerprints + env map. A model never opted in is materialized by its own strategy — full or incremental — in whatever environment references it, never reused via a fingerprint match. Correct, just not cheap: it still does exactly the work it would do today (an incremental model stays incremental against the backend's state), it simply forgoes the pointer-swap reuse.

---

## 4. What smelt already has that SQLMesh approximates

SQLMesh is built on SQLGlot, a parser/transpiler. smelt is built on a *typed compiler* with an LSP-grade analysis layer. Four standing smelt assets line up directly against SQLMesh's mechanisms:

**(a) Single typed CST + byte-identity printer → an exact, principled fingerprint.** `architecture.md` §"Identity properties" guarantees the DuckDB printer emits byte-identical SQL modulo a fixed, documented set of rewrites, and the parser is fingerprint-equivalent to PostgreSQL via pg_query. SQLMesh gets formatting-insensitivity by *normalizing through SQLGlot*; smelt gets it from a printer whose identity is a **property-tested invariant**. The canonical form for fingerprinting is something smelt already maintains and tests. (Parity here, not eclipse — but a more principled foundation.)

**(b) Type system + column-origin foundations → a buildable path to provable, not syntactic, change impact.** `smelt-db` computes full type inference (`type_inference.rs`, the property-tested oracle vs DuckDB) and per-column *type* provenance (`RowExtension.ref_name`, `InputConstraint`, schema inference). This is the foundation for the column-level lineage SQLMesh lists as *future* work — but two gaps must be named honestly, because the §5.2 relation depends on closing them:
- **Column provenance today is leaf-scoped and best-effort, not a cross-model consumption graph.** `RowExtension.ref_name`/`InputConstraint.ref_name` carry the *leaf segment* of an upstream model, resolved through a leaf-only path not shared with the canonical-path resolver (`architecture.md` §Known Divergences, "Schema-inference subsystem still uses leaf names"), and column `source` degrades to `"unknown"` when inference can't determine it (`data_catalog.md`). So smelt has per-model, best-effort column types — not yet the total "which downstream model reads column `c` of M" graph the equivalence relation needs.
- **The output→input *derivation* map (`provenance:`) is author-declared, not inferred, and gated.** The structured `provenance:` key is gated behind `unstable_schema: true` and is **author-declared, validated structurally, not auto-derived** — "auto-derivation requires a full lineage analyser the compiler does not yet have" (`planner_integration.md` §"Properties are author-declared in v1"). So smelt can infer a column's *type* but not (yet) mechanically derive how it was computed.

The honest reframing: smelt can answer "does this change alter the *type* of column `c`?" today, and is uniquely positioned to answer "…the *derivation* of `c` that downstream M reads?" — but the latter needs the cross-model lineage analyser built on this scaffolding. That analyser is the substantive new work the headline proposal implies; it is *buildable on assets that exist*, which is the real claim, rather than something that falls out for free.

**(c) Logical/physical separation → whole categories of change are metadata by construction.** In SQLMesh, the model *is* its query plus its kind/config in one object, so a change to incremental strategy and a change to business logic both perturb the snapshot. In smelt, the model body is pure logical SQL; materialization, incrementality, batch strategy, and engine are **planner decisions** (`architecture.md` §"Models as functions", "Materialization is orthogonal to transparency"). Therefore: *changing how a model is built — view→table, full→incremental, DuckDB→Spark — does not change what it computes, and can be classified metadata-only/forward-only by construction*, not by guessing. This is a structural advantage SQLMesh cannot easily replicate because its abstraction conflates the two.

**(d) Function expansion before analysis → fingerprint over the expanded, typed CST.** `expansion.md` runs function inlining *before* every analysis stage. So a fingerprint computed over the **expanded** CST naturally captures "did the effective SQL change?" — and a change to a `smelt.define` body only re-fingerprints the models whose expanded output actually changes. SQLMesh's macro-expansion equivalent is captured by hashing macro definitions into the snapshot; smelt's is a typed CST it can diff column-by-column.

**(e) Reversible `Transformation` values, non-mutated CST → speculative planning fits.** The planner already returns `Vec<Transformation>` without mutating the CST and is designed for "try a rewrite, measure, discard" (`architecture.md` Design). A plan/diff that builds candidate physical states and diffs them is the same shape the planner is already built around.

---

## 5. Proposal (headline): provable semantic fingerprints

The recommendation is to make smelt's reuse decision a **typed equivalence relation**, computed from the CST + type system + column lineage, rather than a syntactic edit-script. This is the lever that lets smelt reuse physical tables in cases where SQLMesh conservatively rebuilds.

### 5.1 Two fingerprints, mirroring SQLMesh's split but stronger

- **`output_fingerprint(model)`** — a hash over the **canonicalized, expanded, typed CST** of the model body, *projected onto its output schema*. Two model versions with the same `output_fingerprint` produce the same rows and types for the same inputs. This is the table-reuse key. It is insensitive not just to formatting (SQLMesh parity) but to **any rewrite the printer/type layer proves output-preserving** — reordered projections, renamed-but-unused CTEs, a refactor that splits one CTE into two, an added column that no consumer reads (see §5.3).
- **`metadata_fingerprint(model)`** — owner, description, tags, audits: changes here never trigger a build.

Crucially, **physical config is not in either fingerprint of the logical model.** Materialization/incremental/engine live on the planner's physical decision, which gets its *own* physical fingerprint. A view→table change is a physical-only change (§5.4).

### 5.2 The equivalence relation, and what "non-breaking" becomes

Define change impact in terms smelt can *prove*:

> A change to model M is **non-breaking for downstream model D** iff, for every column of M that D's column-origin lineage shows D actually consumes, the change leaves that column's *type and derivation* unchanged.

Then:

- **Reuse M's table** iff `output_fingerprint` is unchanged (the strongest case — output identical for *all* consumers).
- **Backfill M but not D** iff M changed but the change is non-breaking for D by the relation above (D reads only unaffected columns).
- **Backfill M and D** iff D consumes a column whose type/derivation changed.
- **Metadata-only** iff only `metadata_fingerprint` changed.

This is SQLMesh's three categories — but the boundary between them is drawn by **column-level provenance + types** instead of "added a projection vs everything else."

**What's inferable today vs what the relation needs.** The relation above leans on two distinct judgements that are *not* equally available:
- *Type-equivalence* of a column ("does column `c` still have the same type?") — smelt infers this today, modulo the type-system gaps in §5.5 (collation, decimal precision, nullability are not yet tracked, so "same printed type" does not yet imply "same values").
- *Derivation-equivalence* ("is `c` computed the same way, and does D actually consume `c`?") — this needs the cross-model lineage analyser of §4(b), which is buildable but unbuilt. Until it lands, the safe v1 relation is **types-only at the model boundary**: reuse on unchanged `output_fingerprint`; treat any column-type change as breaking-for-D unless D demonstrably doesn't read that column. The full per-column-derivation cascade (worked examples §5.3 #2, #3) is the *target*, gated on the analyser — not a day-one capability. This sequencing matters for §8's staging: Stage 4 ("provable categorization") is where the analyser and the relaxed cascade actually arrive.

### 5.3 Worked examples where smelt reuses and SQLMesh rebuilds

These are the cases that justify the whole proposal:

1. **Refactor with identical output.** Split a monolithic `SELECT` into CTEs, or reorder the projection list, or rename an internal alias. SQLGlot's edit-script contains `Move`/`Update`/`Insert` ⇒ **breaking** ⇒ full rebuild + downstream cascade. smelt: `output_fingerprint` unchanged (same columns, same types, same derivations) ⇒ **table reused, zero compute.**

2. **Remove a column nobody downstream reads.** SQLMesh today: a `Remove` in the edit-script ⇒ **breaking** for the whole subtree (per-column lineage that would fix this is future work). smelt: column-origin lineage shows no downstream consumes it ⇒ **non-breaking; downstream untouched.**

3. **Change a column's logic when only *other* columns flow downstream.** Model M computes `a, b, c`; downstream D selects only `a, b`. A change to how `c` is computed. SQLMesh: edit-script has `Update` ⇒ breaking subtree. smelt: D's lineage touches only `a, b`, both unchanged ⇒ **non-breaking for D**, only M rebuilt.

4. **Physical-strategy change (the structural win).** Flip M from `view` to `table`, or full-refresh to incremental, or pin it to Spark. SQLMesh: model object changed ⇒ snapshot churns. smelt: logical `output_fingerprint` unchanged; only the *physical fingerprint* changed ⇒ classified as a physical migration, no logical backfill cascade (§5.4).

5. **Type-widening that spares the downstream cascade.** Widen `INT`→`BIGINT` on a column whose downstream uses are all type-compatible. Note the precise claim: M's own `output_fingerprint` *does* change (a column type changed), so M's table is **not** reused — M itself takes a physical schema migration (an `ALTER TABLE` widening, per `schema_evolution.md`'s safe-widening matrix, not a from-scratch rebuild). The win is **downstream**: smelt's type system proves every downstream consumer's type-checks still hold, so the change is **non-breaking for D** and the cascade stops at M. SQLMesh, with no type system, treats the edit-script `Update` as breaking and rebuilds the whole subtree. So the eclipse here is "downstream spared," not "M reused" — distinct from examples #1 (true reuse, fingerprint unchanged).

### 5.4 Physical changes are first-class and separate

Because materialization is a planner decision, a *physical* fingerprint (engine, materialization kind, incremental config, partition column) tracks separately. A change to physical strategy with an unchanged logical `output_fingerprint` is a **physical migration**: smelt may need to re-materialize M (a view has no table to reuse; and per `schema_evolution.md`'s backend matrix some engine/format combinations get no in-place `ALTER` and require a full table rewrite) but the *logical contract* is intact, so **no downstream rebuild is implied**. One interaction to carry into the spec: when a physical migration *does* re-materialize M's data, M's interval ledger (§6.4) must be reconciled — the new physical table starts with whatever intervals the migration populated, not M's prior ledger. This cleanly separates "I changed the math" from "I changed how it's stored" — a distinction SQLMesh's model object blurs.

### 5.5 Risks and honest limits of the prover

A semantic equivalence prover must be **sound** (never call a real change non-breaking) even at the cost of completeness (sometimes conservatively rebuild). The hard cases:

- **Black-box calls — but a *deterministic* black box is still fingerprintable.** `smelt.extern`, canonical built-ins, and sources are black-box in their *body* (`architecture.md` two-axis table; `planner_integration.md` §"Black-box opacity is absolute"): the planner can't see how they compute. That is **not** the same as "unfingerprintable." A black-box call is part of the canonicalized CST like any other node, and smelt already tracks `deterministic` (and `idempotent`) as declared extern/function properties (`planner_integration.md`). So a model that *merely contains* a black-box call whose declaration is **deterministic and unchanged** fingerprints normally — its output is a pure function of its inputs, so identical inputs ⇒ identical rows. The prover only has to go dark in two specific cases: (a) the black-box's **declared signature or body changes** (a different extern, or the same extern redeclared), which legitimately moves the fingerprint; or (b) the black-box is **declared non-deterministic** (next bullet). Conservatively rebuilding *every model that touches any extern* — the original framing — would forfeit most of the eclipse, since real pipelines lean heavily on built-ins. The correct rule is narrower: *a deterministic black box with a stable declaration is transparent to the fingerprint even though it is opaque to the planner.*

- **Non-determinism — and the default-`false` problem.** `RANDOM()`, `NOW()`, ordering-without-`ORDER BY`: a model that isn't deterministic can't have its output *proven* equal across versions, so by default a change to such a model is breaking. smelt tracks `deterministic` as a function/extern property and can feed it in — **but two realities blunt this and must be confronted, not glossed**: (i) `deterministic` **defaults to `false` and is never auto-derived** (`functions.md`, `planner_integration.md` §"Properties are author-declared in v1"), so an *un-annotated* pipeline presents as non-deterministic everywhere and the prover conservatively rebuilds — the practical inversion of the eclipse for unannotated code, and the main reason §0 hedges "closer to parity than eclipse" for that case; and (ii) `deterministic` is a property of *functions/externs*, but non-determinism also enters through **inline SQL with no function-call node to tag** — a bare `now()`/`random()` in a projection, or the subtler `LIMIT`/`FETCH` without a total `ORDER BY`, or relying on unordered aggregate/window output. The fingerprint analysis must therefore detect inline non-determinism structurally (a small deny-list of non-deterministic built-ins + an "unbounded-without-total-order" check), not only read declared function properties. So "can't prove it" shouldn't mean "always rebuild" — but neither can the prover assume determinism it hasn't established. Two escape hatches let the author supply what the prover can't derive, both opt-in and both recorded in state so the decision is auditable:
  - **Accept-current.** When a model is non-deterministic, the author can declare that the *existing* materialized value is acceptable across an output-preserving change (e.g. a formatting/refactor change to a model that happens to call `NOW()` in a column nobody recomputes on). smelt reuses the table rather than re-rolling the dice.
  - **Assert-deterministic per model/call.** An author can assert that a specific call (or the whole model) is deterministic-in-practice — e.g. a UDF smelt can't see into but the author knows is pure. This is the same shape as the `deterministic:` declaration on functions, just applied at the call/model granularity. It is an *unproven assertion* the user takes responsibility for; the prover trusts it but the assertion is logged.

  Both are strictly safer than SQLMesh, which has no determinism model at all and simply rebuilds.

- **`SELECT *` is fully resolved — sources are schema-transparent.** smelt resolves `*` through the type system to the columns the upstream actually declares, so star-selects are *better* off than SQLMesh's textual expansion. Sources do not undermine this: a source declares a **complete, typed column schema** in YAML, and that schema *is the contract* — a column not declared in the YAML is a diagnostic, not a silently-passed-through field (`sources.md` §"Schema is the contract", rule 2). So `*` over a source expands to exactly the declared columns with their declared types; there is no schema opacity to inherit. A source is black-box only in its *body/derivation* (its data is produced by an external pipeline), which is a different axis. The one residual risk that *is* real — and which SQLMesh shares — is **external data drifting underneath an unchanged schema declaration**: smelt's fingerprint sees the declaration, not the upstream table's contents, so it cannot detect that the external pipeline started emitting different rows for the same schema. That is a data-*freshness* concern (the interval ledger / restatement story), not a *fingerprint* concern, and it is out of scope for the equivalence relation — the relation reasons about whether *smelt's transformation* changed, given its inputs, not about whether external inputs changed.

- **"Same type" does not yet imply "same rows" — the type-system coverage gap.** The relation in §5.2 reuses on type-equivalence, but several semantics that change *output values* are invisible to smelt's current type system and **must be treated as breaking-by-default until tracked**:
  - **Decimal precision/scale.** smelt's v1 fallback collapses decimal arithmetic to `Decimal(38,10)` regardless of operand precision (`types.md` §Known Divergences, "Decimal arithmetic v1 fallback"); precision/scale changes that alter rounding can occur under an unchanged printed type.
  - **Collation on `Text`.** Collation tracking is explicitly **out of scope for v1** (`types.md` §"Out of scope"), yet a collation change reorders `ORDER BY`, flips `<`/`=` comparisons, and changes `DISTINCT`/`GROUP BY` bucketing — all while the column still prints as `Text`.
  - **Nullability.** Parameter-type nullability is not in the v1 surface (`types.md` §"Out of scope"); a nullability change alters `COUNT`, `JOIN` matching, and `GROUP BY` output even when the base type is unchanged.
  - **Float associativity / timezone / ordering.** Floating-point sum order, `AT TIME ZONE` / session-timezone dependence, and unordered aggregate/window output are value-affecting without being type-affecting.

  The honest consequence: until the type system tracks these axes, the *provable* set is smaller than "all type-preserving changes" — the prover must conservatively exclude any change touching these dimensions. This is the single largest constraint on the soundness claim and the right place to gate coverage expansion (each axis becomes provable only once the oracle property tests cover it). Tracked as Open Question 11.

- **Cross-version fingerprint stability — design a version-stable canonical form, don't hash the version.** If smelt's canonicalization, type inference, or printer changes between releases, a naive fingerprint moves and every model rebuilds on upgrade day. SQLMesh's answer is to hash its SQLGlot version into state — correct but blunt: it forces a full rebuild on every upgrade. smelt should aim higher. The asymmetry matters here: the dangerous error is *reusing a table that should have rebuilt* (silent data corruption); a *spurious rebuild* is merely wasteful. Version-hashing trades the cheap error for the expensive-but-safe one on **every** release, which is the wrong default for a tool whose whole pitch is "don't rebuild what didn't change." The better target is a **fingerprint defined over a documented, version-stable normal form** — a canonical representation of the typed CST that the compiler is *contractually obliged* to keep stable across releases, defended by two test gates: **golden tests** (a corpus of models whose fingerprints are checked into the repo and must not move across a release without an explicit, reviewed bump) and **cross-version property tests** (build version *N* and *N+1*, assert equal fingerprints for output-equivalent models — the same generate-and-compare discipline as `type_property_tests.rs`, but across smelt versions instead of against DuckDB). Version-hashing remains the *fallback*: when a genuinely fingerprint-affecting change can't be made backward-stable, bump a declared `fingerprint_epoch` deliberately and document the forced rebuild — rather than letting every release silently move the hash. Note also that fingerprint drift can come from **more than inference + printer** — canonicalization rules, expansion order, column-origin tracking, and the normal form itself are all surfaces that must be covered by the golden/property gates; the normal form has to enumerate exactly what it includes. (This is the substance behind Open Question 6.)

- **Soundness is a proof obligation, not a vibe.** The equivalence relation needs property tests against the DuckDB oracle in the same spirit as `type_property_tests.rs`: generate two model versions, assert that "smelt says output-equivalent" ⇒ "DuckDB produces identical rows." A false-positive here corrupts data; this test gate is non-negotiable and should exist *before* any reuse is wired to execution.

The honest framing: smelt can *prove* a strictly larger non-breaking set than SQLMesh *guesses*, but only over the transparent, deterministic, typed core — which is exactly the core smelt is built to reason about. Outside it, smelt degrades to the same conservative rebuild SQLMesh defaults to. So the worst case is parity; the typical case is eclipse.

### 5.6 Persist the canonical *form*, not just the hash — "what changed", not only "did it change"

SQLMesh's fingerprint is a hash: the reuse decision it supports is binary — *same* or *different*. A hash discards everything except identity. The Stage 0 prototype (`crates/smelt-fingerprint`) deliberately computes the fingerprint as a SHA-256 *over a structured canonical form* (a by-name projection map, a normalised source/filter/group-by, recursive sub-fingerprints for FROM subqueries); the hash is only the final digest of that form. **Retaining the form turns "they differ" into "here is exactly what differs," and that opens a capability axis SQLMesh structurally cannot reach.**

What the form enables beyond a hash:

- **Column-level change detection, for free.** The form's projection is a map *output-column-name → canonical expression*. Diffing two forms by name yields precisely the set of output columns whose derivation changed. SQLMesh's edit-script knows *a* projection changed; it does not cheaply know *which output columns survive byte-identical*. smelt's form does, today, for the single-model case.
- **Targeted backfill: recompute a column subset, join it back.** A wide table where one column's logic changed need not be rebuilt wholesale. In principle smelt can recompute just the changed column(s) over the build window and join them onto the reused columns of the existing physical table — a backfill far narrower than SQLMesh's "rebuild the table." This is the natural physical realisation of the column-level diff.
- **Predicate/row-scoped migrations.** A change that only *narrows* a `WHERE` is, physically, a `DELETE` of the now-excluded rows — not a rebuild. A diff that recognises "filter strengthened, projection unchanged" can emit that targeted migration. (Widening a filter is the dual: backfill only the newly-admitted range.)
- **It is the natural input to the Stage 4 cascade.** The per-column impact analysis (§5.2) consumes exactly "which columns of M changed"; the structural form *is* that input, so §5.6 and Stage 4 share machinery rather than computing change-impact twice.

**Architectural fit.** The planner already returns reversible `Vec<Transformation>` values without mutating the CST (§4(e)). A structural diff between two canonical forms is precisely a `Transformation` describing the migration — "column `c` recomputed, columns `a, b` reused, rows where `p` deleted." So persisting the form lets the *same* abstraction smelt is built around express the backfill plan, instead of collapsing every change to an opaque "rebuild."

**Recommendation: keep both, with the hash derived from the form.** The hash stays the O(1) reuse *key* — the compact thing the env→table map indexes on and the fast "is this byte-identical?" check. The canonical form is persisted as the *change-analysis artifact*, consulted when the hashes differ to compute a targeted migration. This is not either/or: the hash is a derived index over the form.

**Honest costs and a hard gate.** This is more commitment than a 32-byte hash, and two limits must be named:

- **Don't actually persist the form — persist the SQL (see §5.6.1).** Persisting the *form* would be a serialization-compatibility obligation (its schema becomes an on-disk format needing its own migrations). §5.6.1 shows the better move: persist the expanded **SQL** that built the table and recompute the form (and fingerprint) on demand under the current compiler. That keeps the change-analysis power below while eliminating the on-disk-form-format and cross-version-stability burdens entirely.
- **Column-subset backfill needs row identity, which smelt does not yet track.** Joining a recomputed column subset back onto the reused columns requires a stable row/primary key to align rows. smelt has no row-identity mechanism today, so *column-subset reuse* is materially harder than *table reuse* and is **gated on a row-identity model** — table reuse (whole-table fingerprint match) needs none of this and ships first. The Stage 0 prototype encodes expressions as normalised token strings, which already supports *column-level* diffs ("which output columns changed") but not deep sub-expression or row-predicate diffs without a richer expression form; that richer form is the unlock for the predicate-scoped migrations above. So the staging is: table reuse (form-as-hash) → column-level diff for the Stage 4 cascade (form retained) → column-subset/predicate-scoped backfill (form retained **+** row identity). Each step is independently valuable; only the last needs the new row-identity work. Tracked as Open Question 15.

#### 5.6.1 Store the SQL, not the fingerprint — and the cross-version problem dissolves

There is a simpler move than persisting the form, and it makes the §5.5 / OQ6 cross-version stability problem *go away* rather than merely softening it. **Persist the expanded logical SQL that built each table; treat the fingerprint as an ephemeral, internal comparison function that is never stored.** Then on any plan — a normal edit *or* a version upgrade — the reuse test is always:

> compute `FP_current(stored_sql)` and `FP_current(current_sql)`, **both under the binary running right now**, and reuse iff they are equal.

Because both sides are always fingerprinted by the *same* (current) compiler, the fingerprint algorithm is free to change however it likes between releases — canonicalisation rules, field order, the normal form itself — with **zero migration code and no version-stable-form obligation.** The comparison is always apples-to-apples by construction. The whole apparatus §5.5 reached for (a contractually version-stable normal form, golden cross-version fingerprint tests, `fingerprint_epoch` bumps to force rebuilds) is needed only to keep the fingerprint stable *across versions* — and once the fingerprint is recomputed fresh on both sides every time, that requirement evaporates. On upgrade day, an unchanged model has `stored_sql == current_sql`, so the fingerprints are trivially equal and the table is reused, no matter how much the canonicaliser was rewritten underneath.

This also keeps **everything §5.6 wanted, for less.** The canonical *form* — the structured object that powers column-level diffs and targeted backfills — is simply *recomputed on demand* from the stored SQL under the current compiler, rather than persisted. You store the **source of truth** (the SQL) and derive the form when you need it; you never store a derived artifact you then have to migrate. So storing the SQL strictly dominates persisting the form: same change-analysis power, none of the serialization-compatibility or migration burden. (Persisting the fingerprint or form alongside the SQL is then at most an optional *cache* — recomputable, never authoritative.)

Two points keep it honest:

- **Store the *expanded* SQL (functions/macros resolved), with `smelt.<path>` refs left logical.** Expansion is part of the compiler, and what determines output is the expanded logical query. Freezing the expanded form at build time means a later change to a function body or to expansion itself shows up as a genuine difference between `stored_sql` and `current_sql` → rebuild, which is correct. (Storing only raw source would re-expand identically under the new compiler and miss expansion changes.)
- **This captures *logical* changes, not *output-semantics* changes.** Comparing logical SQL fingerprints detects every change to the query smelt reasons about. It does **not** detect a change to **codegen or the engine** that alters output for *unchanged* logical SQL — a dialect-emission fix, a cast-lowering change, an engine version bump. That class needs a separate, deliberate "output semantics changed → rebuild" signal (e.g. folding the codegen/engine version into table validity). This caveat is not special to storing SQL — *no* logical fingerprint scheme, including the persisted-form one, catches a codegen change, because the fingerprint is computed from the logical query, not the emitted output. It belongs with the engine/codegen-version question, separate from canonicalisation stability.

**The heavier alternative — persist the form and migrate it — is therefore not needed.** It would store each form tagged with a `fingerprint_epoch` and, on a breaking release, run a per-epoch `form → form` migration and compare against the re-derived form; reuse on agreement. That works and is sound under the same conservative-migration discipline (an incomplete migration only costs rebuilds; a meaning-altering one would be unsound, so migrations carry a canonicalisation-rule-grade proof obligation). But it buys nothing over re-fingerprinting stored SQL and costs a migration function per release plus an on-disk form-schema versioning story. Keep it in reserve only for a scenario the SQL path can't serve — e.g. a parser-breaking release where previously-valid stored SQL no longer parses (there, falling back to rebuild is the safe and probably acceptable answer anyway). **Recommended: store expanded SQL, recompute fingerprints fresh, persist no fingerprint as authoritative.** This is the substance of the revised Open Question 6.

---

## 6. The opt-in surface

### 6.1 State posture (from §3)

`state.mode: stateless | intervals | environments`, project-default with per-model frontmatter override. `environments` implies a state store.

### 6.2 Where state lives — default to zero new infrastructure

SQLMesh requires an OLTP database and warns against using the warehouse. smelt should default to the cheapest possible store and let users escalate:

- **Default: `embedded` (DuckDB file).** A smelt-managed `.smelt/state.db` DuckDB file. DuckDB is already a workspace dependency and the analytical engine, so this adds zero new infrastructure and no new dependency; it is the on-ramp. (SQLMesh warns that OLAP-for-state is a POC-only footgun, but that warning is about a *shared, concurrent* warehouse; smelt's embedded store is single-writer and local, so the warning does not bite. This resolves what was Open Question 2.) **The single-writer caveat is the whole story, though**: the embedded file fits the solo-developer and *ephemeral-per-run CI* cases, but it cannot back a **shared team environment** where multiple developers — or CI and a developer — read and write the same state concurrently. That case (which is much of the §2.7 value table, including "CI on every PR" against a shared env) needs `oltp`. So the embedded default is the on-ramp, not the general-purpose store, and the embedded file is also *more* fragile for disaster recovery than SQLMesh's recommended Postgres — a local file in a (typically gitignored) `.smelt/` directory is easy to lose, which makes the backup/DR story (OQ12) load-bearing the moment a team relies on it.
- **Escalate: `oltp`** — a Postgres (or other OLTP) connection string for teams running shared prod environments, matching SQLMesh's production guidance. This is also the substrate that makes **per-environment access scoping** (§2.1) enforceable: real database roles let a dev environment hold read-only credentials on prod's state while writing only to its own.
- **Transactional table stores (e.g. Delta / Iceberg) — worth investigating, not assumed bad.** A third option worth a spike: keep state in a transactional lakehouse table format (Delta, Iceberg) that the deployment already runs. The reflexive objection is performance — these formats are tuned for analytical scans, not OLTP point-updates, and the snapshot/interval ledger is update-heavy. But the workload here runs at **deploy/plan/apply time**, not in a hot query path, so latency tolerance is much higher than it first appears. The genuine sharp edge is *row-per-partition* churn on the interval ledger (many tiny updates → many tiny transactions/files). That may be mitigable by **batching** ledger updates into a single transaction per plan/apply, and by periodic compaction. Worth a measurement before ruling out — the payoff is state that lives in the same governed, backed-up, transactional store as the data, with no separate OLTP service to operate.
- **Pluggable** via a store interface (the existing `smelt-state` `FileStore` is one prototype implementation — see §3, observation 3, on its prototype status). The store is an interface; `embedded`, `oltp`, and any transactional-table backend are implementations behind it.

This directly satisfies "state store is not required for all smelt projects": stateless projects have no store; opted-in projects get an embedded DuckDB file unless they ask for more.

### 6.3 Environment addressing rides on `smelt.<path>`

smelt already resolves `smelt.<path>` to a `<schema>.<emitted_name>` physical location via the target's `schema:` (`architecture.md` §"Default materialization name mapping"). Environments slot in as a **schema-suffix function of the active environment**, exactly like SQLMesh's `db__dev` convention:

```
prod:        smelt.staging.orders  →  main.staging_orders
env "dev":   smelt.staging.orders  →  main__dev.staging_orders   (or a configured pattern)
```

The environment is a runtime parameter (`smelt run --environment dev`), and the env→snapshot map in the state store records which physical table each `smelt.<path>` resolves to in each environment. A **virtual update** is then: rewrite the environment's view layer to point at already-built physical tables — a pointer swap, no compute. This reuses the addressing scheme rather than inventing a parallel one.

### 6.4 Interaction with the incremental interval ledger

Environments and intervals compose. A model in `environments` mode carries both a fingerprint (which physical table) and an interval ledger (which time ranges are built in that table). The batch-safety class already tells us whether partial backfill is even legal: `PerPartitionOnly` / cumulative models are smelt's analogue of SQLMesh's "non-idempotent kinds cannot backfill limited ranges." So smelt's *existing* `FullyBatchSafe`/`BoundedSafe`/`PerPartitionOnly` proof is exactly the input the environment-aware backfiller needs — another place the type/planner layer pays off.

### 6.5 What promotion looks like end-to-end

1. `smelt plan --environment dev` — diff local CSTs vs the `dev` environment's recorded snapshots; compute output/metadata/physical fingerprints; categorize each changed model via §5.2; report the build set and the date range.
2. `smelt apply --environment dev` — build only the changed subgraph (over dev's date window) into new physical tables; reuse everything provably-equal.
3. Inspect / data-diff dev vs prod.
4. `smelt plan --environment prod` then `apply` — because the tables are already built and fingerprints match, this is a **virtual update**: prod's views are repointed, zero recompute.

---

## 7. Open questions and genuine forks

These are the decisions this paper deliberately does **not** settle. A few that earlier drafts left open have since been steered (marked **Decided**); they stay listed so the rationale is visible.

1. **Soundness vs. coverage of the prover — and user-defined rules.** How aggressive should the initial equivalence relation be? Option A: ship only the *unimpeachable* cases (formatting, projection reorder, dead-column removal, physical-only changes) and grow coverage behind property tests. Option B: aim for full column-lineage impact analysis from the start. **Direction: Option A** — every increment of coverage is gated by a new oracle test. The genuinely open part is **user-extensible equivalence rules**: should a project be able to register its own "these two forms are output-equivalent" rules (e.g. a domain-specific rewrite the core prover doesn't know)? This is attractive — it mirrors smelt's planner-rule extensibility philosophy — but it is *exactly* the place where an unsound rule silently corrupts data, so any extension surface must route through the same DuckDB-oracle property gate as the built-in relation (a user rule ships with the generator that proves it, or it doesn't ship). The hard question is whether the proof obligation can be made cheap enough that user rules are practical, or whether user input is better confined to *assertions* (like the per-model determinism assertion in §5.5) that smelt records-but-trusts rather than *rules* it must verify. Open.

2. **State store substrate default. — Decided: embedded DuckDB.** DuckDB is the default embedded store (§6.2): already a dependency, single-writer/local so the OLAP-for-state warning doesn't bite. sqlite was the considered alternative; DuckDB wins on zero-new-dependency. Kept here for rationale; no longer open.

3. **Per-environment state-store access scoping.** (Raised in §2.1.) Should the state-store schema and resolver support giving a dev environment **read-only** access to prod's state (fork-but-don't-mutate) and, symmetrically, letting prod be configured *not* to read dev-created tables (no pollution)? The capability should be *supported* (don't foreclose it in the schema design), with policy left to end users. Likely only enforceable under the `oltp` substrate, where database roles exist; the embedded file store can't enforce it. The open part is what the schema/credential model looks like and whether any of it lands before `oltp` does.

4. **Garbage collection of unreferenced physical tables.** Snapshots accumulate; SQLMesh has TTL/`janitor` semantics. smelt needs a retention policy (keep N versions / time-based / explicit `smelt gc`). When is a table safe to drop? (No environment references it *and* it's older than the revert window.)

5. **Forward-only and unsafe changes. — Direction: yes, expose it.** SQLMesh's `forward-only` (reuse the table, accept no clean revert) is valuable for huge tables where a rebuild is infeasible, and smelt wants it. The shape is the open part. A clean framing: a model that **opts into state** normally gets the safe, revertible, provable-equivalence machinery; forward-only is an explicit **escape hatch on a stateful model** — "apply this change in place, I accept no clean revert" — for genuinely breaking changes to enormous tables. (Models with no state are never snapshot-reused in the first place, so the clean-revert / forward-only question simply doesn't arise for them — it's specific to stateful tables that *would* otherwise get revertible reuse.) There may also be a broader "allow an *unsafe* change" hatch (the author overrides the prover's "this is breaking" verdict and reuses anyway). Both are deliberate, logged, user-owned overrides of soundness — the same trust-the-author pattern as the determinism assertions in §5.5. The open part is the exact surface and how loudly smelt warns.

6. **Cross-version fingerprint stability. — Direction (revised in §5.6.1): persist the expanded SQL and recompute fingerprints fresh, which dissolves the problem.** The original worry: a fingerprint computed and stored by version *N* goes stale when *N+1* changes the canonicalisation, forcing an upgrade-day rebuild of everything (SQLMesh's version-hash does exactly this). The clean answer is to **never store the fingerprint as authoritative** — persist the *expanded logical SQL* that built each table, and on every plan compare `FP_current(stored_sql)` vs `FP_current(current_sql)`, both under the running binary. The algorithm may then change freely with no migration and no version-stable-form obligation; an unchanged model trivially matches on upgrade. The canonical form is recomputed on demand from the stored SQL for change-analysis (§5.6), not persisted. Two residuals: store the *expanded* SQL so expansion/function changes register as logical changes; and a **codegen/engine-semantics change that alters output for unchanged logical SQL** is caught by *no* logical fingerprint (form-based or SQL-based) and needs its own deliberate rebuild signal (orthogonal — belongs with the engine-version question). The persisted-form + per-epoch-`form→form`-migration scheme (earlier draft of §5.6.1) is sound under a conservative-migration discipline but strictly heavier, and is kept only as a reserve for a parser-breaking release where stored SQL no longer parses (rebuild is the acceptable fallback there). A *within*-version-stable fingerprint is still wanted for run-to-run caching; cross-version stability — the original concern — is no longer required.

7. **Multi-backend. — Decided: environments span engines.** An environment is *not* per-engine; a single environment can contain DuckDB-materialized and Spark-materialized models. The env→table map therefore keys on `(environment, smelt.<path>)` and the *backend* is a property of the resolved physical table, not part of the environment identity. The physical fingerprint (§5.4) already carries engine, so a DuckDB→Spark migration is a physical-only change within the same environment. Interacts with the deferred `multi_backend.md`; the env-map schema must carry the resolved backend per table.

8. **Transactional table store for state (Delta / Iceberg).** (Raised in §6.2.) Worth a measurement spike: can a transactional lakehouse table format serve as the state store, given the workload is deploy-time (not hot-path) and the row-per-partition interval-ledger churn can be batched per plan/apply and compacted? Payoff: state in the same governed/backed-up store as the data, no separate OLTP service. Open until someone measures it.

9. **Data diff.** Cheap environments make "diff dev vs prod outputs" the natural next feature. smelt's type system enables *typed* diffs (schema-aware, column-aligned). Separate paper, but it's the obvious companion and a second place the type system eclipses a textual tool.

10. **Relationship to `run_state.md`.** The not-yet-authored `run_state.md` (manifest format, `.smelt/` layout, run IDs) is the natural home for the state-store layout this paper implies — including the snapshot/fingerprint/env-map schema and the per-environment access-scoping hooks (§3 obs. 3, OQ3). This work should probably *trigger* that spec.

11. **Type-system coverage for the equivalence relation.** (Raised in §5.5.) "Same printed type" does not yet imply "same values": decimal precision/scale, `Text` collation, and nullability are not tracked by the v1 type system, and float associativity / timezone / unordered-output are value-affecting without being type-affecting. Each axis must be treated as breaking-by-default and can be promoted into the provable set only once the DuckDB-oracle property tests cover it. The open question is the *ordering* — which axis pays off first (likely nullability, then decimal precision, then collation) — and whether some are better handled by the type system proper vs. a fingerprint-specific normalization.

12. **State backup / disaster recovery for the embedded store.** (Raised in §6.2.) SQLMesh's §2.5 framing — "state loss = forced full rebuilds" — applies to smelt too, and the *embedded* default is **more** fragile than SQLMesh's recommended Postgres: `.smelt/state.db` is a local, typically-gitignored file. What is the backup/restore story? Options: a `smelt state export/import` pair (SQLMesh has an analogue with "back it up first" warnings), checking a serialized form into the repo, or simply documenting that `embedded` is acceptable-to-lose because the warehouse can rebuild it (true today, but it forfeits the revert window and any forward-only tables). Unresolved.

13. **Data-quality gates and external readiness signals (audits / signals parity).** A SQLMesh user evaluating adoption will immediately miss two things smelt has no analogue for yet: **audits** (data-quality assertions that gate promotion — a failed audit blocks a plan from applying) and **signals** (external readiness checks that gate whether a model runs at all). smelt has `materialization: test` models (`testing.md`) which are adjacent to audits but are not wired as a *promotion gate*. Does the environment/plan machinery integrate test-model results as a gate on `apply`, and is there a signals-equivalent? Likely a companion spec, but naming it here so the adoption gap is visible.

14. **`plan` / `apply` / restatement UX and the cost model.** §6.5 sketches `smelt plan/apply --environment`, and §2.7 mentions restatement, but neither is specified as a concrete command surface a SQLMesh user could map their workflow onto. Open: the exact `plan` diff output (what a reviewer sees before approving), the `apply` confirmation/promotion flow, an explicit **restatement** command for re-running a date window (SQLMesh's `sqlmesh plan --restate-model`), and an honest **cost model** for the headline "spin up an env in seconds" claim — view creation over a large DAG is cheap but not free, and the number should be measured rather than asserted.

15. **Persist the canonical form, not just the hash? — Direction: keep both, gated.** (Raised in §5.6.) The fingerprint hash answers *did it change*; the structured canonical form answers *what changed*, which unlocks column-subset and predicate-scoped backfills SQLMesh cannot express. Direction: hash stays the O(1) reuse key, form is persisted as the change-analysis artifact. Open parts: (a) the form becomes a versioned on-disk serialization format (couples to OQ6's `fingerprint_epoch`); (b) column-subset backfill needs a **row-identity model** smelt lacks today, so it's gated behind table reuse and the Stage 4 column diff. The expression encoding may also need to be richer than normalised token strings to drive predicate-scoped migrations.

---

## 8. A staged, low-regret path

Each stage is independently valuable and the early ones de-risk the headline claim before any environment plumbing exists.

- **Stage 0 — Prove the eclipse (smallest experiment, days).** Build a *semantic-equivalence oracle*: given two versions of a model, compute `output_fingerprint` over the expanded typed CST and the column-lineage impact set, and assert against DuckDB that "smelt says equivalent" ⇒ "rows identical." Seed it with the §5.3 examples. **If smelt correctly calls a CTE-refactor or dead-column-removal non-breaking where SQLMesh rebuilds, the core thesis is proven** — with zero state store, zero environment machinery. This is the analogue of the dbt-adapter paper's "find one model where the human's lookback was unsafe" spike.

  **Prototyped — `crates/smelt-fingerprint`.** The structural half of the thesis is proven. The oracle's soundness gate (`fingerprint-equal ⇒ DuckDB relations identical`, multiset matched by column name) is green as a property test at 1000 cases, and the canonicaliser recognises as equivalent — where SQLMesh's edit-script rebuilds — formatting, comments, keyword case, **projection reordering**, **internal CTE/alias renaming**, and **single-use CTE ≡ derived table** (including a refactor *inside* the inlined body, via a recursive sub-fingerprint). A negative corpus confirms real changes always move the fingerprint. The cross-model cases (dead-column removal, downstream-spared changes — §5.3 #2/#3/#5) were **out of scope by design**: they need the lineage analyser gated to Stage 4, and Stage 0 needs no lineage at all. The fingerprint is computed over a *structured* canonical form (the substance behind §5.6), with conservative verbatim fallbacks (recorded as `MissedReuse`) for set operations, joins/multi-table FROM, recursive CTEs, deep subquery flattening, and the untracked type-system axes.

  Findings from the prototype worth carrying forward:
  - **Parser gap — implicit-alias column lists (resolved).** Investigating this surfaced a precise, narrower defect than first thought. The *explicit* form `FROM (…) AS t(c1, c2)` already parsed correctly. Only the *implicit* form `FROM (…) t(c1, c2)` (no `AS`) mis-parsed: the alias path consumed `t` and returned without checking for the trailing `(c1, c2)`, so the list leaked into the *enclosing* `SELECT`'s projection rather than naming the derived table's columns. DuckDB accepts both; smelt mis-parsed the implicit one, silently mangling the outer projection — caught only by the negative corpus, a reminder that a positive-only equivalence suite can pass vacuously. **Fixed** by extracting the column-list parse into a shared helper called from both alias paths (`crates/smelt-parser/src/parser/select.rs`, landed on `main`). The corpus SEED now uses the real `(VALUES …) AS t(id, total)` column-alias list instead of a `SELECT … AS` / `UNION ALL` workaround, and canonicalises structurally end-to-end.
  - **Canonicaliser soundness bug — a derived-table-left join was dropped (resolved).** Measuring the verbatim-fallback rate over the real example workspaces (`retail_analytics` 24/25 structured, `timeseries` 9/9 of the parseable models) corrected an early assumption: joins do *not* force a verbatim fallback — when the single-derived-table inlining doesn't apply, the whole FROM (joins included) is kept as a normalised token string, so most real models fingerprint *structurally*. But the high structured rate rested on a code path the soundness property test never exercised (it generated only single-table reorder/filter/expr edits). Wiring adversarial **join** cases into the property test surfaced a real false-equivalence: `try_inline` represented a query by its single left derived table whenever `table_refs().len() == 1`, but a join's right table is nested in a `JOIN_CLAUSE` and is *not* a `table_ref()`, so `FROM (Q) AS l JOIN r ON …` inlined `l` and **silently dropped the join** — `SELECT l.a FROM (Q) AS l` and the same query with a row-eliminating `INNER JOIN` produced an identical fingerprint. This is the data-corruption class the gate exists to prevent. **Fixed** by bailing from `try_inline` when the FROM has any join (`crates/smelt-fingerprint/src/canonical.rs`), matching the function's own documented intent; captured by three negative-corpus regressions plus a generative join soundness property test. The meta-lesson reinforces the parser finding: a soundness gate that doesn't *generate* the dominant real-world shape (joins) passes vacuously. The §8 de-risking priority is therefore the **§5.5 adversarial axes** (decimal/collation/nullability/determinism) and **equivalence *depth* on join models**, not coverage breadth.
  - **Canonicaliser soundness bug — `LIMIT`/`OFFSET`/`QUALIFY` were dropped from the fingerprint (resolved).** Extending the property generator to the §5.5 axes (row-affecting tail clauses, decimal scale, nullability, `DISTINCT`) immediately found a second, more common false-equivalence: `CanonForm` had no `LIMIT`/`OFFSET`/`ORDER BY`/`QUALIFY` field, so all of `SELECT a FROM t`, `… LIMIT 1`, `… LIMIT 1 OFFSET 1`, and `… ORDER BY a ASC/DESC LIMIT 1` collapsed to **one fingerprint** — every top-N, paginated, or window-filtered model a potential silent reuse of the wrong rows. **Fixed** by a conservative verbatim fallback when the SELECT carries a `LIMIT`/`FETCH`/`QUALIFY` clause (`crates/smelt-fingerprint/src/canonical.rs`); a bare `ORDER BY` with no slice stays soundly ignored (the relation is multiset-by-name). Captured by five negative-corpus regressions and a generative §5.5 property test. The generator also demonstrated a *second-order* §5.5 hazard: a bare `LIMIT`/`OFFSET` **without** a total `ORDER BY` is non-deterministic — two runs of byte-identical SQL returned different rows — i.e. the "unbounded-without-total-order" non-determinism the doc flags (§5.5) bites on ordinary pagination, not just `now()`/`random()`. The property gate therefore only asserts soundness over deterministic queries; the broader determinism story (mark non-deterministic models non-reusable) is the de-risking step prototyped next.
  - **Determinism detector — inline non-determinism makes a fingerprint match not relation-equality (prototyped).** A fingerprint proves two model *versions* compute the same relation *for the same inputs*; that is only relation-equality if the model is a pure function of its inputs. The §5.5 second-order finding above (unordered pagination is non-deterministic) generalised the need: smelt tracks `deterministic` as a declared *function* property, but non-determinism also enters through inline SQL with no call node to tag — a bare `random()`/`now()`, or `LIMIT`/`OFFSET`/`FETCH` without a provably total order. `output_fingerprint` now also returns `deterministic: bool` (+ `non_determinism` reasons), computed by a structural detector (`crates/smelt-fingerprint/src/determinism.rs`): a deny-list of non-deterministic built-ins (`random`/`uuid`/`now`/`current_timestamp`/…, including the parenless temporal specials that surface as a bare identifier) plus a row-slicing-without-total-order check. The signal is **orthogonal to the fingerprint** — it does not change the hash (identical SQL fingerprints identically), and a model can be fully `canonicalisable` yet non-deterministic; it is metadata the reuse layer consults so a match on a non-deterministic model rebuilds rather than pointing a new environment at a stale materialisation. The detector is deliberately conservative (over-flagging is sound — worst case parity), so a `LIMIT` even with an `ORDER BY` is still flagged until sort-key totality can be proven (needs key-uniqueness/lineage). The load-bearing invariant — *anything flagged deterministic is reproducible* — is gated by a new property test (`tests/determinism_prop.rs`): every injected non-deterministic construct must flip the flag, and any query the detector calls deterministic must yield the **same relation built twice in independent DuckDB instances** (green at 2000 cases). The §5.5 author escape hatches (*accept-current*, *assert-deterministic*) remain downstream policy for the reuse layer, not the detector. **Aggregate gap closed (finishes §5.5's value axes):** the deny-list now also covers the order-sensitive aggregates (`array_agg`/`list`/`string_agg`/`group_concat`/`listagg`/`any_value`/`arbitrary`) — their result depends on a fold order a relation does not fix, and since smelt has no aggregate-`ORDER BY`/`WITHIN GROUP` syntax there is no deterministic way to write them today, so the by-name rule is exact, not merely conservative. Order-*insensitive* aggregates (`sum`/`count`/`min`/`max`/`avg`) stay deterministic, pinned by the same property test (which also reproduces them across two DuckDB builds). (`first`/`last` are order-sensitive too but are smelt *keywords*, so they cannot be written as aggregate calls at all — nothing to match.) The one residual non-determinism source is order-sensitive **window** functions (`row_number`/`rank`/`lag`/`first_value`/… over a non-total `ORDER BY`), which need `OVER`-clause analysis rather than a name match and are deferred behind the same gate.
  - **The form is more valuable than the hash** — see §5.6 and Open Question 15.
- **Stage 1 — `state.mode: intervals` (opt-in, embedded store).** Promote the existing `smelt-state` interval ledger to a first-class, opt-in posture with the embedded `.smelt/state.db`. Delivers precise backfill/restatement and gap detection. No environments yet. Stateless stays the default and the untouched path.
- **Stage 2 — Snapshots + fingerprints.** Persist per-model snapshots keyed by `output_fingerprint`; record the env→table map; implement table reuse on fingerprint match. Single (`prod`) environment only — proves reuse end-to-end without the naming layer.
- **Stage 3 — `state.mode: environments`.** Environment-suffixed addressing (§6.3), `smelt plan/apply --environment`, virtual update on promotion. This is the full SQLMesh-parity VDE.
- **Stage 4 — Provable categorization in `plan`.** Wire §5.2's column-lineage impact analysis into `plan` so backfills cascade only as far as types prove necessary — the visible eclipse.
- **Stage 5 (companion) — typed data diff, GC/retention, forward-only.** The operational polish from §7.

Spec-wise: this work should author `run_state.md` (state layout) and a new `virtual_environments.md` spec, and will touch `incremental_models.md` (interval ledger becomes opt-in posture, not "smelt never stores state"), `architecture.md` (a fingerprint/snapshot subsystem; the `state.mode` surface), and `schema_evolution.md` (physical-migration handling from §5.4).

---

## 9. Why this is smelt-shaped, not a SQLMesh port

The temptation is to clone SQLMesh: state DB, snapshots, schema suffixes, plan/apply. smelt should adopt that *plumbing* — it's well-designed and proven — but the **reason to build it in smelt** is that smelt computes the central judgement *better*:

- SQLMesh decides table reuse and backfill cascade with a **syntactic AST edit-script** that is conservative by necessity ("anything but Insert/Keep is breaking"), because SQLGlot has no type system and column-level lineage is still on its roadmap.
- smelt decides the same thing with a **typed equivalence relation** built on its type system and logical/physical separation — the type inference and per-column-type provenance exist today; the cross-model column-lineage analyser the *full* relation needs is the substantive new work this proposal implies (§4(b), §5.2), buildable on those assets rather than free.

The result: every place SQLMesh conservatively rebuilds a provably-unchanged table, smelt *can* reuse it — cheaper environments, smaller backfills, faster CI. But the honest claim is staged, not absolute. **Where smelt's analysis applies — the transparent, deterministic, typed, *annotated* core — the eclipse is real and provable.** Where it goes dark (un-annotated determinism defaulting to `false`, type-system axes not yet tracked: collation/decimal-precision/nullability, black-box non-determinism), smelt degrades to SQLMesh-equivalent conservatism. So the worst case is parity, and the *eclipse is the achievable destination as coverage is built* — not a property the typical un-annotated pipeline enjoys on day one. The size of the day-one win scales with how much of §5.5's coverage and §4(b)'s lineage analyser ships, gated at every step by the DuckDB oracle. And all of it sits behind an opt-in that leaves stateless smelt projects exactly as they are today.

---

## References

**SQLMesh**
- [Environments](https://sqlmesh.readthedocs.io/en/stable/concepts/environments/) — virtual environments, schema suffixing, snapshot references, cost model.
- [Plans](https://sqlmesh.readthedocs.io/en/stable/concepts/plans/) — plan/apply, change categories, backfill, virtual update.
- [Snapshots](https://sqlmesh.readthedocs.io/en/stable/concepts/architecture/snapshots/) — fingerprints, what a snapshot captures, formatting-insensitivity via SQLGlot.
- [State](https://sqlmesh.readthedocs.io/en/stable/concepts/state/) — what state is stored, OLTP requirement, loss/backup, suitable engines.
- [Model kinds](https://sqlmesh.readthedocs.io/en/stable/concepts/models/model_kinds/) — FULL/VIEW/INCREMENTAL_*/SCD_TYPE_2/EMBEDDED/SEED; non-idempotent restatement limits.
- [Tobiko: Automatically detecting breaking changes in SQL queries](https://www.tobikodata.com/blog/automatically-detecting-breaking-changes-in-sql-queries) — the SQLGlot semantic-diff edit-script; "only addition of a new projection is non-breaking"; `SELECT *` expansion; per-column lineage as future work.

**smelt**
- `docs/specs/architecture.md` — single typed CST, byte-identity printer, type system, models-as-functions, materialization⊥transparency, `Transformation` values, `smelt-state` crate, default name mapping.
- `docs/specs/incremental_models.md` — current "smelt does not own state" stance and rationale; batch-safety taxonomy; interval-store.
- `docs/specs/expansion.md` — function expansion before analysis (fingerprint over expanded CST).
- `docs/specs/types.md`, `crates/smelt-db/src/type_inference.rs`, `crates/smelt-db/tests/type_property_tests.rs` — type system + DuckDB oracle (the model for the equivalence prover's test gate).
- `docs/research/20260529-dbt-incremental-adapter.md` — the stateless↔stateful spectrum and per-project/per-model state-posture direction this paper extends.
