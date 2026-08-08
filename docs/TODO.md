# TODO

## `ColumnScopedMerge` reachability gap on membership-sensitive `grain: partition` cells (2026-08-08, corrected same day)

`docs/plans/20260808-membership-sensitivity.md` Phase 3 surfaced (confirmed empirically while
rewriting `crates/smelt-cli/tests/{bakeoff,bakeoff_seam,maintenance_pins,explain_model,
explain_show_sql}.rs`) two things about Phase 1's membership-sensitivity derivation — only the
first of which is a genuine, still-open gap; the second was a real bug in this repo's OWN pin-
resolution code, found by review and fixed the same day (see item #2's correction below):

1. **`Technique::ColumnScopedMerge` is unreachable from any currently-shipped SQL shape.** Any
   `JOIN`'s `ON` predicate (inner or left) reading a `MutableSnapshot` source makes EVERY column
   group of that `SELECT` membership-sensitive (`Technique::DeleteInsert`), not only the columns
   the dimension itself contributes — membership sensitivity is row-scoped, not per-column
   (`membership_sensitivity_sources`, `crates/smelt-logical/src/maintenance/grouping.rs`). There is
   no currently-shipped shape where a mutable, row-admission-joined dimension is ALSO read in a
   select item with *only* value sensitivity toward the same source (Phase 2's own note). Knock-on
   effect: `smelt bakeoff`'s measured/`--pin` code path (`run_bakeoff`'s branch past the
   `candidates.is_empty()` early return, `crates/smelt-cli/src/bakeoff.rs`) has **zero reachable
   test coverage** anywhere in the crate — `admitted_family` maps `Technique::DeleteInsert` to
   `None`.
2. **A `grain: partition` model's `DeleteInsert` membership cell has no live runtime DISPATCH at
   all** (`resolve_live_membership_recompute_cell`'s own doc comment,
   `crates/smelt-runtime/src/maintenance_driver.rs`: left to the plain unconditional region
   `DELETE`+`INSERT` batch loop). This is still true and is NOT a bug — it is the documented,
   correct posture for this shape (no key-addressable staged-candidate write exists for a
   `WholeRow`-identity output). **It is not, however, why a pin/override was ever silently
   ignored** — an earlier pass of this entry conflated the two. A `cells[].technique`/`prefer`
   pin and a request-scope `technique_overrides` entry are validated by `resolve_live_column_
   scoped_cell`'s OWN pin-consulting loop (called unconditionally by the `grain: partition` batch
   loop while looking for a live `ColumnScopedMerge` opportunity, entirely independent of whether
   `resolve_live_membership_recompute_cell`'s dispatch is reachable) — an inadmissible pin refuses
   loudly there via `?`, before the "not ColumnScopedMerge, discard" branch is ever reached. The
   REAL bug (found by review, fixed same day): `Trigger::UpstreamMutation(users)` derives TWO
   sibling cells (`{user_name}` and `{event_id, event_type, user_id}` — membership sensitivity is
   row-scoped, so a shared join admits a cell per column group, not one cell per trigger), and
   `MaintenancePlan::cell_for`'s first-match lookup meant the pin-consulting loop only ever
   evaluated an override against whichever sibling happened to be derived first — a pin scoped to
   the OTHER sibling's columns was silently never matched. Fixed via `MaintenancePlan::cells_for`
   (`crates/smelt-logical/src/maintenance/mod.rs`) — every sibling cell sharing a trigger is now
   offered the override, matched against its own columns
   (`crates/smelt-runtime/src/maintenance_driver.rs`); a hard `technique:` pin naming columns that
   address NONE of a trigger's sibling groups now refuses loudly too
   (`smelt_logical::maintenance::choice::unaddressed_technique_pin`) rather than silently vanishing.
   Loud refusal is restored for both `maintenance_pins.rs::inadmissible_pin_fails_loud` and
   `bakeoff_seam.rs::request_override_subject_to_admission`/
   `request_override_forces_each_admissible_technique`.

Item #1 is a deliberate-shape consequence of Phase 1's derivation swap, not a bug — nothing in
Phases 1-3's critical-file scope was positioned to change the reachability of `ColumnScopedMerge`
itself (that needs a genuinely new SQL shape or a relaxed derivation rule — arguably the "Outer-join
membership semantics"/"Monotone-join admission relaxation" deferred items in that plan's Scope
section). Tracked here rather than silently accepted. Candidate follow-up: extend
`docs/plans/20260808-membership-sensitivity.md`'s successor work (or a new plan) to reassess whether
`ColumnScopedMerge`'s bakeoff/pin machinery should be retired as dead code now that its only
reachable shape is gone, rather than kept around with zero test coverage.

**NULL-keyed row caveat (advisory, found in Phase 3 review).** `emit_staged_candidate_conditional_
recompute`'s departed-key `DELETE` (`crates/smelt-logical/src/maintenance/emit.rs`) joins stored
rows to staged-candidate rows on plain `=` key equality — SQL's `NULL = NULL` is never true, so a
row whose key is (or contains) `NULL` is treated as absent from the staged candidate on EVERY run,
even when it is still genuinely present: it is deleted and immediately reinserted every run rather
than left alone. End-state equivalence with the full-refresh oracle still holds (the row's values
are correct either way), but the change-suppression contract ("nothing changed → nothing written")
silently does not hold for that one row. A NULL-safe key join (mirroring `key_expr_for_columns`'s
`COALESCE`-based pattern, `crates/smelt-logical/src/maintenance/emit.rs` lines ~1093-1135) would
close this; not fixed here — `RowIdentity::Key` is not documented anywhere as excluding a nullable
column, so this is a real, if narrow, gap worth a follow-up.

## `maintenance::grouping`'s column-ref collector — RESOLVED (2026-08-08)

Resolved by `docs/plans/20260808-membership-sensitivity.md` (all phases): `grouping.rs` now uses
the gated `collect_column_refs`; `collect_column_refs_ungated` is deleted; membership
sensitivity is derived as its own kind (join-`ON` + `WHERE`/`HAVING` admission reads of mutable
sources; subqueries fail closed); membership cells take suppressed delete+insert recompute with
a departed-key `DELETE`; the conformance gate now exercises genuine dimension mutations
(add/change/delete) against the full-refresh oracle; pin/override resolution consults every
sibling cell per trigger (dangling hard pins refuse loudly).

## Mutation-campaign residue (2026-08-08)

From `docs/research/20260808-mutation-testing-maintenance-gates.md` (472-mutant campaign over
`smelt-logical/src/maintenance/`; 13 final survivors, all classified). The genuine untested-logic
residue:

- [ ] **`choice.rs:235` liveness arm** — nothing drives `resolve_cell_choice` with
  `backend_supports_column_scoped_merge=false` on a ColumnScopedMerge-admitted cell; deleting the
  liveness-filter arm survives every gate. Add a pure test asserting the fallback when the backend
  lacks MERGE.
- [ ] **`derive.rs:182`** — `||`→`&&` in `source_contributes_to_fold` survives; find the input
  class that distinguishes the disjunction and pin it.
- [ ] **`derive.rs:1279` `group_columns`** — returning an empty/garbage set survives; no test
  observes the grouped-column set directly.
- [ ] **`granularity.rs:68`** — the `alias == partition_column` match guard forced to `true`
  survives `check_declared_granularity`'s tests.
- [ ] **`derive.rs:240`** — provably equivalent match guard (`aliases.len() == 1`); delete the
  guard (cleanup, needs no test) so it stops registering as a survivor.
- [ ] **Label/refusal text** — `trigger_label`, `resolvable_set_label`, `LocalityRefusal::fmt`
  are unpinned. Decide: golden-pin refusal text (fail-loud culture) or accept as advisory.
- [ ] **Re-run after F3** — `model_fingerprint_projections` mutants become killable once
  fingerprint sidecars consume the projection; re-run the campaign then
  (`cargo mutants --iterate` makes this incremental).
- [ ] **Conformance-generator extensions** suggested by the campaign's blind spots: recipes with
  `cells[].write` pins, ColumnAdded triggers with `allow_full_scan: true`, December/era-boundary
  date pools, backends without column-scoped MERGE.
- [x] **walk.rs campaign residue, triaged (2026-08-08)** — the second `analysis/walk.rs`
  campaign's 40-survivor list (see `docs/research/20260808-mutation-testing-maintenance-gates.md`
  §"Bonus campaign addendum") is fully triaged. 21 killed by
  `crates/smelt-logical/tests/walk_hardening.rs` (the `has_unsupported` fail-closed spine,
  `INTERSECT`/`EXCEPT` recognition, `is_constant_literal`, the union discriminator, ambiguous-alias
  guards in `select_lineage`/`resolve_alias_source`, `path_display`, `has_subset_key`, and the
  operator-level property/admission folds). 5 are provably equivalent (every `Transfer::leaf`
  impl + `Grain::unkeyed` literally return their type's `Default`; no test can distinguish the
  mutant). New kill rate: 146/163 viable (89.6%), up from 76.7%.
