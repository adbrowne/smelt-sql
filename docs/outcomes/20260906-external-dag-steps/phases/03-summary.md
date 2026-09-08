# Phase 3 summary — DAG membership

## Shipped

- `crates/smelt-core/src/graph.rs` — `DependencyGraph` gains `external_steps`,
  `source_producer`, `step_consumers` state; `add_external_steps(&[ExternalStepInfo])`;
  accessors `iter_external_steps`, `producing_step_of_source`, `steps_required_by`;
  `NodeSelection { models, steps }` + `select_nodes(selectors, config)`. `select_models`
  is now `select_nodes(..).models`.
- `crates/smelt-core/src/resolver.rs` — `EntityRefKind::ExternalStep`; `resolve_address_map`
  takes an `&[ExternalStepInfo]` slice and registers each step, so a step address colliding
  with a model/seed/source is now a `DuplicateAddress` diagnostic.
- `crates/smelt-db/src/queries/project.rs` — `project_external_steps` tracked query
  (mirrors `project_sources`); `project_source_diagnostics` now consumes it instead of
  re-discovering inline; `project_address_collisions` passes the step slice through.
- `crates/smelt-db/src/resolve.rs` — `resolve_node_path` = `resolve_ref_path` ∪ steps.
  `resolve_ref_path` itself is untouched (still step-free — a step is never a
  `smelt.ref()` target).
- `crates/smelt-cli/src/argument_resolution.rs` — `resolve_argument` resolves through
  `resolve_node_path`, so a bare CLI/selector argument naming a step's address resolves.
- `crates/smelt-cli/src/commands/list.rs` — `EntityKind::ExternalStep`; steps are
  discovered, registered on the graph, and narrowed by `--select`/`--exclude` through
  `select_nodes` (unlike seeds/sources, which stay always-listed-in-full); `--json`
  carries a `produces` array.
- Spec deltas: `docs/specs/model_selection.md` (§"Graph traversal", §"Selection methods",
  a new Constraints item 7), `docs/specs/cli.md` (§"`smelt list`", command table row),
  `docs/specs/sources.md` (Known Divergences narrowed to "a graph node and selectable,
  but not yet invoked").
- Tests: `crates/smelt-core/tests/external_step_graph.rs` (7 cases, all from the plan),
  `crates/smelt-core/tests/address_map.rs` (+1, the step/model collision case),
  `crates/smelt-db/tests/integration/external_step_diagnostics.rs` (+2, the
  `resolve_ref_path`/`resolve_node_path` two-seam split), `crates/smelt-cli/tests/list_external_step.rs`
  (2 cases: JSON `kind`/`produces`, and `--select loader` vs `--select +consumer`).
- `discover_external_step_errors` dropped its unused `_paths` parameter (02-summary.md's
  flagged follow-up — phase 3 confirmed it stays unneeded).

## Decisions

- **Steps are reached only through their sources, never as direct dependency edges.** A
  model's `smelt.sources.*` ref segments equal a per-entity source's own `address_segments`
  (confirmed by reading `discover_source_infos`/`validate_external_steps`), so
  `source_producer` keys off that shared dot-joined form with no extra translation layer.
- **Upstream traversal recomputes the closure rather than reusing the selector's own
  `models` accumulation.** Simpler to reason about (one self-contained pass per selector)
  than threading a "what did this selector just add" delta through the existing loop; cost
  is a second `collect_upstream` walk, negligible at CLI-invocation scale.
- **`select_models` becomes a thin wrapper over `select_nodes(..).models`**, per the plan —
  confirmed byte-identical with and without steps registered via
  `select_models_unchanged_when_steps_registered`.
- **`.claude/large-file-baseline.txt` updated** for `graph.rs` (+168 lines: the DAG-membership
  machinery this phase's objective *is*) and `project.rs` (+19: one new tracked query).
  Sign-off: implementer judges both mechanical to the phase's stated scope, not scope creep.

## For the next planner

- Phase 4 (invocation on the run path) needs `steps_required_by` — already shipped here —
  to order a step ahead of its consumers. It also needs to decide the `command:`
  placeholder-substitution grammar (`{run_date}`), unchanged from the outcome's phase table.
- `select_nodes` currently gives upstream-from-a-step (`+loader`) no special handling (a
  step has no upstream in this graph — it's a producer, not a consumer of anything smelt
  models). Not tested; if phase 4/5 need `+loader` to mean something, that's new scope, not
  a gap in this phase's contract.
- `smelt explain` still doesn't render steps at all (unbuilt, phase 5) — `smelt list` is
  the only surface reaching them today.

## Gates

- `cargo test -p smelt-core --test external_step_graph --test address_map --test external_step_yaml` — 9 + 7 + 14 passed.
- `cargo test -p smelt-db --test integration external_step_diagnostics` — 4 passed.
- `cargo test -p smelt-cli --test list_external_step --test list_clean --test cli_docs_coverage --test example_diagnostics` — all passed (127 example_diagnostics, 1 pre-existing ignore).
- `cargo test -p smelt-lsp --test example_workspaces` — 37 passed, no regression.
- `cargo test -p smelt-runtime --test execute_parity` — 4 passed.
- `cargo test -p smelt-core --test hardening_budget` — 5 passed (ratchet unmoved).
- `bash .claude/scripts/large-file-check.sh` — updated per Decisions above; re-ran green.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full `cargo test`, `example_diagnostics`).
