# Plan: Production readiness W7 — `smelt bakeoff`

**Date**: 2026-07-19
**Spec**: [`docs/specs/maintenance_plan.md`](../specs/maintenance_plan.md) (§CLI, §Design "Offline cost measurement is first-class") + [`docs/specs/cli.md`](../specs/cli.md)
**Spec diff**: none yet — Phase 1 carries the spec-precision edit (the bakeoff surface is specified in one line today; the resolved decisions below sharpen it)
**Tracking PR / branch**: `worktree-production` (production-readiness master; see [`docs/plans/20260719-production-readiness.md`](20260719-production-readiness.md))
**Docs**: code+docs

This sub-plan un-defers ROADMAP §10 (`smelt bakeoff` CLI, deferred 2026-07-10 out of
`20260707-maintenance-plan-impl.md` MP13 on three open design questions). The ladder half of
MP13 (`crates/smelt-logical/src/maintenance/choice.rs`) is landed and tested; this plan wires
it into the runtime and builds the CLI on top.

## Decisions (resolving the MP13 deferral questions, proposed 2026-07-19)

| # | Question | Decision |
|---|----------|----------|
| B1 | How does bakeoff force-execute one named technique against a scratch schema? | **Through the front door, in two steps.** (1) Wire the spec-aligned choice ladder (`resolve_cell_choice` / `effective_override`, currently unwired) into the runtime maintenance driver — this also fixes a real gap: `resolve_live_column_scoped_cell` passes `pin: None`, so operator `cells[].technique` pins are parsed but ignored at execution today. (2) Add an optional `technique_overrides` field to `ExecuteRequest`; overrides enter the same ladder as narrowest-scope entries (admission still enforced — an override can never run an inadmissible technique). Scratch redirect needs **no runtime schema seam at all**: bakeoff clones the chosen target in its in-memory `Config` under a synthetic name with `schema: smelt_bakeoff_<model>_<technique>` and sets `request.target` to it — schema already flows exclusively from `config.targets[target].schema`, and W2 Phase 4's per-target state layout isolates its state. Run-pipeline parity holds: everything still goes through `execute_project`. |
| B2 | `--pin` frontmatter round-trip? | **Emit-only** (per the K6 design, `docs/research/20260705-refresh-as-maintenance-plan/04-knobs.md`): `--pin` prints the winning `cells[]` entry (or a complete `maintenance:` block when the model has none) as ready-to-paste YAML for the user to review and commit. It never rewrites the `.sql` file, so the formatting-destruction risk vanishes. Staleness is already covered: a pin is an ordinary override, re-validated through admission on every compile, and an inadmissible pin fails loud. |
| B3 | In-process CLI testability with `mod commands` private? | **The `explain.rs` precedent.** The measurement engine lives in a `pub mod bakeoff` in `smelt-cli`'s lib (like `smelt_cli::explain`); `commands/bakeoff.rs` in the binary is a thin arg-parsing shim. Integration tests drive the library in-process against real DuckDB (the `LinkCProject`/`execute_project` pattern from `tests/property_discovery/`), plus one `assert_cmd` subprocess smoke for the real binary. `mod commands` stays private. |
| B4 | Replay substrate for "a representative window"? | **Window slicing of real data.** `--runs N` (default 3) splits the driving source's event-time extent into N sequential windows and replays them in order per technique — real `execute_project` runs against the user's actual data. The K6 `--replay <schedule.yml>` / `--from-history` surface is deferred (see Explicitly deferred). |

