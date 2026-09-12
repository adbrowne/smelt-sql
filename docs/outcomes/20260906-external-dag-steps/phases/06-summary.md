# Phase 6 summary — `smelt explain` renders external steps

**Shipped:**
- Spec delta in `docs/specs/cli.md`: `external_steps` top-level map in the `--json` schema
  (never in `execution_order`/`models`), a new `### smelt explain <external step>` section, and
  a `--select`/`--exclude` narrowing sentence. `docs/specs/sources.md` Known Divergences entry
  rewritten from "unbuilt" to landed.
- `DependencyGraph::consumers_of_step` (`crates/smelt-core/src/graph.rs`) — accessor over
  `step_consumers`, unit-tested (`consumers_of_step_returns_source_readers`).
- `ExplainExternalStep` + `ExplainOutput.external_steps: BTreeMap<..>` (`crates/smelt-cli/src/
  explain.rs`); `build_explain_output` gained a `steps: &[ExternalStepInfo]` parameter and
  populates the map from the graph's registered steps + `consumers_of_step`. All 11 existing
  call sites updated to pass `&[]`.
- `commands/explain.rs`: discovers steps, calls `graph.add_external_steps`, and — when `--select`
  is given — uses `select_nodes` instead of `select_models` so the step set narrows through the
  same pass `smelt list` uses; a bare run's model half is unchanged. Text output gained an
  `External steps:` section.
- New `crates/smelt-cli/src/commands/explain_external_step.rs`: resolves the positional argument
  via `resolve_argument` (which already covers steps through `resolve_node_path`), and when it
  names a step, renders text/JSON (`kind: "external_step"`, `address`, produces/command/cadence/
  description/consumers) instead of falling through to the maintenance-plan path. Rejects
  `--show-sql`/`--period`/`--technique` with exit 2 (`UsageFlagOnStep`, wired into `main.rs` via
  `commands::explain_external_step::exit_code_for`). Never spawns `command:`.
- `crates/smelt-cli/tests/explain_external_step.rs`: all 10 planned tests, green.

**Decisions:**
- Recorded in outcome.md decision log (2026-09-08, phase 6 planning): separate top-level
  `external_steps` map, never folded into `execution_order`/`models`.
- Resolution reuses `resolve_argument`/`resolve_node_path` as-is — no new resolution code needed;
  steps were already selector-addressable from phase 3. The dispatcher only needed to check
  which discovered `ExternalStepInfo` the resolved canonical address names.
- Kept `commands::explain_external_step::dispatch` returning `Result<bool>` (handled vs.
  fall-through) rather than restructuring `explain_maintenance_plan`'s resolution — the dispatcher
  does its own lightweight discovery+db-init preamble (mirroring `explain_maintenance_plan`'s),
  accepting the small duplication in exchange for zero risk to the existing, large,
  well-tested maintenance-plan function.

**For the next planner (phase 7 — fixture and docs):**
- The `examples/github_activity/` fixture from phase 7 will exercise this rendering for real;
  no gap found here that phase 7 needs to additionally cover.
- Hardening baseline (`.claude/hardening-baseline.txt`) bumped: `smelt-cli println` 176→188,
  `expect` 42→44 — all in the new CLI report surface and the mirrored `Workspace::try_get`/
  `project_input` pattern; sign-off note added inline. Large-file baseline bumped for
  `commands/explain.rs` (1163→1193), `explain.rs` (2528→2578), `smelt-core/src/graph.rs`
  (1501→1553) — mechanical growth from the new field/module/accessor, no split attempted.
- Not built: a step whose canonical address collides with something that also matches a leaf
  `did_you_mean` hint — untested edge case, but no code path treats it specially since
  `resolve_argument`'s ambiguity detection already covers cross-kind collisions from phase 3.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test explain_external_step --test explain --test explain_model
  --test list_external_step --test cli_docs_coverage` — all green (10 + 4 + 27 + 2 + 3 tests).
- `cargo test -p smelt-core --lib graph` — 29/29 green.
- `bash .claude/scripts/large-file-check.sh` — green after baseline update (sign-off above).
- `cargo test -p smelt-runtime --test execute_parity` — 4/4 green.
