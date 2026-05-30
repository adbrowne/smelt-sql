# Research: smelt as a dbt adapter — planner rules for naive dbt models (incremental first)

**Date**: 2026-05-29
**Topic**: Whether smelt can add value *under* dbt — as an adapter scoped to smelt's own backends — by letting users write naive (full-refresh) dbt models that smelt's planner rewrites (auto-incrementalization first, then the broader rule layer: cubes/grouping-sets, cumulative aggregates), plus an LSP that type-checks dbt files via dbt's own artifacts. Strategic goal: a low-friction **migration on-ramp**, and longer-term **planner rules as a typed alternative to dbt macros**.
**Motivating question** (Andrew): "Could smelt be used as a dbt adapter that adds value, or as a step along the way?" Refined over discussion into: *write naive dbt models, get smelt's planner-driven auto-incrementalization and LSP benefits, without rewriting the dbt project.*
**Related**: `docs/research/20260521-incremental-as-planner-rule.md`, `docs/research/2026-05-20-incremental-gaps-from-web-analytics.md`, `docs/research/20260523-lsp-cli-ui-divergence.md`. Fills part of the acknowledged-empty `migration_from_dbt.md` gap (`docs/specs/architecture.md:383-394`).

## Summary

The generic "smelt as a dbt adapter" idea fails because a dbt adapter operates *after* Jinja is rendered — by then `is_incremental()` is baked in, `ref()` is resolved to table names, and the logical model smelt needs is gone. The adapter seam strips smelt down to "a thing that runs SQL," which is the one thing smelt explicitly is *not* (`architecture.md`: "a compiler and orchestrator, not a query engine").

Three refinements, raised in sequence, rehabilitate the idea into something genuinely smelt-shaped:

1. **Naive models preserve the logical SELECT.** If authors write plain full-refresh `SELECT`s (no `is_incremental()`), the compiled SQL dbt hands down *is* the logical model. Incrementalization becomes smelt's job — the regime its planner is built for. The parser's SELECT-only limitation flips from blocker to asset.
2. **Scope the adapter to smelt's own backends (DuckDB + Databricks/Spark).** The limiting factor in supporting an engine is *dialect adaptation*, not query submission. smelt already does dialect adaptation for its three dialects. Scoping to those engines turns the "no Snowflake/BigQuery" gap into a supported-platforms list and lets smelt **own the execute loop** rather than hand SQL back to dbt.
3. **Present the models to dbt as materialized-view-like.** dbt already has a contract for "a relation whose refresh mechanics are opaque and fully delegated to the adapter" — the materialized-view materialization. Adopting that contract reduces dbt-side incremental state to ~zero and makes delegation *expected* rather than a conflict.

The differentiator vs dbt's own microbatch (shipped 1.9) is sharp and defensible: **dbt makes you declare the time-window safety; smelt derives and proves it.** This is the project's own "derive, don't declare" principle aimed at dbt's biggest operational pain.

A second, independent half — an **LSP that reads dbt's `manifest.json` + `catalog.json`** — recovers types-while-editing on dbt files without making anyone switch file formats. Both halves reward the same author behavior (less Jinja), so they reinforce each other.

Incremental is the *wedge*, not the ceiling: it is one planner rule, and the same seam delivers the rest of the rule layer (cube / grouping-sets, cumulative aggregates, …). The strategic framing is **planner rules as a typed, correctness-preserving alternative to dbt macros** — see "Generalization" below.

## Scenarios considered

| Scenario | Verdict | Why |
|---|---|---|
| dbt-style adapter, broad warehouse support | ✗ | smelt emits only DuckDB/SparkSQL/PostgreSQL (`smelt-dialect/src/dialect.rs`); can't be a general warehouse adapter |
| smelt reads dbt sources (importer) | ◐ | Best pure on-ramp, but needs a full Jinja evaluator; deferred |
| smelt under dbt as post-compile optimizer | ✗ | Compiled SQL has incrementality baked in; planner can't help |
| **dbt-style adapter, scoped to smelt backends, naive models** | **✓** | Subject of this doc — see below |

