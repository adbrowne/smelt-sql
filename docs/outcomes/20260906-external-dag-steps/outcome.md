# Outcome: An externally-produced relation is a node in smelt's DAG

**Created:** 2026-09-06
**Status:** active
**Driver:** outcome loop (`.claude/outcome-backlog`)
**Source:** `docs/research/20260906-bigquery-dogfood.md` §"Black-box steps in the DAG", §Open questions 2
**Spec anchors:** `docs/specs/sources.md`; `docs/specs/models.md`; `docs/specs/model_selection.md`; `docs/specs/run_state.md`; `docs/specs/diagnostics.md`

## The outcome

A relation smelt does not author but does depend on — the GitHub-activity loader is the
motivating case, but any externally-produced table is the same shape — is declared as a
**black-box step**: a node smelt orders in the DAG, invokes when a run reaches it, and
treats as the producer of a declared source. smelt never authors its SQL and never
inspects its internals; the source declaration is the whole contract for what it produces
and where. A run that selects a downstream model runs the step first; a run that cannot
invoke it says so with a named diagnostic rather than reading a stale table silently. The
step's failure is a run failure, its success advances the source's frontier, and both are
visible in the run report.

## Success criteria (checkable)

1. **Spec first.** `docs/specs/sources.md` gains the normative surface — the declaration
   shape, what smelt guarantees (ordering, invocation, failure propagation) and what it
   explicitly does not (authorship, retries beyond the existing policy, idempotence of the
   external program) — written to the timeless-oracle rule. The open question the research
   doc leaves (a `produced_by:` key on today's source YAML, versus a distinct declaration
   kind) is decided in this outcome's decision log with reasoning, before any code.
2. **Declaration and validation.** The declaration parses, and every malformed form is
   refused with a named `DiagnosticCode` exercised by a fixture under `examples/broken/`
   — never a silent default, per the fail-loud discipline. `diagnostics_catalogue` green.
3. **DAG membership.** The step is a node: `smelt run` selecting a downstream model runs
   it first; `smelt list` and the DAG/graph surfaces show it; model selection
   (`docs/specs/model_selection.md`) reaches it through the same selectors as any node.
4. **Invocation and failure.** A run invokes the step, propagates a non-zero exit as a run
   failure with the step named, and leaves downstream models unbuilt. A run that is not
   permitted to invoke it (no command, dry run, or an environment that cannot) refuses
   with a named code rather than proceeding against a possibly-stale table.
5. **Explain.** `smelt explain` (text and `--json`) renders the step: what it produces,
   how it is invoked, and that smelt does not author it. `cli_docs_coverage` green.
6. **Fixture and docs.** An example workspace carries a black-box step and a model reading
   its source, with zero diagnostics (`example_diagnostics`, `example_workspaces`); a
   docs-site page documents the declaration, the contract, and the failure modes.
7. **Gates green.** `bash .claude/scripts/verify-phase.sh`; `execute_parity`;
   hardening ratchets unmoved.

## Out of scope

- Authoring, generating, or type-checking the external program.
- Scheduling it independently of a smelt run (Cloud Scheduler and friends belong to
  `20260906-bigquery-unattended`).
- Incremental reasoning *about* the step's internals — smelt observes what the source
  declaration claims, exactly as it does today for any source.
- Retention semantics for what the step produces — owned by
  `20260906-trimmed-history-sources`.
- A general plugin/hook system. This is one declaration kind for one contract.
- Fixing `smelt list --format json`'s `ListError::ParseErrors` over non-model root SQL, and the
  missing `concepts/incremental-equivalence.md` nav entry — both pre-existing, both reproduce
  without any of this outcome's changes, neither touches the external-step contract. Recorded in
  `docs/TODO.md` and the spine handoff by phase 9.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Decide the declaration shape (`produced_by:` on a source vs. a distinct kind) with reasoning in the decision log, then land the spec delta in `docs/specs/sources.md` | done |