- [ ] **walk.rs `own_region_text*` collector-guard residue (14 mutants, deferred)** — the
  `own_region_text`/`own_region_text_excluding_self_relations` `node==root`/`TABLE_REF` guards
  (13 mutants) and `scope_self_qualifiers`'s `last != key` guard (1 mutant) survived the Phase 1
  triage session; see the research doc addendum's "Deferred, with reason" section for why a
  discriminating test needs a scenario where duplicated/omitted region text changes which
  `derive_partition_skew` text-heuristic pattern matches (not just how many times the same
  pattern matches — `Skew::union`'s max-fold makes the naive construction an equivalent mutant
  in practice), plus a precisely-shaped unaliased-dotted self-reference for the qualifier guard.

## Cross-Model Type Inference

- [x] **Salsa cycle recovery for circular refs** — `salsa::cycle` recovery attributes added to `resolved_model_schema`, `typed_model_schema`, and `type_context` queries. Recovery functions return empty/default schemas and produce diagnostics. Tests cover A→B→C→A cycles and mutual dependencies.

- [x] **Cross-model type mismatch diagnostics** — Compares `model_input_constraints` against upstream `typed_model_schema`, produces warning diagnostics for incompatible types. Tests cover VARCHAR in SUM(), compatible numeric types, multiple mismatches, and chains.