**Sequencing.** Registered **after W6** in the master registry: Phase 2+ touch
`smelt-runtime/src/execute.rs`'s dispatch sites and `ExecuteRequest`, which W2 (Phases 3–7)
actively rewrites, and B1's scratch-as-target lean on W2 Phase 4's per-target state layout.
Running last avoids building the seam against code W2 is about to replace.

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/maintenance_plan.md` §CLI + §Design "Offline cost measurement is first-class" and `docs/specs/cli.md` — they are the correctness oracle. Do not re-open the settled decisions B1–B4 above.
2. Confirm you are on branch `worktree-production`. If not, ask the user before continuing.
3. Confirm W2 is `done` in the master registry (this plan builds on its `ExecuteRequest`/state-layout changes). If W2 has pending phases, emit `<<PHASE_BLOCKED>>` rather than building against the pre-W2 `execute.rs`.
4. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — DuckDB-backed phases exercise `examples/timeseries` or a staged temp project; tests skip **loudly** when `DUCKDB_LIB_DIR` is unset.
- Red-green TDD: failing test before any implementation.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call: fmt + clippy + tests + example_diagnostics, failures-only output) — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Standing gates stay green every phase: `execute_parity`, `statement_parity`, `maintenance_conformance`, `walk_coverage`, the hardening/census/registry ratchets. The run-pipeline-parity and maintenance-plan-purity invariants (CLAUDE.md) bind every seam this plan adds.
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/*.md` and `docs-site/docs/...` describe the feature as if it has always existed.

---

## Context

`maintenance_plan.md` §Design names offline cost measurement a first-class capability: because
per-cell technique choice is contract-preserving at fixed `S`, smelt may measure alternative
physical plans over real data offline and pin the cheapest — a capability per-query optimisers
structurally lack. The surface (`smelt bakeoff <model> [--cells ...]`, §CLI) is specified but
unbuilt (spec §Known Divergences; `cli.md` §Known Divergences; ROADMAP §10). The choice ladder
it measures into is landed (`smelt-logical/src/maintenance/choice.rs`) but not yet consulted by
the runtime, which still resolves techniques through the older pin-less path.

## Scope

### In scope (spec coverage)
- `maintenance_plan.md` §CLI: `smelt bakeoff <model> [--cells ...]` with measured per-cell ×
  per-technique cost report and `--pin` emitting the winning `cells[]` override.
- Wiring `resolve_cell_choice`/`effective_override` into the runtime driver so frontmatter
  `prefer`/`technique` overrides take effect at execution (the ladder MP13 landed, made live).
- The `ExecuteRequest.technique_overrides` seam (B1) and the scratch-as-synthetic-target
  pattern, with the `EXCEPT ALL` cross-variant equivalence safety net.
- Spec precision for all of the above (Phase 1) + user docs (Phase 6).

