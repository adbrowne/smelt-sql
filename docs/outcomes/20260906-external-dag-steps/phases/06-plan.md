# Phase 6 — `smelt explain` renders external steps

**Outcome:** `docs/outcomes/20260906-external-dag-steps/outcome.md` (criterion 5; also closes
criterion 3's "DAG/graph surfaces show it" for `explain`).

## Objective

Make an external step visible on the `smelt explain` surface: the whole-project text and
`--json` output carry every (selected) step as a node alongside the model graph, and
`smelt explain <step>` — the positional argument that today resolves through
`resolve_node_path` but then fails "Model not found" — renders what the step produces, how
it is invoked, which models consume it, and that smelt does not author it. `explain` stays
offline and never spawns the command, which is what makes it the non-refusing preview
surface `sources.md` §Semantics 12 already promises.

## Spec delta (first, per the spec-first rule)

- **`docs/specs/cli.md`**
  - §"`smelt explain --json` output schema": add a top-level, append-stable
    `"external_steps": { "<addr>": { "produces": [...], "command": [...], "cadence": "<display>",
    "description": "<string>", "consumers": [...] } }` block, omitted when the project declares
    none; `cadence`/`description` omitted when unset. State explicitly that steps do **not**
    appear in `execution_order` or `models` — `execution_order` stays a topological sort of
    models (Constraint 5, append-stable), and a step is ordered relative to its consumers by
    the run path, not by this list.
  - New subsection after §"`smelt explain <model>` maintenance-plan report":
    **§"`smelt explain <external step>`"** — the positional argument accepts a step address;
    the report names the produced source addresses, the literal `command:` argv (unsubstituted,
    since `explain` has no run window), the cadence, the consuming models, and the sentence that
    smelt does not author the step. `--json` emits the same as one object with
    `"kind": "external_step"`. `--show-sql`/`--period`/`--technique` are meaningless on a step
    and are rejected as usage errors (exit 2). `explain` never spawns the command.
  - §"`smelt explain` excludes tests" neighbourhood: one sentence that whole-project `explain`
    narrows the step set by `--select` through the same `select_nodes` pass `smelt list` uses.
- **`docs/specs/sources.md`** §Known Divergences: rewrite the "`smelt explain` rendering is
  unbuilt" entry to record it as landed (drop the unbuilt clause; keep the entry describing
  what is now surfaced), and §Semantics 12's `smelt explain` sentence needs no change.

## Tests (red-green)

New `crates/smelt-cli/tests/explain_external_step.rs` (`#![cfg(feature = "duckdb")]`, subprocess
driver + `scaffold` pattern copied from `tests/list_external_step.rs`):

1. `whole_project_json_carries_steps` — `explain --json` has `external_steps["loader"]` with
   `produces == ["sources.raw_events"]`, the argv `command`, and `consumers == ["consumer"]`.
2. `whole_project_json_omits_key_without_steps` — a project with no step emits no
   `external_steps` key at all (not `null`, not `{}`).
3. `execution_order_and_models_stay_model_only` — the step address appears in neither
   `execution_order` nor `models` (append-stable schema guard).
4. `whole_project_text_lists_steps` — text output carries an `External steps:` section naming
   the step and the sources it produces.
5. `select_narrows_steps` — `--select consumer` keeps the step; `--select unrelated` (a model
   reading nothing the step produces) drops it.
6. `explain_step_text_renders_contract` — `smelt explain loader` exits 0 and prints the produced
   address, the argv, the consumer, and the not-authored sentence; no maintenance-plan output.
7. `explain_step_json_shape` — `smelt explain loader --json` parses to one object with
   `kind == "external_step"` and the same fields.
8. `explain_step_never_spawns_the_command` — the step's `command:` writes a marker file; after
   both `explain` forms the marker does not exist.
9. `explain_step_rejects_plan_flags` — `smelt explain loader --show-sql` exits 2 with a message
   naming the step, rather than a "not found" or a panic.
10. `explain_unknown_target_still_not_found` — an address that is neither model nor step keeps
    today's not-found error (no regression from the new resolution branch).

Unit test in `crates/smelt-core/src/graph.rs`: `consumers_of_step_returns_source_readers`.

## Tasks

1. Land the spec delta above (`cli.md`, `sources.md`) — nothing else may precede it.
2. Add `DependencyGraph::consumers_of_step(&self, addr: &str) -> &[String]` (accessor over the
   existing `step_consumers`); unit-test it.
3. Add `ExplainExternalStep` (serde `Serialize`) + `ExplainOutput.external_steps:
   BTreeMap<String, ExplainExternalStep>` with `#[serde(skip_serializing_if = "BTreeMap::is_empty")]`
   in `crates/smelt-cli/src/explain.rs`; populate it in `build_explain_output` from the graph.
4. In `commands/explain.rs`'s whole-project path: discover steps (`discover_external_steps`),
   `graph.add_external_steps(&steps)` before `validate()`, and when `--select` is given use
   `select_nodes(...)` (models drive `execution_order` exactly as today; `.steps` narrows the
   step map) so a bare `--select` run is byte-identical to today for the model half.
5. Render the text `External steps:` section after the Logical Graph section.
6. In `explain()`'s positional branch: resolve the argument first, and when the canonical address
   matches a discovered step, dispatch to a new `explain_external_step(...)` instead of
   `explain_maintenance_plan(...)`; reject `--show-sql`/`--period`/`--technique` there with exit 2.
7. Write `explain_external_step`: text and `--json` renderings sharing one built struct.
8. Update the outcome's decision log with the phase's design call (see below), and flip the row.

## Design call this phase settles (record in the decision log)

**Steps are a separate top-level `external_steps` map, not entries in `execution_order`/`models`.**
`cli.md` §Constraints 5 makes the JSON append-stable and §"output schema" defines
`execution_order` as a topological sort of models; injecting a non-model address into a list
orchestrators feed to model-shaped tasks would break consumers for no gain, since the run path
(phase 4) already orders steps ahead of every consumer structurally rather than through this list.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test explain_external_step --test explain --test explain_model
  --test list_external_step --test cli_docs_coverage`
- `cargo test -p smelt-core --lib graph`
- `bash .claude/scripts/large-file-check.sh` (`commands/explain.rs` is 1163 lines — prefer a new
  `commands/explain_external_step.rs` module over growing it, mirroring `explain_diff.rs`)
- `cargo test -p smelt-runtime --test execute_parity`

## Commit message

`feat(explain): render external steps in the project graph and per-step report`