The first-pass analysis ranked "dbt-style adapter" last. That ranking assumed broad-warehouse dialect support was mandatory. Once support is *scoped* to smelt's backends (refinement 2), the adapter becomes the correct vehicle — dbt drives the lifecycle, smelt is the dialect-aware, incremental-aware engine underneath for the engines it speaks.

## Why naive models are the unlock

The earlier kill-shot: the author bakes `is_incremental()` into the SQL upstream, so smelt sees post-incremental SQL. Naive models invert this — the author writes a plain full-refresh SELECT, incrementalization is smelt's job, and smelt sees the logical model intact. The constraints then line up:

| Earlier blocker | Under "naive models + planner" |
|---|---|
| Parser is SELECT-only | Now an asset — naive models *are* SELECTs |
| Batch-safety needs schemas | Verified **purely structural** — `analyze_batch_safety` reads SQL text + declared event-time/partition columns only, no schemas (`smelt-planner/src/rules/incremental.rs:55-128`) |
| Planner needs whole project | Minor — `incremental::optimize(model_info)` runs on one model (`incremental.rs:570`); or build a 1-model `ModelGraph` |

Targeting is favourable: the models that most need auto-incremental are the largest-data, simplest-SQL fact/aggregate models — exactly the naive ones. This is a "covers the high-value subset, ignores heavy-macro models" tool, and that subset is where the pain lives.

## Differentiator vs dbt microbatch (the pitch)

dbt 1.9 ships a microbatch incremental strategy, so the honest question is "why not just use that?"

| | dbt microbatch | smelt planner |
|---|---|---|
| `lookback` | **Declared.** Guess wrong → silent wrong results | **Derived** from the SQL's actual temporal dependencies: `context_days = max(lookback, lookahead)` (`incremental.rs:66-98`) |
| Chunk size | manual `batch_size` | **auto-sized**: `max(context_days*3, clamp 7..90)` |
| Unsafe SQL (window crossing partitions) | not detected → full scan or corruption | classified `PerPartitionOnly` and handled, not corrupted |
| "covered enough history?" | dbt "doesn't know the minimum event_time"; skipped runs → silent data gaps | safety class is a *proof*, not a config |

Batch-safety classes (`incremental.rs:29-48`): `FullyBatchSafe` (no temporal dep → one query for the whole range), `BoundedSafe { max_chunk_days, context_days, reason }` (bounded lookback → auto-sized chunks), `PerPartitionOnly { reason }` (unbounded dependency → one partition per query). Classification is structural over the SELECT's window frames, `LAG`/`LEAD` offsets, and interval joins — no schema needed.

This is "derive, don't declare" (a standing project principle) applied to dbt's worst operational footgun. The single most convincing validation: **find one real model where a human-declared microbatch `lookback` was unsafe and smelt's derived `context_days` catches it.**

## Generalization: the planner-rule layer, not just incremental

Incremental is the wedge, not the whole value. It is *one planner rule*; the same seam delivers the rest of smelt's rule layer to naive dbt models:

- **Cube / grouping sets** — a naive SELECT marked as a cube over dimensions → planner rewrites to `GROUPING SETS` / `ROLLUP` (the `cube_result` model in `crates/smelt-cli/tests/planner_test.rs` is exactly this shape).
- **Cumulative aggregates** — already its own rule (`Materialization::CumulativeAggregate`; `docs/research/20260522-cumulative-as-its-own-rule.md`).
- **(future)** cross-model fusion, multi-backend routing.

The general pitch is smelt's actual core, delivered under dbt: the user writes the logical *what* (a plain SELECT + a small declarative annotation — "this is a cube over these dims", "this is cumulative", "this is timeseries"); smelt's planner produces the optimized physical *how*, correctness-preserving. Incremental leads because its pain is sharpest, but the product is "the planner under dbt".

### Planner rules as the extensibility wedge (vs macros)

The strategic reframing: dbt's extension mechanism is **Jinja macros** — string templating, untyped, runtime, hard to test. smelt's is **planner rules** — typed CST transformations that are correctness-preserving and statically analyzable. That turns the on-ramp from "get incremental for free" into "**migrate off macro-hell onto planner rules**".