### Explicitly deferred
- `--replay <schedule.yml>` / `--from-history N-runs` (K6's declarative replay surface) — the
  `--runs N` window slicer covers v0.5; a schedule format deserves its own design pass.
- Backend cost proxies beyond wall-clock + row counts (rows-read, DuckDB profiling pragmas,
  Spark metrics) and horizon extrapolation ("over a year at this cadence").
- A runtime `CostModel` hook in `choice.rs` (auto-choosing by measured cost at compile time) —
  bakeoff's output is a *pin*, reviewed by a human; auto-steering is post-0.5.
- Applying the pin to the file for the user (`--pin --write`) — emit-only per B2.
- Spark-backend bakeoff verification — the seam is backend-generic but only DuckDB is exercised;
  Spark inherits whatever W4 concludes.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | pending  |        |      |
| 2     | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |

## Phase detail

### Phase 1: Spec precision for the bakeoff surface

**Goal.** `maintenance_plan.md` §CLI and `cli.md` state the resolved surface exactly: argument
grammar (`smelt bakeoff <model> [--cells <col>@<source>,...] [--runs N] [--target <name>]
[--keep] [--pin]`), default cell selection (every cell with ≥2 admissible techniques), the
window-replay semantics, the scratch-schema behaviour (synthetic target, dropped after
measurement unless `--keep`), the `EXCEPT ALL` equivalence check, and `--pin`'s emit-only
contract. Also states the now-live override semantics: `cells[].technique` pins and `prefer`
are honoured at execution through the same ladder that admission runs (B1's wiring, described
timelessly).

**Pre-conditions.** None (docs-only phase; no TDD — verification commands stand in).

**Verification commands (stand in for TDD — docs-only phase).**
- `rg -n 'bakeoff' docs/specs/maintenance_plan.md docs/specs/cli.md` shows the full grammar above, not the one-line stub.
- `rg -n 'Phase [A-Z0-9]' docs/specs/maintenance_plan.md docs/specs/cli.md` — no new hits in body sections (timeless rule).
- The `cli.md` Known-Divergences pointer for bakeoff cites **this** plan, not MP13.

**Implementation shape.** Expand `maintenance_plan.md` §CLI's bakeoff bullet into the resolved
surface; keep design rationale in §Design (the "Offline cost measurement is first-class"
paragraph gains the emit-only-pin and measurement-is-real-runs statements). `cli.md` gets the
command's argument table alongside its `smelt explain`/`smelt run` siblings. Known Divergences
in both specs updated to say the surface is specified-and-tracked here (it still doesn't exist
until Phases 4–5 land — the divergence text stays, repointed).

**Critical files (allowed to touch in this phase).**
- `docs/specs/maintenance_plan.md` — §CLI, §Design, §Known Divergences
- `docs/specs/cli.md` — bakeoff section + Known Divergences repoint

**Docs touched.** The spec edits above (docs-site waits for the surface to exist — Phase 6).

**Review checklist** (material findings only):
- [ ] Grammar in spec matches B1–B4 decisions exactly; no invented flags
- [ ] Emit-only `--pin` contract explicit; no file-mutation language
- [ ] Admission-still-binds rule stated (an override can never run an inadmissible technique)
- [ ] Spec edits timeless — no phase vocabulary, no plan narration in body

**Commit.** `docs(spec): resolved smelt bakeoff surface + live override semantics`

### Phase 2: Wire the choice ladder into the runtime driver

**Goal.** `resolve_cell_choice`/`effective_override` (smelt-logical) replace the pin-less
resolution in the runtime maintenance driver, so frontmatter `cells[].technique` pins and
`prefer` preferences actually steer execution — a hard pin bypasses the default choice but not
admission, and an inadmissible pin fails the run loudly with the `ChoiceRefusal` diagnostic.

**Pre-conditions.** W2 `done` (this phase edits the post-W2 `execute.rs`/driver dispatch).

**TDD tests to write first.**
- `crates/smelt-cli/tests/maintenance_pins.rs::technique_pin_forces_region_recompute_at_runtime` — staged temp project (LinkCProject pattern) whose mutation cell admits `fold`; frontmatter pins `technique: recompute`; `SqlCapturingReporter` shows the recompute path (no column-scoped MERGE), and the result equals the full-refresh oracle. DuckDB; skips loudly without lib.
- `crates/smelt-cli/tests/maintenance_pins.rs::inadmissible_pin_fails_loud` — pin `fold` on a cell whose write footprint is unbounded → the run errors naming the cell (`MaintenanceUnboundedFootprint` wording from `ChoiceRefusal`), no silent fallback.
- `crates/smelt-cli/tests/maintenance_pins.rs::prefer_is_soft_and_never_refuses` — `prefer: fold` on the same inadmissible cell → run succeeds via recompute, no error.

**Implementation shape.** In `smelt-runtime/src/maintenance_driver.rs`, thread the model's
`MaintenanceConfig` (`defaults`/`cells`) into the live-cell resolution: `resolve_live_column_scoped_cell`
computes `effective_override(...)` for the trigger and calls `resolve_cell_choice(...)` instead
of the older `resolve_cell_technique(..., pin: None)`; a `ChoiceRefusal` becomes a run error
through the existing error path (fail-loud discipline). The older resolver is retired or
reduced to an internal detail of the new path — no dual resolution. `statement_parity` and
`maintenance_conformance` must stay green (the default choice is unchanged when no override is
present).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/maintenance_driver.rs` — ladder wiring
- `crates/smelt-runtime/src/execute.rs` — dispatch-site plumbing only (pass config through)
- `crates/smelt-logical/src/maintenance/choice.rs` — visibility/signature adjustments only
- `crates/smelt-cli/tests/maintenance_pins.rs` — new

**Docs touched.** None beyond Phase 1's spec text (which already describes overrides as live);
docs-site `smelt-yml.md` override docs verified accurate in Phase 6.

**Review checklist** (material findings only):
- [ ] Exactly one resolution path remains; `pin: None` hardcoding gone
- [ ] Refusal is a loud run error, not a fallback (fail-loud discipline)
- [ ] `statement_parity` + `maintenance_conformance` green (no-override behaviour unchanged)
- [ ] No scope creep into `ExecuteRequest` (that's Phase 3)

**Commit.** `feat(runtime): honor maintenance technique pins via the choice ladder`

### Phase 3: `ExecuteRequest.technique_overrides` + scratch-as-target

**Goal.** A request-scoped forcing seam: `ExecuteRequest` gains
`technique_overrides: Vec<CellTechniqueOverride>` (cell address → `CellTechnique`), entering
the Phase-2 ladder as narrowest-scope entries — admission still binds. Demonstrate the
scratch-as-synthetic-target pattern end to end: two runs of the same model, forced to different
techniques, land in two scratch schemas, agree exactly, and leave the real target untouched.

**Pre-conditions.** Phase 2.

**TDD tests to write first.**
- `crates/smelt-cli/tests/bakeoff_seam.rs::request_override_forces_each_admissible_technique` — same staged project run twice via `execute_project` with `technique_overrides` = fold vs recompute, each with a synthetic target cloning the real one under `schema: smelt_bakeoff_test_<technique>`; both scratch outputs non-empty, `EXCEPT ALL` empty both directions, and the real target's schema has no new tables. DuckDB; skips loudly without lib.
- `crates/smelt-cli/tests/bakeoff_seam.rs::request_override_subject_to_admission` — an override naming an inadmissible technique for its cell errors loudly (same refusal wording as Phase 2), never executes.
- `crates/smelt-cli/tests/bakeoff_seam.rs::empty_overrides_change_nothing` — default `ExecuteRequest` behaves identically pre/post field addition (guards `execute_parity`).

**Implementation shape.** `CellTechniqueOverride { columns: Vec<String>, on: String, technique: CellTechnique }`
in `smelt-runtime/src/types.rs` (serde-default empty so existing constructors are untouched).
In the driver, request overrides are appended after frontmatter `cells[]` in the
`effective_override` input (narrower-wins ladder already gives last-narrowest precedence —
request scope is defined as narrower than file scope). No schema field anywhere: the test (and
later the CLI) builds the synthetic target by cloning `config.targets[target]` in memory —
per-target state lands under W2's `.smelt/targets/<synthetic>/` and is deleted with the schema.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/types.rs` — `ExecuteRequest` field + override type
- `crates/smelt-runtime/src/maintenance_driver.rs` — override merge into the ladder input
- `crates/smelt-cli/tests/bakeoff_seam.rs` — new

**Docs touched.** None (internal seam; `ExecuteRequest` is not user surface).

**Review checklist** (material findings only):
- [ ] Admission cannot be bypassed by a request override (test proves it)
- [ ] `execute_parity` green; default-request behaviour bit-identical
- [ ] No new `pub` runtime entrypoints (run-pipeline parity invariant)
- [ ] Scratch pattern leaves zero residue in the real schema and real state dir

**Commit.** `feat(runtime): per-cell technique overrides on ExecuteRequest for offline measurement`

### Phase 4: `smelt bakeoff` command + measurement report

**Goal.** The user-facing command per Phase 1's spec: for each selected cell (default: ≥2
admissible techniques), replay `--runs N` sequential event-time windows of real data per
admissible technique into per-technique scratch targets via the Phase-3 seam; measure
wall-clock per run and final row counts; run the `EXCEPT ALL` cross-variant check; print a
per-cell × per-technique report; drop scratch schemas (and their state dirs) unless `--keep`.

