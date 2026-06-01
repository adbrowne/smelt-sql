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

smelt has, at exactly that layer, the three things SQLGlot lacks:

- a **single typed CST** with a byte-identity printer (`architecture.md` §"Identity properties"),
- a **real type system with column-origin lineage** in `smelt-db` (`RowExtension`/`InputConstraint` carry per-column provenance), and
- **logical/physical separation** — the model body is the pure *what*; materialization, incrementality, and engine are the *how* the planner decides.

That lets smelt replace SQLMesh's *syntactic guess* with a **typed, provable equivalence relation**: reuse a physical table when the change cannot alter any column a downstream model actually consumes — proven from types and column lineage, not guessed from tree edits. This is the eclipse. Everything else (a state store, environment-suffixed schemas, "virtual update" promotion) is plumbing smelt can adopt as an **opt-in layer** that stateless projects never pay for.

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

The thing that makes environments *cheap* — the whole point — is that **an environment is a collection of references to model snapshots, not a copy of data**. When a model's fingerprint matches an existing snapshot, the new environment's view simply points at the **same underlying physical table**. "Any computation that was done in this environment can be safely reused in other environments." A dev environment over an unchanged DAG costs essentially nothing: it is a set of views over prod's physical tables.

Andrew: Can we note somewhere that SQLMesh as I understand it has limitation around dev and prod sharing a state store (in that you can't really isolate them). I think it would be good to consider having dev's state store being linked to prod such that developer have creds in the dev envrionment that can read from prod state store (and therefore fork) but can't write to it. That allows an infra setup that is even safer. This is not a key change but I would like not to lose this as it as we may want to take it into account when desigining the state store schema/infra. This also might only work once we have OLTP store rather than file/duckdb/sqlite. We might also decide to have prod *not* be able to read the dev tables so we never pollute - all this would be a decision for end users - we just need to support.

Two further cost levers:

- **Time-bounded dev data.** Dev environments carry start/end dates so you can build only a recent slice of history.
- **Gap-only compute.** SQLMesh "only computes data gaps that have been directly caused by the changes" — unchanged upstreams are reused; only the changed subgraph is built.

### 2.2 The mechanism underneath: snapshots and fingerprints

A **snapshot** is "a record of a model at a given time… everything needed to evaluate the model and render its query" — the model's query, the macros active at capture, global variables, and the data intervals available for that snapshot ([snapshots](https://sqlmesh.readthedocs.io/en/stable/concepts/architecture/snapshots/)).

Each snapshot has a **fingerprint** "derived from [its] model." The fingerprint is what decides table reuse: "This fingerprint allows SQLMesh to detect if a given model variant exists in other environments or if it's a brand new variant." Critically, "Because SQLMesh can understand SQL with SQLGlot, it can generate fingerprints such that superficial changes to a model, such as applying formatting… will not return a new fingerprint." (SQLMesh splits this into a `data_hash` over things that affect output and a `metadata_hash` over things that don't, so a comment or owner change can be a *metadata-only* change.)

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

2. **The opt-in boundary is the model, not just the project.** A project can run environments while a specific high-churn or externally-fed model stays stateless (always rebuilt, never snapshot-reused). This matches the per-model granularity in the recorded direction and avoids forcing a posture on models where it doesn't pay.

3. **smelt already owns part of the stateful end.** `smelt-state` exists (`RunManifest`, `IntervalStore`, `FileStore`; `architecture.md` crate table) and the runtime already does interval-store updates. The interval ledger is *most* of what "precise backfill/restatement" needs. VDEs add the *fingerprint + environment→table map* on top. So this is an extension of an existing seam, not a greenfield subsystem.

Andrew: note that the smelt-state was a quick attempt and is not well tested. I'm open to evolving it or throwing it away and rebuilding depending on how much it aligns with our decisions here.

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

`stateless` = today's behaviour, no store. `intervals` = durable interval ledger (precise backfill/restatement, gap detection) but no environments. `environments` = intervals + fingerprints + env map. A model never opted in is always built fresh in whatever environment references it — correct, just not cheap.

---

## 4. What smelt already has that SQLMesh approximates

SQLMesh is built on SQLGlot, a parser/transpiler. smelt is built on a *typed compiler* with an LSP-grade analysis layer. Four standing smelt assets line up directly against SQLMesh's mechanisms:

**(a) Single typed CST + byte-identity printer → an exact, principled fingerprint.** `architecture.md` §"Identity properties" guarantees the DuckDB printer emits byte-identical SQL modulo a fixed, documented set of rewrites, and the parser is fingerprint-equivalent to PostgreSQL via pg_query. SQLMesh gets formatting-insensitivity by *normalizing through SQLGlot*; smelt gets it from a printer whose identity is a **property-tested invariant**. The canonical form for fingerprinting is something smelt already maintains and tests. (Parity here, not eclipse — but a more principled foundation.)

**(b) Type system + column-origin lineage → provable, not syntactic, change impact.** `smelt-db` computes per-column provenance (`RowExtension.ref_name`, `InputConstraint`, schema inference) and full type inference (`type_inference.rs`, the property-tested oracle vs DuckDB). This is exactly the column-level lineage SQLMesh lists as *future* work. smelt can answer "does this change alter the type or derivation of column `c` that downstream model M actually reads?" — the question SQLMesh approximates with an AST edit-script.

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

This is SQLMesh's three categories — but the boundary between them is drawn by **column-level provenance + types**, which smelt has today, instead of "added a projection vs everything else."

### 5.3 Worked examples where smelt reuses and SQLMesh rebuilds

These are the cases that justify the whole proposal:

1. **Refactor with identical output.** Split a monolithic `SELECT` into CTEs, or reorder the projection list, or rename an internal alias. SQLGlot's edit-script contains `Move`/`Update`/`Insert` ⇒ **breaking** ⇒ full rebuild + downstream cascade. smelt: `output_fingerprint` unchanged (same columns, same types, same derivations) ⇒ **table reused, zero compute.**

2. **Remove a column nobody downstream reads.** SQLMesh today: a `Remove` in the edit-script ⇒ **breaking** for the whole subtree (per-column lineage that would fix this is future work). smelt: column-origin lineage shows no downstream consumes it ⇒ **non-breaking; downstream untouched.**

3. **Change a column's logic when only *other* columns flow downstream.** Model M computes `a, b, c`; downstream D selects only `a, b`. A change to how `c` is computed. SQLMesh: edit-script has `Update` ⇒ breaking subtree. smelt: D's lineage touches only `a, b`, both unchanged ⇒ **non-breaking for D**, only M rebuilt.

4. **Physical-strategy change (the structural win).** Flip M from `view` to `table`, or full-refresh to incremental, or pin it to Spark. SQLMesh: model object changed ⇒ snapshot churns. smelt: logical `output_fingerprint` unchanged; only the *physical fingerprint* changed ⇒ classified as a physical migration, no logical backfill cascade (§5.4).

5. **Type-narrowing that is provably compatible.** Widen `INT`→`BIGINT` on a column whose downstream uses are all type-compatible. smelt's type system can prove downstream type-checks still hold; SQLMesh has no type system to make that call and rebuilds.

### 5.4 Physical changes are first-class and separate

Because materialization is a planner decision, a *physical* fingerprint (engine, materialization kind, incremental config, partition column) tracks separately. A change to physical strategy with an unchanged logical `output_fingerprint` is a **physical migration**: smelt may need to re-materialize M (a view has no table to reuse) but the *logical contract* is intact, so **no downstream rebuild is implied**. This cleanly separates "I changed the math" from "I changed how it's stored" — a distinction SQLMesh's model object blurs.

### 5.5 Risks and honest limits of the prover

A semantic equivalence prover must be **sound** (never call a real change non-breaking) even at the cost of completeness (sometimes conservatively rebuild). The hard cases, all of which must degrade to "treat as breaking":

- **Opaque calls.** `smelt.extern`, canonical built-ins, and sources are black boxes (`architecture.md` two-axis table). A change inside, or to the declared signature of, an opaque dependency is not analyzable ⇒ conservative rebuild. This mirrors the incremental rule's `NotDerivable` refusal — same philosophy: *no silent unsafe reuse.*

Andrew: not 100% convinced on this - if we declare smelt.externs determistic then there should be no need to rebuild simply because a model contains it - obviously if they change to using one we can't guaratee much.

- **Non-determinism.** `RANDOM()`, `NOW()`, ordering-without-`ORDER BY`: a model that isn't deterministic can't have its output proven equal across versions. smelt already tracks `deterministic` as a function/extern property — feed it in.

Andrew: We may want to give the user some choice here - if the initial build wasn't deterministic maybe they are ok with just leaving the current value. Maybe they make functions safe per model or something?

- **`SELECT *` and row-polymorphism.** smelt resolves `*` through the type system, so star-selects are *better* off than SQLMesh's textual expansion — but a `*` over an opaque source inherits that source's opacity.

Andrew: I don't think smelt supports opaque sources - and I believe * resolves to what the source explicitely declares - not all fields - can you check?

- **Cross-version type-inference drift.** If smelt's own type inference changes between releases, fingerprints can move. SQLMesh hashes its SQLGlot version into state for exactly this reason; smelt must hash an inference/printer version too. (This is a real maintenance tax, named here, not waved away.)

Andrew: Can we explore whether we can produce some other sort of expression that we maintain from version to version. And we could use some golden tests to ensure it remains consistent (or property based tests that go from version to version?). It seems more a risk that we for rebuild by mistake rather than don't force rebuild? Breaking on every release seems bad - also I suspect the potential changes come from more than just those two places?

- **Soundness is a proof obligation, not a vibe.** The equivalence relation needs property tests against the DuckDB oracle in the same spirit as `type_property_tests.rs`: generate two model versions, assert that "smelt says output-equivalent" ⇒ "DuckDB produces identical rows." A false-positive here corrupts data; this test gate is non-negotiable and should exist *before* any reuse is wired to execution.

The honest framing: smelt can *prove* a strictly larger non-breaking set than SQLMesh *guesses*, but only over the transparent, deterministic, typed core — which is exactly the core smelt is built to reason about. Outside it, smelt degrades to the same conservative rebuild SQLMesh defaults to. So the worst case is parity; the typical case is eclipse.

---

## 6. The opt-in surface

### 6.1 State posture (from §3)

`state.mode: stateless | intervals | environments`, project-default with per-model frontmatter override. `environments` implies a state store.

### 6.2 Where state lives — default to zero new infrastructure

SQLMesh requires an OLTP database and warns against using the warehouse. smelt should default to the cheapest possible store and let users escalate:

- **Default: `embedded`** — a smelt-managed file in `.smelt/state.db` (DuckDB file, or sqlite). Zero new infrastructure; fits the "POC / single developer / CI" cases that are most of the value. This is the on-ramp.
- **Escalate: `oltp`** — a Postgres (or other OLTP) connection string for teams running shared prod environments, matching SQLMesh's production guidance.
- **Pluggable** via the existing `smelt-state` crate (`FileStore` already exists). The store is an interface; `embedded` and `oltp` are implementations.

Andrew: I also wonder whether there is some way to support some table stores (like delta) that have transactions. How bad can the perf really be? Particularly as it's on deployments. The worst part might be row per partition - but maybe we could come up with a clever way to batch updates?

This directly satisfies "state store is not required for all smelt projects": stateless projects have no store; opted-in projects get an embedded file unless they ask for more.

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

These are the decisions this paper deliberately does **not** settle:

1. **Soundness vs. coverage of the prover.** How aggressive should the initial equivalence relation be? Option A: ship only the *unimpeachable* cases (formatting, projection reorder, dead-column removal, physical-only changes) and grow coverage behind property tests. Option B: aim for full column-lineage impact analysis from the start. Recommendation leans A — every increment of coverage is gated by a new oracle test.

Andrew: Agree with building up from some quite safe starting point - I wonder whether we could actually allow user extensions for extra rules?

2. **State store substrate default.** Embedded DuckDB file vs sqlite for `.smelt/state.db`. DuckDB is already a dependency and the analytical engine; sqlite is a better OLTP fit and tiny. SQLMesh explicitly warns OLAP-for-state is a POC-only footgun — but smelt's embedded store is single-writer/local, so the warning may not bite. Open.

Andrew: this one seems fine to me - duckdb should be the default.

3. **Garbage collection of unreferenced physical tables.** Snapshots accumulate; SQLMesh has TTL/`janitor` semantics. smelt needs a retention policy (keep N versions / time-based / explicit `smelt gc`). When is a table safe to drop? (No environment references it *and* it's older than the revert window.)

4. **Forward-only changes.** SQLMesh's `forward-only` (reuse the table, accept no clean revert) is valuable for huge tables where a rebuild is infeasible. Does smelt expose this as an explicit per-plan/per-model escape hatch, or does the provable-equivalence machinery make it rarely needed? Probably still needed for genuinely breaking changes to enormous tables.

Andrew: yes I think we 100% want that - although maybe those are default smelt tables without state - but maybe we want to have a mechanism to allow *unsafe* changes or forward only changes to tables that opt into state.

5. **Cross-version fingerprint stability.** Hash the inference/printer version into the fingerprint (correct but causes a one-time rebuild on smelt upgrades) vs. a compatibility manifest that proves an upgrade is fingerprint-neutral. The latter is more work but avoids upgrade-day full rebuilds.

Andrew: as above I don't want everything to rebuild when smelt version changes.

6. **Multi-backend.** Environments across DuckDB+Spark: does an environment span engines, and does the env→table map key on `(environment, backend)`? Interacts with the deferred `multi_backend.md`.

Andrew: yes environments should span engines.

7. **Data diff.** Cheap environments make "diff dev vs prod outputs" the natural next feature. smelt's type system enables *typed* diffs (schema-aware, column-aligned). Separate paper, but it's the obvious companion and a second place the type system eclipses a textual tool.

8. **Relationship to `run_state.md`.** The not-yet-authored `run_state.md` (manifest format, `.smelt/` layout, run IDs) is the natural home for the state-store layout this paper implies. This work should probably *trigger* that spec.

---

## 8. A staged, low-regret path

Each stage is independently valuable and the early ones de-risk the headline claim before any environment plumbing exists.

- **Stage 0 — Prove the eclipse (smallest experiment, days).** Build a *semantic-equivalence oracle*: given two versions of a model, compute `output_fingerprint` over the expanded typed CST and the column-lineage impact set, and assert against DuckDB that "smelt says equivalent" ⇒ "rows identical." Seed it with the §5.3 examples. **If smelt correctly calls a CTE-refactor or dead-column-removal non-breaking where SQLMesh rebuilds, the core thesis is proven** — with zero state store, zero environment machinery. This is the analogue of the dbt-adapter paper's "find one model where the human's lookback was unsafe" spike.
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
- smelt decides the same thing with a **typed equivalence relation over column-level provenance**, which it already computes for the LSP, and with **logical/physical separation** that makes a whole class of changes metadata-only by construction.

The result: every place SQLMesh conservatively rebuilds a provably-unchanged table, smelt can reuse it — cheaper environments, smaller backfills, faster CI — while degrading to SQLMesh-equivalent conservatism exactly where smelt's own analysis goes dark (opaque calls, non-determinism). Worst case parity, typical case eclipse, and all of it behind an opt-in that leaves stateless smelt projects exactly as they are today.

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