| 2 | Parse and validate the step declaration in `smelt-core` — discovery, discriminator, `produces:`/`command:`/cadence, one named `DiagnosticCode` per malformed form with `examples/broken/` fixtures, catalogue rows in `docs/specs/diagnostics.md` | done |
| 3 | DAG membership — the step is a graph node with an edge to each source it produces; `smelt list`, the graph/DAG surfaces and model selection reach it through the same selectors as any node | done |
| 4 | Invocation on the run path — decide and spec the `command:` placeholder-substitution grammar (`{run_date}`), order the step ahead of its consumers, invoke it, propagate a non-zero exit as a run failure naming the step with downstream models unbuilt, and refuse (named code) when the run may not invoke it | done |
| 5 | Run-path reporting — `RunReporter` gains step start/completed/failed callbacks, the CLI renders them, and the run manifest/report artifact records every step a run invoked (spec delta in `run_state.md`) | done |
| 6 | `smelt explain` — the whole-project text and `--json` output carry external steps as nodes, and `smelt explain <step>` renders what it produces, how it is invoked, and that smelt does not author it; `cli_docs_coverage` green | done |
| 7 | Fixture — `examples/github_activity/` declares its loader as a step producing both raw sources at zero diagnostics, and actually runs it: one extracted day-loader program, invoked by `smelt run`, with the replay/oracle drivers rewired onto it and the `duckdb` CLI provisioned in CI | done |
| 8 | Docs — docs-site page covering the declaration, the contract and the failure modes, cross-linked from the sources guide/reference and added to the nav | done |
| 9 | Close-out — verify each success criterion's evidence at HEAD, hold the ratchets, hand findings back to `20260906-bigquery-dogfood-spine` | planned |

## Decision log

