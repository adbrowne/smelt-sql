# Phase 4 plan — presentation projection: state columns invisible to consumers

## Objective

Make the `__part` state columns phase 3 materialises invisible to every consumer of a
state-bearing model: a downstream `SELECT *` / `t.*` over such a model compiles to the
model's presented columns only, and the analysis layer's public schema (`ref()` expansion,
declared-schema checks, downstream type inference) never mentions a state column. Advances
success criterion 4, and is a precondition for 1–3 being *safe* once rows 5–6 widen admission.

Admission is still closed (`AggregatorColumn.state` is `None` everywhere), so — as in phase 3 —
tests drive the new mechanism with directly-constructed state maps rather than real SQL.

## Spec delta

`docs/specs/incremental_models.md` §"Decomposed state (rung 2) in keyed models" →
**Presentation projection** paragraph. Add two sentences (spec-first, before code):

- *How* the exclusion is achieved for `SELECT *`: because state columns share the stored table,
  a wildcard in a consumer that reads a state-bearing model is rewritten at compile time to that
  model's presented columns (sibling relations in the same `FROM` keep their own `<rel>.*`);
  explicit column references are untouched, and a `__part` name written by hand is an ordinary
  unresolved-column diagnostic, since it is not in the model's public schema.
- The refusal: if a wildcard's relations cannot be resolved while a state-bearing model is in
  scope, the compile fails loud with the model and the unresolvable wildcard named — never a
  pass-through that would leak state columns into the consumer's schema.

## Tests

`crates/smelt-logical/tests/emit_statements.rs` (new pure emitter next to
`state_augmented_projection` in `maintenance/emit.rs`):

1. `presentation_projection_expands_bare_star` — `SELECT * FROM smelt.models.agg` with `agg`
   state-bearing → the presented column list, in schema order.
2. `presentation_projection_expands_qualified_star` — `SELECT a.* FROM smelt.models.agg AS a`
   → `a.<col>` per presented column; the alias is honoured.
3. `presentation_projection_keeps_sibling_star` — `SELECT * FROM smelt.models.agg JOIN users …`
   → `agg`'s columns explicit, `users.*` left as-is.
4. `presentation_projection_is_identity_without_state_refs` — no state-bearing ref in scope →
   the SQL is returned byte-identical (no rewrite risk for existing projects).
5. `presentation_projection_refuses_unresolvable_star` — a wildcard whose relation cannot be
   resolved with a state-bearing ref in scope → `Refusal`, never silent pass-through.
6. `presentation_projection_ignores_star_in_string_literal` — a literal containing `*` and a
   state column name is untouched (CST-location rewrite, not a whole-text scan).

`crates/smelt-runtime` (unit tests in `compile.rs`):

7. `compile_hides_state_columns_from_downstream_star` — a compiler carrying a state-bearing
   model map compiles a downstream `SELECT *` into the presented column list.
8. `compile_is_unchanged_without_state_bearing_models` — empty map → compiled SQL identical to
   today's output (parity guard).

`crates/smelt-db` (tests in `src/tests.rs` or `queries/schema.rs` unit tests):

9. `public_schema_excludes_state_columns` — a keyed model's `model_schema` / a downstream
   `SELECT *`-derived `resolved_model_schema` contains only presented columns, and a hand-written
   reference to a `__part` name is an unresolved-column diagnostic (locks criterion 4 at the
   analysis layer against future regressions).

## Tasks

1. Land the spec delta above.
2. Add `presentation_projection(sql, &state_bearing: BTreeMap<String, Vec<String>>) ->
   Result<String, PresentationRefusal>` to `smelt-logical`'s `maintenance/emit.rs`: parse, walk
   the select list, and rewrite wildcard items via `text_range` locations only (walk rule; no
   whole-text scan). Resolve each wildcard's relations from the `FROM`/join clause, tracking
   aliases; refuse on anything unresolvable while a state-bearing ref is in scope.
3. Thread the presented-column list: the map's values come from the *public* schema
   (`UpstreamSchemas::models`), the keys from the set of state-bearing models — no new source of
   truth for "which columns are presented".
4. Add `SqlCompiler::set_state_bearing_models` (+ registry-wide `..._all`, mirroring
   `set_upstream_schemas`) and call `presentation_projection` inside the compile path, after ref
   resolution has the ref names still recoverable (choose the point that keeps refusal messages
   naming the user's `smelt.models.*` path).
5. Populate the map in `execute.rs` where `set_upstream_schemas_all` is called (~line 986) and in
   the dry-run path (~line 709), by classifying each keyed model with `classify_cumulative_sql`
   and collecting `AggregatorColumn.state`'s state-column names. Empty today; correct the moment
   rows 5–6 widen admission.
6. Confirm `smelt-db`'s public schema already excludes state columns (test 9); if it does not,
   fix it there rather than in the runtime.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test emit_statements --test walk_coverage`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_conformance` (must stay at its current verdicts —
  no admission widened, and the identity path must not perturb any existing recipe)

## Commit message

`feat(incremental): hide decomposed state columns behind a presentation projection`