- [x] **Multi-model property tests** — `prop_multi_model_type_inference` (128 cases), `prop_three_model_type_inference` (three-model chains), and `prop_join_type_inference` all implemented with `multi_model_scenario_strategy()` generator.

## Property Test Generator Coverage Gaps

The generators in `crates/smelt-db/tests/prop_helpers/generators.rs` currently only cover a small subset of what the parser and type inference support. Each item below means: add the feature to the proptest generators so it gets randomly tested against DuckDB.

### Functions

- [x] **String functions** — Added LTRIM, RTRIM, CONCAT, CHAR_LENGTH, CHARACTER_LENGTH, REPLACE, LPAD, RPAD, REPEAT, SUBSTRING, SUBSTR, SPLIT_PART, STRPOS, LEFT, RIGHT. Omitted: INITCAP (not in DuckDB, no simple equivalent), TRANSLATE, QUOTE_IDENT, QUOTE_LITERAL, POSITION (STRPOS covers same path), TO_CHAR
- [x] **Math functions** — Added POWER, EXP, LN, LOG, LOG10, LOG2, MOD, SIGN, SIN, COS, TAN, ATAN, ATAN2, SINH, COSH, TANH, PI. Omitted: ASIN/ACOS (domain-restricted, sample values cause errors)
- [x] **Temporal functions** — Added DATE_PART, DATE_TRUNC. Omitted: EXTRACT (special syntax, DATE_PART covers same path), MAKE_DATE/MAKE_TIME/MAKE_TIMESTAMP/MAKE_TIMESTAMPTZ/AGE (complex multi-arg with specific value requirements)
- [x] **Statistical/advanced aggregates** — Added STDDEV, VARIANCE, STDDEV_POP, STDDEV_SAMP, VAR_POP, VAR_SAMP, BOOL_AND, BOOL_OR, EVERY (via dialect remapping), BIT_AND, BIT_OR, BIT_XOR. Omitted: MEDIAN/MODE/PERCENTILE_CONT/PERCENTILE_DISC (DuckDB syntax differences), APPROX_COUNT_DISTINCT/ANY_VALUE/FIRST/LAST (limited DuckDB support), CORR/COVAR_POP/COVAR_SAMP/REGR_SLOPE (need 2-column aggregates)
- [x] **Null handling functions** — Added COALESCE, NULLIF
- [x] **Comparison functions** — Added GREATEST, LEAST
- [x] **JSON functions** — Canonicalized JSON functions: JSON_OBJECT (accepts json_build_object, json_object), JSON_ARRAY (accepts json_build_array, json_array), TO_JSON (accepts to_jsonb, row_to_json), JSON_EXTRACT, JSON_EXTRACT_TEXT (accepts json_extract_string, get_json_object, json_value), JSON_ARRAY_LENGTH, JSON_OBJECT_KEYS (accepts json_keys), JSON_CONTAINS. Added TO_JSON, JSON_ARRAY, JSON_OBJECT to property test generators. Not yet generated: JSON_EXTRACT, JSON_EXTRACT_TEXT, JSON_ARRAY_LENGTH (require JSON input column which the generator framework doesn't produce yet).

### Expressions and Operators

- [x] **Window functions** — Added ROW_NUMBER, RANK, DENSE_RANK, NTILE, CUME_DIST, PERCENT_RANK, LAG, LEAD, FIRST_VALUE, LAST_VALUE, NTH_VALUE with random OVER clauses (PARTITION BY, ORDER BY). Frame specs not yet covered.
- [x] **BETWEEN / IN expressions** — Added BETWEEN and IN for numeric columns (both return Boolean). EXISTS not yet covered.
- [x] **GROUP BY + window functions combined** — `QueryShape::GroupByWindow` generates `RANK() OVER (ORDER BY SUM(x))` alongside GROUP BY. Window frame specs (ROWS/RANGE/GROUPS BETWEEN) also added to window function generators.
- [x] **Scalar subqueries** — `ExprKind::ScalarSubquery` generates `(SELECT COUNT(*) FROM data)` and `(SELECT MIN(col) FROM data)`.
- [x] **Regex operators** — Parser already lexed `~`, `~*`, `!~`, `!~*`; added to `BinaryExpr::operator()`, type inference (`→ Boolean`), and generators.
- [x] **JSON operators** — Added type inference for `->`, `->>` (Text), `#>`, `#>>` (Text), `@>`, `<@` (Boolean). Added `->` and `->>` to property test generators against DuckDB.
- [x] **Mixed-type binary operations** — `BinaryOp` generator now picks two columns of different numeric types (e.g., `int_col + bigint_col`) with correct type promotion (Double > Decimal > BigInt > Integer).
- [x] **Boolean/unary expressions** — Added `ExprKind::IsNull` (`col IS NULL`/`IS NOT NULL`), `Comparison` (`col = col`, `<`, `>`, etc.), `UnaryNot` (`NOT bool_col`), `UnaryMinus` (`-num_col`), `Exists` (`EXISTS (SELECT ...)`).
- [x] **LIKE / ILIKE operators** — Added `LIKE_KW`/`ILIKE_KW` to parser lexer, parsed as binary expressions, type inference returns Boolean, generators produce `str_col LIKE '%pattern%'`.
- [x] **Additional functions** — Added STRING_AGG, ANY_VALUE, APPROX_COUNT_DISTINCT to generators. EXTRACT and MAKE_DATE/MAKE_TIMESTAMP re-enabled in `expr_kind_strategy()` in Phase 58 (April 27, 2026): the historical `FROM`-inside-EXTRACT bug had already been fixed end-to-end (parser, AST `Expr::cast`, alias extraction, type inference); a regression test in `crates/smelt-db/tests/extract_alias_extraction.rs` now pins the behaviour, and 500 prop cases run cleanly. TO_CHAR omitted (not available in DuckDB).

### Types

- [x] **Interval type** — Added `BaseType::Interval` with `CAST('1 day' AS INTERVAL)`, Arrow mapping already handles `Duration`/`Interval`.
- [x] **Time type** — Added `BaseType::Time` with `CAST('12:00:00' AS TIME)`, Arrow mapping already handles `Time32`/`Time64`.
- [x] **Array types** — Added `ExprKind::ArrayLiteral` (`ARRAY[1, 2, 3]` / `[1, 2, 3]`), `ArraySubscript` (`arr[1]`), `ArraySlice` (`arr[1:2]`), and `ARRAY_AGG(col)` (wraps the aggregate's argument type in `Array<T>`). Arrow's `List`/`LargeList` already mapped recursively to `Array<T>` in `arrow_mapping.rs`, including nested `List<List<T>>`. One divergence registered: DuckDB returns `Array(Varchar)` from `ARRAY_AGG(str_col)` where smelt infers `Array(Text)` — folded into the existing string-family (`Text`/`Varchar`) leniency in `type_comparison.rs` by unwrapping one level of `Array` before the compatibility check, rather than adding separate Array-of-X registry entries.
- [x] **Row/Struct types** — Added `ExprKind::RowConstructor` (`ROW(<lit1>, <lit2>)` with two distinct field base types) and `BraceStructLiteral` (`{'a': <lit1>, 'b': <lit2>}`) with field-exact struct comparison. `STRUCT(1 AS a, ...)` literal syntax omitted from generation: smelt parses it but real DuckDB does not (verified against DuckDB — `struct_pack`'s named-arg form is its actual equivalent).

### Syntax Variants

- [x] **PostgreSQL `::` cast syntax** — `ExprKind::Cast` now randomly chooses between `CAST(col AS TYPE)` and `col::TYPE`.
- [x] **CAST to more types** — Added INTEGER, BIGINT, DOUBLE, VARCHAR, BOOLEAN, DATE, TIMESTAMP targets
- [x] **GROUP BY / HAVING** — `QueryShape::GroupByHaving` generates multi-column GROUP BY with HAVING predicates via `generate_having_predicate()`
- [x] **DISTINCT / DISTINCT ON** — `QueryShape::Distinct` generates `SELECT DISTINCT` queries.

### Deferred

- [ ] **SET operations (UNION/INTERSECT/EXCEPT)** — Type coercion rules across union branches, requires different query shape
- [ ] **QUALIFY clause** — Parsed and rewritten by dialect printer, needs query-shape-level testing
- [ ] **PIVOT/UNPIVOT** — Complex syntax, rare usage, needs dedicated generator shape
- [ ] **Lambda expressions** — No type inference support yet, needs inference work first
- [ ] **Ordered-set aggregates (MEDIAN/MODE/PERCENTILE_CONT/PERCENTILE_DISC)** — DuckDB syntax differences for ordered-set aggregates
- [ ] **Two-column aggregates (CORR/COVAR_POP/COVAR_SAMP/REGR_SLOPE)** — Needs multi-column aggregate generator support
- [ ] **Aggregate FILTER clause** — `COUNT(*) FILTER (WHERE cond)`, parsed but not generated
- [ ] **WITHIN GROUP (ORDER BY)** — For STRING_AGG/LISTAGG, parsed but not generated
- [x] **EXTRACT parser support** — `EXTRACT(YEAR FROM col)` and `MAKE_DATE`/`MAKE_TIMESTAMP` are now exercised end-to-end by `expr_kind_strategy()`. Phase 58 (April 27, 2026) confirmed the historical `FROM`-inside-EXTRACT bug had already been fixed across the parser, AST, alias extraction, and type inference; a dedicated regression test (`crates/smelt-db/tests/extract_alias_extraction.rs`) was added before re-enabling the generator entries.

### Known DuckDB Incompatibilities (discovered during generator expansion)

- **INITCAP**: Not available in DuckDB (no simple equivalent)
- **EVERY**: Not natively in DuckDB; dialect printer remaps to BOOL_AND (now tested)
- **LEFT/RIGHT**: Parser keyword conflict fixed; now tested
- **ASIN/ACOS**: Domain-restricted to [-1,1]; sample values (42, 100, etc.) cause errors
- **SIGN**: DuckDB and Spark both return TINYINT (SmallInt) regardless of input type; fixed smelt inference to match
- **`~*`, `!~`, `!~*`**: DuckDB only supports `~` (case-sensitive regex); `~*` (case-insensitive), `!~`, `!~*` are not available. Generator uses `~` only.
- **TO_CHAR**: Not available in DuckDB (PostgreSQL-only). Omitted from generators.

## smelt.as_struct follow-ups (Phase 38 deferred)

- [x] **Relocate as_struct lowering helper to smelt-planner** — Phase 42 (2026-04-25): `as_struct_to_sql` and `backend_supports_struct_literal` moved to `crates/smelt-planner/src/lowering/as_struct.rs`. The smelt-db site re-exports them so existing call sites and tests keep working; the lowering helper is now the canonical production location.
- [x] **Wire as_struct into `format_plan` / SQL emission** — Phase 55 (2026-04-27): `PrintContext` in `smelt-dialect` now carries optional `smelt_as_struct` and `smelt_fn` closure fields. `SMELT_AS_STRUCT_CALL` and `SMELT_FN_CALL` nodes are expanded during SQL printing; `SqlCompiler` in `smelt-cli` wires up both closures from the TypeContext and function body map. See commit 6d6e5b1.
- [x] **as_struct in function bodies that declare no backends** — Phase 42 (2026-04-25): `as_struct_backend_diagnostics_for_file` now consults `project_active_backends`, a new Salsa-tracked query that parses `smelt.yml`'s `targets:` map. When a function's `BackendSet` is `All` (no explicit `backends:` frontmatter), the diagnostic intersects against the workspace's active backends and fires when any of them lacks struct-literal capability.
- [x] **ABS(Decimal) returns Double** — Phase 53 (2026-04-26): divergence registered in `crates/smelt-db/tests/prop_helpers/divergences.rs` as `abs_decimal` and `abs_decimal_schema_resolved`. Property-test regressions file updated. See commit 0ee244c.

## VALUES / sources resolver — follow-ups from 2026-05-28 plans

Reviewer-raised gaps adjacent to `docs/plans/20260528-source-leaf-name-collision.md` and `docs/plans/20260528-values-derived-table-typing.md`. Both are worth probing but neither blocks the closed UNKNOWN clusters.

- [x] **JOIN-side parallel of the `smelt.sources.*` shadow.** Closed incidentally by the generator-emission Phase 2 fix (commit `37bc3845`): `resolve_table_ref_schema` was updated to route `smelt.<path>` value-form JOIN references through the same `resolved_columns_for_path` method that `process_table_ref_pure` uses. The hand-authored-first ordering in that method, combined with W3's pre-existing collision discard, makes the same leaf-collision class unreachable in JOIN context. No targeted test was added — the existing per-entity-fixture test in `crates/smelt-db/tests/source_leaf_collision.rs` exercises the FROM-clause path; a JOIN-clause analogue is a worthwhile defensive addition but not blocking.
- [ ] **VALUES-body CTE arity check.** `check_cte_alias_arity` (`crates/smelt-db/src/type_inference/values.rs`, added in commit 47d874a4) returns early when the CTE body isn't a `SELECT`, so `WITH cte(a) AS (VALUES (1, 2)) SELECT * FROM cte` is silently exempt from the `AliasColumnArityMismatch` diagnostic that the SELECT-body case enforces. Symmetric coverage is a small extension — the underlying VALUES column count is already available via `Subquery::values_clause()` (commit 86f755fe) and `infer_values_columns` (commit 01fc027f). Mirror the SELECT-body path's arity check.

## P7c (diagnostic-parity) — resolved 2026-07-19

The Map-loader decision this section tracked (direction A/B/C below) resolved
as **(B) Wire Map consumption**: P7d (commit `ab22f990`) implemented Map API
postfix method calls (`.entries()` / `.keys()` / `.values()` / `.get()` /
`.has()`) on loader-result values, lowered at build time by
`smelt-runtime::meta_eval`. `tenants.sql` was given a clean consuming form
using this API, `examples/meta_config` builds and executes cleanly end-to-end,
and it is no longer on the `example_builds` `KNOWN_UNBUILDABLE` allow-list.
See `docs/specs/meta_config_loading.md` §"Known Divergences / Open Questions"
for the residual gaps (recursive schemas, per-key deep-merge overlays,
`Optional<V>` schema fields) and `docs/plans/20260509-meta-language-overall.md`
for the P7d phase history.

## Pre-existing issues surfaced by the clock-vs-root sessions plan (2026-07-12)

Found during `docs/plans/20260711-clock-vs-root-anchored-sessions.md`; both predate that work (confirmed reproducible on unmodified trees) and were left untouched as out of scope.

- [ ] **`extract_interval_days_from_combined` mis-parses sub-day intervals as days.** `crates/smelt-logical/src/analysis/temporal.rs` has no MINUTE/SECOND branch, so `INTERVAL '5 minutes'` parses as 5 days. Impact is limited to the advisory `analyze_batch_safety` JSON label (e.g. `context=5d`); actual runtime chunk sizing uses `batch_safety_from_bounds` and is unaffected. Fix: add sub-day unit branches (round up to 1 day, or carry finer granularity) plus a regression test pinning `INTERVAL '5 minutes'`.

- [ ] **Rare parallel-execution flake in DuckDB-backed integration suites.** Third sighting
  2026-08-08 (captured output, as this item requested): `smelt-runtime/tests/fingerprint_sidecar.rs`
  `a_hand_corrupted_stamp_is_detected_treated_as_absent_and_logged_loudly` failed under a full
  parallel `cargo test -p smelt-logical -p smelt-runtime` with
  `a corrupted/mismatched stamp must be logged loudly (tracing::warn!) ...; captured WARN messages: []`
  (`fingerprint_sidecar.rs:1008`), then passed 4/4 in isolation. This one is a **tracing-capture
  race**, not DuckDB load: the assertion depends on capturing `tracing::warn!` while other test
  binaries/threads contend for the global subscriber. Fix direction: use a scoped
  `tracing::subscriber::with_default` (or `set_default` guard) around the assertion instead of a
  global subscriber. Earlier sightings observed twice during the plan run: `smelt-datagen/tests/example_web_analytics.rs::test_identity_backward_fill_materializes` and one `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs` test failed under a full parallel `cargo test`, then passed in isolation and on re-run of the full suite — same failure family both times (parallel load), reproduced on an unmodified tree. Worth capturing the exact failure output next time it fires and checking for a shared-resource collision (e.g. temp DB paths, memory pressure) before it erodes trust in the gates.

## Refresh-as-maintenance-plan: ratification queue (2026-07-06) — CLOSED 2026-07-07

All items done: decisions 1–11 ratified 2026-07-06 (`09-spec-readiness.md` §1); F4 (`25c04a70`)
and F1/G-11 (`770c77f1`) landed; G-12 probed (double-fold pinned, `b1ead60f`); the spec-diff map's
headline rows landed (the maintenance-plan spec, since consolidated into `incremental_models.md`, new `3f65a671`; `models.md` rewrite `aa326a3f`;
`sources.md` `fb9a5977`); M0 deleted (`7d1b4f17`). A four-way spec-vs-research review (2026-07-07)
confirmed the landed specs faithful and identified the residue, now queued as autonomy-loop
sub-plans: `docs/plans/20260707-maintenance-plan-spec-alignment.md` (SA1–SA5) and
`docs/plans/20260707-maintenance-plan-impl.md` (MP1–MP16). Pre-framework registry rows
(keyed-collapse K3–K6, keyed-time-partitioned, L4 batched/versioned/mv) superseded — see the
master's 2026-07-07 note.

## 2026-07-12 — ON-join `SELECT *` schema expansion drops the right side

Found while fixing NATURAL/USING join-star dedupe (parser-gap-closure). `model_schema`'s
wildcard expansion (`row_extensions` in `crates/smelt-db/src/queries/schema.rs`) now covers
NATURAL and USING joined refs with join-shared columns deduped (DuckDB-verified [x, y, z]),
but ON-joined refs are still not expanded at all: `SELECT * FROM smelt.models.a JOIN
smelt.models.b ON a.x = b.x` infers only a's columns, where DuckDB yields all four
[x, y, x, z] with the shared name duplicated. Expanding ON joins means admitting duplicate
column names into inferred schemas (find-by-name consumers, LSP completion, input-constraint
keying all assume unique names today), so it needs its own pass. Current behavior is pinned
by `on_join_star_current_behavior_left_side_only` in
`crates/smelt-db/tests/integration/join_star_schema.rs` — update that test when fixing.
Related limitation to fix in the same pass: `SharedWithPrior` (NATURAL) dedupes against ALL
prior refs' names, but DuckDB's NATURAL binds only to the adjacent join operand — in
`FROM a, b NATURAL JOIN c`, a column shared between a and c (but not b) is wrongly dropped
where DuckDB would carry the duplicate.

## 2026-07-12 — TABLESAMPLE/PIVOT/UNPIVOT vs alias ordering (parser + printer)

Found during the parser-gap-closure review (PR #158, derived-table alias fix f68ebd86).
smelt's parser accepts only `base TABLESAMPLE(...) AS alias` and the printer emits that
same order — but real DuckDB v1.5.4 REJECTS it and requires `base AS alias TABLESAMPLE(...)`
(oracle-verified). Pre-existing parser grammar bug (`parser/select.rs` parses TABLESAMPLE
before alias); previously masked because the old printer dropped both clauses, now live:
the printer emits DuckDB-invalid SQL whenever TABLESAMPLE/PIVOT/UNPIVOT co-occurs with an
alias. Not exercised by the current corpus/seed gates. Fix: swap grammar+printer to
alias-first order and add a seed line; PIVOT/UNPIVOT ordering unprobed, verify while there.

## 2026-07-12 — External-ledger `smelt_fails_unclassified` triage (236 → root-caused)

Triaged all `smelt_fails_unclassified` entries (236 measured via re-parse against the live
ledger; the task brief that kicked this off estimated 237, likely a stale count from before
an unrelated prior commit) in
`crates/smelt-parser-compat/tests/corpus/external_ledger.toml` by re-parsing each corpus
statement and bucketing by first-error signature + syntactic pattern (script discarded,
see `docs/plans/` for methodology if resurrected). Three genuinely small parser/lexer gaps
were fixed with red-green tests (26 ledger entries closed); the remaining 209 entries were
reclassified into 56 named root-cause categories (`gaps.rs`-style vocabulary) with an
honest note per category — no fabricated root causes, each was independently confirmed by
direct parse inspection.

**Fixed this pass:**
1. Double-quoted-identifier aliases (`AS "median_delay"`) — `parse_select_item` and
   `parse_table_ref`'s explicit-`AS` branch only accepted `IDENT`, not the `STRING` token
   smelt's lexer produces for double-quoted text (`consume_string` doesn't distinguish `'`
   from `"`). Fixed in `parser/select.rs` + `parser/mod.rs` (`at_quoted_ident_alias`) +
   `ast.rs` (`alias()`/`alias_token_text()`) + `printer.rs` (re-quote on print — the
   printer must not silently drop the quotes DuckDB/PostgreSQL require for aliases that
   need them). Closed 21 entries. Surfaced a second, unrelated printer bug in the same
   pass (`Display for Subquery` drops a VALUES-clause CTE body) — reclassified as
   `roundtrip_mismatch`, not fixed (see below).
2. `FIRST(x)`/`LAST(x)` as aggregate function names — `FIRST_KW`/`LAST_KW` weren't in
   `at_keyword_as_function_name`'s allowlist (unlike `LEFT`/`RIGHT`). One-line addition.
   Closed 3 entries (a 4th, `FIRST(i ORDER BY i)`, still needs `aggregate_call_order_by_clause`).
3. Leading-dot decimal literals (`.5`, `.000_005`) — the lexer's digit dispatch never
   tried `consume_number` when the current char was `.`; added a guarded match arm before
   the `...`/`..` spread-operator arms. Closed 3 entries.

**Not fixed — explicitly out of scope this pass** (real, would need `smelt-db` nullability/
join-topology work, not just parser grammar): `FROM t "quoted"`-style comma-joins
(`implicit_cross_join_comma_syntax`) would need a "no explicit modifier ⇒ inner join"
default in `JoinClause::join_type()` to special-case comma-joins as cross joins, which is
inference-semantics territory, not a grammar-only change.

**Top of the follow-up list** (full 56-category table below; effort is a rough guess, not
sized): `NOT IN` / `NOT ILIKE` (`not_prefixed_binary_operator`, 6 entries) is surprisingly
broken — `SELECT 2 NOT IN (2, 3)` alone fails — and is likely a very small fix (binary
operator precedence/lookahead in `expr.rs`), probably the single highest-leverage item here.
`quoted_table_name_in_from` (`FROM "flights"`) is the same root cause as fix #1 above but in
`parse_table_ref`'s primary-identifier path rather than the alias path — also likely small,
and the resulting scope (any double-quoted table/schema name) is probably underrepresented
in this bucket's raw count (1) because most instances get preempted by an earlier error in
the same statement.

Counts below are the entries *this triage pass* moved into each category — categories that
already had entries before the pass (e.g. `file_glob_or_path_literal_from`,
`sqllogictest_template_placeholder`) have higher ledger totals; the ledger itself is the
authoritative count.

| Category | Count | Effort guess | DuckDB-relevant? |
|---|---|---|---|
| `implicit_cross_join_comma_syntax` | 25 | Medium (join-topology semantics, see above) | Yes |
| `sqllogictest_template_placeholder` | 23 | N/A (not real SQL, test-harness artifact) | No |
| `postgres_typed_literal_prefix` | 14 | Medium (extend typed-literal-prefix keyword set) | No |
| `postgres_geometric_operators` | 11 | Medium (new operator tokens + geometry types) | No |
| `file_glob_or_path_literal_from` | 10 | Medium (string-literal-as-table-source grammar) | Yes |
| `aggregate_call_order_by_clause` | 8 | Medium (WITHIN GROUP / agg ORDER BY grammar) | Yes |
| `at_time_zone_or_time_tz_type` | 7 | Small–Medium | Partial |
| `postfix_dot_field_access_on_parenthesized_expr` | 7 | Medium | Partial |
| `not_prefixed_binary_operator` | 6 | **Small** (see above — top pick) | Yes |
| `jsonb_operators` | 6 | Medium | No |
| `sql_json_constructor_functions` | 6 | Medium | Partial |
| `postgres_interval_range_qualifier` | 5 | Small–Medium | No |
| `range_keyword_as_identifier_or_function` | 5 | Small (same shape as the FIRST/LAST fix) | Yes |
| `row_value_comparison` | 5 | Medium (row-constructor comparison grammar) | Yes |
| `postgres_table_inheritance_wildcard` | 4 | Small (grammar) but pg-only, low value | No |
| `star_exclude_or_rename_clause` | 4 | Medium (nested contexts + trailing comma) | Yes |
| `select_into_clause` | 3 | Medium | Partial |
| `double_equals_operator` | 3 | **Small** (lexer: `==` as EQ alias) | Yes |
| `group_by_tuple_expression` | 3 | Medium | Yes |
| `numeric_literal_followed_by_ident_no_space` | 3 | N/A (intentional fail-loud, see lexer.rs) | No |
| `malformed_corpus_statement` | 3 | N/A (extraction artifact, not real SQL) | No |
| `quoted_table_name_in_from` | 1 (undercounted, see above) | **Small** (same fix shape as #1) | Yes |
| everything else (33 categories, ≤2 entries each) | 41 | Mixed | Mixed |

Ledger delta this pass: 236 `smelt_fails_unclassified` → 0 (26 entries closed outright — the
27th fix, a leading-dot-decimal CTE, surfaced the unrelated printer bug above and was
reclassified rather than closed; 209 entries redistributed across the other 55 named
categories in the table above, which together with `roundtrip_mismatch` account for all
236). `cargo test -p smelt-parser-compat --test external_corpus` stays green throughout
(`ledger_has_no_stale_entries` re-validated after every batch of edits).

## 2026-07-12 — Residue from walrus named-arg work (PR #158, 47e74c1c)

- `NULL::VARCHAR` (top-level cast of NULL, and casts inside named-arg values) fails to
  parse — likely a parse_expression precedence gap around `::` on NULL/named-arg value
  positions; fails standalone too, pre-existing. 1 ledger entry recategorized under
  `smelt_fails_unclassified` carries the actual error (`Expected expression, found DOUBLE_COLON`).

## 2026-07-12 — Final whole-branch review residue (PR #158)

- `SELECT a FROM t UNION (SELECT a FROM t) ORDER BY a` fails to parse (DuckDB accepts;
  trailing ORDER BY/LIMIT after a parenthesized set-op operand is unhandled in
  `parse_select_stmt`'s set-op tail). Fail-loud, but the parenthesized form exists precisely
  to attach a trailing ORDER BY to the whole union — likely the most user-visible hole in
  the new set-op surface. Same family as the ledgered `((A) UNION B)` scalar-subquery residual.
- `SELECT {'a': 1}.a` (dot-access on a brace-literal receiver) fails to parse — same class
  as the ledgered `postfix_dot_field_access_on_parenthesized_expr` (7 entries); fold in.
- `printer.rs` `Display for TableRef` final raw-text `else` fallback: believed unreachable
  now that subquery/nested/identifier branches are explicit, but if ever reached the
  TABLESAMPLE loop + alias printing after it could double-print — add a defensive early
  return when touching this next.
- `ast.rs` `strip_ident_quotes` handles `"…"`/`'…'` but not `$$…$$` dollar-quoted STRING
  tokens (e.g. `CollateExpr::collation_name`) — delimiters silently kept if one ever reaches
  such a call site.

## Incremental-models spec redraft (2026-07-22)

- PR #166 (branch `spec-redraft-incremental-models`): phases 0–5 done. Remaining: Phase 6 follow-up PR — run `/smelt:validate incremental_models`, sweep §-name references in code comments + sibling specs per the plan Appendix A heading map (docs/plans/ and docs/research/ stay untouched), delete the claims scaffolding file. See `docs/plans/20260722-incremental-models-spec-redraft.md`.

## Backbuild property-test hardening (from 2026-08-03 mutation audit)

Full findings: `docs/handoffs/2026-08-03-backbuild-property-test-review.md`.

- [x] **Rerun-safety leg** (2026-08-07) — composed scripts now apply twice in `generated_options_match_full_rebuild_oracle` when all chosen options are `rerun_safe: true`; `e2_idempotent_with_identity` + `f1_idempotent_with_identity` added as E2/F1 siblings of `e4_idempotent_with_identity`. M4b re-injected and caught by both the property leg (at default N=24) and the new conformance tests.
- [x] **Generator additions** (2026-08-07) — `EditRecipe::TightenFilterStatusOpen` (`o.status = 'open'`, E1 3VL; guaranteed slot), WHERE conjuncts on `Shape::Grouped` + `AmountPositive` on the `AddAggregate` guaranteed slot (B5 WHERE-carry), and the guaranteed `LoosenFilter` slot swapped to `StatusOpen` (E2/E4 3VL). M2, M7, M3 all re-injected and caught at default N=24.
- [x] **Optional** (2026-08-07) — documented conformance-only coverage of H drop-ordering in the property harness module doc (generatively unobservable: drop+reader combos are correctly refused at admission).

## Planner metamorphic gate follow-ups (2026-08-08)

New gate: `cargo test -p smelt-cli --test planner_metamorphic` — generative
metamorphic equivalence for the `cube_split` rewrite (recipes → in-memory
DuckDB → two-way `EXCEPT ALL` vs the naive query; unsupported clauses must be
refused, never silently dropped). Candidate extensions found while mapping the
planner surface:

- [ ] **Logical-plan rewrite rules are unprovable end-to-end** — the
  `smelt_planner::logical_plan_rules` family (`EliminateUnusedLeftJoin`,
  `PushFilterIntoTransparentFunction`, `ExpandTransparentFunctionCalls`,
  `ElideEmptySelectItemsSplices`) is display-only: no Plan→SQL printer
  exists, so their correctness cannot be executed against a backend. Notably
  `EliminateUnusedLeftJoin` carries a documented soundness caveat (§20E:
  trusts declared cardinality) with no executable check. Either build a
  Plan→SQL printer (which would also let `--show-plan` output be verified) or
  keep the rules display-only and say so in a doc comment.
- [x] **Window-clamp metamorphic relation** (2026-08-08) — `cargo test -p
  smelt-cli --test transformer_metamorphic`: for generated partition-aligned
  models, data, window partitions, and pushdown margins, asserts (A) the
  full-domain clamp drops exactly the NULL-event-time rows, (B) union of
  per-window clamps == full clamp, (C) union of pushdown+clamp windows ==
  full clamp in the production compose order. Soaked at 500 cases.
- [ ] **Cube-split rewrite is dead in the runtime** — `Transformation::ReplaceWithPlan`
  is never executed by `smelt-runtime` (only `smelt explain` and tests read
  it). The new gate proves the rewrite correct when it does fire; decide
  whether to wire it into execution or retire it.