- 2026-09-08 (phase 9 planning): **no reshape — the phase table is final.** Row 9 is the
  close-out and every other row is `done`; nothing phase 8's summary surfaced serves a success
  criterion in a way an existing row does not already cover. Two calls settled about what
  close-out *is*: (a) it re-runs each criterion's gate at HEAD and records a criterion → evidence
  table, rather than trusting the summaries — in particular `execute_parity`, which criterion 7
  names and phase 8 declined to re-run; (b) the one asymmetry phase 8 flagged
  (`docs-site/docs/reference/cli.md`'s `smelt explain` section never naming the step form, though
  `smelt explain <step>` is spec'd in `cli.md`) is **fixed here with a gate test**, not deferred:
  it is a docs-vs-behaviour gap inside criterion 5/6's surface. The two genuinely unrelated
  residues (the `smelt list` `ListError::ParseErrors` scoping bug over project-wide-discovered
  root SQL, and `concepts/incremental-equivalence.md` missing from the docs-site nav) go to
  `docs/TODO.md` and the spine handoff rather than growing a row here — see "## Out of scope".

- 2026-09-08 (phase 8 planning): **no reshape; two docs-placement calls settled.** (a) The
  page is a **Guide** page (`docs-site/docs/guide/external-steps.md`, nav directly after
  Sources), with the key table mirrored into `reference/sources-yml.md` — the same
  guide/reference split sources already use, rather than a reference-only page: the
  declaration is cheap but the *contract* (what smelt guarantees vs. what stays the
  external program's problem) is the part users get wrong, and that is guide material.
  (b) `guide/sources.md` §"Loading source data" — today a flat "smelt does not load source
  data" — is **corrected**, not merely cross-linked: it is now false as an absolute, and
  leaving it would contradict the new page. That is a user-doc correction, not a spec
  change; no spec delta is needed, since phases 1/5/6 already landed the whole normative
  surface. Row 9 (close-out) unchanged; nothing left the outcome.

- 2026-09-08 (phase 7 implementation): **shipped as planned, no reshape. One
  new discovery, unrelated to external steps, deferred rather than fixed.**
  `examples/github_activity/load_day.sh` is the single day-loader
  implementation, idempotent per day via `main._loader_days` (a day already
  recorded is a no-op), with the previous-day 2% redelivery expressed as a
  SQL interval (`DATE '{d}' - INTERVAL 1 DAY`) rather than shell date
  arithmetic — a day at the start of the fixture range needs no special
  case, since the redelivery predicate simply matches zero rows.
  `models/sources/raw/github_loader.yml` declares it, producing both raw
  sources. `run_incremental.py`'s `load_day` became `redelivered_count`
  (a post-run read against the parquet fixture, since the load itself now
  happens inside `smelt run`); `github_activity_support::load_day` now
  shells out to the same script (call sites unchanged); `replay_days` and
  the two gated oracle incremental loops (`every_window_matches_the_full_
  refresh_oracle`, `every_window_deep_sweep`) dropped their manual pre-load,
  relying on the step; the oracle's direct multi-day staging loops
  (building the full-refresh comparison side) were left untouched, per the
  plan. `.github/actions/setup-duckdb/action.yml` and `mise run setup-duckdb`
  now also install the `duckdb` CLI. All 9 planned tests pass, plus the full
  existing `github_activity_replay`/`github_activity_oracle` suites (37
  tests) unaffected. **Discovery**: `smelt list --format json` hard-fails
  with `ListError::ParseErrors` on `examples/github_activity` (and
  identically on `examples/web_analytics` and `examples/retail_analytics`) —
  `crates/smelt-cli/src/commands/list.rs` treats every project-wide-
  discovered SQL file's parse errors as fatal, not just the selected set,
  and root-level utility scripts (`sample.sql`, `setup_sources.sql`) aren't
  valid smelt models. Pre-existing, reproduced identically without any of
  this phase's changes (verified via a temporary stash), unrelated to
  external steps. Test 6 (`github_activity_declares_its_loader_step`) uses
  `smelt explain --json` instead, which — like `smelt run` — only compiles
  what selection reaches. Left for whoever next touches `smelt list`: either
  scope `ListError::ParseErrors` to the selected set, or give `load_workspace`
  callers a way to exclude non-model root SQL from the project-wide walk.
- 2026-09-08 (phase 7 planning): **reshape — the old row 7 is split in two**, and the fixture
  half grew a real design. Split rationale: "declare the step in the fixture" and "write the
  docs-site page" share no code, no gate and no failure mode, exactly like the phase-5 split;
  rows 7 (fixture) and 8 (docs), old row 8 renumbered to 9. Nothing left the outcome. The
  fixture half is bigger than the row implied, because a step declared in
  `examples/github_activity/` is *invoked* by every `smelt run` the replay and oracle tests
  make — so the fixture cannot merely declare a loader, it has to be driven by one. Three
  calls settled: (a) the loader is `load_day.sh` (bash + the `duckdb` CLI), since CI installs
  only `libduckdb.so` and has neither the CLI nor the DuckDB Python module — the phase
  provisions the CLI rather than gating the fixture's tests off, and a Rust loader binary was
  rejected for putting fixture code in a shipped crate and inside the hardening ratchet's
  "production" derivation; (b) the loader carries **its own per-day ledger and is idempotent
  per day**, which is load-bearing rather than decorative — the oracle leg stages N days
  without running smelt N times and then runs `--full-refresh` (whose step invocation would
  otherwise re-append a staged day), and `run.rs`'s propagated-region loop calls
  `execute_project`, and therefore the step, once per region; smelt guarantees no idempotence
  of the external program, so this is the loader's own bookkeeping, opaque to smelt, exactly
  as a real at-least-once day loader would carry it; (c) the day-load semantics (the 2%
  `MOD(id,50)=0` redelivery of D-1, stamped `ingested_date = D` on the arrival twin) collapse
  from three drifting copies — `run_incremental.py`, the Rust test helper, and the BigQuery
  shell loader — to one, pinned by a test.

- 2026-09-08 (phase 6 implementation): **shipped as planned, no reshape.** `docs/specs/cli.md`
  gained the `external_steps` JSON schema block and the `### smelt explain <external step>`
  section; `docs/specs/sources.md`'s "unbuilt" divergence entry now records it landed.
  `DependencyGraph::consumers_of_step` and `ExplainOutput.external_steps`/`ExplainExternalStep`
  are the new production types; `commands/explain.rs` wires discovery + `select_nodes` narrowing
  + the text section; the new `commands/explain_external_step.rs` owns positional-argument
  resolution and rendering for a step (reusing `resolve_argument`/`resolve_node_path` unchanged
  from phase 3 — no new resolution code needed). All 10 planned tests plus the existing
  `explain`/`explain_model`/`list_external_step`/`cli_docs_coverage`/`execute_parity` suites pass.
  Hardening and large-file baselines bumped with sign-off notes for the new CLI report surface
  (println/expect) and the mechanical line growth in `explain.rs`/`commands/explain.rs`/
  `graph.rs`. See `phases/06-summary.md`.
- 2026-09-08 (phase 6 planning): **no reshape; one design call settled.** External steps are
  rendered as a **separate top-level `external_steps` map** in `smelt explain --json` (and a
  distinct `External steps:` text section), **not** as entries in `execution_order`/`models`.
  `cli.md` §Constraints 5 makes the explain JSON append-stable and defines `execution_order` as a
  topological sort of *models*; orchestrators feed that list to model-shaped tasks, so injecting a
  non-model address would break them for no gain — the run path (phase 4) already orders a step
  ahead of every consumer structurally, not through this list. Also settled: `smelt explain <step>`
  rejects `--show-sql`/`--period`/`--technique` as usage errors (exit 2) rather than silently
  ignoring them, and `explain` never spawns the `command:` — which is what lets §Semantics 12 keep
  naming it the non-refusing preview surface for a step. Rows 7-8 unchanged; nothing left the outcome.

- 2026-09-08 (phase 5 implementation): **shipped as planned, no reshape.** `RunManifest`/
  `RunReport` gained `external_steps: BTreeMap<String, ExternalStepRunRecord>`
  (`#[serde(default, skip_serializing_if)]`); `RunReporter` gained `external_step_started`/
  `_completed`/`_failed`; `invoke_required_steps` now fires them and returns the successfully-
  invoked steps' records for `execute/project/mod.rs` to fold into the manifest; `CliReporter`
  renders all three. All 8 planned tests pass against a real DuckDB backend
  (`crates/smelt-runtime/tests/external_step_reporting.rs`, plus 2 pure unit tests in
  `smelt-state`). One incidental finding: the hardening-budget `println!` ratchet's substring
  match also counts `eprintln!`, so the one new CLI `eprintln!` (the failure line) bumped
  `smelt-cli println` 175→176 — baseline updated with a sign-off note, not a real new `println!`.
  Three already-oversized files grew a handful of lines each from adding the new field to
  existing struct literals — baseline bumped for all three, no split attempted (mechanical, not
  scope creep). See `phases/05-summary.md`.
- 2026-09-08 (phase 5 planning): **reshape — the old row 5 is split in two**, and one design
  call settled. The row bundled two surfaces with separate spec anchors, separate gates and
  no shared code (`RunReporter`/`smelt-state` on one side, `smelt-cli/src/commands/explain.rs`
  on the other); split into row 5 (run-path reporting) and row 6 (`smelt explain`), with the
  old rows 6-7 renumbered to 7-8. Nothing left the outcome. The design call: **a step failure
  produces no run-report artifact, by construction, and is surfaced through the reporter and
  the CLI's run output instead.** Required steps are invoked at the top of `execute_project`
  (before plan derivation, per the phase-4 decision that plan decisions must not be derived
  from frontiers a step has not advanced), which is ~330 lines before the `FileStore` and
  ~680 before the `RunManifest` exists; a step failure therefore aborts exactly like any other
  pre-execution failure (a bad target, a selection error), which also writes no report today.
  Rejected: hoisting `FileStore`/manifest construction above the step pass — it would write
  state artifacts outside the advisory lock that is acquired later, to record a run that never
  reached a model. The manifest's new `external_steps` record therefore covers the steps a run
  *successfully* invoked; the failure leg is the reporter's `external_step_failed` plus
  `run_failed`, both of which the CLI prints.

- 2026-09-08 (phase 4 implementation): **shipped as planned, no reshape.** Closed
  `{run_date}`/`{run_end}` placeholder grammar landed in `smelt-core::external_step`
  (declaration-time rejection of an unknown placeholder, pure `resolve_command`);
  `SelectionPlan::required_steps` landed in `select.rs`; a new
  `execute/external_steps.rs` invokes required steps sequentially before
  `build_model_plans`, covering both the dry-run and live paths; `ExecuteRequest`
  gained `invoke_external_steps` (default `true`). 11/11 planned tests pass against a
  real DuckDB backend (one test additionally gates on the `duckdb` CLI being on PATH,
  since no other test in this repo already depends on that binary — it skips
  gracefully, mirroring the Spark/BigQuery-gated-test posture, rather than failing CI
  runners that provision only `libduckdb.so`). No UI call site sets
  `invoke_external_steps: false` yet — the "plan-preview endpoint" the phase-4-planning
  decision log (below) names does not exist in `smelt-ui` today; left for whoever builds
  it. See `phases/04-summary.md`.

- 2026-09-08 (phase 4 planning): **five design calls settled, no reshape.** (a) The
  `command:` **placeholder grammar** is a closed set — `{run_date}` and `{run_end}`,
  substituted per argv element, `{{`/`}}` escaping to literal braces; an unknown `{name}`
  is rejected at *declaration* time (`MalformedExternalStep`, so the LSP shows it) rather
  than at run time, and a placeholder the run has no value for is `ExternalStepNotInvocable`.
  Closed rather than open (no arbitrary env interpolation) because the step's contract is
  "smelt never parses the command" — a substitution set smelt cannot enumerate could not be
  validated fail-loud. (b) **A dry run refuses**, per §Semantics 12 as landed in phase 1: a
  preview whose plan decisions are derived from frontiers the step has not advanced is
  exactly "proceeding against a possibly-stale table" in the plan-derivation sense. The
  consequence is real and accepted — the UI's plan-preview endpoint refuses on a workspace
  whose selection reaches a step — and `smelt explain` (phase 5) is named in the spec as the
  non-refusing preview surface for a step. (c) The "environment that cannot execute" leg is
  one `ExecuteRequest.invoke_external_steps` field defaulting true, **not** a new CLI flag:
  embedders (UI) opt out, the CLI surface stays unchanged until a user actually needs it.
  (d) **Required steps run in one sequential pass before the model loop**, not interleaved
  into `execution_waves` — ordering ahead of every consumer is then structural rather than
  scheduled, and no wave logic changes. (e) §Semantics 10 (**a step's success advances the
  produced sources' frontier**) needs **no code**: there is no recorded source-frontier state
  to advance — a source's frontier is derived from its landed data on the next consuming
  read — so the guarantee holds by construction. Recorded here rather than given a phase row.
  Rows 5-7 unchanged; nothing left the outcome.

- 2026-09-08 (phase 3 implementation): **shipped as planned, no reshape.** `DependencyGraph`
  gained `add_external_steps`/`select_nodes`/`steps_required_by`; `select_models` is now a
  thin wrapper over `select_nodes(..).models`, confirmed byte-identical with/without steps
  registered. `smelt list` narrows external steps by `--select`/`--exclude` (unlike
  seeds/sources, which stay always-listed-in-full) and emits `produces` in `--json`. See
  `phases/03-summary.md`.

- 2026-09-08 (phase 3 planning): **two design calls settled, no reshape.** (a) `ExternalStep`
  **does** participate in `resolve_address_map` (the question `phases/02-summary.md` left open):
  once a step is selector-addressable, a step address colliding with a model/seed/source address
  would make selection ambiguous with no diagnostic, so `EntityRefKind` gains the variant and
  steps register alongside the other three kinds. (b) **Node resolution is a distinct seam from
  ref resolution** — a step is not a `smelt.ref()` target (a model references the produced source,
  never its producer), so `resolve_ref_path` stays step-free and CLI argument resolution moves to
  a new `resolve_node_path` = refs ∪ steps; a SQL ref naming a step keeps failing
  `UndefinedModelRef` rather than silently resolving. Phase table rows 4-7 unchanged; nothing left
  the outcome.

- 2026-09-08 (phase 2 implementation): **shape landed** — `crates/smelt-core/src/external_step.rs`
  parses and validates `external_step:` declarations fail-loud (`MalformedExternalStep`,
  `SourceProducerConflict`), wired into `project_source_diagnostics` as a third pass and into
  `resolver::classify` as `EntityKind::ExternalStep` (checked before the seed-sidecar tiebreaker).
  `command:` argv-list validation, the cross-entity `produces:` resolution, and six
  `examples/broken/` fixtures are all in place. DAG membership, invocation, and reporting are
  untouched (phases 3-5). Left open for phase 3: whether `ExternalStep` participates in
  `resolve_address_map`'s cross-kind collision detection. See `phases/02-summary.md`.
- 2026-09-08 (phase 2 planning): **cross-entity validation placement decided** (the question
  phase 1's summary left open) — per-file shape checks live in `parse_external_step_yaml`; the
  two checks needing the whole project (a `produces:` address resolving to no declared source,
  and two steps naming one source) live in a pure `validate_external_steps` called as a second
  pass from the Salsa query, mirroring how `project_source_diagnostics` already runs the
  per-target `name:`-key check. Phase 2 registers only the two parse-time codes
  (`MalformedExternalStep`, `SourceProducerConflict`) in the enum and catalogue; the two
  run-path codes arrive with their behaviour in phase 4, so no variant sits unused.
- 2026-09-08 (phase 2 planning): **reshape — row 4 widened**, not added: the `command:`
  placeholder grammar (`{run_date}`) that phase 1's summary flagged as unspecified is folded
  into phase 4's row, since it is the invocation phase's own question and serves criterion 4.
  No work left the outcome.

- 2026-09-08 (phase 1 implementation): **spec delta landed** in `docs/specs/sources.md` —
  §Surface `### Externally-produced sources (black-box steps)`, four diagnostic rows, six
  §Semantics items, a §Design rejected-alternative paragraph, three §Constraints items, one
  §Known Divergences entry, §References links. No code changes (spec-only phase). See
  `phases/01-summary.md` for the full list and follow-ups handed to phase 2.
- 2026-09-08 (phase 1 planning): **decided — a distinct declaration kind, not a `produced_by:`
  key on a source.** Reasoning, from evidence the spine actually produced rather than from the
  scaffold's guess: (a) `scripts/bq-dogfood-loader.sh` populates **two** relations
  (`raw.github_events` and `raw.github_events_arrival`) in one invocation, which the scaffold
  named as the shape a per-source key cannot express — a key on each source would duplicate the
  producer identifier and let the two copies drift, and smelt's graph would carry two nodes for
  one invocation; (b) the source YAML grammar is shared verbatim with seed sidecars
  (`docs/specs/seeds.md`), so a key that is meaningful on exactly one of the two overloads a
  grammar the resolver already disambiguates by a sibling-`.csv` rule; (c) the direction chosen
  — the step names the sources it `produces:`, sources stay untouched — makes "at most one
  producing step per source" a single validation over the step set, and adding a step needs no
  edit to any source. Conversely the source keeps the whole contract for *what* is produced
  (schema, world-facts, and `mutation_profile.key_recurrence`, which already carries the
  handoff's requirement (c)); the step carries only *who* produces it, *how* it is invoked, and
  its cadence (requirements (a) and (b)). Written up in phase 1's spec delta.
- 2026-09-08 (phase 1 planning): **phase table reshaped** from the scaffold's placeholder row 2
  into rows 2-7, derived from `docs/handoffs/2026-09-08-github-activity-findings.md`
  §"Requirements handed to `20260906-external-dag-steps`" and this outcome's success criteria —
  one row per criterion 2-6 plus a close-out. Nothing left the outcome.
- 2026-09-08 (bigquery-dogfood-spine phase 15): **the interim findings handoff now
  exists** at `docs/handoffs/2026-09-08-github-activity-findings.md`, covering the
  DuckDB half only ("Requirements handed to `20260906-external-dag-steps`" section) — the
  real loader's shape and what a `produced_by:`-style declaration would need to express to
  replace it. The live-BigQuery half lands in that outcome's phase 16.
- 2026-09-06 (scaffold): **deliberately short.** Only the spec-decision phase is written.
  The phase list is completed by the phase-1 planner once
  `docs/outcomes/20260906-bigquery-dogfood-spine` has produced a real loader and its
  findings handoff names what the contract actually has to carry. Scaffolding a full phase
  table now would freeze the shape against an imagined loader — the failure mode the
  outcome loop exists to avoid.
- 2026-09-06 (scaffold): open question for the human, to settle in phase 1 —
  the research doc's §Open questions 2. A `produced_by:` key reuses the source's existing
  contract surface and costs one field; a distinct declaration kind separates "a relation
  someone else fills" from "a relation smelt builds" at the type level. The spine's loader
  is a scheduled BigQuery query, which is close enough to a source that the cheap answer
  may be right — but the research doc notes the feature generalises well past ingest, and
  a key on a source cannot express a step producing several relations.

## Blocked

(none)