**Pre-conditions.** Phases 2–3.

**TDD tests to write first.**
- `crates/smelt-cli/tests/bakeoff.rs::bakeoff_reports_measured_cost_per_admissible_technique` — in-process via `smelt_cli::bakeoff` against a staged project with a genuinely multi-technique cell: report contains one row per admissible technique with nonzero wall-clock and equal row counts; equivalence check passes. DuckDB; skips loudly without lib. (This is MP13's originally-named TDD target, honoured.)
- `crates/smelt-cli/tests/bakeoff.rs::bakeoff_with_no_multi_technique_cells_says_so` — single-technique model → clear "nothing to measure" report, exit success, no scratch schemas created.
- `crates/smelt-cli/tests/bakeoff.rs::bakeoff_drops_scratch_unless_keep` — scratch schemas + `.smelt/targets/` state dirs absent after a default run, present with `--keep` (and named in the report).
- `crates/smelt-cli/tests/bakeoff.rs::bakeoff_runs_via_real_binary` — `assert_cmd` subprocess smoke: `smelt bakeoff <model> --runs 2` against `examples/timeseries` exits 0 and prints the report header.

**Implementation shape.** `pub mod bakeoff` in `crates/smelt-cli/src/lib.rs` (B3): a
`run_bakeoff(config, model, opts) -> Result<BakeoffReport>` engine that (a) derives the plan +
admissible sets via the existing pure derivation, (b) computes the driving source's event-time
extent and slices `--runs` windows, (c) for each technique clones the target under
`smelt_bakeoff_<model>_<technique>`, executes the window sequence through `execute_project`
with the Phase-3 override, timing each run, (d) compares variants pairwise with `EXCEPT ALL`
through the backend connection, (e) renders the report (plain table on stdout — `smelt-cli`
stdout is legitimate). `commands/bakeoff.rs` is a thin shim: parse args → load workspace via
the canonical `load_workspace` path → call the lib. `--pin` parsing lands here but the flag
errors "not yet supported" until Phase 5 (fail-loud, not silent).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/bakeoff.rs` — new (engine + report types)
- `crates/smelt-cli/src/lib.rs` — `pub mod bakeoff`
- `crates/smelt-cli/src/main.rs`, `crates/smelt-cli/src/commands/bakeoff.rs` — subcommand
- `crates/smelt-cli/tests/bakeoff.rs` — new

**Docs touched.** None yet (Phase 6 documents once `--pin` completes the surface).

**Review checklist** (material findings only):
- [ ] Everything executes through `execute_project` (run-pipeline parity); no bespoke SQL authoring outside the pure emitters (maintenance-plan purity — the `EXCEPT ALL` probe is a read-only comparison query, allowed)
- [ ] Scratch naming collision-safe per model×technique; cleanup covers state dirs
- [ ] Report distinguishes measured fact from extrapolation (none in v0.5)
- [ ] Skips loudly without DuckDB lib, never green-skips

**Commit.** `feat(cli): smelt bakeoff — measure admissible techniques over replayed windows`

### Phase 5: `--pin` emits the winning override

**Goal.** `smelt bakeoff <model> --pin` appends to the report the winning technique per
measured cell as a ready-to-paste YAML fragment: a `cells[]` entry when the model already has
a `maintenance:` block, a complete `maintenance:` block otherwise. Emit-only (B2): no file is
modified.

**Pre-conditions.** Phase 4.

**TDD tests to write first.**
- `crates/smelt-cli/tests/bakeoff.rs::pin_emits_parseable_cells_entry` — the emitted YAML deserializes into `MaintenanceCellConfig`, and feeding it through `effective_override` + `resolve_cell_choice` yields exactly the winning technique for that cell's trigger.
- `crates/smelt-cli/tests/bakeoff.rs::pin_mutates_no_files` — byte-identical model files (and no new files) after a `--pin` run.

**Implementation shape.** Serialize via the existing `Serialize` derives on
`MaintenanceCellConfig`/`MaintenanceConfig` (serde_yaml), with the cell's `columns`/`on`
address taken from the measured cell. Winner = lowest total wall-clock across the replayed
windows; ties keep the current default choice and say so. The emitted block is printed under a
"to pin this choice, add to <model>.sql frontmatter:" header.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/bakeoff.rs` — winner selection + YAML emission
- `crates/smelt-cli/src/commands/bakeoff.rs` — `--pin` unstubbed
- `crates/smelt-cli/tests/bakeoff.rs` — extend

**Docs touched.** None yet (Phase 6).

**Review checklist** (material findings only):
- [ ] Round-trip proven: emitted YAML → parsed config → ladder → same winner
- [ ] No file writes on any `--pin` path
- [ ] Tie-break behaviour explicit in the report text

**Commit.** `feat(cli): smelt bakeoff --pin emits reviewable frontmatter override`

### Phase 6: User docs + divergence closure

**Goal.** The surface exists; document it and close the spec divergences. docs-site CLI
reference gains the `smelt bakeoff` section (grammar, report anatomy, pin workflow, scratch
behaviour); `smelt-yml.md`'s `maintenance:` docs state that `prefer`/`technique` overrides are
honoured at execution and show the pin-paste workflow. `maintenance_plan.md` + `cli.md` Known
Divergences drop their "bakeoff doesn't exist" entries; `docs/ROADMAP.md` §10 becomes ✅ with
date, pointing here.

**Pre-conditions.** Phases 1–5.

**Verification commands (stand in for TDD — docs phase).**
- `rg -n 'bakeoff' docs-site/docs/reference/cli.md` shows the documented section.
- `rg -in 'bakeoff' docs/specs/maintenance_plan.md docs/specs/cli.md` — no remaining "doesn't exist yet"/"unwired" divergence language.
- `rg -n 'Phase [A-Z0-9]' <touched docs-site pages>` — no phase vocabulary.
- `/smelt:validate maintenance_plan` and `/smelt:validate cli` drift reports clean for the bakeoff surface.

**Implementation shape.** Timeless feature prose seeded from Phase 1's spec text; a worked
example against `examples/timeseries` (the same model the Phase 4 smoke test drives) showing a
real report and a real emitted pin block.

**Critical files (allowed to touch in this phase).**
- `docs-site/docs/reference/cli.md`, `docs-site/docs/reference/smelt-yml.md`
- `docs/specs/maintenance_plan.md`, `docs/specs/cli.md` — Known Divergences closure
- `docs/ROADMAP.md` — §10 completion

**Review checklist** (material findings only):
- [ ] Docs match the shipped surface exactly (flags, defaults, report columns)
- [ ] All bakeoff divergence entries closed; none orphaned in other specs
- [ ] Timeless throughout

**Commit.** `docs(bakeoff): user docs + spec divergence closure for smelt bakeoff`

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `cargo test -p smelt-cli --test maintenance_pins --test bakeoff_seam --test bakeoff` (DuckDB env exported)
- `smelt bakeoff` run by hand against `examples/timeseries` produces the documented report; `--pin` emits a block that, pasted into the model, is honoured on the next run (Phase 2 wiring)
- `bash .claude/scripts/verify-phase.sh`
- Standing gates green: `execute_parity`, `statement_parity`, `maintenance_conformance`, `walk_coverage`
- `/smelt:validate maintenance_plan` + `/smelt:validate cli` report zero bakeoff drift
