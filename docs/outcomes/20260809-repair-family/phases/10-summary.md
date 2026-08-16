# Phase 10 summary — repair over a decomposed combiner

## Shipped

- `CumulativeClassification::state_columns()` (`crates/smelt-logical/src/rules/cumulative.rs`) —
  single derivation of every state-bearing aggregator column's `StateColumn`s, replacing three
  hand-rolled copies in `smelt-runtime` (`cumulative.rs` ×2, `diagnostics.rs`).
- `repair_augmented_model_sql` (`crates/smelt-runtime/src/maintenance_driver.rs`) — named wrapper
  over `state_augmented_projection` for the repair path, unit-tested independently.
- `execute.rs`'s repair leg now widens `clean_sql_for_merge` with the fold's own hidden state
  columns before compiling, so `repair_candidate_select`'s `INSERT` matches the fold-created
  table's column list; the `diff_patch` leg's `compared_columns` gains the state column names too
  (a group whose presented value is unchanged but whose state moved is still rewritten).
- `diagnostics.rs`'s `PerGroupRecompute` preview applies the identical augmentation (falling back
  to "no state" when the preview's synthetic model doesn't classify as a cumulative aggregate at
  all, rather than refusing the whole preview).
- Fixed a real, previously-latent bug this phase's own test exposed: a repair cell's `PlanCell.group`
  string was built from the fold's SQL-declaration column order
  (`crates/smelt-logical/src/maintenance/derive.rs`), while the canonical `ColumnGroup::name()`
  (used by `matching_write_pin`) is alphabetical — the two diverged whenever a combiner's SQL order
  wasn't already alphabetical (`OrderMonotone`'s `(max_by_val, max_by_ord)`), silently defeating any
  `write: diff_patch` pin over such a cell. Fixed by sorting the repair cell's own column list before
  building its group string.
- `docs/specs/incremental_models.md` §"The repair family" gained "Repair over a decomposed
  combiner"; the matching Known Divergences entry was deleted.
- Conformance: `repair_pool_upholds_equivalence_under_retraction` now drives `OrderMonotone`
  through the full insert/update/delete mutation loop (previously creation-only); new
  `diff_patch_repair_over_decomposed_state_upholds_equivalence` proves the `diff_patch` leg over
  decomposed state. `registry.rs`'s `known_bug_repair_candidate_select_ignores_decomposed_state`
  entry and its `known_bug_still_reproduces` arm are deleted; `KnownBug` kept alive via
  `#[allow(dead_code)]` since no entry currently uses it.

## Decisions

- The repair-recipe testkit's `write: diff_patch` frontmatter pin must name every column in the
  cell's FD-linked column group, not just the value alias — `render_repair_model_file`
  (`crates/smelt-maintenance-testkit/src/render.rs`) now includes the ordering companion column
  for `OrderMonotone`.
- The group-string ordering fix lives in `derive.rs` (sort before building the string) rather than
  changing `ColumnGroup::name()`'s own convention or `matching_write_pin`'s comparison — the
  canonical alphabetical order is `grouping::derive_column_groups`'s own, and the repair path's
  ad hoc group-string construction is what should conform to it.

## For the next planner

- The group-string ordering bug fixed here likely affected `write:`-pin matching for **any**
  multi-column FD group whose SQL declaration order isn't alphabetical, not just repair cells —
  worth an audit of the other `group.name()`-adjacent call sites (`append_model_edge_cells` and
  similar) for the same class of bug, though every one checked in this phase already builds its
  group string from the canonical `ColumnGroup` object directly.
- Phase 9's flagged hardening items (bit_xor digest collision risk, unconfirmed snapshot-reconcile
  sidecar seed, untested stale group comparandum) remain untouched — out of this phase's scope.
- Phase 11 (surface: `smelt explain` rendering, docs-site update) is next; this phase touched no
  `smelt explain` rendering code.

## Gates

- `bash .claude/scripts/verify-phase.sh` — all green (fmt, clippy zero-warnings, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test statement_parity --test repair_lowering --test diagnostics`
  — 21 + 17 + 10 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 59 passed.
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed.
