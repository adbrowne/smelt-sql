# Phase 4 summary — cumulative classifier's whole-SQL scans move onto the walk

**Shipped:**
- `analysis/walk.rs`: refactored `own_region_text`'s pruning traversal into a shared
  `visit_own_region_elements` (element-level) + `visit_own_region` (node-level, `pub(crate)`)
  entry point; `own_region_text` now re-expressed on it, unchanged in behaviour.
- Two new leaf classifiers, both doc-tagged `Leaf classifier`: `scope_has_window_function`
  (any `WINDOW_SPEC` in a scope's own region) and `scope_nondeterministic_fn` (first parsed
  `FUNCTION_CALL` whose name is in `monotonicity::NONDETERMINISTIC_FUNCTIONS`, exact match).
- `ScopePresenceTransfer<T>` + `first_scope_hit<T>`: a shared scope-presence `Transfer` folding
  `Option<T>` as parallel-OR (first `Some`) over the whole children slice, with the same
  CST-flat-enumeration fallback shape `model_has_trajectory_column`/`model_partition_skew` use
  for `Unsupported` trees.
- `rules/cumulative.rs`: `classify_cumulative`'s two whole-SQL scans (`upper_sql.contains("OVER(")`
  and the `NONDETERMINISTIC_FUNCTIONS` loop) replaced by `first_scope_hit` calls; dead
  `upper_sql` binding and the stale "Known walk-invariant violation" comment block deleted.
- `walk_coverage.rs`: `KNOWN_NONCOMPLIANT` emptied; `is_raw_scan_line` widened via
  `case_folded_variables`/`contains_receiver` to catch `<ident>.contains(...)` where `<ident>`
  is bound to a `.to_uppercase()`/`.to_lowercase()` expression, not just a string-literal scan.
- Spec delta: `incremental_shapes.md`'s `KeyedForbidsWindowFunctions`/`KeyedForbidsNondeterministic`
  rows now say "any SELECT scope", correcting the pre-existing "outer SELECT" text to match what
  the implementation always enforced; `model_properties.md` gained a "Keyed-admission presence
  verdicts" consumption-rule bullet and lost the now-false `cumulative.rs` Known Divergences bullet.
- 8 new tests: 6 in `cumulative.rs` (string-literal false positive gone, CTE/expr-scope window
  still refuses, function-name-suffix false positive gone, CTE nondeterministic still refuses,
  `RECURSIVE`-CTE fallback still refuses) and 2 in `walk_coverage.rs` (file no longer skip-listed;
  case-folded-variable scan form detected, plain collection `.contains` not flagged).

**Decisions:**
- Reused `own_function_call_names`' "exclude nested SUBQUERY" idea but built the two new
  classifiers on the more general `visit_own_region` rather than reusing `own_function_call_names`
  directly, since the latter operates over one `Expr`, not a whole scope's clauses (WHERE/HAVING/
  window specs live outside any single select-list expression).
- `first_scope_hit`'s fold checks children first, then the node's own classification (mirrors
  `TrajectoryTransfer`'s `child_hit || … || scope_has_running_fold_over_axis` order) — order is
  immaterial for correctness (first `Some` wins either way) but keeps the shape consistent with
  the crate's other scope-presence transfers.
- Bare `CURRENT_TIMESTAMP`/`CURRENT_DATE` (no parens) parse as a plain column reference, not a
  `FUNCTION_CALL` — confirmed by reading `parser/expr.rs`'s `IDENT` branch (only enters the
  `FUNCTION_CALL` path when followed by `LPAREN`). The pre-migration scan's pattern was always
  `format!("{}(", nd)` (name + open-paren), so it never matched the bare form either — the new
  `FUNCTION_CALL`-node-based classifier is behaviour-preserving here with no special-casing needed.

**For the next planner:**
- Phase 5 (declared-RI closure reaching every `JoinContext`-taking maintenance-cell route) is
  next; nothing from this phase changes its scope.
- `own_region_text`'s doc comment was previously stale (claimed expression-position subqueries
  "are NOT walk nodes", contradicted by the code's own SUBQUERY-pruning branch, which predates
  this phase but was corrected here as a drive-by fix while refactoring the same function) —
  worth a general sweep for other stale walk-node doc comments if one surfaces later, but not
  pursued further here since it's outside this phase's named scope.
- `PartitionGrainAdmission`'s `collect_scope_region` (walk.rs ~1618) still recurses into
  expression-position `SELECT_STMT`s for window/LIMIT/subquery enumeration and its doc comment
  still says expr-position subqueries "are NOT walk nodes" — that function was NOT touched by
  this phase (out of the named scope: it feeds a different admission rule, not
  `classify_cumulative`) and may itself be stale/pre-phase-1 residue worth a future look.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test walk_coverage --quiet` — 6 passed.
- `cargo test -p smelt-logical --quiet` — all passed (749 lib tests + all integration suites).
- `cargo test -p smelt-runtime --test statement_parity --quiet` — 37 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — 78 passed.
- `rg -n 'contains\("OVER' crates/smelt-logical/src` — empty.
