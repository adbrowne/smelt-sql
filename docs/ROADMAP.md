# smelt Development Roadmap

This document summarizes where each area of smelt stands and what's next. For detailed implementation plans, see [`docs/plans/`](plans/). For the canonical behavior of a feature, see [`docs/specs/`](specs/) — specs are the source of truth and plans cite them.

The **What's Next** section below is the prioritized work queue. Component sections that follow provide context on current state and per-area backlog items.

## Process

This project uses a spec-driven workflow. The flow:

1. `/smelt:spec <feature>` — capture or update `docs/specs/<feature>.md` (the canonical answer to "how does this feature work?")
2. `/smelt:plan <feature>` — derive a phased plan from the spec diff; plan cites the spec rather than restating it
3. `/smelt:implement <plan>` — per-phase implementer + reviewer subagents, red-green TDD on real fixtures, atomic commits
4. `/smelt:validate <feature>` — drift report comparing spec, code, and user docs

The mandatory plan structure (execution prompt, per-phase TDD tests, implementer/reviewer loop, code+docs phases by default) is encoded in `/smelt:plan`. See `CLAUDE.md` § Workflow & Slash Commands for the workflow overview.

## What's Next

The items below are the current priority queue, top to bottom. See completed items in [Recently Completed](#recently-completed) below.

The spine of the near-term roadmap is a single through-thread: **harden against silent failures → cover the missing type-system axes → build virtual environments on that precision → generalise to schema migration.** Spark hardening runs as an elevated parallel track; the remaining items are lower priority. The shared-runtime consolidation, the feature-sweep bug ledger, and the silent-failures hardening that previously headed this queue are now complete — see [Recently Completed](#recently-completed).

**Spec re-architecture (2026-08-12):** the incremental-models spec was rewritten from scratch
per [`docs/research/20260811-delta-signatures-and-definition-deltas.md`](research/20260811-delta-signatures-and-definition-deltas.md)
and split into three files: [`incremental_models.md`](specs/incremental_models.md) (delta
signatures as the front door, the equivalence invariant, contract lattice, plan, frontier,
graph layer), [`incremental_shapes.md`](specs/incremental_shapes.md) (the partition/key shape
profiles — the demoted "four corners"), and [`definition_deltas.md`](specs/definition_deltas.md)
(definition changes as deltas; plan-and-approve `smelt migrate`; the previously-unrecorded
unwired-backbuild gap now recorded). Verified claim-preserving via the claim-inventory method
(486 claims graded) plus adversarial IVM-expert and data-engineer reviews; both contract-lattice
oracles were restated (the deferral oracle was vacuous as previously written; frozen horizon is
now stated per output partition). Next steps are the research doc's §6 sequencing — scheduler
consumes delta signatures; wire backbuild behind `smelt migrate`; lattice v2; proofs-as-product.

**Parallel track (2026-07-18):** the **quality-grind programme** ([master plan](plans/20260718-quality-grind.md)) works the small root-caused deferred items (parser ledger categories, VALUES arity, UTF-8 positions, registry gaps, doc gaps) and the well-understood larger ones (generator deferred coverage, smelt-planner↔smelt-logical consolidation, the cold-Salsa benchmark regression) via a second autonomy loop on `worktree-roadmap_todo`; decision-gated items are queued in the master's "Tier 3 — decision queue".

### 1. Type-System Axes — Collation

~~Silent Failures & Code-Health Hardening~~ ✅ (2026-06-10) — see [Recently Completed](#recently-completed).

### 2. Type-System Axes — Collation

smelt's type system tracks base types and NULL propagation structurally, but four axes are coarse or untracked. They are simultaneously (a) real-world correctness gaps and (b) the precision blocker for virtual environments — `output_fingerprint.md` lists decimal/collation/nullability among the untracked axes that force conservative rebuild. Covering them sharpens both, which is why they are sequenced immediately before Virtual Environments.

Each axis is delivered end-to-end before the next begins: spec contract → soundness oracle → inference fix → signature surface → hover/diagnostics. The delivery order is **nullability → decimal → timezone → collation**, driven by dependency depth and existing research.

- ✅ **Nullability** (2026-06-11) — sound-upper-bound contract in `docs/specs/types.md` §11; outer-join / set-operation rules; value-based DuckDB soundness oracle (`cargo test -p smelt-db --test nullability_property_tests`); non-nullable-claim audit; `NOT NULL` qualifier in `smelt.define` signatures; canonical type renderer shared by hover and diagnostics. See [plan](plans/20260610-nullability-soundness.md).
- ✅ **Decimal** (2026-06-12) — Spark-style growth formulas (`+/-*%`) with integer lifting; `DecimalPrecisionOverflow` diagnostic when `p' > 38`; `Decimal / T` rejected as non-portable (`TypeMismatch`); `numeric_lub` UNION coercion formula; `ABS(Decimal(p,s)) → Decimal(p,s)`; parser fixes for `LEFT()`/`RIGHT()` keyword-as-function-name. See [plan](plans/20260611-decimal-arithmetic.md) and spec §15.
- ✅ **Timezone** (2026-06-12) — `Timestamp WITH TIME ZONE` and `Timestamp` (naive) are distinct types; `NOW()`, `CURRENT_TIMESTAMP`, `MAKE_TIMESTAMPTZ()` return tz-aware; `DATE_TRUNC` mirrors the tz-axis of its second argument; mixing tz variants in UNION, arithmetic, or CASE emits `TypeMismatch`; tz-aware `AT TIME ZONE` divergence documented as known; property-test oracle extended to cover `TimestampTz`; hover/signature surface verified (`TIMESTAMP WITH TIME ZONE` renders correctly and `Expr<Timestamp WITH TIME ZONE>` annotation accepted in `smelt.define`). See [plan](plans/20260612-timezone-axis.md) and spec §16.
- 🔄 **Collation** (portable contract 2026-06-13) — research doc ([`20260612-collation-type-system.md`](research/20260612-collation-type-system.md)) and spec §17 landed; the **portable binary-only surface is enforced**: a `COLLATE` clause naming a non-binary collation emits `NonPortableCollation` (binary names — `C`, `POSIX`, `BINARY`, `UTF8_BINARY` — pass through unchanged), with live-DuckDB-oracle coverage of binary string `=`/`<`/`GROUP BY`/`DISTINCT`/`ORDER BY`/`MIN`/`MAX`. Deferred (each gated on a not-yet-built feature): the value-domain collation field on the string `DataType` variants + engine-bound native collation (engine-declaration feature); the `COLLATE "C"` Postgres emission pin (Postgres backend); and the fingerprint fold (fingerprint-runtime wiring). See [plan](plans/20260613-collation-axis.md) and spec §17.

As each axis lands, the fingerprint oracle gains real precision on it instead of falling back to verbatim rebuild.

### 3. Total Output-Schema Resolution

smelt does not always know a model's output schema. `CompiledModel::output_columns` (and the
`derive_projection` owner behind it) returns *empty = unknown* the moment a select list contains a
surviving wildcard, even when the upstream columns are fully declared and already resolved for LSP
completion (`available_columns`). An ETL tool that cannot name its own output is the wrong default:
too much downstream precision depends on the projection being total — output-fingerprint reuse and
virtual-environment sharing (#5), cross-model column lineage and backbuild eclipse, schema
migration planning (#6), and the BigQuery whole-row `MERGE`, which refuses a `ColumnScopedMerge`
model whose projection is not statically enumerable (`multi_backend.md` §Constraints).

The goal is a *total* output schema: every model resolves to a concrete column list, or names the
exact reason it cannot. The unresolvable cases today are each independently closable:

- **`SELECT *` derivation gives up unconditionally.** `derive_projection`
  (`crates/smelt-runtime/src/compile.rs`) returns `None` on any wildcard without consulting the
  upstream schema it already holds. Teach it to expand a wildcard against resolvable upstream refs
  (reusing the `RowExtension` / `RefSchemaProvider` machinery `available_columns` already uses), so
  `SELECT *` over declared upstreams yields a real column list.
- **External physical tables carry no declared schema.** Support declaring an external table's
  columns so a projection over one resolves instead of falling to `ColumnSource::ExternalTable` /
  `Unknown`.
- **CSV seeds do not participate in `SELECT *` resolution.** Require/declare seed schemas so a
  `SELECT *` over a seed is enumerable.
- **`smelt.functions.<f>(args).*` over an unresolved return.** Closed-struct returns already
  expand; row-tail (`Struct<{…, ..r}>`) and unresolved signatures fall back to a synthetic
  `Unknown` column — close the gap so the spread is always enumerable or a named refusal.
- **`SELECT *` over an `ON`-join is not expanded** (duplicate-name hazard, tracked in
  `docs/TODO.md`). Expand it with explicit duplicate-name handling.

**Done when** `output_columns` is total for every model over declared upstreams, each residual
unresolvable case is a named diagnostic rather than a silent empty list, and the BigQuery
`ColumnScopedMerge` constraint narrows to genuinely unresolvable upstreams. Sequenced as a
precision precondition for the fingerprint fold (#5) and backbuild lineage. Extends
[`docs/plans/20260819-source-derived-projection.md`](plans/20260819-source-derived-projection.md),
the projection owner this builds on.

### 4. Safety-Overrides Review — Partition-Grain Admission Checks

The partition-grain recompute-a-region quadrant admits SQL only past a set of per-cell safety
checks (window functions, `HAVING`, `DISTINCT`, `LIMIT`, FROM/JOIN subqueries, non-deterministic
functions), each individually bypassable via `safety_overrides.allow_<check>: true`
(`incremental_shapes.md` §"Safety checks (per-cell admission for recompute-a-region)",
`models.md` §Surface `safety_overrides`). The overrides were added as a uniform escape hatch when
each check landed; they haven't been revisited as a set since, and at least some no longer look
like the right shape for the underlying problem:

- **Subqueries** (FROM/JOIN) are rejected wholesale unless overridden, but a `WITH` CTE already
  flows through bound derivation via the body-structure classifier and isn't gated at all. The
  gap looks like missing coverage rather than a genuine hazard — extending the same
  body-structure classifier to FROM/JOIN subqueries would very likely let this check go away
  as an override entirely rather than stay a bypass users reach for.
- **Window functions** currently gate on a syntactic shape (`PARTITION BY <keys> ⊇
  partition_column`, or a bounded `RANGE BETWEEN INTERVAL … PRECEDING` frame) and the override
  just turns the check off, trusting the user to have gotten the frame right unchecked. A better
  shape is likely an LSP code action: detect an inadmissible window spec and offer to rewrite it
  into the minimal admissible form (add the missing `PARTITION BY` key, bound an unbounded
  frame) — the user gets SQL that *is* provably safe rather than SQL the checker has stopped
  looking at. Worth asking whether other checks (`HAVING`, `DISTINCT`) admit the same treatment.
- **Non-deterministic functions**: the spec already flags `allow_nondeterministic` as dropping
  the guardrail wholesale and calls it out as discouraged — this one is probably closest to
  correctly shaped today (opt-in, recorded, but genuinely a "you are choosing to accept risk"
  situation rather than a coverage gap).
- **`LIMIT`** is correctly never overridable (survival depends on which other rows are present,
  which differs run vs full refresh) and should stay that way.

Next step: an audit pass per check — classify each as (a) a coverage gap the walk/classifier
should just close (subqueries), (b) a candidate for an LSP quick-fix that emits the provably-safe
rewrite instead of a bypass (window functions, possibly `HAVING`/`DISTINCT`), or (c) a genuine
opt-in risk acceptance that should stay a recorded override (non-deterministic functions) — then
a spec diff to `incremental_shapes.md` §"Safety checks" for whichever checks change shape.

### 5. Virtual Environments + Backbuild Change-Detection (specs authored, prototype proven)

SQLMesh-style opt-in virtual data environments: cheap isolated environments that share physical tables with production whenever a model's output is *provably* unchanged, rebuilding only what provably changed. The differentiator over SQLMesh is a **typed, provable equivalence relation** in place of a syntactic edit-script. The same machinery powers **backbuild change-detection** — deciding precisely which models a change forces to rebuild versus spares.

**Proven** (research + Stage 0 prototype): the semantic output-fingerprint oracle ([`crates/smelt-fingerprint`](../crates/smelt-fingerprint)) with its soundness gate (`fingerprint-equal ⇒ DuckDB relations identical`) and determinism detector, all green as property tests against DuckDB. See Recently Completed below and [`docs/research/20260601-virtual-environments.md`](research/20260601-virtual-environments.md).

**Specced**: [`output_fingerprint.md`](specs/output_fingerprint.md) (normative, implemented), [`virtual_environments.md`](specs/virtual_environments.md) (the orchestration layer — `state.mode`, environment addressing, fingerprint-keyed reuse, promotion, override hatches), [`run_state.md`](specs/run_state.md) (`.smelt/` layout + snapshot store).

**Next** (each increment gated by the DuckDB oracle, derived via `/smelt:plan`):
1. Wire `output_fingerprint` into the runtime (it is a standalone prototype today). Depends on the now-complete runtime consolidation (see [Recently Completed](#recently-completed)).
2. Snapshot store + `(environment, model) → table` map (`run_state.md`); fingerprint-keyed reuse for a single environment.
3. `state.mode: environments` addressing, `smelt plan/apply --environment`, `smelt promote`.
4. **Fold tracked type-system axes into the output fingerprint.** Precondition: `docs/specs/types.md` §11 sound-upper-bound contract (satisfied by ROADMAP item 4, nullability axis ✅). The fold must hash the structured `TypedColumn` (type + nullability — and, as later axes land, decimal precision/scale, timezone-awareness, collation) rather than a rendered display string, so display conventions can evolve without invalidating fingerprints. Verification gate: `cargo test -p smelt-db --test nullability_property_tests` must stay green after the fold, and the fingerprint soundness oracle (`fingerprint-equal ⇒ DuckDB relations identical`) must hold for schemas that differ only in nullability. Each subsequent axis (decimal, timezone, collation) extends this fold when it lands.
5. **Backbuild change-detection** — the cross-model column-lineage analyser computing the full "eclipse" (downstream models spared by an output-preserving upstream change); the gating new analysis, and the substrate item #6 builds on.
6. Polish: typed data-diff, GC/retention, forward-only.
7. **Probe-gated G2 admission for backbuild synthesis.** Join-multiplicity changes (INNER→LEFT, LEFT→INNER, edited join conditions) refuse unconditionally in the backbuild-synthesis module today (`crates/smelt-logical/src/backbuild/`, research §4 G2). The future rung: admit the no-op subset data-dependently via a runtime count-preservation probe (`emit_count_preservation_probe` already exists) — e.g. INNER→LEFT is a no-op iff no row actually lacked a match. A data-dependent verdict is a different contract from the pure definition-diff module, so this lands with backbuild wiring, not before. See [`docs/research/20260802-backbuild-synthesis.md`](research/20260802-backbuild-synthesis.md) §4 G2.

Explicit non-goal for now: the un-annotated determinism inversion remains conservative-rebuild until covered (worst-case parity; see `output_fingerprint.md` Known Divergences). The type-system axes that previously forced conservative rebuild are addressed in #2 and unlock fingerprint precision as they land.

### 6. General Schema Migration on the VE Substrate

Generalise schema change management on top of the fingerprint + column-lineage machinery from #5. smelt already has schema evolution (ALTER vs full-refresh, complex/nested types) and offline `smelt diff`; this item makes migration planning lineage-aware, so a plan knows precisely which downstream models are output-affected versus spared (the same eclipse analysis), and can stage and preview migrations across environments before promotion. Sequenced after Virtual Environments because it reuses that substrate.

### 7. Spark — Production Hardening

The Spark backend is functionally complete (PySpark/PyO3 bridge, zero-copy Arrow, Spark Connect / Databricks Connect). Remaining gaps to production-grade:

- **Integration-test parity** — run the DuckDB integration suite against a local Spark Connect server, so Spark is verified at the same depth as DuckDB.
- **JSON incompatibility rewrites** — `TO_JSON(scalar)`, `JSON_CONTAINS`/`@>`/`<@`, `JSON_OBJECT`/`JSON_ARRAY`; emit compile-time warnings where no faithful rewrite exists.
- **Authentication docs** — tokens, OAuth, and instance profiles for Databricks Connect / EMR / Dataproc.

### 8. `smelt check` — LLM-Optimised Diagnostic CLI

Structured diagnostic output designed for LLM consumption. Exposes Smelt's semantic analysis (parse errors, type errors, resolution failures, schema compatibility) via `smelt check --format json` with severity filtering, file/project scope, token budget control (`--budget-lines`), and optional extended context (`--explain`). Replaces the previously planned `smelt validate`. Includes a Claude Code skill and eval harness for empirically tuning diagnostic sufficiency.

See [design doc](plans/20260405-smelt-check.md) for full interface spec, JSON schema, and eval plan.

### 9. Orchestrator Integration

Dagster/Airflow plugin API. `smelt explain --json` already provides the graph structure; next step is a thin adapter layer for orchestrator consumption.

### 10. PostgreSQL Backend

Third backend after DuckDB and Spark. Deprioritized earlier in favor of Spark, now the remaining major backend gap.

### 11. Databricks Support + Metrics-View Compatibility (low priority)

Deeper Databricks integration beyond the existing Spark / Databricks-Connect path, treated as low priority. The long-deferred **Metrics DSL** (`smelt.metric()`) is folded in here: Databricks now ships first-class **metrics views**, so the concrete, testable goal is that smelt metric definitions are compatible with — and can target — Databricks metrics views. That compatibility test is the forcing function that gives the Metrics DSL a real spec to hit; absent that, the Metrics DSL stays low priority and is tracked here rather than as its own item.

---

## Recently Completed

### ~~Registry-owned dialect emission and the cross-engine audit~~ ✅ (August 24, 2026)

BigQuery dialect coverage was incident-driven: probe a live warehouse, hit a failure, add a
lowering. Nothing walked the builtin registry and asked whether each name is native, needs a
lowering, or — the dangerous class — exists on both engines with *different semantics*, so the
query succeeds and returns a different number. Closes #171.

- **The registry owns emission.** `DialectId`, `SyntaxForm`, `Emission` and `RewriteId` on
  `Signature`; `printer.rs` resolves the entry and dispatches on the verdict, and holds no
  name-matched dialect arm. `remap_function_name` and the three `matches!(ctx.dialect, …)` guards
  are gone. The one residual dialect branch — the pipe `SET`/`DROP`/`RENAME` lowering — turned out
  to be capability-shaped, not emission-shaped, and became
  `BackendCapabilities::supports_pipe_set_drop_rename`.
- **`Unsupported` is a compile-time refusal**, not a warehouse round trip. `UnsupportedOnBackend`
  names the construct, the backend, and the registry's own reason. The ephemeral-CTE path is
  checked too, since an ephemeral model is inlined into its consumer and never passes through the
  consumer's own check.
- **The audit is derived, not authored.** 186 probes over 165 registry entries: a parameter's
  `TypeConstraint` picks a fixture column, `SyntaxForm` picks the spelling, `ExprKind` picks the
  query shape. Aggregates are probed in both aggregate and window position, because `MEDIAN`
  proves the lowering differs between them. Two legs — schema (does it run?) and value (does it
  compute the same thing?) — against DuckDB per-PR and Spark nightly.
- **`^` was silently wrong on Spark.** smelt's grammar reads `^` as power; Spark SQL and GoogleSQL
  both read it as bitwise XOR. `10 ^ 2` returned 8 on Spark and 100 on DuckDB. Proven against a
  live Spark by reverting the emission row and watching the value leg catch it.

**The type leg.** The schema leg does not stop at "does the printed SQL run?" — it also
compares smelt's inferred output type against what the engine reports, for every entry the
enumeration reaches. `type_property_tests` generates from `core_functions()`, a hand-maintained
registry-blind table, so most of the registry had never been type-checked against any engine.
The comparison shares `prop_helpers/divergences.rs` rather than building a second registry: a
type difference belongs in the table both suites read.

It found two inference families immediately, both confirmed independently on more than one
engine:

- **Unnesting an `ARRAY<T>` infers `Unknown(Dynamic)`** rather than the element type `T`
  (`UNNEST`, `EXPLODE`).
- **`FIRST` and `LAST` never parse as calls at all.** Both are lexed as keywords for
  `NULLS FIRST` / `NULLS LAST`, so `FIRST(x)` yields an `Unknown` type in aggregate position
  and, in window position, a select item with no alias at all — while the registry classifies
  both as aggregates. Closing it is a contextual-keyword change in `smelt-parser`.

Sixteen gaps closed in the same pass, as verified `Emission::Rename` rows — each measured
against the live engine, never read from documentation: `ARG_MAX`/`ARG_MIN` → `MAX_BY`/`MIN_BY`,
`STRPOS` → `INSTR`, `JSON_EXTRACT`/`JSON_EXTRACT_TEXT` → `GET_JSON_OBJECT` on Spark;
`RANDOM` → `RAND`, `NOW` → `CURRENT_TIMESTAMP`, `TRUNCATE` → `TRUNC`,
`GROUP_CONCAT`/`LISTAGG` → `STRING_AGG`, `JSON_EXTRACT_TEXT` → `JSON_VALUE`,
`MAKE_DATE`/`MAKE_TIME`/`MAKE_TIMESTAMP` → `DATE`/`TIME`/`DATETIME` on BigQuery;
`JSON_OBJECT_KEYS` → `JSON_KEYS` and `TRUNCATE` → `TRUNC` on DuckDB.

A rename now suppresses itself when the author already wrote the target spelling, so a user
writing DuckDB's own `json_extract_string` keeps their text — the byte-identity promise held.

**Residual gaps**, all recorded in `crates/smelt-db/tests/dialect_audit/ledger.rs` and ratcheted
per-dialect by `.claude/dialect-gaps-baseline.txt`:

- **DuckDB: 12.** Seven names smelt's registry recognises that DuckDB has no function for
  (`INITCAP`, `TO_CHAR`, `QUOTE_IDENT`, `QUOTE_LITERAL`, `DATE_SUB`, …), the five
  type-inference rows above, and the ordered-set percentiles in window position.
- **Spark: 27.** Mostly loud refusals, but two are the silent class: `LOG` is the natural
  logarithm on Spark and base 10 on DuckDB, and `DAYOFWEEK` numbers the week from a different
  day. Four more are permanent semantic differences no rename can close (`CONCAT`'s NULL
  propagation, `ARRAY_AGG`'s NULL elements, `CORR`/`REGR_SLOPE`'s NaN-versus-NULL convention).
- **BigQuery: 42**, from a live sweep on August 24. `LOG` diverges the same way it does on
  Spark. Two findings are sharper than a missing name: **`%` lowers to `MOD`, and GoogleSQL's
  `MOD` accepts only `INT64`/`NUMERIC`** — so the lowering is correct for integer operands and a
  hard failure for floating-point ones, the same operand-type dependence that made `//`
  unlowerable; and **`DATE_TRUNC`'s argument order is reversed** relative to DuckDB's. Eleven are
  accepted permanent divergences (`GREATEST`/`LEAST` NULL propagation, `MD5`'s BYTES-versus-hex
  return, `TO_JSON`'s JSON `null`, `POWER`'s domain on a negative base, `ARRAY_AGG`'s refusal of
  NULL elements).
- **PostgreSQL: unverified.** A `SqlDialect` variant with no backend crate and no oracle, so
  nothing exercises its verdicts. Marked as such in the published table.

Every residual row points at a live tracking issue rather than at #171, which this work closes:
[#173](https://github.com/adbrowne/smelt-sql/issues/173) (`%` on BigQuery),
[#174](https://github.com/adbrowne/smelt-sql/issues/174) (`LOG` / `DAYOFWEEK` silent divergence),
[#175](https://github.com/adbrowne/smelt-sql/issues/175) (`FIRST` / `LAST` lexed as keywords),
[#176](https://github.com/adbrowne/smelt-sql/issues/176) (inference returning `Unknown`),
[#177](https://github.com/adbrowne/smelt-sql/issues/177) / [#178](https://github.com/adbrowne/smelt-sql/issues/178) / [#179](https://github.com/adbrowne/smelt-sql/issues/179)
(the per-dialect verdict backlogs),
[#180](https://github.com/adbrowne/smelt-sql/issues/180) (documenting the accepted divergences for users),
[#181](https://github.com/adbrowne/smelt-sql/issues/181) (PostgreSQL is unverified).

The coverage table issue #171 asked for is generated and drift-gated at
[`docs/reference/dialect-coverage.md`](reference/dialect-coverage.md).

### ~~Schema-evolution DDL for Spark~~ ✅ (August 21, 2026)

The same dispatch bug the BigQuery work uncovered was live on Spark too: only the *complex* change
kinds ever reached `ddl_spark`, so `ADD COLUMN`, `DROP COLUMN`, `ALTER COLUMN … TYPE` and
`SET NOT NULL` went to a Spark server in DuckDB's dialect. Spark now routes the whole diff through
its own generator, and the generator's rules were re-derived from measurement.

- **Measured, not read.** `scripts/spark-probe-ddl.sh` runs each form against a fresh Delta and a
  fresh Parquet table on a live server. Five answers contradicted what the generator claimed:
  `NOT NULL` on `ADD COLUMNS` is refused by *both* formats (the generator emitted it for Delta), a
  `DEFAULT` clause needs Delta's `allowColumnDefaults` feature, `DROP COLUMN` needs
  `delta.columnMapping.mode`, every `ALTER COLUMN … TYPE` widening needs `delta.enableTypeWidening`
  — the documented-safe integer chain included — and `SET NOT NULL` is refused even on a column
  holding no NULLs.
- **The rules are about the table smelt creates**, not the format in the abstract. smelt writes
  `CREATE TABLE … USING DELTA` with no table properties, and enabling any of the three features
  above irreversibly raises the table's protocol version — not something a migration should do to a
  user's table unasked. Those changes resolve to a table rewrite on Delta and a named full refresh
  on Parquet.
- **A `default:` still fills the rows already there.** Delta will not take the clause, so the
  generator emits the plain add followed by `UPDATE … WHERE col IS NULL` — the shape the GoogleSQL
  generator uses, for the same reason.
- **Verified against the generator, not the server.** Three new legs in
  `crates/smelt-backend-spark/tests/ddl_observed.rs` execute the statements
  `plan_migration_for_backend` actually emits; all green against a live Delta-enabled server, and
  restoring the old fall-through makes them fail with `ParseException` on
  `ADD COLUMN note VARCHAR`.

### ~~BigQuery worklist closed~~ ✅ (August 22, 2026)

The remaining items of the [BigQuery worklist](plans/20260821-bigquery-remaining.md) are
closed, three as decisions and three as work.

- **Decisions recorded as Constraints, not left as open divergences** — cross-engine exchange
  is a two-engine, filesystem-local capability by design (a third engine that cannot read a
  host path needs a new object-store boundary, which is cross-cutting, not a BigQuery
  feature); BigQuery has no CI tier, by decision, with the credential and billing reasoning
  recorded in place; and a BigQuery `ColumnScopedMerge` model must have a statically
  enumerable projection, the broader fix being #3 "Total Output-Schema Resolution".
- **`refresh: materialized_view` now emits on BigQuery** — `supports_native_ivm` flips to
  `true` and smelt emits `CREATE OR REPLACE MATERIALIZED VIEW`, running no combiner and no
  ledger. The design was measured (`scripts/bigquery-probe-mv.sh`), and the finding that
  shaped it was that materialization flips are hazardous in *both* directions:
  `DROP TABLE/VIEW IF EXISTS` both fail against a materialized view, so a model flipping away
  from the mode would have errored — a hazard the feature itself introduced.
- **The non-vacuity assertion is on the object's type, not its rows** — a substituted plain
  table serves identical rows, so `materialized_view_parity` reads `INFORMATION_SCHEMA`;
  swapping the emitter to `CREATE OR REPLACE TABLE` fails it while the row assertion would
  still have passed.
- **The conformance sweep is no longer pinned to one thread** — per-case dataset isolation
  was already in place; what blocked concurrency was a preflight that budgeted the credential
  window *per test*, so concurrent tests each passed their own budget while the sweep
  overran. Budgeting moved to the sweep, checked once per process against a decided 2700s
  estimate. Measured live: **22 passed / 0 failed in 621.61s** at 4-way concurrency, against
  2190.85s sequentially — a 3.5x reduction, finishing with 2828s of the credential window
  unspent, and with no quota refusals or dataset collisions.

### ~~Schema-evolution DDL for BigQuery~~ ✅ (August 21, 2026)

BigQuery gained its own GoogleSQL DDL generator (`crates/smelt-state/src/ddl_bigquery.rs`), so a
schema change on a BigQuery model migrates in place instead of resolving to a full refresh. Item 1
of the [BigQuery worklist](plans/20260821-bigquery-remaining.md).

- **Measured, not read.** `scripts/bigquery-probe-ddl.sh` runs 55 DDL forms against the live
  warehouse, one fresh table each (repeating on one table trips a per-table update quota, and a
  quota refusal says nothing about the form). Three answers contradicted the obvious guess: a
  `DEFAULT` cannot ride on an `ADD COLUMN`, `BIGNUMERIC → FLOAT64` is refused despite being a
  documented widening, and `SET DATA TYPE` on a `REQUIRED` column is refused outright.
- **The dispatch was wronger than the divergence entry claimed.** Only the *complex* change kinds
  ever reached a backend generator; `ADD COLUMN`, `DROP COLUMN`, `ALTER COLUMN … TYPE` and
  `SET NOT NULL` were emitted inline in DuckDB's dialect for every backend, so the BigQuery arm
  that was supposed to refuse sat in a branch those changes never took. A flat schema change on
  BigQuery emitted DuckDB SQL the warehouse rejects.
- **Verified against the generator, not the warehouse.** The pre-existing parity leg evolves the
  schema with hand-written DDL, which measures BigQuery rather than smelt. Two new legs execute
  the statements `plan_migration_for_backend` actually emits; both are green live, and reverting
  `SET DATA TYPE` to DuckDB's spelling makes the BigQuery leg fail.
- **What GoogleSQL cannot express stays a named refusal** — struct/array changes, adding a
  `NOT NULL` column, tightening to `NOT NULL`, widening a `REQUIRED` column — each resolving to a
  full refresh whose reason names the column and the limitation.

### ~~BigQuery generative maintenance-conformance leg~~ ✅ (August 21, 2026)

10-phase plan ([plan](plans/20260817-bigquery-generative-conformance.md)) giving BigQuery its own
leg of the generative dual-execution harness, so its incremental coverage is generative rather
than fixed-recipe-only — and making `multi_backend.md`'s claim that "the backend under test is a
parameter, not a duplicated implementation" true, by extracting the shared test families into one
target-parametrized owner instead of adding a third copy.

- **Measured green in one sweep** — `bash scripts/bigquery-conformance.sh`, `--test-threads=1`:
  21 passed / 0 failed / 0 ignored, 2190.85s against the live warehouse. Earlier sessions could
  only ever verify cases in targeted runs, because a one-hour credential could not cover a sweep
  plus an already-spent window.
- **Four product-side dialect gaps closed, not just harness ones.** A keyed-fold `MERGE` emitted
  `INSERT *` where GoogleSQL needs `INSERT ROW`; infix `%` and the power operators reached
  GoogleSQL unlowered — and `^` was the dangerous one, since smelt reads it as power while
  GoogleSQL defines it as bitwise XOR, so an unlowered `^` returned a *different number* rather
  than failing. Each affects real user models on BigQuery, not only the harness.
- **An exact median left the warehouse rounded** — the output-schema cast wrap re-parsed
  already-lowered SQL, could not read the GoogleSQL `FLOAT64` spelling, and narrowed a
  `-284.5` median to `-285`. Division with one unresolved operand now yields no type at all.
  This is the same re-parse-your-own-output bug class the source-derived projection work closed.
- **The oracle is demonstrably non-vacuous** — `harness_self_check_bigquery` catches a
  deliberately seeded divergence live, so the leg's green is evidence rather than absence of
  checking.
- **A measurement correction worth keeping** — an all-green sweep costs nearly twice a failing one
  (2190.85s vs 1142.10s), so the credential window's "large headroom" was an artefact of measuring
  a red suite. A sweep now must start on a freshly minted token.

Both items deferred during the plan have since closed. The Spark `dags` family really was
comparing a project against itself — the full-refresh twin shared the incremental project's
schema, so the equality assertion could read one already-overwritten table for both sides, and
every earlier green `dags` run on the Spark leg carried no evidence about the incremental engine
at all. The twin now stages into its own schema, threaded from the target through every Spark
staging path, and a seeded-divergence self-check proves the comparison refuses a divergence it
previously reported as equal (August 21, 2026).

And `hardening-budget.sh` no longer mis-classifies the test-support testkit crate as production:
test-support is now *derived* — a crate that some crate dev-depends on, that no crate depends on
normally, and that produces no binary — rather than listed somewhere that could go stale. The
gate also refuses an orphaned baseline entry, so a crate silently dropping out of the budget is
an error rather than an unnoticed hole (August 21, 2026).


### ~~Source-derived projection — one owner for a model's output schema~~ ✅ (August 20, 2026)

6-phase plan ([plan](plans/20260819-source-derived-projection.md)) closing a bug class in which
several consumers fed `smelt_dialect::print`'s *output* back into `smelt_parser::parse`, asking
smelt's parser to read dialect-lowered SQL it was never designed to read. Every site failed soft,
so the damage was silent. A model's projection — its output column names and their inferred
types — is now derived once, from the model's own source CST, before printing.

- **The median bug fixed** — a BigQuery `MEDIAN` reached the cast wrap as a `FLOAT64`-spelled
  expression smelt's type parser could not read, and an exact median left the warehouse rounded
  by a narrowing `CAST(... AS SMALLINT)`. `multi_backend.md` §Known Divergences retires the
  unfixed-hazard paragraph.
- **Projection aliases are bound, not invented** — an unaliased expression column previously had
  a `_colN` name invented at *reference* time that the inner query never exposed. Alias synthesis
  now splices a real ` AS _smelt_col{n}` into the source before printing, and `_smelt_` is a
  reserved prefix enforced by a new `ReservedProjectionAliasPrefix` diagnostic emitted from the
  analyzer, so the editor and the build agree. This unblocked in-model list spread.
- **A dead safety probe revived** — the count-preservation probe was fed the cast-wrapped body,
  so the enrichment join it looks for was buried in a derived table and had never fired in
  production on any dialect. It now receives the pre-wrap body via `CompiledModel::body_sql`, and
  its derived-table widening is gated on the shared `TYPE_CAST_WRAP_ALIAS` marker and bounded to
  one level, so a user's own subquery is never mistaken for a cast wrap.
- **Standing gate** — `cargo test -p smelt-runtime --test projection_dialect_invariance` compiles
  one model exercising every construct the printer lowers (`MEDIAN`, `%`, `**`, `QUALIFY`, date
  literals, `::` casts, array literals) for DuckDB, Spark and BigQuery and asserts `output_columns`
  and the cast-wrap column names are byte-identical across all three. Verified load-bearing:
  restoring the printed-SQL derivation makes it fail, because Spark's `QUALIFY` lowering emits
  `SELECT * FROM (...)` which reads back as an empty projection. Needs no live warehouse.


### ~~Keyed Frontier — column-family union + snapshot-reconcile executor~~ ✅ (August 9, 2026)

5-phase plan ([plan](plans/20260809-keyed-frontier.md)) widening the keyed classifier past the direct-monoid families and building the second keyed run shape. Every family arrived with its admission-matrix conformance recipes including the refusal directions.

- **Order-monotone overwrite** (`MAX_BY`/`MIN_BY`) classified and rendered with incumbent-wins ties, admitted window-forward, refused under snapshot-reconcile. Requires an explicit companion `MAX(<ord>)`/`MIN(<ord>)` projection, with `MAX_BY(x, x)` admitted as the degenerate self-companion.
- **`KeyedReprocessedWindow`** — the ledger's reprocessing refusal is now a named diagnostic carrying the window bounds and the full-refresh remedy, not an unnamed `bail!`.
- **Snapshot-reconcile run shape** — a keyed model over zero clocked sources derives the shape instead of refusing: plain-overwrite columns admit (incoming row wins), fold families refuse per the matrix, retained-departed-keys is the default, and event-time flags are rejected loudly.
- **Once-write** (`COALESCE`) with an FD-backed provenance proof, the first production consumer of the functional-dependency declaration. The admitted surface is deliberately narrow: `COALESCE(MAX(col))` under a declared FD naming the *source* column, and `COALESCE(<unique_key col>, …)` key-derived. Fallback-bearing (`COALESCE(MAX(col), -1)`) and multi-candidate (`COALESCE(MAX(a), MAX(b))`) spellings refuse — each diverges from full refresh, and admitting them needs decomposed state. Fan-out joins and set-op barriers are structural disproofs no declaration can widen past.

`BIT_XOR` was found to be graded idempotent, so a redelivered window cancelled its own contribution instead of refusing — fixed to ledger-keeping. The generative conformance pool could not have caught it (`KeyedCombiner` renders additive as `SUM` only), so it is guarded by a pinned hazard case; widening the pool is recorded as deferred work.

Residues are itemized in `docs/specs/incremental_models.md` §Known Divergences — notably that a window-forward keyed run with missing or half-supplied event-time flags full-refreshes where the spec mandates refusal, `KeyedRetractableContribution` is unimplemented, and the ledger fold is transactional on DuckDB only. Ladder rungs 2–4 and the `smelt.latest`/`once`/`current` pattern functions need a spec pass first.

### ~~`smelt bakeoff` CLI~~ ✅ (July 20, 2026)

The maintenance-plan programme's cost-model override ladder (`crates/smelt-logical/src/maintenance/choice.rs` — `ChosenTechnique`, `resolve_cell_choice`, the override ladder, `ChoiceRefusal`) now drives both live runs and offline measurement: [`docs/plans/20260719-prod-w7-bakeoff.md`](plans/20260719-prod-w7-bakeoff.md). `smelt-runtime`'s maintenance driver resolves `cells[].technique`/`prefer` through the same ladder `smelt explain` reports (frontmatter pins are honoured at execution, not just parsed); `ExecuteRequest.technique_overrides` gives an in-process forcing seam that never bypasses admission. `smelt bakeoff <model> [--cells <col>@<source>,...] [--runs N] [--target <name>] [--keep] [--pin]` measures every admissible technique for a cell over `--runs` replayed windows of real data, each technique landing in its own scratch schema (`smelt_bakeoff_<model>_<technique>`, dropped unless `--keep`) and cross-checked against its siblings with `EXCEPT ALL`; `--pin` emits the winning `cells[]` entry as ready-to-paste YAML, never writing the model's `.sql` file. See [`docs-site/docs/reference/cli.md`](../docs-site/docs/reference/cli.md#smelt-bakeoff) and [`docs/specs/incremental_models.md`](specs/incremental_models.md#cli).

### ~~Composed Axes (Key + Time) and Conditional Maintenance~~ ✅ (July 19, 2026)

34-phase plan ([plan](plans/20260715-composed-axes-conditional-maintenance.md)) landing the composed shape and the conditional-maintenance mechanisms it makes affordable — the flagship demo is `examples/web_analytics`'s `silver.events_deduped`, a redelivery-prone event feed deduplicated by a keyed extremal fold under a declared `key_recurrence` window, with `Refusals: (none)` where the equivalent `QUALIFY`-window shape needs a `safety_overrides` comment.

- **The composed shape** (Group A — locality; Group W — the tracer). A `grain: key` model may now also declare `timeseries:`, time-partitioning its keyed output, once **key temporal locality** is established via one of three routes: key-embedded (`partition_column` is itself a `unique_key` column), key-determined (`partition_column` is a declared per-key functional dependency, checked once-write), or recurrence-bounded (a declared `key_recurrence` window on the driving source, checked transactionally at merge time — `KeyedRecurrenceBoundViolated` on violation). A model satisfying none of the three refuses fail-loud (`KeyedForbidsTimeseries`, naming all three routes). The composed output derives a **settle bound** and is visible to the rest of the DAG exactly like a declared source — downstream pushdown, `--source`/`--since-upstream` origin, `--include-upstreams` traversal all reach through it.
- **The Relation Contract + open write-pattern registry** (Group S — surface; Group R — per-cell write addressing). `grain:` is now a derived, check-only label over the declared shape facts (`timeseries:`/`unique_key:`); physical write addressing is a per-cell choice, not a model-wide verdict, resolved against an open registry of write patterns (`region`, `keyed`, `column`, `update`, `full_rebuild`, and backend-contributed patterns) that a `maintenance.cells[].write` pin can target — refusing fail-loud (`MaintenanceWritePatternUnavailable`/`MaintenanceWriteAddressingRefused`) rather than silently downgrading.
- **The graph layer** (Group B). Forward propagation (`--since-upstream`) and backward resolution (`--include-upstreams`) both walk through a locality-admitted composed node as an ordinary DAG member, projecting a key-level change to its exact touched partitions under routes 1–2, widened by the recurrence bound under route 3.
- **Change-suppressed writes** (Group C — M1). The column-scoped and keyed-fold `MERGE` families gain an `IS DISTINCT FROM`-guarded matched arm (a `MERGE`-less staged-candidate DELETE+INSERT for backends without `MERGE`) that writes zero rows for an unchanged-input re-run, fail-closed over a proven row identity and per-column change-comparability. A structural preference (steady-state prefers suppression; first-build/backfill prefers unconditional) is now steerable via `prefer:`/`technique: suppress|unconditional`, and `smelt explain` prints the resolved variant and why.
- **Observed output deltas** (Group D — T5). A change-suppressed column-scoped `MERGE` records its changed-row set in the same backend transaction as the write; a composed model's recorded delta projects to exact partition dirt (routes 1–2) or a recurrence-widened window (route 3). `smelt explain` surfaces both halves (`observed-delta recording:`, `observed-delta projection:`) as static plan facts.
- **Delta-restricted compute** (Group E — M2). Where a model edge's enrichment join provably preserves the driving row's skeleton (the skeleton-source-closure proof, licensed by a `LEFT JOIN` or a declared `referential_integrity:`) and an exact input delta exists, the region recompute narrows to a semi-joined `DELETE`+`INSERT` over just the affected rows instead of the whole widened scan.
- **The fingerprint sidecar** (Group F — M3). A warehouse-resident row-content-digest sidecar synthesizes a change feed for an external `mutable_snapshot` source with no native CDC, built for DuckDB with live invalidation (a projection change, model-definition edit, or corrupted stamp degrades to the same whole-table delta an absent sidecar produces, logged loudly). Composed with the skeleton-closure proof, it turns a renamed dimension row into a point-lookup recompute instead of a full-table scan.
- **Choice + docs sweep** (Group G). The write-suppression preference ladder and per-cell technique choice now compose; the drift report against `docs/specs/incremental_models.md` is clean, and `docs-site/` documents every surface above (guide + reference) with the "the two axes are orthogonal" framing consistently applied — no surviving "partitioned or keyed" exclusive-mode phrasing.

Several mechanisms are proven and wired at the derivation/reporting level but not yet reached by every live execution path — see `docs/specs/incremental_models.md` §Known Divergences for the itemized list (e.g. the keyed-fold path's own suppression consumer doesn't yet honor the `prefer`/`technique` steering ladder; delta-restricted compute over an external `mutable_snapshot` source is proven against a real fixture but not yet dispatched by a live run's own trigger/technique selection). Deferred items and open questions surfaced during implementation are recorded in the plan's own "Deferred during implementation" section.

### ~~Incremental-Models Spec Consolidation~~ ✅ (July 15, 2026)

`docs/specs/maintenance_plan.md`, `batched_models.md`, `keyed_models.md`, and `versioned_models.md` are retired, redrafted as the single [`docs/specs/incremental_models.md`](specs/incremental_models.md): the maintenance contract, the derived plan, the graph layer, and the three declared shapes (`grain: partition`, `grain: key`, `versioning: interval`) in one document — the file-per-shape cut read as mutually exclusive modes when the shapes are one feature with one declared axis. All normative content preserved (verified by per-spec nothing-lost reviews); inbound references across specs, `CLAUDE.md`, and crate doc-comments retargeted; several cross-spec inconsistencies resolved in the merge (dangling `§"No write-eligibility clamp"` anchor, the nonexistent `CumulativeForbidsBatched` code, `horizon_ceiling:` missing a frontmatter home, run-flag wording vs `--since-upstream`, stale "all seven proofs unbuilt" claim in `model_properties.md`).

### ~~Derived Output Window for Partition-Grain Runs~~ ✅ (July 11, 2026)

Six-phase plan ([plan](plans/20260711-derived-output-window.md)) closing the silent under-write for models whose `partition_column` is derived and skews from the driving date column (Form B relation). The output window is now **derived** from the run window — identity in the common case, skew-inverted `[start − after, end + before)` otherwise — and DELETE range, output clamp, and per-batch scan all key off it; the transparent fast path is restricted to zero-skew models. Skew derivation is a pure walk-composed leaf classifier in `smelt-logical` (`model_partition_skew`). Also fixed `smelt explain --show-sql` clamp injection for function-at-FROM models (wrap-then-compile, matching the live run), strengthened the `per_partition_equivalence` harness (full session 5-tuple + injected cross-midnight and two-boundary chains through the real `sessionize`), and added freshness-gated tutorial sections covering the cross-midnight prior-day rewrite and the two-boundary truncation semantics (the declared relation is a semantic cap). The 60-day `verify_incremental_equivalence.py` replay passes with zero local-column divergence. Deferred items tracked in the plan: name-only skew-anchor matching (over-wide, correctness-safe), `apply_type_casts` inert under the output clamp, and an optional bot-motivated session-continuation rule.

### ~~Spec-Remediation W8/misc-spec — Provenance-Tag Preservation + Record Overlay Shallow Replace (D-54, D-55)~~ ✅ (June 22, 2026)

Two-phase remediation plan ([plan](plans/20260620-w8-misc-spec.md)) closing out D-54 (nested expansion leaves prior `Tagged` nodes intact — verified by a new regression test in `smelt-planner`) and D-55 (record overlay is shallow replace, not deep recursive merge — `merge_values` in `loader.rs` simplified; nested-record discriminator test locks the new behaviour).

- **Verify + lock (P1)** — confirmed `clone_with_tag` already preserves existing tags on re-entry; added a two-level nested-expansion test in `phase41_body_splice_tests.rs` to prevent silent regression.
- **Fix (P2)** — simplified the `SmeltType::Record` branch of `merge_values`: overlay field now replaces the base field wholesale (including nested records) rather than recursively blending; renamed the existing flat-field test to `…shallow_replaces…`; added the canonical D-55 discriminator test (partial nested-record overlay → base nested-record sub-keys absent from overlay must not survive).

### ~~Spec-Remediation W8/datagen — Scale Factor, FK Bound, Zero-Row Guard (D-53)~~ ✅ (June 22, 2026)

Two-phase remediation plan ([plan](plans/20260620-w8-datagen.md)) landing the D-53 decision from the 2026-06-13 spec review: `floor()` for effective row counts, FK bounds equal the effective (scaled) count at any scale factor, and zero-row is a hard configuration error.

- **Core fixes (P1)** — `run_config()` now uses `floor()` not `round()` when scaling row counts; `fk_counts` is built from floor-scaled values; a zero effective row count (e.g. `num_rows: 1`, `scale_factor: 0.4`) is a hard error naming the offending dataset; a FK column whose referenced dataset has an effective row count of 0 is also an error; `unwrap_or(1)` in the FK generator arm hardened to a loud error. Five new TDD tests cover each contract.
- **Help text close-out (P2)** — `--list-generators` `foreign_key` description updated from "Random id in [1, num_rows]" to "Random integer in [1, effective_row_count] where effective_row_count = floor(referenced_num_rows × scale_factor)"; `linked_pools` section gains the scale-invariance qualification note (pool contents are scale-invariant only when no shape field uses `foreign_key`). No KD retractions (the existing KDs are unrelated to scale-factor behavior).

### ~~Spec-Remediation W8/schema_evolution — NOT NULL Column Add: `backfill:`-only is Safe (D-58)~~ ✅ (June 22, 2026)

Two-phase remediation plan ([plan](plans/20260620-w8-schema-evolution.md)) landing the D-58 decision from the 2026-06-13 spec review: either `default:` or `backfill:` (or both) classifies a NOT NULL column addition or NULL→NOT NULL tighten as Safe.

- **Backfill-only reclassification (P1)** — Both classifier call sites (in `plan_migration_for_backend` and `plan_schema_operations`) now admit backfill-only as Safe. DDL codegen restructured: ADD COLUMN with backfill-only emits `NOT NULL` (no DEFAULT clause) then the UPDATE backfill; ChangeNullability tighten with backfill-only emits a gap-scoped `UPDATE … WHERE col IS NULL` (no clobber) then `SET NOT NULL`. When both are present, backfill takes precedence for the gap fill. Fail-quiet bug fixed: SET NOT NULL is always emitted on the safe path. Six new TDD tests cover each case.
- **Close-out (P2)** — No KD retraction needed (D-58 had no open KD entry); master registry and ROADMAP updated.

### ~~Spec-Remediation W8/planner — `joins:` Cardinality String→Enum Mapping (D-57)~~ ✅ (June 22, 2026)

Two-phase remediation plan ([plan](plans/20260620-w8-planner.md)) landing the D-57 decision from the 2026-06-13 spec review: the `cardinality:` frontmatter string→`Cardinality`-enum mapping is exact and fail-safe.

- **`cardinality_from_str` (P1)** — `"1:1"` → `OneToOne` (the only value that enables `EliminateUnusedLeftJoin`); every other string → `OneToMany` (silent, conservative). Three TDD test layers: unit mapping tests in `smelt-logical`, a named fail-safe test in `join_elimination_tests.rs` confirming the gate already pattern-matches on `Cardinality`, and an end-to-end constructed-plan test in `show_plan.rs` bridging frontmatter string → rule gate.
- **Close-out (P2)** — KD retraction in `planner_integration.md` (the "mapping is normative, wiring note" item removed; the mapping is now implemented); ROADMAP and master registry updated.

### ~~Spec-Remediation W8/timeseries — Partition & Pruning Column Invariants (D-52)~~ ✅ (June 22, 2026)

Two-rule static diagnostic pass ([plan](plans/20260620-w8-timeseries.md)) landing the D-52 decisions from the 2026-06-13 spec review. The R2-incremental-cadence execution changes are explicitly out of scope (deferred to the R2 rewrite).

- **NOT-NULL invariant (D-52 rule 7, P1)** — `partition_column` (and a distinct `event_time_column` when it drives pruning) must be NOT NULL on the model's output schema. A nullable pruning column silently escapes the `>= start AND < end` window and is never re-inserted — a correctness hole. Fires `MalformedTimeseries`. Checks only `Computed` columns to avoid false positives from CTE pass-throughs and cross-model inheritance. Also tightened nullability defaults for CAST (None → false) and `date_trunc` (propagates from input) to eliminate false positives on clean timeseries examples.
- **Sub-day granularity type constraint (D-52 rule 8, P2)** — `granularity: hour` requires a timestamp-resolution `partition_column`; pairing it with a `DATE` column silently coarsens pruning to whole-day boundaries. Fires `MalformedTimeseries`.
- KD retraction: rules 7 and 8 removed from the "Output-schema-dependent validation rules" Known-Divergence note in `timeseries.md`; rules 2, 3, 4 remain pending the R2 rewrite.

### ~~Spec-Remediation W8/catalog — Catalog JSON Shape Fixes (D-50)~~ ✅ (June 21, 2026)

Three-phase remediation plan ([plan](plans/20260620-w8-catalog.md)) landing the D-50 decisions from the 2026-06-13 spec review:

- **`source` always present (D-50-i, P1)** — `CatalogColumn.source` is always serialized; `CatalogColumnSource::Unknown` serializes as `{"type":"unknown"}`, never omitted; snapshot test locks the invariant.
- **Workspace-relative `path` (D-50-iii, P2)** — `CatalogModel.path` and `origin.generator_file` are now workspace-relative (relative to the `smelt.yml` directory), never absolute filesystem paths; catalog diffs identically across machines.
- **`--select` preserves full lineage (D-50-ii, P3)** — full `DependencyGraph` kept for edge resolution; `build_catalog` filters `models`/`tag_index`/`execution_order`/`model_count` to the selected set while `upstream`/`downstream` retain all edge names including excluded deps; `render_model_page` renders excluded deps as plain text, not broken links.

### ~~Spec-Remediation W8/virtual_env — Virtual Environment Data Model & Reuse Evaluator (D-46, D-47)~~ ✅ (June 21, 2026)

Five-phase remediation plan ([plan](plans/20260620-w8-virtual-env.md)) landing the D-46 and D-47 decisions from the 2026-06-13 spec review (data model and pure logic only; runtime wiring deferred):

- **`StateMode` + `state.mode` config (D-47, P1)** — `StateMode` (`Stateless`/`Intervals`/`Environments`) added to `smelt-core`; parsed from the `state: {mode: …}` block in `smelt.yml`; `PartialOrd` encodes the posture lattice so per-model narrowing can be validated; unknown values fail loudly.
- **`reuse.*`/`forward_only` frontmatter hatches (D-46, P2)** — `ModelMetadata` carries `reuse.accept_current`, `reuse.assert_deterministic`, and `forward_only`; a model may narrow the project posture via frontmatter `state: {mode: …}`; widening fires a new `MetadataError` variant wired into the diagnostic pipeline.
- **`SnapshotStore` / `SnapshotEntry` types (D-47, P3)** — `smelt-state` gains the `(environment, model) → physical table` snapshot map with per-entry `source_sql`; `find_candidate` implements the candidate-precedence rule (target env E first, then base/production, then lexicographic).
- **Reuse-condition evaluator (D-46/D-47, P4)** — `evaluate_reuse` in `smelt-fingerprint` checks all four conditions in order, returning a typed `ReuseDecision`; conditions 3a (rebuild-identity preserved) and 3b (output-preserving, `accept_current`) are distinct code paths with their own logged-trust notes; condition 4 is a stub always-pass pending `schema_evolution.md` work.
- **Close-out (P5)** — orchestration-layer KD in `virtual_environments.md` updated to note what is now implemented vs still missing; master registry W8/virtual_env row flipped to done; ROADMAP updated.

### ~~Spec-Remediation W8/testing — Testing Framework (D-42/43/44/45)~~ ✅ (June 21, 2026)

Four-phase remediation plan ([plan](plans/20260620-w8-testing.md)) landing the D-42…D-45 decisions from the 2026-06-13 spec review:

- **Dot-separated `inputs` keys (D-42, P1)** — `inputs` maps now use the bare address path with dot separators (`silver.orders`, not `silver_orders`). The CTE name in generated SQL continues to use `_`-joining; a translation layer maps the public dot-key to the internal identifier.
- **DECIMAL exact compare + decimal-string coercion (D-44, P2)** — YAML strings that look like decimal numbers (`"300.00"`) coerce to `CAST(… AS DECIMAL(18, scale))` rather than `VARCHAR`. `Decimal128` Arrow columns compare by exact value; only `Float32`/`Float64` use the `1e-6` tolerance path.
- **CTE-level tests mock external deps, not internal CTEs (D-45, P3)** — `compile_cte_test` now collects the transitive internal CTE chain reachable from the target, replaces each `smelt.<path>` ref in the chain with a mock from `inputs`, and emits all internal CTEs as-written. Internal CTEs are never mocked.
- **`UnknownTestInput` diagnostic (D-43, P4)** — every key in `inputs` is validated against the model's actual `smelt.<path>` dependencies. An unmatched key (typo or internal CTE name) immediately fails the test with an `UnknownTestInput` message naming the bad key and the actual deps.

### ~~Spec-Remediation W8/sources — Target-Aware `name:` Override (D-35)~~ ✅ (June 21, 2026)

Four-phase remediation plan ([plan](plans/20260620-w8-sources.md)) landing the D-35 decision from the 2026-06-13 spec review:

- **`SourceNameOverride` enum (D-35 parse, P1)** — `name:` in per-entity source YAMLs now accepts both a bare `<schema>.<table>` literal (existing form, preserved) and a YAML mapping `{ <target>: <schema>.<table>, … }` (new per-target form). Invalid map values (not `<schema>.<table>`) produce `MalformedSource`.
- **`db_name_for_target` resolution (D-35 resolution, P2)** — `SourceInfo::db_name_for_target(target_name, schema)` resolves the active target: Literal → verbatim; PerTarget → map lookup, fallback to default mapping on miss; None → default mapping. The old `db_name(schema)` shim delegates to it.
- **Runtime wiring (D-35 runtime, P3)** — `SqlCompiler` stores the active target name (set via `set_target_name` in `CompilerRegistry::new`). The path-ref resolver calls `db_name_for_target` so per-target maps resolve correctly at compile time.
- **MalformedSource for undeclared target keys (D-35 close-out, P4)** — `SourceNameOverride::validate_target_keys` checks all `PerTarget` map keys against `smelt.yml::targets`. `project_source_diagnostics` runs this semantic pass after the parse-error scan, emitting `MalformedSource` (Error) for any key naming a non-existent target.

### ~~Spec-Remediation W8/config — smelt.yml Format, Default-Materialization Validation, state: Key (D-32/33/34)~~ ✅ (June 21, 2026)

Three-phase remediation plan ([plan](plans/20260620-w8-config.md)) landing the D-config cluster from the 2026-06-13 spec review:

- **`format` in `ModelConfig` + three-tier precedence `get_format` (D-32, P1)** — `smelt.yml` `models.<name>` entries now accept `format: delta|parquet`; `Config::get_format` implements the precedence chain (SQL frontmatter > `smelt.yml` model config > target default), matching the `materialization:` pattern.
- **Reject `default_materialization: test/cumulative_aggregate` (D-33, P2)** — a project-level `default_materialization` of `test` or `cumulative_aggregate` is now a hard validation error at load time; `table`, `view`, `materialized_view`, and `ephemeral` remain legal.
- **Add `state:` to `KNOWN_KEYS` (D-34, P3)** — `smelt.yml` files containing a `state:` block no longer produce a spurious unknown-key warning; `state:` is an allowlisted out-of-band key alongside `vars:`.

### ~~Spec-Remediation W7 — LSP Watched-File Set, Downstream Republication, Rename Scope & Hover (D-lsp)~~ ✅ (June 21, 2026)

Five-phase remediation plan ([plan](plans/20260620-w7-lsp.md)) landing the D-lsp cluster from the 2026-06-13 spec review (D-48/49/56):

- **Discovery-derived watch set (D-48, P1)** — `workspace/didChangeWatchedFiles` watchers are now derived from the loaded project's discovery rules (every non-excluded `.sql` plus model `.py` files), replacing hardcoded `**/models/**/*.py` + `**/functions/**/*.sql` globs. An external edit to any discoverable file triggers re-analysis.
- **Cross-file diagnostic republication (D-48, P2)** — on any watched-file change, the server republishes diagnostics for the changed file plus every file whose Salsa-derived diagnostics changed (conservative superset: all tracked files). Upstream edits now refresh downstream diagnostics in open buffers.
- **Column-rename rooted at definition site (D-49, P3)** — column rename traversal is rooted at the resolved definition site and rewrites all transitive consumers; an `AS` re-alias terminates propagation; `SELECT *` chains propagate. A source-column rename is refused at `prepare_rename` with an explanatory message (external table cannot be safely renamed via the LSP).
- **Drop mtime from hover (D-56, P4)** — `hover_text_for_loader_call` no longer accepts or emits a last-modified timestamp; hover is now a pure function of `(file bytes, schema, target)` with no mtime Salsa input.
- **Close-out (P5)** — KD sections were already clean (no retractions needed); master registry W7 row flipped to done; ROADMAP updated.

### ~~Spec-Remediation W3 — Diagnostics Codes, Ownership & Severities (D-diag) + `Unknown` Discriminant~~ ✅ (June 21, 2026)

Seven-phase remediation plan ([plan](plans/20260613-w3-diagnostics.md)) plus a follow-up discriminant sub-plan ([plan](plans/20260620-unknown-reason-discriminant.md)) landing the D-diag cluster from the 2026-06-13 spec review (D-07/08/09/14/19/30/31):

- **`HofNamedArgument` (D-19)** — a HOF call (`map`/`filter`/`reduce`) passing any argument by name fires the new `HofNamedArgument` (Error) before the kind check; named-lambda no longer silently accepted.
- **Frontmatter routing (D-14, D-31)** — malformed function frontmatter now routes to `FrontmatterParseError` instead of `BackendsWideningNotAllowed`; `FrontmatterParseError` is Error in all cases (no Warning path).
- **Directory-scoped `DuplicateFunctionDefinition` (D-30)** — two `smelt.define`s collide only when they share a name in the same directory; same name in different directories is allowed.
- **D-08/D-09 cleanup** — `smelt.source()` call-form retired; bare unresolved `smelt.<path>` routes `smelt.sources.*` → `UndefinedSource`, else → `UndefinedModelRef`.
- **`Unknown` reason-discriminant (D-07 prerequisite)** — `DataType::Unknown` carries a closed three-way reason (`Unresolved`/`Dynamic`/`Propagated`) with reason-agnostic type identity (LUB/dedup unaffected). Census-as-reason-map: every construction site declares its reason in `.claude/unknown-census.toml`; guard enforces reason presence + `error`→`unresolved` constraint.
- **`ColumnTypeUnresolved` wired (D-07)** — `DiagnosticCode::ColumnTypeUnresolved` (Error) is minted and fires at schema-layer projections producing `Unknown(Unresolved)` columns. `Unknown(Dynamic)` and `Unknown(Propagated)` are now diagnostic-free by construction (no more `CannotInferType` for those; `CannotInferType` retained for `None` data_type columns).

### ~~Spec-Remediation W5 — Python Model Reconciliation~~ ✅ (June 15, 2026)

Five-phase remediation plan ([plan](plans/20260613-w5-python-models.md)) landing the D-python cluster from the 2026-06-13 spec review (D-22/23/25/26/27):

- **Circularity = non-convergence (D-23)** — the self-tag/self-dir check is removed; circularity is now defined as a model set that never stabilises across the 5 bounded rounds. Convergent self-referential generators (a generator tags a model whose tag another generator queries) are legal.
- **Path-derived Python address (D-26)** — Python model `address_segments` is now path-derived (directory prefix from the `.py` file's workspace-relative path minus any `paths:` prefix, plus function name as the leaf), identical to SQL model addressing. Collapses when the file stem equals the function name. Closes the `test_workspace` pre-flight regression introduced by P2 (stem-equals-function collapse rule).
- **Full workspace-relative `path` in `find_models` (D-25)** — `ModelInfo.path` exposes the full workspace-relative path (forward-slash normalised); `directory` is now derived from it (final path component). The Python SDK `core.py` and Rust `ProjectModelInfo` are updated; `find_models(directory=…)` unchanged in behavior.
- **Plain single-model frontmatter + name-mismatch blocking (D-22, D-27)** — Python output is always single-model: `--- name: X ---` section-delimiter format is normalized to plain `---\nname: X\n…` before processing; `FileMetadata::Multi` is never produced from Python output. A `name:` key that mismatches the function name emits `PythonModelNameMismatch` (Error, blocks the build) while retaining all other frontmatter keys (`materialization`, `tags`, `owner`). Frontmatter is now stripped before SQL parsing to eliminate spurious parse errors from YAML keys.
- **Close-out** — retracted `directory`-implementation divergence note (implementation is now correct); updated note to user-guide gap only.

### ~~Spec-Remediation W6 — CLI & Selection (D-36–D-41)~~ ✅ (June 21, 2026)

Six-phase remediation plan ([plan](plans/20260613-w6-cli-selection.md)) landing the D-cli cluster from the 2026-06-13 spec review (D-36/37/38/39/40/41) in `smelt-cli` and `smelt-core`:

- **`smelt.` prefix round-trip (D-36, P1)** — every CLI entity argument accepts and strips a leading `smelt.` prefix before resolution, so any printed canonical `smelt.<path>` copy-pastes straight back into a command.
- **No cwd-scope fall-through (D-40, P2)** — when a `--scope` is active, a shorthand resolves only as `<scope>.<arg>`; if that exact path does not resolve, the command errors (no silent retry of the bare `<arg>`).
- **`+` operators stripped before entity resolution (D-38, P3)** — leading/trailing `+` graph operators are stripped from a `ModelName` selector before entity lookup and re-attached to the resolved full path afterward.
- **Hard error vs no-op asymmetry (D-37, P4)** — an entity-name selector that resolves to no entity is a non-zero "not found" error; a method selector (`tag:`/`generator_file:`) that legitimately matches no models is a quiet exit-0 no-op with a stderr message.
- **`--exclude +model` inconsistent-set refusal (D-39, P5)** — if `--exclude +model` removes a transitive upstream that a retained model still needs, smelt refuses the inconsistent set with a diagnostic naming the retained model and the missing upstream.
- **`smelt test --select` uses full selector syntax (D-41, P6)** — `smelt test --select` now uses the same methods (`ModelName`/`tag:`/`generator_file:`) and `+` graph operators as every other command, not a substring match on test names. Now-satisfied Known-Divergence notes retracted from `model_selection.md` and `cli.md`.

### ~~Spec-Remediation W5b — Combined SQL↔Python Fixed-Point Evaluation (D-24, ISOLATED)~~ ✅ (June 20, 2026)

Five-phase remediation plan ([plan](plans/20260613-w5b-combined-eval.md)) implementing D-24 (B) from the 2026-06-13 spec review: a single fully-interleaved fixed-point loop where SQL `generates: models` generators and Python `@model` generators run together, each observing the other's emissions across rounds:

- **Python discovery migrated to `smelt-runtime` (P1)** — `discover_python_models` and its iterative evaluation moved from `smelt-cli/src/python.rs` into `smelt-runtime/src/python.rs` so both CLI and UI consume it via the shared pipeline (`execute_project`). Closes the UI-omits-Python gap (BUG-077 class). `smelt-cli/src/python.rs` kept as a thin re-export shim.
- **Combined fixed-point loop driver (P2)** — one bounded loop in `smelt-runtime/src/combined_loop.rs` runs the SQL-generator Salsa pass and Python discovery each round against the accumulated model set, stopping when the set is byte-identical to the prior round; within-round order is `path` then `name`; bound is 5 rounds.
- **Inter-round bidirectional visibility (P3)** — generator literal `smelt.<path>` references resolve to models emitted by either family in a prior round (`workspace_shape_includes_generators` inter-round half); Python `find_models` already observed SQL emissions. Intra-round `smelt.models.*` forbid (`GeneratorBodyForbidsModelReflection`) unchanged.
- **Cross-type tests + non-convergence error (P4)** — bidirectional e2e fixtures (`sql_generator_consumes_python_emission`, `python_consumes_sql_generator_emission`), oscillating-set non-convergence error anchored at the combined loop's round bound.
- **Close-out (P5)** — `execute_parity` gate green (CLI and UI both run the combined loop via `execute_project`); `architecture.md` crate table and KD updated (Python discovery now in `smelt-runtime`); `python_models.md` References updated.

### ~~Spec-Remediation W4 — Meta-Language Reflection & Precedence~~ ✅ (June 15, 2026)

Six-phase remediation plan ([plan](plans/20260613-w4-meta-language.md)) landing the D-meta cluster from the 2026-06-13 spec review (D-15/16/17/18/20/21):

- **Spread `...` outermost (D-15)** — `...expr |> map(f)` now parses as `...(expr |> map(f))`; the spread operand is parsed through the pipe-level grammar so spread is the lowest-precedence (outermost) operator. No parentheses needed.
- **Wide-reflection ordering by `path` then `name` (D-17)** — all four wide-reflection accessors (`models_with_tag`, `models_all`, `sources_with_tag`, `sources_all`) now sort by generator-file path then model/source name as a tiebreaker, giving co-emitted models a total, deterministic order.
- **`ColumnRef` head-constructor predicates (D-21)** — five new Boolean fields on `ColumnRef` (`is_decimal`, `is_string`, `is_temporal`, `is_integer`, `is_boolean`) allow family-level column tests without spelling out precision/scale. `c.type` exact-structural equality remains deferred (unlanded meta-`DataType` change; divergence note retained).
- **Deferred `m.has(k)` stays Boolean (D-18)** — a non-static-key `m.has(k)` resolves to `Boolean` (not `Unknown`), so `if m.has(k) then m.get(k) else default` short-circuits correctly for dynamic keys; `Unknown`-collapse is reserved only for CONDs that surfaced a diagnostic.
- **`ModelRef`/`SourceRef` name/path carved out of identifier lift (D-16, D-20)** — `m.name` and `m.path` render as SQL string literals in all positions (column-ref, AS-alias, ORDER BY, GROUP BY), never bare identifiers. `ModelRef.path` remains generator-file provenance; path-keyed operations (collision, dedup, goto-def) key on the per-emission `smelt.<path>` address.
- **Close-out** — no Known-Divergence retractions needed (specs already describe the correct behavior for all D-15/16/17/18/20/21 items; the `c.type`-Unknown divergence is preserved by design).

### ~~Spec-Remediation W2 — Type-System Correctness Fixes~~ ✅ (June 14, 2026)

Six-phase remediation plan ([plan](plans/20260613-w2-type-system.md)) landing the D-types cluster from the 2026-06-13 spec review (D-28/29, C16/17/26, NOW-null, decimal-arith):

- **`Char(_)` folded into string family (D-29)** — `normalize()` now folds `Char(_)` alongside `Text`/`Varchar(None)` so all three are interchangeable for type-equality.
- **VALUES temporal-family LUB with strict tz-mixing (D-28)** — a VALUES column that mixes naive `Timestamp` with `Timestamp WITH TIME ZONE`, or `Date` with `Timestamp`, now emits `TypeMismatch` at the VALUES span; no silent `Unknown`.
- **Non-nullable origin for NOW/CURRENT_TIMESTAMP; signature-nullability lock (NOW-null, C26)** — registry-declared non-nullable nullary built-ins carry a non-nullable origin (§11); bare parameter/return annotations stay nullable, `NOT NULL` is the opt-in (locked with tests).
- **Decimal widening safety predicate (C16)** — `decimal_widening_is_safe(p1,s1,p2,s2)` enforces `s2≥s ∧ (p2−s2)≥(p−s)` in `types_assignment_compatible`; decimal-arithmetic integer-lift trigger locked with a regression test.
- **FragmentKindMismatch direction (C17)** — a Scalar-only splice point now rejects Agg/Window fragments; `ExprKind::Scalar` in the kind check no longer admits any kind unconditionally.
- **Close-out** — property oracles green at 1000 cases; no Known-Divergence retractions needed (specs already describe the correct behavior).

### ~~Spec-Remediation W1 — Universal Discovery & `paths:`-Strip Addressing~~ ✅ (June 14, 2026)

Five-phase remediation plan ([plan](plans/20260613-w1-resolve-addressing.md)) landing the D-resolve cluster from the 2026-06-13 spec review (D-01/02/04/05/06):

- **Address derivation (D-01)** — `paths:` repurposed from a scan gate to a pure strip-list; address = project-root-relative path minus any matching `paths:` prefix; a root-level file addresses as its bare stem (`smelt.<stem>`). Default `paths: ["models"]` preserves all existing example-workspace addresses.
- **Universal discovery (D-01/D-05)** — one project-wide walk replaces the per-kind gated scans; kind comes from `classify()` (content/extension), never from directory; functions discoverable anywhere (hardcoded `functions/` gate dropped); seeds and sources found project-wide; eager (`load_workspace`) and lazy (`project_seeds`/`project_sources`) share the same universe — CLI↔LSP parity by construction.
- **`schema` default (D-04)** — `targets.<name>.schema` now optional, defaulting to `"main"` via serde; downstream emitted-name path unchanged.
- **`DuplicateEmittedName` (D-02)** — structural emitted-name collision check (`(schema, segs.join("_"))`) alongside `project_address_collisions`; Error severity; persisted entities only (functions/ephemeral excluded, sources included); evaluated per active target; surfaced in both CLI and LSP.
- **Close-out / single-rule audit (D-06)** — verified address-based collision is the only uniqueness rule (no residual stem rule); no Known-Divergence retractions needed (specs already describe the new model cleanly).

### ~~Silent Failures & Code-Health Hardening~~ ✅ (June 10, 2026)

Eleven-phase hardening plan ([plan](plans/20260608-silent-failures-hardening.md)) that made "fail loud, or handle it" a **tracked, ratcheted discipline** across four fronts:

- **Front 1 (silent `Unknown`)** — Enumerated every `DataType::Unknown` construction site in production (~110 sites); classified each as `legitimate` or `error` in `.claude/unknown-census.toml`; converted the `error`-classified sites (worst case: `signatures.rs` struct-field parse failure) to emit real diagnostics (`UnknownStructFieldType` and others) instead of silently falling back to `Unknown`. New `docs/specs/diagnostics.md` documents the catalogue.
- **Front 2 (swallowed errors)** — Triaged `let _ =`, `.ok()`, and no-op `Err(_) =>` arms in the type-inference and LSP layers; surfaced genuinely-reportable failures as diagnostics or `tracing::warn`; annotated legitimately-ignored results.
- **Front 3 (panics / `unwrap` / `println!`)** — Converted the three input-driven `panic!`s in `smelt-datagen/src/generic.rs` to typed errors; annotated the seven internal-invariant guards in `smelt-db/src/diagnostics_types.rs`; gated library-crate `println!` at zero and migrated residual sites to `tracing`; paid down the worst production `unwrap` hotspots (`plan_printer.rs`, `smelt-backend-duckdb`, `smelt-db/src/lib.rs`).
- **Front 4 (`build_fn_body_map` single-source)** — Collapsed the duplicated default-extraction logic in `smelt-runtime/src/fn_bodies.rs` into one private function; both the Salsa and non-Salsa entry points delegate to it; parity test asserts identical output.

Durable CI gates now enforce the discipline: `unwrap`/`expect` and `println!` production counts frozen at `.claude/hardening-baseline.txt`; zero unresolved `error`-Unknown sites enforced by `cargo test -p smelt-types --test unknown_census`; zero library `println!` by `cargo test -p smelt-core --test hardening_budget::no_println_in_libraries`. Invariants landed in `CLAUDE.md` and `docs/specs/architecture.md`.

### ~~Feature Sweep / Bug Ledger — sweep complete~~ ✅ (June 8, 2026)

The autonomy-loop-driven [feature sweep](plans/20260530-feature-sweep.md) (a two-level backlog: a master feature-sweep ledger plus focused remediation sub-plans) has probed every dimension and landed every remediation cluster. All probe dimensions (S0–D8) are done, all 11 sub-plans landed, and the last structural residue closed — BUG-064 (the `smelt-db → smelt-planner` layering inversion) fixed via a new `smelt-logical` crate. The `BUG-*` closures across May 31–June 8 cover diagnostic parity, frontmatter parity, codegen soundness, address-collision enforcement, smelt.yml/CLI surface alignment, function-default self-containment, cross-family arithmetic strictness, per-target overlay wiring, and property-test dispatch + `week_start` domain enforcement.

**Residual (post-sweep human triage, explicitly scoped out of the loop):** 6 `needs-review` judgment calls from the final D2–D8 wave — BUG-067/068 (meta-language: `smelt.config.var` in `smelt.define` body; `List<T>` as a `smelt.define` parameter type), BUG-070/071 (cumulative: backbuild uses legacy full-refresh path; Known-Divergences gap for Month/Quarter/Year), BUG-072/073 (sources: source `timeseries` silently ignored; `inject_source_filters` not wired into the incremental path) — plus BUG-062 (a deferred docs-gap). The silent-failure-class items (BUG-072/073) feed into [What's Next #1](#1-silent-failures--code-health-hardening); the rest await a human pass over `docs/bug-hunt/2026-05-30-findings.md`.

### ~~CLI Execute-Loop Migration to `smelt-runtime`~~ ✅ (June 7, 2026)

[Seven-phase migration](plans/20260524-cli-runtime-migration.md) finishing the Run-Pipeline-Parity refactor: incremental windows, planner safety gate, temporal bound derivation, and schema-evolution checks moved into `smelt-runtime`; `LogicalGraph` (884 lines) and `PhysicalGraph` (1184 lines) deleted in favour of a `PlanSummary` for `--show-plan`; `smelt-cli`'s `commands/run.rs` migrated to `execute_project` with a `StdoutReporter`; shim modules deleted. The durable enforcement landed too: the `execute_parity` CI gate (`cargo test -p smelt-runtime --test execute_parity`) runs identical fixtures through both the CLI and UI entry points, and `pub(crate)` lockdown of the compile internals makes a half-compile a type error — so future CLI↔UI divergence is a compile error, not a review catch. Completes the predecessor [`smelt-runtime` extraction](plans/20260523-smelt-runtime-extraction.md).

### ~~Per-Target Config Overlay — production wiring complete~~ ✅ (June 6, 2026)

Closed BUG-014: a headline `meta_config_loading` feature (`smelt build --target prod` reading a sibling `<basename>.prod.<ext>` overlay and merging it into the base) that was implemented and unit-tested at the loader layer but had zero production callers ([plan](plans/20260605-per-target-overlay-wiring.md)):

- **`active_target` Salsa input** (P1) — `Option<Arc<str>>` field on the singleton `Workspace` input in `crates/smelt-db/src/lib.rs`; `set_active_target` setter; retiring the lib.rs stub comment. Salsa invalidation verified.
- **Target threading through `load_workspace`** (P2) — `smelt-core::workspace::load_workspace` reads the `smelt.yml` `target:` field as the default; CLI `--target` overrides it on the build/run path. Overlay files (`cohorts.prod.yaml`) paired as overlay inputs rather than orphan base inputs in `workspace_ingest.rs`. Dual gate (`example_diagnostics` + `example_workspaces`) stays green — CLI↔LSP discovery symmetric.
- **`collect_loader_values` dispatches to overlay** (P3) — computes `<basename>.<target>.<ext>` from the effective target and calls `loader_resolved_value_with_overlay` when the overlay input exists; falls back to base-only resolution when target is unset or the overlay is absent. `examples/meta_config_overlay_probe`: `smelt build --target prod` now yields `revenue >= 999` (not the base `>= 100`).
- **Overlay validation diagnostics wired** (P4) — a schema-violating overlay surfaces `ConfigLoaderUnknownField` anchored at the overlay file's offending row and fails the build; same diagnostic family as a base-file mismatch. `examples/meta_config_overlay_probe_invalid` gate.
- **Close-out** (P5) — end-to-end regression test `meta_config_e2e.rs::e2e_per_target_overlay_wires_into_generator_build` confirms dev/prod row counts; spec References updated; BUG-014 flipped to fixed in ledger.

### ~~Cross-Family Arithmetic Strictness — `Unknown` + `TypeMismatch` enforced~~ ✅ (June 6, 2026)

Enforced the types spec rule that cross-family binary arithmetic (e.g. `42 + '3'`, `TRUE + 1`) must produce `Unknown` and emit a `TypeMismatch` diagnostic ([plan](plans/20260605-cross-family-arithmetic-strictness.md)):

- **Family guard added to `promote_numeric_operands`** (P1) — `crates/smelt-db/src/type_inference/binary.rs`: the catch-all `(Some(l), _) => Some(l)` arm now checks family compatibility before returning the left type; a cross-family pair yields `DataType::Unknown` instead of the left operand's type. Numeric/numeric promotion and `INTERVAL * numeric` special cases unchanged. Closed BUG-017 (1/2). Regression tests: `test_cross_family_arithmetic_numeric_plus_string`, `test_cross_family_arithmetic_boolean_plus_numeric`, `test_cross_family_arithmetic_numeric_plus_string_literal`.
- **`check_crossfamily_arithmetic_diagnostics` + `TypeMismatch` emission** (P2) — new walker in `binary.rs`; wired into `file_diagnostics` and the `check_types` query so a `TypeMismatch` Error is emitted at the operator span for each cross-family arithmetic operation. Closed BUG-017 (2/2). Keepable fixture `examples/types_broken_crossfamily_add/`. Regression tests: `file_diagnostics_emits_type_mismatch_crossfamily_numeric_plus_string`, `file_diagnostics_emits_type_mismatch_crossfamily_boolean_plus_numeric`, `types_broken_crossfamily_add_emits_type_mismatch`.
- **§296 Known Divergence narrowed** (P3) — `docs/specs/types.md` updated to clarify that cross-family binary arithmetic is now enforced while the composite-path gaps (array-literal / `UNION` unification) remain deferred.

### ~~Function Default Self-Containment — Semantics #9 enforced~~ ✅ (June 6, 2026)

Enforced the functions spec rule that a default expression must not reference other parameters in the same signature ([plan](plans/20260605-function-default-self-containment.md)):

- **`DefaultReferencesParameter` diagnostic code** — new variant added to `DiagnosticCode`; documented in `functions.md` §Diagnostic codes table. Closed BUG-003 (1/2).
- **AST-side validator** — near `default_type_lookup` in `crates/smelt-db/src/queries/function_diagnostics.rs`; for each parameter with a default, scans the default expr's identifier/column-ref tokens for a sibling-param-name match and emits `DefaultReferencesParameter` anchored at the default expr's range. Self-contained defaults (`= 1`) produce no diagnostic. Closed BUG-003 (2/2). Regression tests: `default_referencing_sibling_param_is_error`, `self_contained_default_is_ok` in `crates/smelt-db/tests/function_body_check.rs`.

### ~~smelt.yml Surface Alignment — spec drift closed, unknown-key warning~~ ✅ (June 5, 2026)

Closed spec-vs-code drift on the `smelt.yml` and CLI model-selection surfaces ([plan](plans/20260605-smelt-yml-surface-alignment.md)):

- **`backbuild` positional-selector documented** (P1) — `model_selection.md` §Flags amended: removed `backbuild` from the `--select` "Available on" cell; added a note that `backbuild` takes a single positional selector, always forces `+` upstream, and has no `--exclude`. Closed BUG-046 (spec-only).
- **`timeseries` row added to model-config table** (P1) — `smelt_yml.md` §"Model-config shape" now lists `timeseries | object | absent | Time-dimension declaration`. Closed BUG-059 (spec-only).
- **`cumulative_aggregate` added to `default_materialization` accepted values** (P1) — the Surface table and its `docs-site/` mirror now list all six valid values. Closed BUG-061 (spec-only).
- **Unknown top-level key warning implemented** (P2) — `Config::parse_with_warnings` now iterates the raw YAML map against a 10-key allow-list and emits a named warning per unknown key (including typos like `default_matrialization`). Legacy keys (`model_paths`, `seed_paths`) and `unstable_schema` are allow-listed to avoid duplicate/false-positive warnings. Closed BUG-060. Regression tests: `config::tests::unknown_top_level_key_warns`, `valid_config_with_all_known_keys_emits_no_generic_warnings`, `legacy_path_key_does_not_also_get_generic_unknown_key_warning`.

### ~~Address Collision Enforcement — workspace identity, discovery consolidation~~ ✅ (June 5, 2026) [BUG-002/021/040/063; P4 blocked]

Enforced the one-path-one-entity invariant across all entity kinds and consolidated seed discovery ([plan](plans/20260605-address-collision.md)):

- **Pure address-map resolver** (P1) — `smelt_core::resolve_address_map` computes the canonical `smelt.<path>` address for every discovered model (including within-file `--- name ---` sections), function, seed, and source; detects cross-kind and within-file collisions without a filesystem re-walk. Closed BUG-002 (1/2), BUG-021 (1/2). Regression test: `smelt-core::resolver::tests`.
- **`project_address_collisions` Salsa query** (P2) — surfaces `DuplicateAddress` (Error) diagnostics to CLI and LSP via the shared diagnostic channel; fixture `examples/architecture_broken_path_collision` (model `dup.sql` + seed `dup.csv`) confirmed. `smelt build` and `smelt explain --json` now exit non-zero on a collision. Closed BUG-002 (2/2). Regression tests: `crates/smelt-cli/tests/address_collision.rs`.
- **Model-name uniqueness surfaced** (P2b) — spec re-scoped uniqueness to canonical `smelt.<path>` address (Constraint 4 re-worded; stale cross-file example dropped); Python `ModelFile.address_segments` populated; `resolve_address_map` applied CLI-side over combined SQL+Python model set. Closed BUG-021 (2/2), BUG-040. Regression test: `test_python_model_name_collision` + `within_file_section_collision_surfaces_duplicate_address`.
- **Seed discovery consolidated** (P3) — `smelt-cli::discover_seeds` deleted; all seed lookups route through `smelt_core::discover_seed_infos_strict`; sources/functions audited (single-sourced). Eliminates the third seed-discovery path that was the structural root cause of the asymmetric-discovery bug family. Closed BUG-063.
- **BUG-064 closed via `smelt-logical` extraction** (P4 of address-collision plan + dedicated extraction plan P1–P5) — `smelt-logical` crate extracted; `smelt-db → smelt-planner` production edge removed. Closed BUG-064 (2026-06-08). Structural gate: `cargo tree -p smelt-db -i smelt-planner` shows no production path.

### ~~Codegen Soundness — CTE collisions diagnosed, `source.*` valid~~ ✅ (June 5, 2026)

Closed the silent-until-`smelt run` function-expansion defects found in the feature sweep ([plan](plans/20260604-codegen-soundness.md)):

- **`source.*` over a `smelt.<path>` argument now emits valid SQL** (C2) — the transparent-function splice aliases the argument to the parameter name (`FROM <arg> AS <param>`) instead of text-replacing `<param>` with the qualified name. `main.base.*` → `source.*` stays single-part and DuckDB-valid. Closed BUG-009. Regression fixture: `examples/fn_tableexpr_star/`.
- **CTE-collision diagnostic `CteShadowsCallerCte`** (C3) — new analysis-time check in `check_file_diagnostics`: when a model's top-level CTE name collides with a CTE in the body of a directly-called transparent function, the compiler emits `CteShadowsCallerCte` (Error) anchored at the call site, refusing the build rather than silently emitting wrong data. v1 covers direct collisions only; transitive collisions are a known gap (alpha-rename is v2). Closed BUG-007. Regression fixture: `examples/expansion_broken_cte_caller_collision/`.
- **`make_generator_frame` signature corrected in `expansion.md`** (C1) — doc-only retraction of the stale 3-arg form. Closed BUG-008.

### ~~Frontmatter Parity — unified catalogue, no silent drops~~ ✅ (June 4, 2026)

Collapsed two divergent frontmatter parsers into one over a key catalogue ([plan](plans/20260604-frontmatter-parity.md), [spec](specs/architecture.md) §"Unified frontmatter rule"):

- **`deny_unknown_fields` on `TimeseriesConfig`** (U1) — unknown `timeseries:` sub-keys now produce a serde error instead of being silently ignored. Closed BUG-025.
- **`FrontmatterCatalogue` + `parse_frontmatter`** (U2) — single entry point in `smelt-core::frontmatter`; catalogue maps each key to its applicable declaration kinds; unknown key → `Error`, inapplicable key → `Warning`, valid key → kept.
- **Model path wired** (U3) — `ModelMetadata` deserialized from the validated map; errors surfaced as `FrontmatterParseError`/`MalformedTimeseries` diagnostics in `file_diagnostics`. Closed BUG-016, BUG-023.
- **Function/extern path wired; second parser deleted** (U4) — `FunctionProperties` built via `parse_frontmatter`; hand-rolled `parse_function_properties` deleted. One parser remains.
- **E2E example fixtures + gates** (U5) — four regression examples: `frontmatter_function_key_on_model` (positive, builds as TABLE with Warning), `timeseries_broken_invalid_granularity`, `timeseries_broken_unknown_key`, `frontmatter_broken_unknown_key` (all build-refused with Error).
- **Deferred**: dynamic schema-registration API for non-built-in planner rules (tracked in [planner_rule_api_design.md](planner_rule_api_design.md)).

### ~~Diagnostic Parity (analysis ↔ build) + Meta-Language Codegen~~ ✅ (June 2026)

Closed the "LSP-clean but unbuildable" bug class surfaced by the feature sweep ([plan](plans/20260531-diagnostic-parity.md), [spec](specs/architecture.md) §"Diagnostic parity rule"):

- **Shared Error-severity build gate** (P2, June 1) — `smelt_runtime::gate_diagnostics` runs the full `file_diagnostics` surface (not just `UnknownSmeltFn`) before any model compiles; wired into both the CLI run path and `execute_project`. Closed BUG-015, 019, 024.
- **Uniform planner rule → diagnostics interface** (P2b, June 1) — cumulative classifier and incremental batch-safety/bounds checks now surface via `file_diagnostics` and are visible to both the editor and the build. Closed BUG-011.
- **Per-entity source diagnostics** (P2c, June 1) — new `project_source_diagnostics` Salsa query maps `SourceError` variants to `MalformedSource`/`SourceTypeError` diagnostics and publishes them to the LSP at init time. Closed BUG-032.
- **Nested `smelt.define` fixpoint** (P3, June 1) — printer's body-reparse now re-expands nested `SMELT_PATH_CALL` nodes to a fixpoint via a synthetic `SELECT`-prefix reparse; `functions_demo` nested-compose models execute correctly. Closed BUG-013.
- **Block `PASSING` fragment binding** (P4, June 2) — printer merges `PASSING <name> AS (<body>)` clauses into the existing named-arg vector before substitution; `rollup_with_passing` executes correctly. Closed BUG-018.
- **In-model meta-language at build** (P5–P7d, June 2–3) — a pure-text build-path meta evaluator in `smelt-runtime::meta_eval` lowers all analyzer-accepted constructs before codegen: list spread (P5), HOF/pipe/lambda/ternary/config.var (P6), `smelt.columns_of` reflection (P7a), wide reflection `smelt.models.*`/`smelt.sources.*` (P7b), bare List/Map loader detector + List-loader lowering (P7c), Map-loader via `MAP_METHOD_CALL` postfix parsing + `.keys()`/`.values()`/`.entries()` lowering (P7d). Closed BUG-006 (all sub-issues).
- **`example_builds` CI gate** (P1) — builds + executes every example workspace on DuckDB; `meta_config` removed from `KNOWN_UNBUILDABLE` after P7d; remaining entries are unseeded-source workspaces (structural, not codegen gaps).

### ~~Virtual Environments — research, Stage 0 prototype & specs~~ ✅ (June 1–4, 2026)

Proved the core thesis of opt-in virtual data environments — *reuse a physical table when a change is provably output-preserving* — without any state or environment machinery, then specced the feature set.

- **Semantic output-fingerprint oracle** ([`crates/smelt-fingerprint`](../crates/smelt-fingerprint), [spec](specs/output_fingerprint.md)): hashes a canonical normal form of a model's `SELECT` so two versions with the same fingerprint provably compute the same relation (multiset, columns by name). Recognises as equivalent — where SQLMesh's edit-script rebuilds — formatting, comments, keyword case, projection reorder, internal CTE/alias rename, and single-use-CTE ≡ derived-table (recursive sub-fingerprint). Conservative verbatim fallback everywhere else.
- **Soundness gate**: `fingerprint-equal ⇒ DuckDB relations identical` as a property test against DuckDB, with positive/negative golden corpora — the load-bearing invariant before any reuse is wired to execution.
- **Three soundness bugs found and fixed**, each via the discipline "generate the real-world shape and let DuckDB judge": implicit-alias column lists mis-parsed (`FROM (…) t(c1,c2)`, fixed on `main`); a derived-table-left **join** silently dropped by inlining; `LIMIT`/`OFFSET`/`QUALIFY` entirely absent from the canonical form (every top-N/paginated model collapsed to one fingerprint).
- **Determinism detector**: structural deny-list (non-deterministic built-ins, parenless temporal specials, order-sensitive aggregates) + row-slice-without-total-order check, surfaced as `deterministic` on the result. Gated so anything flagged deterministic reproduces across two independent DuckDB builds. Closes §5.5's value axes; window-function non-determinism is the noted residual.
- **Specs authored**: [`output_fingerprint.md`](specs/output_fingerprint.md) (normative), [`virtual_environments.md`](specs/virtual_environments.md) (staged orchestration design), [`run_state.md`](specs/run_state.md) (`.smelt/` layout); touched `architecture.md`, `incremental_models.md`, `schema_evolution.md`.

Research: [`docs/research/20260601-virtual-environments.md`](research/20260601-virtual-environments.md). Next: the implementation queue under [What's Next #4](#4-virtual-environments--backbuild-change-detection-specs-authored-prototype-proven).

### ~~Typed Meta-Language — Phase E2: Multi-Model Production~~ ✅ (May 16, 2026)

Completed Phase E2 of the typed meta-language plan ([plan](plans/20260509-meta-language-E2.md), [spec](specs/meta_language.md)):

- **`generates: models` frontmatter directive** — marks a file as a generator file whose body is a `List<ModelDef>` meta-expression. The `.gen.sql` extension is a recommended convention.
- **`ModelDef` built-in closed record type** — five fields: `name` (required, `Text`), `body` (required, `TableExpr`), `materialization` (optional, `Text`), `tags` (optional, `List<Text>`), `description` (optional, `Text`). User-constructible only inside generator file bodies.
- **W1–W4 workspace-shape resolution pipeline** (Salsa-cached): W1 discovers generator files; W2 evaluates each generator's body in isolation; W3 collision-checks and emits survivors/discarded; W4 type-checks the full workspace including emitted models.
- **Ten diagnostic codes**: `GeneratesUnknownValue`, `GeneratesMixedWithBareModel`, `GenerateFileBareSelectForbidden`, `GenerateFileBodyTypeError`, `ModelDefOutsideGeneratorFile`, `ModelDefInvalidName`, `ModelDefInvalidMaterialization`, `ModelDefDuplicateName`, `ModelDefHandAuthoredCollision`, `GeneratorBodyForbidsModelReflection`.
- **`<generator>` expansion frame** — `evaluate_generator` stamps the `<generator>` anonymous frame onto every diagnostic from inside the generator body's HOF chain. The frame has `function = "<generator>"`, `decl_path = generator_file_path`, `call_site_range = body expression range`.
- **Generator-file CLI integration** — `build_logical_graph` and `discover_emitted_model_files` in `smelt-cli` wire emitted models into the logical graph. `register_loader_files_from_disk` in `init_db` auto-registers YAML/JSON/TOML loader files so `smelt.config.load_yaml` calls in generator bodies can evaluate.
- **LSP pure helpers** — `hover_text_for_generates_frontmatter`, `hover_text_for_model_def_literal_open_brace`, `hover_text_for_model_def_name_field_value`, `hover_text_for_model_def_body_field_value`, `completion_for_generates_value`, `completion_for_model_def_field_key`, `goto_def_for_emitted_model_reference` — all unit-tested. Backend dispatch wiring is Phase G.
- **`examples/per_cohort_union/`** killer demo — three cohorts from `cohorts.yaml`, union in `all_cohorts_unioned.sql`, zero LSP diagnostics.
- **`examples/staging_from_sources/`** secondary demo — staging layer generator from source YAML files, zero LSP diagnostics.
- **Ten broken sub-fixtures** — one per diagnostic code under `examples/broken/meta_language_e2_broken/`.
- **User docs**: `docs-site/docs/meta-language/generators.md`, index/reflection/reference page additions.

See [plan](plans/20260509-meta-language-E2.md). Next: Phase G (rename, LSP completeness sweep, `/smelt-loop` `large` tier).

### ~~Smelt Functions — Steps 6–13 (PASSING, planner, struct row vars, review remediation)~~ ✅ (April 24–26, 2026)

Completed the remaining eight steps of the smelt-functions experimentation roadmap (Phases 28–53 of [plan](plans/20260422-smelt-functions.md)):

- **Step 6** (Phases 28–29, April 24): Context-sensitive `PASSING name AS (...)` parser (peek `PASSING` only after `smelt.fn.*` / user-defined call closings); binding PASSING fragments to `SelectItems` parameters with type-checking and kind-ceiling enforcement. `rollup_with_passing.sql` demo.
- **Step 7** (Phases 30–34, April 25): Functions as first-class `LogicalNode::FunctionCall` nodes in the logical plan; column provenance + declared-property propagation (`provenance:`, `joins:`, `deterministic:` frontmatter); `PlannerRule` trait + `apply_rules_to_fixed_point`; `ExpandTransparentFunctionCalls`, `PushFilterIntoTransparentFunction`, and `EliminateUnusedLeftJoin` rules. `--show-plan` CLI flag wired in Phase 39.
- **Step 8** (Phases 35–38, April 25): Struct row variables (`Struct<{..r}>`), value-level spread (`..event`), call-site row-var unification with erasure at expansion, `smelt.as_struct(<alias> EXCEPT ...)` with backend-specific struct-literal emission.
- **Steps 9–13 — review remediation** (Phases 39–53, April 25–26): 15 phases closing all 28 findings from the post-Phase-38 plan review. Key deliverables: `--show-plan` CLI integration (Phase 39), CAST emission from canonical-return registry (Phase 40), transparent-call body splice into logical plan (Phase 41), list-splice comma elision (Phase 41), `smelt.as_struct` lowering to `smelt-planner` + broadened capability gate (Phase 42), serde_yaml frontmatter parser replacing line-walker (Phase 43), `safe_divide` / `monitored_session_rollup` canonical fixtures (Phases 44–44b), JOIN alias visibility in `TableExpr` bodies + `enriched_order` workaround removed (Phase 45), `TableExpr` argument shapes extended to CTEs / derived tables / subqueries (Phase 46), cross-function CTE schema inference + opaque-CTE suppression dropped (Phase 47), LSP hover + PASSING completion + multi-level frame trace in message (Phase 48), `WindowInScalarContext` deep-walk into scalar subqueries (Phase 49), built-in registry expansion (operators, aggregates, window functions — Phase 50), `provenance:` / `joins:` validator (Phase 51), missing-provenance pushdown advisory + extern fragment-param rejection (Phase 52), plan audit / SHA table + cross-file extern collision fixture (Phase 53).

See [plan](plans/20260422-smelt-functions.md) for the full phase-by-phase record. User documentation: [Functions guide](../docs-site/docs/guide/functions.md).

### ~~Smelt Functions — Steps 1–5~~ ✅ (April 22–24, 2026)

Implemented the first five steps of the smelt-functions experimentation roadmap (Phases 1–27 of [plan](plans/20260422-smelt-functions.md)):

- **Step 1** (Phases 1–6, April 22): `smelt.define` / `smelt.fn.*` parser, Salsa signature index, `Expr<T>` type-reference resolution, Tier 1 body type-check, call-site expansion with single-level frame trace. `safe_divide` end-to-end demo. `examples/functions_demo/` workspace created and registered with CI.
- **Step 2** (Phases 7–12, April 23): `Ordered` constraint, canonical built-in signature registry (~40 functions, generics + variadics), `infer_function_type` rewired through registry, `smelt.extern` declarations, per-declaration frontmatter with `backends:` inference and backend-namespace sugar, multi-level frame rendering in LSP, CAST-enforcement flag on canonical returns.
- **Step 3** (Phases 13–18, April 23–24): `TableExpr` / `AggExpr` / `WindowExpr` / `SelectItems` type-ref grammar; `ExprKind { Scalar, Agg, Window }` with linear subtyping and `SelectItems<K>` kind ceiling; `TableExpr` bare-column row polymorphism with parameters-first scoping and shadow warnings; row-requirement annotations (`TableExpr<{col: Type, ..r}>`); `sessionize` end-to-end with TableExpr output-schema inference; LSP hover for `smelt.define` parameter types (`TableExpr<{...}}` and `Expr<...>` rendered); `add_margin → sessionize` pipeline fixture.
- **Step 4** (Phases 19–22, April 24): Context-binding parsing and resolution for `Expr<T, ctx>` and `SelectItems<Kind, ctx>`; CTE schema extraction (`extract_function_body_cte_schemas`) with topological ordering and opaque-CTE suppression for `SELECT * FROM smelt.fn.*` patterns; `unknown_context_diagnostics_for_file` extended to accept CTE names alongside parameter names; `check_fragment_context_bindings` extended to look up CTE column schemas; `()` empty-default parser support; `session_rollup` end-to-end demo added to `examples/functions_demo/`.
- **Step 5** (Phases 23–27, April 24): Tier 2 body check in isolation, Tier 3 return-type verification + LSP hover, call-site bidirectional pre-expansion checking (Phase 25), Tier 2 → Tier 1 inline expansion with frame-stack propagation (Phase 26), and bidirectional generics (`unify_call_with_expected` with `expected_return: Option<DataType>` propagated from `TypeContext`, Phase 27). Upgrade story documented in [`docs/smelt-functions-upgrade-story.md`](smelt-functions-upgrade-story.md).

**Deferred during Steps 1–5**: See "Deferred during implementation" appendix in the plan for the full list. Key items: structured `Synthesized` marker for default-value provenance, broad TableExpr argument shapes beyond `smelt.ref()`/`smelt.source()`, SQL comma-elision for empty `SelectItems` defaults (Phase 32/planner). PASSING clauses (Step 6), planner visibility (Step 7), struct row vars (Step 8).

### ~~Type Inference, Parser & Ref Resolution Fixes~~ ✅ (April 10, 2026)

All critical/major bugs from the smelt_shop real-world validation report fixed:

- **Seeds as `smelt.ref()` targets** — Seeds are now first-class dep-graph citizens. `resolve_ref()` searches seeds after SQL models; CSV column types inferred and provided to the type-checking layer. No more `sources.yml` workaround.
- **JOIN type inference** — Qualified column refs (`p.col`) no longer fall through to `infer_literal_type()`. Fixed by detecting dot patterns before decimal literal inference.
- **CASE expression column names** — `CAST(? AS TYPE) AS ?` bug fixed; compiler generates `_col1, _col2` deterministic names for unnamed CASE outputs.
- **CASE expression type widening** — `infer_case_expr_type` now promotes across all branches; `promote_types` widens Decimal+Integer to Decimal(38,10).
- **EXTRACT(EPOCH FROM ...)** — New dedicated `EXTRACT_EXPR` syntax kind in the parser handles `EXTRACT(field FROM expr)` without treating the FROM keyword as SQL FROM.
- **CTE type inference** — `parse_when_clause()` fixed to use `parse_or_expr()`, enabling full logical expressions in CASE WHEN.
- **Subquery ref replacement** — Subquery type inference now clones context and processes inner FROM before calling `infer_select_column_types`.
- **FLOAT→DOUBLE normalization** — `CAST(x AS FLOAT)` infers as DOUBLE; `float_division` and `cast_float_as_double` divergences documented.
- **Materialization type changes** — `execute_model()` now drops both table and view before creating either, handling view↔table transitions automatically.
- **Datagen geometric min** — `GeneratorSpec::Geometric` accepts optional `min: i32` to prevent zero values.

See [plan](plans/20260409-smelt-shop-fixes.md) for full details.

### ~~Packaging — Source Distribution & Python 3.14 Wheels~~ ✅ (April 10, 2026)

- Added `build-sdist` job to release workflow using `maturin sdist`
- sdist included in PyPI and TestPyPI publish steps
- `bindings = "bin"` in pyproject.toml produces `py3-none-{platform}` wheels, compatible with Python 3.9–3.14 on all platforms

### ~~Testing Strategy Improvements~~ ✅ (April 10, 2026)

- Added `examples/ecommerce/` workspace (19 models, 2 seeds, 3 sources) as regression scaffold
- Added `ecommerce_no_diagnostics` test to `example_diagnostics.rs`
- Added `ecommerce_execution.rs` compile-and-execute integration test against DuckDB
- Property tests cover CTEs, set operations, joins, and type inference across full model patterns

### ~~LSP Refactorings & Code Actions~~ ✅ (April 5-6, 2026)

Full refactoring support in the LSP: rename (CTEs, models, sources, columns with cross-file lineage tracing), code actions (CAST fixes, create model, add source/column, extract CTE, inline CTE), and find-references. All implemented as pure functions in smelt-db with thin LSP wrappers. Also fixed arrow 57→58 version mismatch and extracted duplicated functions to shared crates.

See [plan](plans/20260405-lsp-refactorings.md) for details.

### ~~LSP Goto-Definition & Column Diagnostics~~ ✅ (April 3-4, 2026)

Major LSP expansion: goto-definition now covers sources, CTEs, columns, and qualified references. Undeclared column reference diagnostics added. Python model LSP integration with real `ProjectContext`. Multiple stability fixes.

See [LSP & Editor Support](#lsp--editor-support) below for full details.

### ~~Code Quality & Hardening~~ ✅ (March 28, 2026)

All four sub-items completed:
- ✅ Snapshot tests: 30 `insta` tests for `smelt-dialect` covering all dialect rewrite paths
- ✅ CLI decomposition: `main.rs` split from 2,656 → 339 lines + 12 per-subcommand modules
- ✅ Structured logging: `tracing` crate replaces ~90 `println!`/`eprintln!` calls across 14 files
- ✅ unwrap() audit: ~35 production `unwrap()` → `expect("reason")` across 13 files

See [Code Quality & Hardening](#code-quality--hardening) below for details.

### ~~Data Testing Framework — `smelt test`~~ ✅ (March 27, 2026)

Fully implemented. See [Data Testing Framework](#data-testing-framework) below for details.

### ~~Data Catalog — `smelt docs generate`~~ ✅ (March 29, 2026)

Static data catalog / data dictionary generation. Outputs Markdown (default) or JSON.

- ✅ Per-model pages: description, owner, tags, materialization, columns with inferred types and lineage, upstream/downstream deps, incremental config
- ✅ Column enrichment: merges Salsa type inference with frontmatter descriptions and column-level tests
- ✅ Project index: model table, tag index, execution order
- ✅ JSON format: structured `catalog.json` for machine consumption
- ✅ `--select` filtering reuses existing selector infrastructure
- ✅ Nested subcommand (`smelt docs generate`) for future `smelt docs serve`

See [plan](plans/20260329-docs-generate.md) for details.

### ~~Schema Diff — `smelt diff`~~ ✅ (March 29, 2026)

Offline schema change detection. Compares inferred model schemas (from SQL parsing/type inference) against deployed schemas (`.smelt/schemas/`) without requiring a database connection.

- ✅ Per-model diff: column additions, removals, type changes, nullability changes
- ✅ Risk assessment: safe ALTER TABLE vs full refresh vs column removal flag
- ✅ `--select`/`--exclude` filtering reuses existing selector infrastructure
- ✅ `--json` output for CI integration (machine-readable)
- ✅ Exit code 1 when changes detected (CI-friendly)
- ✅ Removed model detection (deployed schema exists but model deleted from code)
- ✅ Per-model target resolution (works with multi-backend projects)

### ~~Schema Evolution~~ ✅ (March 30, 2026)

Efficient schema migrations using ALTER TABLE + DEFAULT values instead of full table refresh.

- ✅ Column `default:` in frontmatter — NOT NULL column additions use `ALTER TABLE ADD COLUMN ... DEFAULT val` instead of full refresh
- ✅ Column `backfill:` in frontmatter — SQL expression for UPDATE backfill after ALTER TABLE ADD COLUMN
- ✅ `schema_evolution: { strategy: full_refresh }` — opt out of ALTER-based migration per model
- ✅ Nullable-to-NOT-NULL with default — `UPDATE ... WHERE IS NULL` + `ALTER SET NOT NULL`
- ✅ `smelt diff` shows migration plan with defaults (ALTER with DEFAULT instead of full refresh)

### ~~Schema Evolution — Complex Types~~ ✅ (April 5, 2026)

Production schema evolution for nested/complex types (Struct, Array, Map). Previously, any change to a complex type column triggered a full table refresh.

- ✅ `parse_type()` extended for `STRUCT(...)`, `TYPE[]`, `MAP(K, V)` with recursive nesting
- ✅ `Map(Box<DataType>, Box<DataType>)` variant added to `DataType`
- ✅ Recursive type normalization (`DataType::normalize()`)
- ✅ Structural diff for complex types — field-level additions, removals, type widening, nested changes
- ✅ Safe widening rules for nested types (e.g., `INTEGER` → `BIGINT` inside a struct)
- ✅ Abstract `SchemaOperation` enum for backend-agnostic migration planning
- ✅ DuckDB DDL generation: struct dot-notation, `struct_pack` rewrites, `list_transform` for array-of-struct
- ✅ Spark DDL generation: `mergeSchema` for safe additions, `TableRewrite` for unsupported operations
- ✅ Table format config (`format: delta|parquet`) at target and model level
- ✅ `--allow-full-refresh` CLI gate for expensive operations
- ✅ `default:` changed from YAML value to SQL expression string (breaking change)
- ✅ Identifier quoting for SQL keywords and special characters
- ✅ Graceful fallback for unparseable type strings with warnings
- ✅ Round-trip verification: `DataType` → `to_sql()` → `parse_type()` → `DataType`
- ✅ User-facing documentation on smeltsql.com (schema-evolution guide, backend capability matrix)

See [plan](plans/20260405-schema-evolution-complex-types.md) for details.

### ~~Spark / Databricks Backend~~ ✅ (March 28, 2026)

Spark backend implemented via PySpark/PyO3 bridge. All Backend trait methods are now functional, connecting to Spark through PySpark's SparkSession.

- ✅ PySpark bridge via PyO3 — thin Python adapter (`spark_adapter.py`) wraps SparkSession
- ✅ SQL execution with zero-copy Arrow result conversion (`pyarrow.Table` → `RecordBatch` via C Data Interface)
- ✅ Table/view materialization (DROP + CREATE TABLE AS, CREATE OR REPLACE VIEW)
- ✅ Incremental support: DELETE+INSERT, MERGE INTO, INSERT OVERWRITE, APPEND
- ✅ Catalog/schema management (three-part names: `catalog.schema.table`)
- ✅ pyo3 upgraded from 0.24 → 0.26 (required for arrow-pyarrow compatibility)
- ✅ Works with local Spark Connect, Databricks Connect, EMR, Dataproc
- 🔮 Integration test parity with DuckDB tests (requires Spark Connect server)
- 🔮 Authentication configuration docs (tokens, OAuth, instance profiles)

---

## Code Quality & Hardening ✅ (March 28, 2026)

### Structured Logging ✅

- `tracing` crate with `EnvFilter` (controlled via `RUST_LOG` env var)
- ~90 `println!`/`eprintln!` calls converted to `tracing::info!`/`debug!`/`warn!` across 14 files
- Program output (tables, JSON, test results) kept as `println!` for piping

### Error Handling ✅

- ~35 production `unwrap()` calls replaced with `expect("reason")` across 13 files
- Focused on smelt-cli, smelt-db, smelt-core, smelt-backend-duckdb
- Test code left as-is (idiomatic Rust)
- Remaining `unwrap()` calls are in test code or already have proper error handling

### CLI Decomposition ✅

- `main.rs` split from 2,656 → 339 lines (arg structs + dispatch only)
- 11 per-subcommand modules under `src/commands/` (run, backbuild, seed, build, status, history, explain, table, type, ui, test)
- Shared utilities extracted to `src/helpers.rs` (352 lines)

### Snapshot Testing ✅

- 30 `insta` snapshot tests for `smelt-dialect` printer
- Covers all dialect rewrite paths: QUALIFY, ARRAY, DATE, `::` cast, trailing comma, function remapping, ref/source resolution, ephemeral refs, combined rewrites
- All three dialects tested: DuckDB, SparkSQL, PostgreSQL

---

## Data Testing Framework ✅ (March 27, 2026)

### Test Types
- **CTE isolation tests**: Test a single CTE by mocking all its direct dependencies
- **Whole-model tests**: Test entire model by mocking `smelt.ref()` inputs
- **Singular tests**: Custom SQL assertion tests (`materialization: test`, pass when 0 rows returned)
- **Property-based tests**: Omit columns from inputs → framework generates random values using type inference, runs N times (configurable via `test.cases`)
- **Column-level data quality tests**: `not_null`, `unique`, `accepted_values`, `min`, `max` defined in model frontmatter

### CLI
- `smelt test` with `--select`, `--verbose`, `--show-all`, `--seed` flags
- Tests excluded from `smelt run`/`build`/`explain`
- Example tests across ephemeral_demo, retail_analytics, timeseries projects

### Remaining work
- `smelt docs generate` for data catalog / data dictionary output
- Recursive CTE support in test isolation
- Snapshot/golden file mode (auto-capture expected output)
- LSP validation of test references (`test.model`, `test.target_cte`)
- Seed data integration with tests
- Statically-checkable assertions and type-system-leveraged testing (exploratory)

---

## Language & Parser

**Current state**: Full SQL parser with error recovery (Rowan CST), covering SELECT, FROM, JOIN (all types), WHERE, GROUP BY, HAVING, ORDER BY, LIMIT, CTEs, window functions, set operations, subqueries, QUALIFY, lambda expressions, array/struct/JSON literals, and all standard operators.

- smelt extensions: `smelt.ref()`, `smelt.metric()`, `smelt.source()` with `=>` named parameters
- Trailing commas in SELECT/GROUP BY
- YAML frontmatter for model configuration
- Python model support via `@model` decorator (subprocess + optional PyO3)
- Multi-dialect superset: PostgreSQL base with DuckDB and Spark features
- PIVOT/UNPIVOT: rejected with diagnostic error (not yet supported, March 31, 2026)
- Parser structural assertion tests and AST accessor bug fixes (April 3, 2026)
- Fixed bare-token problem and implicit alias detection (April 3, 2026)

**Next steps**:
- ~~Smelt Functions Steps 1–5~~ ✅ (April 22–24, 2026) — `smelt.define`, `smelt.fn.*`, `TableExpr`, call-site type checking, LSP hover, context binding, CTE-derived `SelectItems` contexts, `session_rollup` end-to-end, Tier 2/3 body/return checking, Tier 2 → Tier 1 inline expansion, bidirectional generics. See [plan](plans/20260422-smelt-functions.md) and [discussion paper](research/20260413-smelt-functions.md).
- ~~Smelt Functions Step 6 (PASSING clauses)~~ ✅ (April 24, 2026) — Phase 28 (parser: context-sensitive `PASSING name AS (...)` syntax) and Phase 29 (binding + type-checking) complete. `session_rollup` demonstrated with block-syntax `PASSING metrics AS (COUNT(*))` in `examples/functions_demo/`. `UnknownPassingParameter` diagnostic, LSP code mapping, and basic `body_expr()` / `name_range()` AST helpers added. LSP cursor-in-body column completion deferred (see Phase 29 deferral note in plan). Steps 7–8 (planner, struct row vars) remain. See [plan](plans/20260422-smelt-functions.md).
- ~~Smelt Functions Step 7 Phases 30–34~~ ✅ (April 25, 2026) — Phase 30: `smelt-planner::logical::LogicalNode::FunctionCall` with `transparent` flag and `FunctionProperties`; `logical_plan` Salsa query in `smelt-db`. Phase 31: column provenance declared via per-declaration frontmatter `provenance:` key, gated by `unstable_schema: true` in `smelt.yml` (`DiagnosticCode::UnstableSchemaRequired` fires when the flag is absent). Phase 32: `PlannerRule` trait, `RuleResult`, `RuleContext`, `apply_rules_to_fixed_point` fixed-point loop, and `ExpandTransparentFunctionCalls` rule. Phase 33: `PushFilterIntoTransparentFunction` rule. Phase 34: `EliminateUnusedLeftJoin` rule — elides a `LogicalNode::LeftJoin` whose RHS columns are unused in the parent projection list, when cardinality is declared `1:1`. Demo: `enriched_order` function (declares `joins:` with 1:1 cardinality against `dim_customer`) + `order_totals` model (projects no dimension columns → join eliminated). Soundness caveat documented in §20E: the rule trusts the declared cardinality without data verification. Step 7 complete.
- ~~Smelt Functions Step 8 Phases 35–38~~ ✅ (April 25, 2026) — Phase 35: `STRUCT_TYPE`, `ROW_TAIL`, `BRACE_STRUCT_LITERAL`, `SPREAD_ITEM` syntax kinds; `SmeltType::Struct { fields, tail }` + `StructRowTail` in smelt-types; two-named-row-var constraint check. Phase 36: call-site struct row-var unification (`check_struct_row_var_binding`), extras bound via `set_row_var_binding`, spread-item erasure for `..event` in bodies. Phase 37: return-type row-var resolution — `Expr<Struct<{hour: BigInt, ..r}>>` return resolves to a concrete `DataType::Struct` at call sites; `BraceStructLiteral` type inference in `infer_brace_struct_literal_type`; LSP hover shows expanded fields. Phase 38: `smelt.as_struct(<alias> [EXCEPT <cols>])` expression — `SMELT_AS_STRUCT_CALL` syntax node, `SmeltAsStructCall` AST wrapper, `infer_as_struct_type` resolving columns via `TypeContext::columns_for_qualifier`, `as_struct_to_sql` emitting DuckDB/Spark/Postgres backend SQL, `AsStructUnsupportedBackend` diagnostic for functions declaring unsupported backends. Step 8 and the full smelt-functions v1 experimentation roadmap are complete.
- ~~Smelt Functions Steps 9–13 (plan review + polish) Phases 39–53~~ ✅ (April 26, 2026) — 14 phases closing the 28 review findings from the plan's §20 audit. Key deliverables: Phase 39 (`--show-plan` CLI flag wiring logical-plan rule pipeline end-to-end), Phase 40 (CAST emission resolved from canonical-return registry), Phase 41 (transparent-call body splice into logical plan), Phase 42 (list-splice comma elision at lowering), Phase 43 (`as_struct` backend SQL emission), Phase 44 (canonical fixture tightening: `safe_divide` body guards, `monitored_session_rollup`), Phase 44b (fragment-forward parser + `SelectItems<K, ctx>` type system), Phase 45 (JOIN alias visibility in `TableExpr`-returning bodies), Phase 46 (`TableExpr` argument shapes: CTEs, derived tables, subqueries), Phase 47 (cross-function CTE schema inference, drop opaque-CTE suppression), Phase 48 (LSP hover wiring, `PASSING` completion, multi-level frame trace), Phase 49 (`WindowInScalarContext` deep-walk into scalar subqueries), Phase 50 (built-in registry expansion: arithmetic operators, missing aggregates, window functions), Phase 51 (`provenance:` / `joins:` validator with `ProvenanceMismatch`, `JoinsMismatch`, `DeclaredCardinalityUnverifiable` diagnostics), Phase 52 (missing-provenance pushdown advisory `Hint` + extern fragment-param rejection `ExternFragmentParamUnsupported`), Phase 53 (plan audit: empty commit-SHA cells filled, stale `Context` comment corrected, cross-file extern same-name collision fixture). See [plan](plans/20260422-smelt-functions.md).
- ~~Smelt Functions Phase 55 — `smelt.as_struct()` and `smelt.fn.*` SQL emission during `smelt build`~~ ✅ (April 27, 2026) — Wired both `smelt.as_struct()` and `smelt.fn.*` into actual SQL emission in the dialect printer. Added `AsStructEmitter` and `SmeltFnExpander` closure type aliases as optional fields on `PrintContext`; the printer's `SMELT_AS_STRUCT_CALL` and `SMELT_FN_CALL` handlers invoke them when present. In `SqlCompiler::compile()`: builds a `TypeContext` from the original SQL before ref-resolution so alias→columns mappings are available, constructs both closures from upstream schemas and function body maps, and passes them into `PrintContext`. Added `set_function_bodies()` on `SqlCompiler` for tests. `substitute_params()` does whole-word parameter substitution skipping string literals. 5 new tests in `crates/smelt-cli/tests/as_struct_emission_tests.rs` cover DuckDB struct literal emission, EXCEPT exclusion, function body expansion, pass-through when no body map, and TypeContext alias building. All tests pass, zero clippy warnings.
- Metrics DSL (Layer 1 — declarative metric definitions, `smelt.metric()` resolution)
- `smelt.param()` for parameterized models
- PIVOT/UNPIVOT support (currently rejected with diagnostic)

## Type System

**Current state**: Full type inference for expressions, functions, aggregates, window functions, and cross-model schemas. NULL tracking, row polymorphism (`SELECT *` propagation), and `resolved_model_schema()` Salsa query.

- Property-based testing against DuckDB and Spark (via `smelt-parser-compat`)
- Comprehensive generator coverage (March 29, 2026): 12 expression kinds (IS NULL, comparisons, unary NOT/minus, EXISTS, LIKE/ILIKE, regex, scalar subqueries, mixed-type binary ops, `::` cast), 5 query shapes (Scalar, GroupBy, GroupByHaving, GroupByWindow, Distinct), 10 base types (incl. Time, Interval), window frame specs
- LIKE/ILIKE parser support with type inference
- Known divergence registry for backend-specific type differences
- JSON operator type inference

**Next steps**:
- ~~LSP quick-fixes for type errors (CAST suggestions)~~ ✅ (April 5, 2026) — see [LSP Refactorings](#lsp-refactorings--code-actions--april-5-6-2026)
- LSP quick-fixes for COALESCE suggestions on NULLs
- Stricter boundary type checking (explicit input/output schemas)
- *See also*: snapshot tests for type inference output ([Code Quality & Hardening](#code-quality--hardening)), type-system-leveraged data testing ([Data Testing Framework](#data-testing-framework))

## Planner

**Current state**: `smelt-planner` crate with model-graph-level planning:

- Cube split: splits multi-`COUNT(DISTINCT)` queries into parallel sub-queries
- Incremental materialization: detects time-partitioned GROUP BY, generates DELETE+INSERT
- Temporal dependency inference: analyzes window functions, LAG/LEAD, JOIN intervals to determine lookback/lookahead requirements
- Batch safety analysis: classifies models as FullyBatchSafe/BoundedSafe/PerPartitionOnly
- DAG-aware range computation for backfill planning

**Deferred**:
- ⏸️ Per-ref upstream filtering — wrapping `smelt.ref()` in filtered subqueries requires column lineage tracing through query AST; currently applies single wider filter range
- ⏸️ Custom time granularities — plugin API for fiscal quarters, 4-4-5 retail calendars; placeholder `Custom` variant exists
- ⏸️ Rule conflict resolution — how planner rules compose when they conflict (e.g., shared sub-expression vs incremental on same model); currently last-transformation-wins

**Next steps**:
- Three-level rule architecture: (1) Logical→Logical transforms with functions as opaque typed nodes, (2) Logical→Physical with strategy-dependent function expansion, (3) Physical→Execution plan with multi-statement orchestration. See [smelt functions discussion paper](research/20260413-smelt-functions.md) §8.
- Function-aware optimizations: join elimination for unused 1:1 LEFT JOINs, predicate pushdown into function blocks, cross-function fusion
- Shared materialization detection (multiple models computing same intermediate)
- Model fusion (trivial passthrough models)
- Cost-based optimization (requires backend statistics)
- Orchestrator integration — Dagster/Airflow plugin API (deferred to separate plan)

## Backends

**Current state**:
- **DuckDB**: Full implementation — table/view materialization, incremental DELETE+INSERT, bundled (no system install needed)
- **Spark**: Full implementation via PySpark/PyO3 bridge (March 28, 2026) — all Backend trait methods implemented, zero-copy Arrow conversion, works with Spark Connect and Databricks Connect. Requires PySpark in Python environment.
- **PostgreSQL**: Not started. Deprioritized in favor of Spark/Databricks.
- **Dialect printer**: `smelt-dialect` crate — single-pass CST walk emitting target SQL, handles QUALIFY, array literals, DATE literals, JSON function remapping

**Deferred**:
- ⏸️ Spark JSON incompatibilities — `TO_JSON(scalar)`, `JSON_CONTAINS`/`@>`/`<@`, `JSON_OBJECT`/`JSON_ARRAY` rewrites; compile-time warnings planned but not yet implemented

**Next steps**:
- ~~Spark/Databricks backend implementation~~ ✅ (March 28, 2026) — see [What's Next #1](#1-spark--databricks-backend)
- ~~Multi-backend execution in a single run~~ ✅ (March 25, 2026) — `BackendRegistry` with per-model `target:` frontmatter override, cross-backend validation
- ~~Cross-engine data exchange~~ ✅ (March 29, 2026) — cross-engine ref resolution via direct Parquet reads (no copy step); DuckDB resolves `smelt.ref('spark_model')` to `read_parquet('{warehouse}/{schema}/{model}/**/*.parquet')`. Example at `examples/multi_engine/`. See [plan](plans/20260328-multi-engine-example.md).
- Integration test parity: run DuckDB integration tests against local Spark Connect
- *Deferred*: PostgreSQL backend

## LSP & Editor Support

**Current state**: Full LSP server (`smelt-lsp`) with Salsa incremental compilation:

- Diagnostics: parse errors, undefined refs, type errors, undeclared column references (with accurate positions)
- Go-to-definition for `smelt.ref()`, `smelt.source()`, CTEs, columns, and qualified references (e.g., `t.column`)
- CTE wildcard tracing (`SELECT *` through CTE chains)
- Hover with type information and model schemas
- Completions: model names, column names, table alias columns
- Python model awareness: real `ProjectContext` passed to Python models in LSP, valid ref targets, execution error diagnostics
- `sources.yml` live reload (changes update LSP without restart)
- Salsa 0.26 with `#[salsa::tracked]` free functions and `cycle_initial` fixpoint iteration (upgraded from 0.16)
- Find references for models, sources, and CTEs
- Rename: CTEs (single-file), models (cross-file with file rename), sources (cross-file + YAML), columns (full lineage tracing)
- Code actions: CAST quick-fixes, create model, add source/column to YAML, extract CTE, inline CTE
- VSCode extension with syntax highlighting and auto-activation
- CI verification: example workspaces checked for zero LSP diagnostics

**Recent** (April 3-4, 2026):
- ✅ Expanded goto-definition to sources, CTEs, columns, and qualified references
- ✅ CTE wildcard tracing for `SELECT *` column resolution
- ✅ Diagnostics for undeclared column references
- ✅ Python model LSP integration: real `ProjectContext` enables cross-boundary type inference
- ✅ Fixed LSP crash from Salsa cycle detection during memo validation
- ✅ Upgraded Salsa 0.16 → 0.26: `#[salsa::tracked]` free functions, `#[salsa::input]` structs, `#[salsa::accumulator]` diagnostics, `cycle_initial` fixpoint iteration; removed `catch_unwind` workaround (April 18, 2026)
- ✅ Fixed `sources.yml` changes not updating LSP until reload
- ✅ Fixed 35 LSP diagnostics across example workspaces + CI verification gate
- ✅ Fixed Python model `E2BIG` error on large projects and PyO3 `dict_items` extraction

**Next steps**:
- Dialect-specific informational hints ("QUALIFY will be rewritten for PostgreSQL")
- Optimizer opportunity suggestions as code actions
- Code action: extract to model (promote subquery/CTE to a new smelt model)

## CLI & Execution

**Current state**: `smelt-cli` with full pipeline:

- `smelt run` — execute models with optional `--start`/`--end` for incremental ranges, `--dry-run`, `--full-refresh`, `--auto` (range from interval store)
- `smelt backbuild` — target-focused rebuild with DAG-aware range expansion
- `smelt explain` — dependency graph + JSON export
- `smelt status` — interval coverage and gaps for incremental models
- `smelt history` — run history with model filtering
- `smelt test` — data testing framework (CTE isolation, whole-model, singular, property-based, column-level tests)
- `smelt type` — function type signatures
- `smelt docs generate` — static data catalog (Markdown/JSON) with column types, lineage, descriptions, tests (March 29, 2026)
- `smelt diff` — offline schema change detection, compares inferred vs deployed schemas without database connection (March 29, 2026)
- Smart batching based on batch safety analysis
- `smelt-state` crate for run manifests + interval tracking (`.smelt/` directory)
- Two-stage graph architecture: `LogicalGraph` (user intent) → `PhysicalGraph` (execution plan)
  - `LogicalGraph` with eagerly-resolved config per node (March 26, 2026)
  - `PhysicalGraph` with strategy resolution, ephemeral resolver ownership (March 26, 2026)
  - Graph-level planner transformations: `CreateNode`, `RemoveNode`, `RedirectRef`, `SetMaterialization` (March 26, 2026)
  - `smelt explain` shows physical execution plan with strategies, ephemerals, planner optimizations (March 26, 2026)

**Next steps**:
- ~~`smelt test`~~ ✅ (March 27, 2026) — see [Data Testing Framework](#data-testing-framework)
- ~~`smelt docs generate`~~ ✅ (March 29, 2026) — see [What's Next](#1-data-catalog--smelt-docs-generate)
- ~~`smelt diff`~~ ✅ (March 29, 2026) — see [What's Next](#1-schema-diff--smelt-diff)
- `smelt check` — LLM-optimised diagnostic CLI ([design doc](plans/20260405-smelt-check.md))
- ~~Schema evolution with efficient migrations~~ ✅ (March 29, 2026) — see [What's Next](#1-schema-evolution)

## UI Dashboard ✅ Phases 1-4 (March 24-25, 2026)

**Current state**: Web dashboard (`smelt-ui`) with React frontend and Axum backend:

- Phase 1: Live backend with file watching and WebSocket updates
- Phase 2: Full REST API, batch safety diagnostics, type information in UI
- Phase 3: Run planner with interactive preview, select/exclude with CLI command preview
- Phase 4: Run execution and monitoring with real-time WebSocket progress streaming
- Model graph visualization with dependency explorer
- Run history with expandable model details
- Model sidebar with type signatures and metadata

**Next steps**:
- See [docs/plans/20260324-ui-dashboard-expansion.md](plans/20260324-ui-dashboard-expansion.md) for Phases 5-6

## Ecosystem

**Recent** (March 25 – April 4, 2026):
- ✅ Documentation site for smeltsql.com (MkDocs Material, 15+ pages covering all features)
- ✅ Frontmatter validation with `deny_unknown_fields` (catches typos like `materialized:` vs `materialization:`)
- ✅ Multi-model file discovery with `ModelId` (`--- name: model_name ---` delimiters)
- ✅ Testing documentation: guide, CLI reference, and project structure docs
- ✅ ACE-FCA workflow: slash commands, tutorial, and artifact directories for structured development (March 31, 2026)
- ✅ SQL dialect analysis report: confirmed multi-dialect superset approach is sound (March 30-31, 2026)
- ✅ System DuckDB as default build mode — faster builds, no bundled C++ compilation (April 3, 2026)
- ✅ CI verification: example workspaces checked for zero LSP diagnostics (April 3, 2026)
- ✅ CI release builds fixed for bundled-duckdb feature (April 4, 2026)

- ✅ smelt-datagen bundled in `smelt-sql` PyPI wheel and standalone archives (April 9, 2026)
- ✅ smelt-datagen documentation: guide page on smeltsql.com covering all features (April 9, 2026)
- ✅ New datagen generators: `date`, `timestamp`, and `string_pattern` for realistic test data (April 9, 2026)

**Next steps**:
- Pre-built binaries via GitHub Releases (dev-release.yml workflow exists)
- Source distribution (sdist): verify the `maturin sdist` job actually ships on release — `release.yml` currently builds the four platform wheels but has no sdist job, despite the April 10 packaging work claiming one. (Wheels themselves are resolved: `bindings = "bin"` produces `py3-none` wheels covering Python 3.9–3.14 on all four platforms, so there is no remaining cp314-specific gap.)
- Datagen: geometric distribution `min` parameter (currently can produce 0, unsuitable for quantity fields)
- dbt-to-smelt cheat sheet showing common pattern equivalents
- Publish Python SDK to PyPI (currently TestPyPI only)
- Generic LSP configuration guides for Neovim, Emacs, and JetBrains
- Community readiness: `CODE_OF_CONDUCT`, issue/PR templates, and a "good first issue" onboarding path (repeatedly flagged across codebase reviews, no roadmap presence until now)

## Deferred-Work Backlog (untracked follow-ups)

Concrete work deferred during plan implementation (`docs/plans/`) that is not otherwise tracked above. Listed so it is *visible* and can be triaged into the queue — inclusion here is identification, not commitment. Maintained as part of [What's Next #1](#1-silent-failures--code-health-hardening). Grouped by area; each item cites its source plan.

**Parser / dialect conformance** (`20260711-parser-type-testing-hardening.md`)
- Grammar support for the remaining registered DuckDB gaps, sized by the differential-gate ratchet and the external-corpus ledger (~500 entries): `LIKE ANY` (currently fail loud). Dollar-quoted strings (`$$…$$` and `$tag$…$tag$`) are now lexed as ordinary string literals (parse/print/infer Text) — closed July 12, 2026. Underscore digit-separator numeric literals are now accepted syntax; hex-integer literals (`0x1F`) remain fail-loud, matching DuckDB's own lack of hex-literal grammar in this position. `GLOB` is now supported (parse/print/infer Boolean; `NOT GLOB` is not, matching DuckDB's own rejection of that form). SQL-standard function forms (`trim(BOTH…FROM…)`, `substring(FROM FOR)`, `position(IN)`) are now supported (parse/print/infer, reusing the existing TRIM/SUBSTRING/POSITION function-call typing) — closed July 12, 2026. `MAP {key: value, …}` literals are now supported (parse/print/infer `Map(key_type, value_type)`) — closed July 12, 2026. List comprehensions (`[expr FOR x IN list (IF cond)?]`) are now supported (parse/print/infer `Array<T>`, with the element type read off the source list when the element is exactly the loop variable and classified `Unknown` otherwise) — closed July 12, 2026. This closes the seed-corpus registered gaps down to the single parse-level `POSITION` divergence noted in `docs/specs/architecture.md` Known Divergences.
- Spark-side differential parsing beyond the existing sqlparser-rs checks — needs the gated Spark server; the DuckDB harness shape applies as-is.
- ~~Nullability-oracle generator extension — mirror the type-side generator widening (temporal/decimal/tz/function coverage) against the value-based nullability oracle now that the type-side pattern has settled.~~ ✅ (July 12, 2026) — the property tests already drove the shared, widened `test_scenario_strategy()`/`generate_expr` generators (single source with `type_property_tests.rs`); added reachability smoke tests mirroring the type-side pattern to prove coverage, and fixed a `null_data.rs` real-table-builder gap where `with_timezone: true` timestamp sources were silently created as plain `TIMESTAMP` (losing the tz-aware storage semantics the tz-mixing column-pool weight is meant to exercise). `PROPTEST_CASES=1000`–`5000` runs surfaced no soundness violations in the widened space.

**Type system / function registry**
- ~~Dialect-alias resolution (`NVL`, `GET_JSON_OBJECT`, `JSON_BUILD_OBJECT`, …) lives in `SqlFunction::from_name`, not `BuiltinRegistry`~~ ✅ (July 12, 2026) — moved into `BuiltinRegistry`: each canonical `Signature` now carries an `aliases: &'static [&'static str]` table, and `SqlFunction::from_name` resolves every name (canonical or alias) through `BuiltinRegistry::canonical_name`. New `registry_consistency::every_alias_is_registry_backed` gate asserts every registered alias is recognised, resolves to the right canonical function, and classifies consistently. See `architecture.md` §Constraints #14.
- 30 functions remain on the legacy hand-written inference match (named exception list under the shrink-only `registry-migration` ratchet) because their return types are argument-dependent (widening, first-concrete-of-N, tz-mirroring); shrinking further needs a richer signature language (`20260711-parser-type-testing-hardening.md`).
- `to_seconds` returns `Interval` but inference doesn't recognise it — forces `epoch_us()` microsecond-arithmetic workarounds in models (`20260517-web-analytics-4-sessionize.md`).
- `md5` absent from the function registry — emits a Warning that fails the diagnostics gate; models fall back to `CONCAT` surrogate keys (`20260517-web-analytics-4-sessionize.md`).
- `arg_min` deliberately unregistered (only `arg_max` shipped), pending a real call site (`20260517-web-analytics-5-forward-only.md`).
- `ArgTypeMismatch` message format diverges from `functions.md` §8 ("expected X, got Y") (`20260422-smelt-functions.md`).
- `infer_expression_kind` parallel-walker gap — sub-expression kinds nested in array/struct/`ROW(...)`/`IN`/`EXISTS` silently drop to `Scalar` (overlaps the silent-`Unknown` front in #1) (`20260422-smelt-functions.md`).
- Per-arg-type canonical-return resolution — decimal precision is encoded per-signature today, not per-arg-type (`20260422-smelt-functions.md`).

**Functions / TableExpr**
- `f(x).field` single-field projection + `..r` row-tail struct-spread descoped — no field-postfix on a function call; schema/codegen disagree on row-tail spread (`20260527-function_schema_inference.md`).
- ~~`build_fn_body_map` vs `build_fn_body_map_from_model_files` duplication~~ ✅ (2026-06-10, hardening Front 4).
- Phase 57 deferred function tests: FROM-position aliasing, Spark struct-literal lowering, literal-`VALUES` models (`20260519-functions-meta-gaps.md`).

**Meta-language**
- `smelt.sources.with_tag |> map(...)` generator driver yields zero emissions at runtime — only `load_yaml`/`load_json` drivers are enumerated; `staging_from_sources` ships a hardcoded `[ModelDef …]` workaround (`20260509-meta-language-E2.md`).
- `emitted_models` does an O(workspace) re-scan on any single-file edit (Salsa input granularity) (`20260509-meta-language-E2.md`).
- Unshipped `List<T>` derivations (`flat_map`, `zip_with`, `take`, `drop`, `any`, `all`, `find`, `partition`, …) + user-defined reducers; ship only if examples force them (`20260509-meta-language-overall.md`).
- `smelt.config.var(...)` rejected inside a `smelt.define` body — compile-time config not resolvable in define scope (BUG-067; sweep needs-review, `2026-05-30` ledger).
- `List<T>` not admissible as a `smelt.define` parameter type — typed list parameters can't be declared (BUG-068; sweep needs-review, `2026-05-30` ledger).

**VALUES / typing**
- `LATERAL (VALUES …)`, bare top-level `VALUES`, and `INSERT … VALUES` are untyped (`20260528-values-derived-table-typing.md`).

**Diagnostics encoding**
- `body_position_to_byte` counts codepoints, not UTF-8 bytes — non-ASCII emission bodies get shifted diagnostic positions (one-line fix pending a fixture) (`20260529-emission-body-diagnostics.md`).
- `smelt-ui` `DiagnosticInfo` still uses legacy `offset_to_position` — must move to `LineIndex` before that function is deleted (`20260530-byte-offset-diagnostics.md`).

**CLI ergonomics**
- No project-wide compile-only flag — extend `--show-plan` to the whole project vs a `smelt build --dry-run` is undecided (`20260502-smelt-loop-findings.md`).
- `smelt build --verbose` produces no extra output despite advertising compiled-SQL display (`20260502-smelt-loop-findings.md`).
- `smelt build --show-plan <generator>.sql` prints only the top-level loader node, not the emitted `ModelDef` paths (`20260509-meta-language-G.md`).
- `smelt test` silently skips `materialization: test` files with a boolean-`SELECT` body — no "discovered but skipped" diagnostic (`20260509-meta-language-G.md`).
- `smelt seed` command fate undecided (repurpose as `list`/`describe` vs remove; seed type-inference + caching open) (`20260406-seed-schema.md`).

**Incremental eligibility — monotonicity primitive follow-ons** (`20260702-monotonicity-primitive-tested.md`)
- **Consumers of the trace, unwired today** — `UNION`-branch partitionability (§2.5/E1), subquery/CTE pushdown conservatism (§4.6/B4/E2), and join driving-fact resolution (§5.4). Each is a separate gated plan; the primitive's output type is designed for all three.
- **Tree-annotation injection redesign** — replace the *textual* `inject_time_filter` / `inject_source_filters` (`transformer.rs:65`,`:272`) with logical/physical-tree annotation consumed by `smelt-planner`'s `plan_printer.rs`. The trace names a *semantic* `(source, source_column, offset)` target; consumers annotate the tree and the printer emits SQL — no consumer ever computes source-text edits (research §6.7, decision 2026-07-02).
- **Retain-parsed-AST cleanup sweep** — Phase 0 retains the parsed `Expr` on `analyze_select` items; other analyses still re-scan raw text and should retain what's parsed instead: `analysis/mod.rs` clause string-scanning, `source_bounds.rs` textual `INTERVAL`/`RANGE BETWEEN` recognition, `rules/incremental.rs` `Frontmatter::strip`+re-scan, and the `temporal.rs` re-parse sites.

**smelt-logical / smelt-planner extraction**
- ✅ **`analysis/{mod,source_bounds,temporal}.rs`, `logical.rs`, `types.rs` consolidated** (2026-07-19). `smelt-planner`'s local copies were already thin `pub use smelt_logical::…` shims; `lib.rs` now re-exports the `smelt-logical` modules directly (`pub use smelt_logical::{analysis, logical, types};`) and the shim files are deleted. No behaviour change — `cargo tree -p smelt-db -i smelt-planner` still shows no production path.
- **Remaining duplication.** `smelt-planner/src/` still carries a parallel copy of `rules/{incremental,cumulative,rule_diagnostics,cube_split}.rs`, `graph.rs`, `lowering/as_struct.rs` from the recent (incomplete) extraction into `smelt-logical`. Finish the extraction so each analysis lives once (in `smelt-logical`, consumed by both `smelt-db` and `smelt-planner`), leaving `smelt-planner` only its planner-only pieces (`logical_plan_rules.rs`, `plan_printer.rs`, `python_bridge.rs`). Prerequisite context for where any future type-aware analysis moves.

**Datagen / incremental**
- Foreign-key resolution inside `JsonObject`/`entity.columns` always resolves to id 1 (`20260517-web-analytics-1-datagen-json-object.md`).
- Cumulative reprocessing detection has no watermark store — refuses via a heuristic pending a state-tracking spec (`20260523-cumulative-aggregate.md`).
- Backbuild over a cumulative model takes the legacy full-refresh path instead of the merge loop (BUG-070; sweep needs-review, `2026-05-30` ledger).

**Sources** (from the `2026-05-30` bug ledger — sweep needs-review)
- Source `timeseries:` is silently ignored — `SourceInfo` carries no `timeseries` field, so a declared source time-window has no effect (BUG-072).
- `inject_source_filters` is not wired into the incremental execute path — source filters don't constrain incremental reads (BUG-073).

**Cross-engine**
- Partition-level Parquet reads (only changed partitions vs full glob) + cross-engine schema validation / type coercion (Spark `STRING` vs DuckDB `VARCHAR`) (`20260328-multi-engine-example.md`).

**Documentation gaps** (from the `2026-05-30` bug ledger)
- `test` materialization mode missing from `materializations.md` (BUG-022).
- Calendar-invalid seed date (`2025-02-30`) infers DATE then hard-fails at load — undocumented interaction; note in `seeds.md` (BUG-031).
- ~70 diagnostic codes undocumented — needs a `diagnostics.md` (BUG-052).
- `incremental_models.md` Known Divergences omits the window-function safety check as a third non-expanding classify site (BUG-062).
- `cumulative_aggregate.md` Known Divergences omits the Month/Quarter/Year grain limitation (BUG-071).
- `smelt docs` follow-ons: HTML output, `smelt docs serve`, column-lineage visualization, `smelt docs diff` (`20260329-docs-generate.md`).

**CI / Performance**
- ~~Cold-Salsa full-diagnostics regression (2000-model benchmark)~~ — **resolved 2026-07-11** by `bf881006` ("fix(db): cache smelt.yml parse for maintenance/state queries, fix O(N) CI bench regression"): `maintenance_plan`/`maintenance_plan_report`/the `state.mode` widening check were re-parsing `smelt.yml`'s full YAML text on every per-file Salsa query instead of a cached tracked query, an O(N) `serde_yaml` cost across the 2000-model workspace. Fixed via `project_maintenance_config`/`project_state_mode` tracked queries. Confirmed via `docs/plans/20260718-quality-grind-t2.md` Phase 8 (2026-07-19): `initial_load_ms`/`full_diagnostics_ms` are ~395ms/~334ms, ~25x under the 10s ceiling. This entry was previously stale (written the same day the regression appeared, before the same-day fix landed hours later).

---

## Future / Exploration

Items here are interesting design problems without committed timelines.

- **External models in the graph**: Non-smelt models (e.g., PySpark jobs, legacy pipelines) as first-class DAG participants. User-annotated output schema and temporal behavior (partition column, granularity). Configurable execution: smelt-triggered (command/webhook) or externally-managed. Enables gradual migration and mixed-technology pipelines. Smelt's backbuild range computation would account for these models' declared temporal mappings. Declaration format needs design work.
- **Virtual environments / plan-apply workflow**: Compare schemas across dev/prod without materializing; require approval before execution. Interesting state management problem — smelt's logical/physical graph split could enable lightweight virtual environments.
- **OpenLineage / column-level lineage**: Export model and column-level lineage in OpenLineage format for catalog integration (DataHub, Amundsen, Atlan). Internal lineage tracking partially exists — interesting graph analysis problem.
- **Substrait integration**: Portable plan representation, DataFusion interop
- **Smelt Functions — next frontiers**: Steps 1–13 are ✅ complete (April 2026). Remaining open design problems: generics in `smelt.define` (user-polymorphic functions, §16 #14 deferred), variadics in `smelt.define` (§16 #15), parameterized models (`smelt.param()`), metrics DSL integration (`smelt.metric()`), and full function-body SQL lowering (replacing `LogicalNode::Raw` placeholders with structured plan nodes for end-to-end `smelt build` code generation from function bodies). See [plan](plans/20260422-smelt-functions.md) and [discussion paper](research/20260413-smelt-functions.md).
- **Python as meta-layer input** (not whole models): Today Python authors entire models via the `@model` decorator. A lighter mode would let a Python script *feed the meta layer* — emit values (lists, maps, records) that the typed meta-language consumes to generate models — rather than producing model SQL itself. This keeps generation logic in smelt's typed meta-language while using Python only as a data/config source (e.g. pulling a cohort list or schema map from an external system). Interface and the typed boundary between Python output and meta-language input need design work; relationship to existing YAML/JSON/TOML loaders (`smelt.config.load_*`) should be worked out so they share one ingestion model.
- **Learning from history**: Use run statistics to suggest optimizations