Two honesty caveats so this isn't oversold:

1. **Rules replace the *optimization* class of macros, not all macros.** Planner rules fit macros where users hand-roll a *physical strategy* in Jinja (incremental logic, grouping sets, window-based dedup, partition pruning) — the most painful, error-prone ones, so a good target. Pure **code-generation** macros (`dbt_utils.star`, surrogate keys, date spine) are a different need that maps to smelt **functions / meta-language**, not planner rules.
2. **User-authored rules are still ahead of us.** The *built-in* rules (incremental, cumulative, cube) are real today and are the immediate value. *User-authored* planner rules — the true macro-replacement extensibility point — depend on the planner-rule API, still design-stage (`docs/planner_rule_api_design.md`). So "dbt users write their own rules instead of macros" is the destination; the on-ramp ships with the built-ins first.

## Incremental done well is a multi-query loop — smelt owns it

Running an incremental model well is not one statement. It is a stateful, sequential, multi-query loop:

1. read the current watermark (max event_time) from the target,
2. derive batch boundaries (`BoundedSafe(n)` / `PerPartitionOnly` → possibly hundreds of batches),
3. per batch, sequentially: `DELETE` the partition range, then `INSERT` the recomputed slice — batch N+1 may depend on N being committed,
4. track processed intervals so re-runs don't double-count or leave gaps.

That is `1 + 2N` queries plus state — and it is exactly what `smelt-runtime`'s execute loop already does (`crates/smelt-runtime/src/execute.rs:174-240` → `delete_partitions` + `insert_into_from_query` per batch, sequenced, `RunManifest` tracking intervals).

**Design consequence:** an earlier idea — "smelt returns a plan, dbt's materialization macro executes it" — is rejected. It would mean reimplementing this loop in a Jinja materialization, the precise imperative-orchestration-in-templates smelt exists to abolish. The loop stays where it already works. The smelt adapter's connection layer *is* `smelt-runtime`, targeting DuckDB/Databricks.

## State: the materialized-view contract (resolution to the state-ownership question)

The open worry was: dbt's *incremental* materialization assumes dbt owns incremental state (via `{{ this }}` and `is_incremental()`), while smelt's `RunManifest` also tracks processed intervals — two watermark owners that drift.

The resolution is to **not use dbt's incremental materialization at all**, and instead model the smelt materialization on dbt's **materialized-view** contract. dbt already treats an MV as a relation whose refresh mechanics are opaque and fully delegated to the adapter: dbt issues "ensure it exists" / "refresh" / (on `--full-refresh`) "recreate", and the engine owns all the under-the-hood state. dbt holds essentially no incremental state for an MV — only relation existence and the model definition (for change detection).

Mapped to smelt:

| dbt action | smelt adapter behaviour | State owner |
|---|---|---|
| normal run ("refresh") | run the incremental delete/insert loop from the stored watermark | smelt (`RunManifest`) |
| `--full-refresh` ("recreate") | full recompute of the whole range | smelt |
| model SQL/config changed (`on_configuration_change`) | decide full rebuild vs in-place — hook into smelt's schema-evolution logic (`docs/specs/schema_evolution.md`) | smelt |

So *how much state is there really?* On dbt's side, almost none: relation existence + the compiled definition, which dbt manages for any relation anyway. All incremental state lives with the engine data, owned by smelt — exactly where an MV's refresh state lives. The "two owners drift" problem dissolves because, under the MV contract, dbt was never trying to own it. The adapter implements the MV materialization macros itself, so this works even on engines without native MVs (DuckDB): "materialized view" here means "smelt-managed incrementally-refreshed relation," defined by the adapter.

`--full-refresh` stays coherent (→ full recompute), and `on_configuration_change` gives the right hook for definition changes — a cleaner fit than dbt's incremental materialization provides.

### How much state does dbt actually maintain? (None, by design)

dbt-core is deliberately **stateless about data progress** — there is no dbt-managed watermark store, cursor table, or "partitions processed" ledger:

