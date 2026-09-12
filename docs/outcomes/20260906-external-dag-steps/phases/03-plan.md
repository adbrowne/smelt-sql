# Phase 3 plan — DAG membership

## Objective

Make a declared external step a **node**: a graph entity keyed by its own address, carrying an
edge to each source it produces and, through those sources, to every model that reads them; and
make it reachable through the selector surface and `smelt list` exactly as any other node.
Advances success criterion 3, and hands phase 4 the ordering primitive (`steps_required_by`) it
needs for run-path invocation. Invocation, run reporting and `smelt explain` stay out (phases 4–5).

## Design calls settled here

- **The step participates in `resolve_address_map`** (the question `02-summary.md` left open).
  A step is now selector-addressable, so a step address silently colliding with a model/seed/
  source address would make selection ambiguous with no diagnostic. `EntityRefKind` gains an
  `ExternalStep` variant and steps are registered alongside the other three kinds.
- **Node resolution is a distinct seam from ref resolution.** A step is *not* a `smelt.ref()`
  target — a model references the source, never its producer. So `resolve_ref_path` stays
  step-free (a SQL ref to a step keeps failing `UndefinedModelRef`, fail-loud), and CLI argument
  resolution goes through a new `smelt_db::resolve_node_path` = `resolve_ref_path` ∪ steps.

## Spec delta (made first, by the implement step)

- `docs/specs/model_selection.md` — §"Graph traversal": upstream traversal from a model reaches
  the producing step of any source it reads; downstream traversal from a step reaches every
  consumer of the sources it produces (and their downstreams). §"Selection methods": a
  `ModelName` selector resolving to a step selects the step. Add a §Constraints item: **a step is
  a selectable node but never a `smelt.ref()` target.**
- `docs/specs/cli.md` — §"`smelt list`" and the command table row: external steps are listed
  (kind `external_step`), and are narrowed by `--select`/`--exclude` like models.
- `docs/specs/sources.md` — §Known Divergences: narrow the external-step entry from "not yet
  ordered or invoked" to "a graph node and selectable, but not yet invoked"; §Semantics item 9
  keeps ownership of the ordering rule (unchanged wording).

## Tests (red-green)

`crates/smelt-core/tests/external_step_graph.rs`
1. `step_registers_as_node_with_edge_per_produced_source` — after `add_external_steps`, the step's
   canonical address is a node and `producing_step_of_source` answers for every `produces:` entry.
2. `model_reading_a_produced_source_is_a_consumer` — consumers are derived from the model's own
   `smelt.sources.*` refs, not from the source's YAML.
3. `upstream_selector_on_consumer_includes_the_step` — `select_nodes(["+consumer"])` returns the
   step in `.steps`, transitively through an intermediate model too.
4. `downstream_selector_on_step_includes_consumers` — `select_nodes(["step+"])` returns the
   consumer and its downstreams in `.models`.
5. `bare_step_selector_selects_only_the_step` — no "not found" error; `.models` empty.
6. `steps_required_by_selection` — given an arbitrary selected model set (what the run path holds,
   not a selector), the required step set is exactly the producers of sources those models read.
7. `select_models_unchanged_when_steps_registered` — the existing model-only API is byte-identical
   with and without steps registered (no caller breakage).

`crates/smelt-core/tests/address_map.rs`
8. `step_address_colliding_with_a_model_is_a_collision` — reported with both kinds named.

`crates/smelt-db/tests/integration/external_step_diagnostics.rs`
9. `resolve_node_path_resolves_a_step` / `sql_ref_to_a_step_is_undefined` — the two-seam split.

`crates/smelt-cli/tests/list_external_step.rs`
10. `list_shows_external_step_kind` — `--json` carries `kind: "external_step"` and `produces`.
11. `list_select_downstream_model_includes_producing_step` — and `--select <step>` lists just it.

## Tasks

1. Spec delta above (three files) — first commit-shaped change, per the spec-first rule.
2. `smelt-db`: add a `project_external_steps` tracked query mirroring `project_sources`; refactor
   `project_source_diagnostics` to consume it instead of discovering inline.
3. `smelt-db`: add `resolve_node_path` (ref resolution ∪ steps) and export it; leave
   `resolve_ref_path` untouched.
4. `smelt-core/resolver.rs`: `EntityRefKind::ExternalStep`; `resolve_address_map` takes the step
   slice and registers each; update the three call sites.
5. `smelt-core/graph.rs`: `external_steps` + `source_producer` + derived `step_consumers` state;
   `add_external_steps(&[ExternalStepInfo])`; accessors `iter_external_steps`,
   `producing_step_of_source`, `steps_required_by(&HashSet<String>) -> Vec<String>` (sorted).
6. `smelt-core/graph.rs`: `NodeSelection { models, steps }` + `select_nodes(selectors, config)`;
   reimplement `select_models` as `select_nodes(..).models` so every existing caller is unchanged.
7. CLI `argument_resolution.rs`: resolve through `resolve_node_path`.
8. CLI `list.rs`: `EntityKind::ExternalStep`, discover steps via
   `smelt_core::discover_external_steps`, register them on the graph, narrow them with the same
   selector pass as models, emit `produces` in the entry.
9. If `discover_external_step_errors`'s unused `_paths` parameter is still unused after this
   phase, drop it (`02-summary.md` follow-up).

## Verification

- `cargo test -p smelt-core --test external_step_graph --test address_map --test external_step_yaml`
- `cargo test -p smelt-db --test integration external_step_diagnostics`
- `cargo test -p smelt-cli --test list_external_step --test list_clean --test cli_docs_coverage --test example_diagnostics`
- `cargo test -p smelt-lsp --test example_workspaces`
- `cargo test -p smelt-runtime --test execute_parity`
- `bash .claude/scripts/verify-phase.sh` and `bash .claude/scripts/large-file-check.sh`
- `cargo test -p smelt-core --test hardening_budget` (ratchets unmoved)

## Commit message

`feat(sources): make an external step a selectable node in the DAG`
