# Phase 5 summary — declared facts reach every `JoinContext`-taking maintenance-cell route

## Shipped

- `JoinContext::union` (`analysis/join_shape.rs`) — combines two independently-built contexts.
- `append_model_edge_cells` now takes `sources: &[SourceFacts]` and
  `source_referential_integrity: &SourceReferentialIntegrity`; its shared `join_ctx` is
  `model_edges_join_context(..).union(source_facts_join_context(..))`.
- `model_edge_enrichment_closure` widened: the P1 AND now folds every external source actually
  joined in the scope alongside model edges, each judged with its own declared `unique_key`
  (conjunct 3) / `referential_integrity` (conjunct 4) — mirrors `mutation_enrichment_closure`'s
  per-source `skeleton_source_closure` call exactly.
- `repair::admit_per_group_recompute` takes `join: &JoinContext` (was a literal
  `JoinContext::new()`); its one production caller (`derive_new_data`'s key-grain branch)
  builds the real context via `source_facts_join_context(inputs.sql, &inputs.sources)`.
- `derive_model_maintenance_plan_with_edges` (smelt-db) threads its existing `sources`/
  `source_referential_integrity` into `append_model_edge_cells`.
- New structural gate `crates/smelt-logical/tests/join_context_reach.rs`: every
  `JoinContext::new()` in `src/maintenance/`+`src/analysis/` production code must carry an
  inline `// join-context: <reason>` tag (builder / no-context-field / excluded-with-reason).
- 4 new tests (`maintenance_referential_integrity.rs` x3, `repair_cell.rs` x1) plus updated
  ~16 existing `append_model_edge_cells`/`admit_per_group_recompute` call sites across
  `smelt-logical`, `smelt-db`, `smelt-cli`, `smelt-runtime`.
- Spec deltas: `model_properties.md` §"Skeleton-source closure" documents the AND-over-every-
  enrichment-relation rule; `sources.md`'s stale "only one route" divergence sentence corrected.

## Decisions

- Model edges have no `referential_integrity:` of their own — widened `model_edge_enrichment_
  closure` to fold *external sources joined in the same scope* instead (the reachable fact),
  not a new model-level RI surface (rejected as out-of-mandate product decision, per the
  outcome's plan-phase decision log).
- `grouping.rs`'s closure-pruning pass, and two "no-context-field" sites (`locality.rs`,
  `choice.rs`) plus one genuinely-out-of-scope site (`rules/cumulative.rs`'s once-write FD
  route, which DOES read `has_fan_out_join` but has no declared source facts to build a real
  context from) are tagged `excluded`/`no-context-field` rather than widened — none is a
  model-edge/repair admission route in criterion 3's sense.
- `admit_per_group_recompute` needed `#[allow(clippy::too_many_arguments)]` (8 args) —
  matches the existing allow on `classify_once_write`/`derive_model_maintenance_plan_with_edges`.

## For the next planner

- `rules/cumulative.rs`'s once-write route and `locality.rs`'s route-2 FD check both read
  `vector.has_fan_out_join` off an always-empty `JoinContext` today — real fail-closed gaps,
  but neither caller currently holds declared source facts to widen with. Not scheduled by
  this outcome (out of criterion 3's route set); worth a future outcome if a real model hits it.
- No production route was found silently rebuilding an empty context outside the ones this
  phase fixed — the `join_context_reach` gate's zero-violations pass is the evidence.
- Fixture triage: no conformance/statement-parity fixture's technique or verdict moved —
  every existing fixture's SQL either had no external-source enrichment join in a model-edge
  scope, or the widened AND still resolved the same way (declared facts were already absent on
  both proofs, or already present on both).

## Gates

- `bash .claude/scripts/verify-phase.sh` — GREEN (fmt, clippy both feature sets, workspace
  tests, example_diagnostics).
- `cargo test -p smelt-logical --quiet` — all green (no fixture flips).
- `cargo test -p smelt-logical --test walk_coverage --quiet` — green.
- `cargo test -p smelt-logical --test join_context_reach --quiet` — green (new gate).
- `cargo test -p smelt-runtime --test statement_parity --quiet` — 37 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — 78 passed.
- `rg -n 'JoinContext::new\(\)' crates/smelt-logical/src` — every `src/maintenance`/`src/analysis`
  survivor tagged; `src/backbuild`/`src/rules` sites (outside test 5's own scope) tagged too,
  as documentation.