- **Classic incremental:** watermark recomputed every run via the author's `select max(ts) from {{ this }}`. Not stored — re-derived from the target table.
- **Microbatch (1.9):** batches computed from `begin` / `batch_size` / `lookback` + the **wall clock**, not from the data and not from a ledger. dbt "doesn't track which batches have been processed" — skip a run and you get silent gaps.
- **Snapshots (SCD2):** state lives in the data (`dbt_valid_from`/`dbt_valid_to`, `dbt_scd_id` columns), diffed at runtime.
- **Partitioned `insert_overwrite`:** partitions to overwrite derived at runtime; no record of partitions previously loaded.

The only thing dbt persists is **run artifacts** (`manifest.json`, `run_results.json`, `sources.json`) — project/run *metadata*, not data watermarks. `state:modified` / `--defer` compare a prior manifest to find changed **model definitions** (slim CI selection); `run_results.json` is an audit log; `source freshness` is a live check. None feed incremental boundaries. dbt's philosophy in one line: **"the warehouse is the state."**

**Implication:** because dbt maintains no native data watermark, smelt owning the watermark conflicts with nothing — there is no dbt mechanism to drift against. The MV contract holds cleanly.

### The watermark: derive vs. store

smelt's `RunManifest` *introduces* stored watermark/interval state that dbt never had — which makes smelt the component that can desync from reality (crash mid-run, manual table edits, a restore from backup). Two postures:

- **Derive from data (dbt-aligned, crash-safe):** read `max(partition_col)` from the target each run; treat any manifest as a *cache*, not the source of truth. Keeps "the warehouse is the state" and has no separate-store failure mode.
- **Store intervals (SQLMesh-aligned, precise):** a durable per-model interval ledger enables exact backfills, plan/diff, and cheap environments — at the cost of a state store to operate and keep in sync.

### Future: stateless vs. stateful as a per-project/model choice

These are the two ends of a real spectrum, and neither is universally right:

| | dbt (stateless) | SQLMesh (stateful) |
|---|---|---|
| Source of truth | warehouse data | separate state store (intervals + model fingerprints + env→table map) |
| Watermark | recomputed / wall-clock | tracked intervals per model |
| Cheap dev environments | no (rebuild, or `--defer` hacks) | yes — virtual environments: views reusing prod physical tables for fingerprint-identical models |
| Precise backfill of a range | manual | first-class |
| Failure surface | minimal | the state store can desync / must be backed up |

The design intent is that **smelt eventually lets state mode be chosen per project and per model** — *stateless* (derive-from-warehouse, dbt-style) where simplicity and robustness win, *stateful* (tracked intervals) where precise backfills and plan/diff matter. And, as an opt-in, **SQLMesh-style environments** — virtual data environments backed by a state store that reuses physical tables across environments for fingerprint-identical models — carrying their own state concerns (the store becomes operationally load-bearing, must be backed up, and is the new desync surface).

This is broader than the dbt-adapter work and should get its own spec. The dbt-adapter seam is just *one consumer* of whatever state model lands: a stateless smelt project maps naturally onto the MV "refresh from warehouse" contract above; a stateful one would expose its interval ledger / environments as smelt concerns that dbt is unaware of.

## The LSP half — recover authoring benefits via dbt's artifacts

Independent of execution, the editor experience can be recovered by teaching the LSP to read dbt's own artifacts:

- **`manifest.json`** → the DAG, how each `{{ ref() }}`/`{{ source() }}` resolves, and configs. The LSP resolves refs from this instead of needing a smelt project.
- **`catalog.json`** (from `dbt docs generate`) → column names + types from the actual warehouse. *This is the answer to "smelt can't introspect schemas" (`docs/specs/sources.md`: schema is declared, not introspected): dbt already introspected; smelt reads the artifact.*

