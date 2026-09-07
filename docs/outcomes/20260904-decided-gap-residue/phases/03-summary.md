# Phase 3 summary (reopened) — once-write route-2 `unique_key` skip + generative-pool witness

**Shipped:**
- `classify_once_write` (`crates/smelt-logical/src/rules/cumulative.rs`) — route-2's candidate
  loop now `continue`s (admitting the candidate with no declared FD) when the column is itself a
  `unique_key` member, per human decision (c). Consults no `PropertyVector`, exactly as route 1
  does not.
- 3 unit tests replacing the superseded `once_write_not_null_route_still_requires_the_functional_dependency`:
  `once_write_key_member_candidate_admits_without_a_declared_fd`,
  `once_write_bare_key_reduction_admits_without_a_declared_fd` (no-fallback spelling too — the
  skip is in the candidate loop, covers every route-2 spelling), and the regression guard
  `once_write_non_key_candidate_still_requires_the_fd`.
- `crates/smelt-db/tests/maintenance_fold_spec_companion.rs::fold_spec_admits_the_key_member_candidate_without_a_declared_fd`
  — plan-layer/runtime parity for the FD-free route.
- `crates/smelt-maintenance-testkit`: new `KeyedCombiner::OnceWriteKeyFallback`
  (`COALESCE(MAX(<key>), 0) AS once_val`) — `projection_sql` gained a `key` parameter (both call
  sites, `render_keyed_model_body` and the repair-recipe body, updated); left out of
  `arb_keyed_combiner` and out of `render_keyed_model_file`'s `fd_block` match (falls to
  `_ => String::new()` — the absent block is the witness).
- Two new `crates/smelt-cli/tests/maintenance_conformance/gate.rs` tests:
  `once_write_key_fallback_pool_upholds_end_state_equivalence` (asserts the rendered model file
  has no `functional_dependencies:` block, asserts a `KeyedFold` cell, drives the constant-payload
  schedule) and the `combiners` array in `once_write_null_pool_upholds_end_state_equivalence`
  extended with `OnceWriteKeyFallback`.
- `docs/specs/incremental_shapes.md` §"The column-family catalogue" (new clause) and §Known
  Divergences "The key grain" (rewritten — the multi-column-`unique_key`/no-witness clause is
  gone; the driving-clock-derived and bare-key-reference clauses stay verbatim).
- `docs-site/docs/reference/cumulative-aggregate.md` §"Once-write columns" — multi-candidate
  paragraph now names the key-member exemption explicitly.

**Decisions:**
- Human decision (c) implemented as specified: skip lives in the candidate loop (covers every
  route-2 spelling), no `PropertyVector` consulted.
- The generative-pool witness needed a NEW combiner variant projecting the KEY column
  (`OnceWriteKeyFallback`), not a wider `KeyedRecipe`/FD-declaration shape — this sidesteps both
  validation walls the first phase-3 attempt hit (self-contradictory single-column FD;
  `KeyedGroupByContainsPartitionColumn` for a second key column), since decision (c) means no FD
  is declared at all for this route.
- `once_write_null_pool_upholds_end_state_equivalence`'s NULL schedule varies `val`, not the key
  column; `OnceWriteKeyFallback` projects the key (`id`), which is never NULL by construction, so
  this extension exercises the classify+execute path under the shared harness rather than a new
  NULL direction — still a valid generative-pool witness per criterion 3's wording.

**For the next planner:** nothing outstanding — criterion 3 is now fully closed (classifier
route, unit tests, plan-layer parity, AND the end-to-end DuckDB generative-pool witness). No new
gaps surfaced.

**Gates:**
- `cargo test -p smelt-logical --lib cumulative::` — pass (32 tests)
- `cargo test -p smelt-db --test maintenance_fold_spec_companion` — pass (16 tests)
- `cargo test -p smelt-cli --test maintenance_conformance once_write` — pass (7 tests, 142s)
- `cargo test -p smelt-runtime --test statement_parity` — pass (37 tests)
- `cargo fmt --all -- --check` — clean
- `bash .claude/scripts/clippy-gate.sh` — clean (both feature sets)
- `cargo test --quiet` (full workspace) — exit 0
- `cargo test -p smelt-cli --test example_diagnostics` — pass (121 passed, 1 ignored)