The LSP parses the dbt `.sql` with a **Jinja-lite front-end** (recognize `{{ ref }}`, `{{ source }}`, `{{ config }}`, `{{ this }}`, treat them like `smelt.ref`/`smelt.path_ref`), resolves via manifest, pulls source schemas from catalog, and runs smelt's *existing pure type-inference engine* (the pure-function rule in CLAUDE.md makes this reuse mechanical). Result: hover types, column-level diagnostics, goto-def, and "derived-lookback-exceeds-your-config" warnings — on dbt files, in the editor.

Prefer parsing the **source** file (Jinja-lite) over `target/compiled/*.sql`: Jinja has no source map, so diagnostics computed on compiled SQL can't be placed precisely back on source tokens.

**Synthesis:** "less Jinja" is not just an incremental enabler — it is also what makes the Jinja-lite LSP tractable. A plain SELECT with `{{ ref }}` and a config block parses cleanly; a 60%-macro model does not. Both refinements reward the same behavior, giving a single adoption message: *write your big time-series models as plain SELECTs; get types-while-editing and provably-safe incrementalization for free.*

## Architecture

```
dbt: parse project, build DAG, resolve refs, render naive model -> compiled SELECT
        | (dbt still owns: DAG ordering, selection, tests, docs, scheduling)
        v
smelt adapter's connection layer = smelt-runtime  (targets DuckDB / Databricks)
  |- naive model w/ smelt MV-like materialization -> delegate the WHOLE
  |     classify -> batch -> delete/insert loop to smelt-runtime (the multi-query dance)
  |     state (watermark/intervals) owned by smelt's RunManifest, MV-style
  '- everything else (housekeeping DDL, tests, information_schema) -> backend.execute_sql passthrough

Editor (independent):
  smelt LSP  <- manifest.json (DAG, ref resolution, configs)
             <- catalog.json  (warehouse column types)
             -> type-check dbt .sql via Jinja-lite front-end + existing inference engine
```

This seam captures **incremental + dialect** value. It does *not* capture cross-model fusion: dbt hands models down one at a time, so smelt can't fuse across boundaries here. Acceptable for an on-ramp — incremental correctness is the wedge — but worth stating that one headline differentiator is dormant at this seam.

## Config ingestion: dbt's config dict, not smelt frontmatter

smelt's config *representation* is already decoupled from the frontmatter *format*, so the adapter does **not** embed smelt frontmatter (or a `-- smelt:` comment block) in dbt files — it maps dbt's own config structure onto smelt's internal structs:

- The planner consumes only `ModelInfo { name, sql, refs, timeseries_config, incremental_config }` (`crates/smelt-planner/src/graph.rs:8-18`) — it has zero knowledge of YAML frontmatter.
- Frontmatter is just a serde deserialize onto `ModelMetadata` (`crates/smelt-core/src/metadata.rs:577`); **validation is a separate pure function**, `validate_timeseries(metadata, sql_body)` (`metadata.rs:320-398`), that takes the parsed struct, not YAML. Tests already build `ModelMetadata` / `ModelInfo` directly (`metadata.rs:1325`, `planner_test.rs:108`), proving a second front-end is viable.
- `Granularity` (hour/day/week/month/quarter/year) is a **superset** of dbt's `batch_size` (hour/day/month/year) — no coverage gap.

So the path is: **`dbt manifest config dict → ModelMetadata` mapper → reuse `validate_timeseries` → extract `ModelInfo`.** smelt frontmatter and dbt config become two front-ends onto one representation. Reuse dbt-native keys (`event_time`, `batch_size`, `unique_key`); reserve `config(meta={'smelt': {...}})` only for fields dbt lacks (a distinct `partition_column`, `safety_overrides`). This supersedes the earlier `-- smelt:` commented-frontmatter idea. The one thing to keep invariant: `validate_timeseries` stays the single shared validator for both front-ends.

**Declarative-vs-procedural caveat (for the LSP):** literal config values are statically readable from the editor buffer (full diagnostics); Jinja-*computed* config is only resolvable post-render via the manifest (execution-time only). Recommend literal config; manifest is the execution-time fallback.

## Real work required

- **Expose a public raw `execute_sql`** on `smelt-runtime`. The method exists (`crates/smelt-backend/src/lib.rs:32`) but is crate-internal. The adapter must serve *all* of dbt's SQL — information_schema introspection, `create schema`, drops, tests — not just models. Small, mechanical, required.
- **Single-model entry into the planner/runtime.** `incremental::optimize(model_info)` works on one model already; runtime execution is project-oriented and would need a one-model injection path.
- **A custom materialization** (modeled on dbt's MV contract) that delegates refresh/full-refresh to smelt and cedes state ownership.
- **The adapter's Python connection layer** invoking `smelt-runtime` (subprocess returning JSON, or PyO3). smelt has no Python/JSON interface today; a stateless "SELECT + config → plan/execute" surface is the easiest possible thing to expose.
- **Jinja-lite LSP front-end** + manifest/catalog ingestion (the larger net-new chunk; highest authoring value).

## Accepted limits

- smelt-backend warehouses only (DuckDB, Databricks/Spark). By design, per refinement 2.
- High-value naive-model subset only; heavy-macro / snapshot models get neither benefit.
- No cross-model fusion at this seam.
- Type-checking depth on Databricks/Spark-specific functions is partial (inference "tracks PostgreSQL semantics"); structure is typed, vendor functions are punted.

## Open questions

- **Artifact coupling / two sources of truth.** manifest/catalog schema versions move with dbt releases (maintenance), and dbt's renderer vs smelt's re-parser are two parsers of the same model (divergence risk). How much drift is tolerable, and is there a conformance test?
- **On-ramp vs coexistence.** This is a coexistence tool that doubles as a stealth on-ramp: a "naive dbt model + declarative time config" is structurally already a smelt model, so adopting it for performance drifts the highest-value models into smelt's shape with no rewrite. Is the eventual switch an explicit goal, or is durable coexistence the product?
- ~~Where does the event-time/partition config live on the dbt side?~~ **Resolved:** reuse dbt's config structure directly (`{{ config() }}` → manifest), mapped onto `ModelMetadata` — see "Config ingestion" above. Keep config declarative (literal, not Jinja-computed) for full LSP support.
- **State model is unsettled and broader than this work.** smelt's `RunManifest` needs refinement into a chosen posture: *stateless* (derive watermark from the warehouse, dbt-style) vs *stateful* (durable interval ledger, SQLMesh-style), selectable **per project and per model**, plus an **opt-in to SQLMesh-style environments**. This deserves its own spec; the dbt-adapter is one consumer of whatever lands. See the "Future: stateless vs. stateful" section above.

## Smallest experiment

One narrow spike, no warehouse driver needed: take 5–10 real naive time-series dbt models, feed each compiled SELECT + its `event_time` config to `incremental::optimize`, emit the safety class + derived lookback/chunk, and compare against what the team would have hand-declared for dbt microbatch. **If smelt catches even one model where the human-declared lookback was unsafe, the core differentiator is proven** — ~a day of glue, zero new dialect/LSP/driver work.

Staged path after the spike: expose raw `execute_sql` + single-model entry → custom MV-style materialization on DuckDB → Jinja-lite LSP over manifest+catalog.

## References

- smelt: `crates/smelt-planner/src/rules/incremental.rs` (batch safety, chunk derivation, `optimize`), `crates/smelt-runtime/src/execute.rs:174-240` (incremental loop), `crates/smelt-backend/src/lib.rs:32` (`execute_sql`), `crates/smelt-dialect/src/dialect.rs` (`SqlDialect`), `docs/specs/incremental_models.md`, `docs/specs/schema_evolution.md`, `docs/specs/sources.md`, `docs/specs/architecture.md:383-394` (migration gap).
- dbt: [microbatch incremental models](https://docs.getdbt.com/docs/build/incremental-microbatch), [manifest.json](https://docs.getdbt.com/reference/artifacts/manifest-json), [programmatic invocations](https://docs.getdbt.com/reference/programmatic-invocations), [adapter creation](https://docs.getdbt.com/guides/adapter-creation), [adapter system (DeepWiki)](https://deepwiki.com/dbt-labs/dbt-core/8-adapter-system).
