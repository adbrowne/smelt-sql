# Outcome: Wire the definition-delta vertical (`smelt migrate`, v2 — narrow)

**Created:** 2026-08-16
**Status:** active
**Source:** `docs/handoffs/2026-08-16-delta-signature-closure-programme.md` (programme outcome 3);
supersedes `docs/outcomes/20260815-definition-delta-migrate/` (this is that outcome's original
narrow vertical — its phases 2–9 — without the 2026-08-15 build-everything widening)
**Spec anchors:** `docs/specs/definition_deltas.md`

## The outcome

The classification and emission machinery for definition changes
(`crates/smelt-logical/src/backbuild/`: diff factoring, per-group verdicts, the technique
catalogue, script assembly) stops being dead code and becomes the `smelt migrate` verb
`docs/specs/definition_deltas.md` specifies: a definition change is classified per column group
into a verdict (eclipsed / backfill in place / re-derive / skeleton change), a plan is printed
naming the technique per group, and `--apply` executes only a plan whose hash matches what was
printed and approved — giving CI a gate against unreviewed migrations. The ranged-rebuild verb
ships under its spec name (`smelt rebuild`, not `smelt backbuild`), the diagnostic code ships as
`MaintenanceSkeletonChanged`, and the generative conformance suite exercises definition edits so
the equivalence invariant covers this mechanism the way it covers the maintenance ladder.

## Success criteria (checkable)

1. `smelt migrate` exists: given a changed model definition it invokes the backbuild synthesis
   layer end to end (diff → classify → emit) and prints the per-group verdict/technique plan,
   executing nothing.
2. `smelt migrate --apply` executes only a plan whose stored hash matches the re-derived plan; a
   stale or unapproved plan refuses with a distinct CI exit code; an approval store persists the
   hash per the already-decided plan-hash scope (see Decision log). Closes "No approval store
   exists."
3. The ranged-rebuild verb is `smelt rebuild` end to end: CLI, `--help`, docs-site, examples,
   tests, and the sibling-spec sweep (`cli.md` verb table and `--dry-run` prose,
   `model_selection.md` positional-selector callout, `architecture.md` prose where it means the
   CLI verb — the `backbuild/` module path and "backbuild synthesis" mechanism name may stay).
4. The generative maintenance-conformance suite gains a definition-edit step kind — staged
   definition changes mid-history asserted against the full-refresh-on-new-definition oracle.
   Closes "The conformance harness has no definition-edit step kind yet."
5. The atomicity divergence is resolved, not left conditional: the
   `schema_evolution: strategy: full_refresh` escape either routes through the migration gate or
   gets a real repair path; the choice is recorded in the spec and the divergence bullet removed.
6. `MaintenanceSkeletonColumnAdded` is renamed to `MaintenanceSkeletonChanged` in code (single
   code, per the recorded decision), swept across sibling specs (`model_transforms.md`,
   `model_properties.md`, `incremental_models.md`, `schema_evolution.md`, `diagnostics.md`), and
   the definition-change diagnostic is surfaced ahead of a run (LSP + `smelt explain`), not only
   via the maintenance driver's I/O path.
7. A docs-site migration guide ships: `docs-site/docs/guide/backbuild-synthesis.md` rewritten in
   place around `smelt migrate`/`--apply`, its "no CLI command yet" and naming-collision
   callouts removed; `models.md`/`seeds.md`'s "no `smelt migrate`" divergence bullets removed or
   reworded to precisely what `smelt migrate` doesn't cover.
8. `/smelt:validate definition_deltas` reports no drift; every bullet this outcome closes is
   removed from the spec, not just addressed in code. All standing gates green, including the
   extended conformance suite, `statement_parity`, and `walk_coverage`.

## Out of scope

Everything the 2026-08-15 widening pulled in and the programme reassigns elsewhere: scheduler
delta-signature consumption and `smelt explain` headline (programme outcome 2), per-cell frontier
addressing, write-pin equivalence, observed-delta consumption, plan-consumer/graph-layer/proof
residues, conditional-maintenance gaps, key-grain validation gaps (keyed/partition residue
outcomes), and every `(Open Question)` product decision (decision track). See the handoff.

- **The pending-delta run refusal** (`definition_deltas.md` §Detection: `smelt run` refuses to
  fold data deltas while a non-eclipsed definition delta is pending). None of this outcome's
  success criteria name it, it changes the semantics of every ordinary run rather than the
  migrate verb, and closing it needs its own equivalence argument. Recorded as a spec Known
  Divergence in phase 6 instead.

- **The `InPlaceUpdate` FROM-alias bug** (phase-7 summary "For the next planner":
  `resolve_live_in_place_update_cell` carries the model SQL's FROM alias verbatim into the folded
  `UPDATE ... SET`, invalid for any aliased single-table FROM). It sits on the *maintenance*
  driver's live-cell resolution, not on `smelt migrate`'s own emission path
  (`backbuild::classify` requalifies its assignment expressions), so no success criterion here
  depends on it. Pre-existing and orthogonal; worth its own bug fix.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Wire `smelt migrate` (plan-only): CLI verb invokes the backbuild synthesis layer end to end and prints the per-group verdict/technique plan | done |
| 2 | Approval gate: pure plan hash, approval store, `--json`, CI exit-code contract, `--apply` hash-mismatch/staleness refusal (executes nothing yet) | done |
| 3 | `--apply` execution: run the approved plan's statements against the backend, re-record the deployed definition, resume-on-reinvoke | done |
| 4 | Rename `smelt backbuild` → `smelt rebuild` across CLI, docs-site, examples, tests, and the spec sweep named in success criterion 3 | done |
| 5 | Generative definition-edit schedules: a definition-edit schedule generator + standing pool gate asserting the new-definition oracle mid-history | done |
| 6 | Make `smelt migrate` reachable mid-incremental-history (windowed runs record the deployed definition) and add a migrate-driven recovery step to the conformance harness | done |
| 7 | Close the atomicity divergence (unify the `schema_evolution` full-refresh escape with the migration gate, or land its repair path) | done |
| 8 | Diagnostic rename lands in code (`MaintenanceSkeletonChanged`, one code across the maintenance and migrate mechanisms) + sibling-spec sweep | done |
| 9 | Surface the definition-change refusal ahead of a run: deployed columns become a Salsa input, so LSP diagnostics and `smelt explain` both fire it | done |
| 10 | docs-site migration guide: rewrite `guide/backbuild-synthesis.md` in place; update `models.md`/`seeds.md` bullets | planned |
| 11 | Validate + close out: `/smelt:validate definition_deltas` clean, Known Divergences bullets removed, full standing-gate sweep | pending |

## Decision log

- **Inherited from `20260815-definition-delta-migrate` phase 1 (done, 2026-08-15).** Plan-hash
  scope: hash the plan data structure the emitters consume (verdicts, techniques, input facts —
  source declarations, backend capabilities), not only rendered SQL; exclude region
  *enumeration*, resolved at apply time from the frontier so `--apply` stays reachable on an
  actively-loading warehouse. Diagnostic rename: `MaintenanceSkeletonColumnAdded` →
  `MaintenanceSkeletonChanged`, one code, not an add/changed split. Both already landed in
  `docs/specs/definition_deltas.md` §Design / §Known Divergences; this outcome implements
  against them.
- **2026-08-16 (phase 1 planning).** No phase reshape. Scoped into phase 1: state records only a
  `definition_hash`, never the definition text, so there is no "before" side to diff. The recorded
  definition SQL is persisted in the per-model deployed-schema snapshot
  (`.smelt/targets/<target>/schemas/<model>.json`, a `#[serde(default)]` field so pre-existing
  snapshots read back as "no recorded definition" — fail-closed). Spec §Detection already promises
  smelt records the definition the table was last maintained under, so this implements the spec
  rather than widening scope. Also decided: plan *derivation* is pure data in `smelt-logical`
  (`backbuild/plan.rs`), fact assembly lives in `smelt-runtime` (so the UI can consume it), and the
  CLI is a renderer only — keeping maintenance-plan purity and CLI↔UI parity intact.

- **2026-08-16 (phase 1 implementation).** `smelt migrate <model>` diffs the model's raw SQL text
  (`model.content`) against `DeployedSchema::definition_sql`, not `SqlCompiler`-compiled output —
  `execute.rs` always records raw text as the deployed definition, and comparing compiled output
  against it would spuriously diff every unchanged model. `SourceRef`s are built from
  `DependencyGraph::get_upstream` (one entry per direct upstream model), not a full FROM-tree
  alias walk — real for self-joins/multi-alias upstreams is deferred (see phase 1 summary "For
  the next planner"). `MigrateArgs` dropped the plan's `--database` field: this command never
  opens a backend connection, so the flag would be dead until `--apply` (phase 2) needs it.

- **2026-08-16 (phase 2 planning). Reshape: old phase 2 split in two; old 3–8 renumbered 4–9.**
  The single row bundled the *gate* (pure plan hash, approval persistence, `--json`, exit codes,
  drift refusal) with *execution* (backend connection, per-technique statement execution,
  deployed-snapshot re-record, resume). The gate is fully testable without a write path and the
  execution half needs a real backend and its own equivalence argument, so they are now phase 2
  and phase 3. Nothing left the outcome — success criterion 2 is met only when both land. Also
  decided for phase 2: exit code `3` becomes a new normative CLI code ("a non-trivial migration
  is pending and unapproved"), and `plan_hash` is a pure function in `smelt-logical` over the
  plan data *plus* its `BackbuildInputs` facts (per the inherited plan-hash scope), which
  requires `TechniqueCandidate` to carry its statement text rather than only a count.

- **2026-08-16 (phase 2 implementation).** Plan mode always records the freshly derived hash
  (even for a non-eclipsed plan it still exits `3` for) — recording and approval are distinct
  acts; a human reviewing the printed plan is the approval, `--apply`'s hash match is the check
  that nothing drifted since. Eclipsed plans never touch the approval store at all. Discovered
  and reverted (not committed, unrelated to this phase): a `smelt-db`
  `prop_multi_model_type_inference` proptest regression (`PERCENTILE_DISC` over `SmallInt`
  infers `Double`) — see phase 2 summary "For the next planner".

- **2026-08-16 (phase 3 planning).** No phase reshape — the phase-2 summary's "no rework expected"
  held. Decided for phase 3: `--apply` executes each group's **first presented candidate** (the
  plan is deterministic and the hash covers every candidate, so approving the plan approves that
  selection — no per-candidate selection flag); one transactional `StatementGroup` per column
  group realises §"The atomicity rule"; admission is checked over all groups *before* anything
  executes, and a plan containing a skeleton-change group, a candidate-less group, or a
  destructive (`ColumnDrop`) candidate executes nothing and exits `3` — destructive legs stay
  refused until their verification probes are emitted. Resume is recorded **per column group** in
  the approval store, not per region: the per-region frontier reset §"Frontier semantics"
  describes is per-cell frontier addressing, which this outcome lists under "Out of scope". Both
  narrowings land as Known Divergences bullets rather than silent gaps. Exit code `3`'s spec
  wording widens from "unapproved" to "a non-trivial migration remains pending" so it covers the
  refused-to-execute case too.

- **2026-08-17 (phase 4 planning).** No phase reshape — the phase-3 summary's carry-forwards
  (CLI-level resume durability test, `atom_label` fragility, destructive-leg probes) are all
  already covered by later rows or by the outcome-level Known Divergences, not new work. Decided
  for phase 4: the rename is **hard, with no `backbuild` alias** — the project carries no
  backward-compatibility constraint and criterion 3 asks for the verb end to end, so a stale
  invocation gets clap's unrecognized-subcommand error. A standing docs ratchet
  (`no_backbuild_verb_in_user_docs`) keeps the verb from creeping back into `docs-site/docs` or
  `docs/specs`; the `backbuild/` module path, "backbuild synthesis" mechanism name, and
  `guide/backbuild-synthesis.md` (rewritten in place by phase 8) are deliberately untouched.

- **2026-08-17 (phase 4 implementation).** Landed the hard, no-alias rename end to end: CLI
  (`Commands::Rebuild`/`RebuildArgs`, `commands/rebuild.rs`), `docs/specs/cli.md` and
  `model_selection.md`, the `definition_deltas.md` Known Divergences bullet removed, `docs-site`
  (`reference/cli.md`, `guide/incremental-models.md`, `developing/architecture.md`), the
  `web_analytics` tutorial template + generator + regenerated page, and `README.md`. Also fixed
  two stale `smelt backbuild` mentions inside `guide/backbuild-synthesis.md` (the naming-collision
  callout and the "Related pages" cross-reference) that the plan's file list didn't call out but
  the standing ratchet test requires — the page's title/structure/content otherwise stay
  untouched for phase 8's rewrite. `docs/specs/architecture.md` audited: all four occurrences name
  the mechanism/module, no edit needed.

- **2026-08-17 (phase 5 planning). Reshape: old phase 5 split in two; old 6–9 renumbered 7–10.**
  Reading the harness found that a definition-edit step kind already exists
  (`ConformanceStep::RewriteModel` + `s_restricted_oracle_sql_with_edit`), but it is only ever
  hand-driven — `arb_schedule_for` never emits one, so the *generative* suite does not stage
  definition edits, which is what criterion 4 and `definition_deltas.md` §Constraints claim. That
  half (a definition-edit schedule generator plus a standing pool gate) is phase 5. The second
  half — proving the equivalence invariant over the migrate mechanism itself, i.e. rewrite →
  `smelt migrate --apply` → assert — is blocked on a real production gap: only the full-refresh
  arm of `execute.rs` calls `save_deployed_schema`, so an incrementally-maintained model has no
  recorded definition mid-history and `smelt migrate` fails closed with `NoRecordedDefinition`.
  Closing that (windowed runs record the deployed definition) plus the migrate-driven recovery
  step is now phase 6; nothing left the outcome. Also decided for phase 5: the definition-edit
  generator is a NEW sibling strategy, not a widening of `arb_schedule_for` — six other suites
  (`probes.rs` permutations, `state_deletion.rs`, `contract_points.rs`, the Spark mirrors) consume
  that generator and a mid-schedule rewrite is order-dependent by construction, so folding it in
  would silently change their meaning. `is_permutable` gains `RewriteModel` to its exclusion list
  regardless.

- **2026-08-17 (phase 5 implementation).** Landed `arb_schedule_with_definition_edit` (the new
  sibling generator) plus two standing gate tests: `definition_edit_pool_upholds_equivalence`
  (generic leg over the deterministic sample) and `definition_edit_grouping_column_upholds_equivalence`
  (the `AddGroupingColumn` skeleton-widening leg, pinned by manual splice rather than left to the
  generator's random edit draw — the aggregate constructs' evolution also contains
  `AddPayloadColumn`, so leaving it to the draw would only probabilistically cover the
  skeleton-widening leg). Both gate tests admitted and passed cleanly on the first run; no
  diagnosis or generator narrowing was needed. Spec delta applied: the "no definition-edit step
  kind" divergence bullet removed; the diagnostic-rename bullet's stale phase pointer fixed
  (6 → 8).

- **2026-08-17 (phase 6 planning). No phase reshape; one item added to "## Out of scope".**
  Read the code the phase-5 summary pointed at: the gap is narrower and more precise than "only
  the full-refresh arm records". `execute.rs` already saves a first-deployment baseline for
  `plan.incremental.is_some()` at the bottom of the per-model unit — but the cumulative arm and
  the key-addressed arm both `return Ok(ModelOutcome::Completed(..))` before reaching it, so
  models taking those routes never record a definition. Decided: the fix hoists that one save
  into a helper called from all three sites, and the `!already_stored` guard **stays** — a
  windowed run under changed SQL must never overwrite the recorded definition, or the pending
  delta would vanish before `smelt migrate` could see it. That rule becomes normative spec text
  (§Detection). Decided for the harness leg: `ConformanceStep::MigrateApply` drives the real
  `smelt` binary (`CARGO_BIN_EXE_smelt`) as a subprocess rather than calling
  `smelt_runtime::migrate::apply_migration_plan` directly, so the approval store and the exit-code
  contract are exercised end to end; the step is **pinned, not generated** this phase (whether
  `--apply` can execute depends on the edit's verdict — the pure-backfill leg applies, the
  skeleton leg is refused by design — and the generator cannot cheaply predict which). Criterion 4
  is met by phase 5's generative definition-edit pool plus phase 6's two pinned migrate legs.
  Also noted: closing this gap makes the conformance registry's
  `known_bug_incremental_path_skips_schema_snapshot` entry stale, so phase 6 prunes it.

- **2026-08-17 (phase 6 implementation).** The extracted `record_first_deployment_definition`
  helper drops the old single-call-site's `plan.incremental.is_some()` gate — that field is
  `None` for every `grain: key` model unconditionally (`Config::get_incremental_with_metadata`
  requires `Grain::Partition`), so keeping it would have made the new cumulative-arm call site a
  permanent no-op. `!already_stored` alone is the correct guard; every call site already implies
  "non-full-refresh route" by construction. Also found and fixed a real, previously-undetected bug
  in `smelt-runtime/src/migrate.rs`: the backbuild diff parser needs a bare `SELECT` at the file's
  top level, so any frontmatter-bearing model (i.e. any real `refresh: incremental` model) always
  collapsed to `Opaque`/"not a plain SELECT statement" — undetected because phases 1-3's CLI test
  fixture carried no frontmatter. Frontmatter is now stripped from both diff sides before parsing.
  This is the phase's own acceptance target (its own pinned gate test required it), so fixed inline
  rather than deferred. `ConformanceStep::MigrateApply` is hand-pinned only (no generator draws
  it) — `drive_and_assert`'s return type widened to a 3-tuple carrying observed `--apply` exit
  codes; the two destructuring call sites updated, every other call site was already discarding
  the whole result.

- **2026-08-17 (phase 7 planning). No phase reshape.** Read the two escapes the divergence bullet
  names. (a) `schema_evolution: strategy: full_refresh` sets `use_alter = false` in `execute.rs`,
  so the migration gate never runs and the derived `InPlaceUpdate` cell falls through to the
  standalone `execute_in_place_update` dispatch — the non-atomic two-step, and today the model's
  *declared* "always DROP + CREATE on schema changes" intent is not honoured at all (no rebuild
  happens either). (b) On a backend without transactional DDL (Spark, both formats) the migration's
  `StatementGroup` is executed non-transactionally and `ALTER TABLE ... ADD COLUMN` is not emitted
  `IF NOT EXISTS`, so a partial apply cannot be retried — and `execute.rs` currently swallows the
  failure with `tracing::warn!` and continues incrementally. Decided: **unify, don't inherit** —
  one pure `resolve_definition_change_route` in `smelt-runtime` decides `AtomicGroup` (default
  strategy + transactional DDL, today's path unchanged), `FullRebuild` (declared
  `strategy: full_refresh` — the declaration is the consent; also the non-transactional case when
  `--allow-full-refresh` is set), or `Refuse` (non-transactional without opt-in: apply nothing,
  error naming the recovery flag). Refusal *is* the repair path — nothing is written and the
  recorded definition is untouched, so the next invocation re-derives the identical change. The
  standalone fallback dispatch in `execute.rs` is deleted outright; the `InPlaceUpdate` emitter
  stays (owned by `smelt migrate --apply` and the maintenance driver). Rejected: emitting
  `ADD COLUMN IF NOT EXISTS` + `WHERE col IS NULL`-scoped backfill to make the two-step retry-safe
  — Spark's `ALTER TABLE ... ADD COLUMNS` has no `IF NOT EXISTS` form, so it would close (b) only
  on backends that already have transactions. Also noted: the deleted block's comment cites an
  `incremental_models.md` §Known Divergences bullet that does not exist — a stale pointer, fixed
  in this phase rather than left for phase 10's validate pass.

- **2026-08-17 (phase 7 implementation).** Landed `DefinitionChangeRoute` /
  `resolve_definition_change_route` in `crates/smelt-runtime/src/schema_evolution.rs` and wired it
  into `execute.rs`'s schema-evolution gate, replacing the `use_alter` boolean. Deleted the
  standalone `execute_in_place_update` fallback call site (the emitter stays, owned by `smelt
  migrate --apply` and the maintenance driver). Decided: the routing's `has_pending_column_add`
  input is computed from an actual schema diff against the loaded deployed snapshot, not just
  `InPlaceUpdate`-cell presence — this also fixes the `full_refresh` strategy's previously-silent
  "declared rebuild intent not honoured" bug for every kind of schema change, not only
  backfill-needing column adds (in scope per this phase's own framing of the bug in the prior
  planning entry). Spec updated: `definition_deltas.md` §"The atomicity rule" states the rule
  unconditionally and names all three routes; the "atomicity rule is conditional" Known
  Divergences bullet is deleted; `schema_evolution.md` gained §"Routing on a maintained model".
  Discovered, not fixed (pre-existing, orthogonal): the derived `InPlaceUpdate` backfill
  expression carries the model SQL's FROM-alias verbatim, which is invalid inside the folded
  `UPDATE` — every existing fixture in the repo avoids aliasing to route around it.

- **2026-08-17 (phase 8 planning). Reshape: old phase 8 split in two; old 9–10 renumbered 10–11.
  One item added to "## Out of scope".** The row bundled a mechanical rename with a real plumbing
  change, and reading the code showed the two have nothing in common. The rename is a
  string/variant sweep: the refusal→diagnostic mapping already exists in
  `file_diagnostics()` (`smelt-db/src/lib.rs`), and `ledger::render_refusal` already names the
  code for `smelt explain`'s refusal block. The *surfacing* half is blocked on an input, not a
  mapping: `derive_model_maintenance_plan` takes `deployed_column_names`, and `smelt-db`'s own
  call site passes `&[]` because a Salsa query does no I/O (Salsa-purity rule), so no
  `Trigger::ColumnAdded` is ever derived ahead of a run and the refusal cannot fire for either
  the LSP or `smelt explain`. Closing that means a new project-scoped Salsa **input** carrying the
  deployed column names, populated at the edge by both consumers (workspace-loading-parity rule) —
  its own phase, phase 9. Nothing left the outcome; criterion 6 is met only when both land.
  Also decided for phase 8: the rename is of the diagnostic-code *identity* (the `DiagnosticCode`
  variant, the `ledger.rs` code string, the `diagnostics.md` catalogue row, message text), and it
  extends to the migrate renderer so the `SkeletonChange` verdict names the same single code — the
  internal pure `Refusal::SkeletonColumnAdded` / `MaintenanceRefusal::SkeletonColumnAdded` variant
  names are *not* user-visible and stay, with a doc comment stating which code they map to, to
  keep the diff bounded to the identity the spec's Known Divergences bullet names.
  Recorded out of scope: phase 7's carry-forward FROM-alias bug (maintenance driver's live-cell
  resolution, not `smelt migrate`'s emission path — no criterion depends on it).

- **2026-08-17 (phase 9 planning). No phase reshape.** The phase-8 summary's carry-forward was
  exactly this row's scope. Reading the code settled the shape: the deployed column names become a
  field on the existing project-scoped `ProjectInput` Salsa **input** (not a new input struct) —
  the tracked `maintenance_plan` query already resolves its `ProjectInput` via `find_project`, and
  `maintenance_plan_report` (`smelt explain`) can read the same field, so one field serves both
  consumers. Populated in exactly one place, `workspace_ingest::ingest_loaded_workspace`, which
  both the CLI's `init_db` and the LSP's `initialize` already call — the workspace-loading-parity
  rule gives CLI↔LSP symmetry for free and keeps the file I/O at the edge, outside every Salsa
  query (the Salsa-purity rule; `set_project_input` already reads `smelt.yml` from disk at this
  same edge, so the precedent exists). The snapshot reader needs `smelt-state` as a new production
  dependency of `smelt-db`; `smelt-state` depends only on `smelt-core`/`-types`/`-dialect` and not
  on `smelt-db`, so no cycle. `maintenance_plan_diagnostics` gains a `deployed_column_names`
  parameter and stays pure. Also scoped in rather than deferred: LSP staleness — the watcher glob
  set (`derive_watch_globs`) gains `.smelt/targets/*/schemas/*.json` so a run that rewrites a
  snapshot refreshes the diagnostic without an editor restart; without it the surfaced diagnostic
  would be correct only until the next run, which criterion 6 would not honestly meet.

- **2026-08-17 (phase 9 implementation).** Landed the `ProjectInput::deployed_columns` Salsa
  input, populated at the workspace-loading edge (`workspace_ingest::read_deployed_columns`) and
  consumed by `maintenance_plan` (LSP) and `maintenance_plan_report` (`smelt explain`), so
  `MaintenanceSkeletonChanged` now fires ahead of a run. Caught, mid-phase, a real build
  regression: threading the real snapshot straight into the primary maintenance-plan derivation
  also surfaced `MaintenanceScanUnbounded` (and would surface other admission refusals) for
  ordinary nullable-column additions that `smelt build`/`smelt run` actually handles via
  `schema_evolution.rs`'s simpler ALTER-with-NULL-default route (phase 7) rather than
  `smelt-logical`'s backfill-technique catalogue. Fixed with a double-derivation: the primary
  plan derivation stays `&[]` (byte-identical pre-phase-9 behaviour for every non-skeleton
  refusal); a secondary derivation with the real snapshot is consulted only to extract
  `Refusal::SkeletonColumnAdded`, merged into the primary result. See phase 9 summary "Decisions"
  for the full rationale and the follow-up this leaves (whether `Refusal` variants should carry
  trigger provenance to collapse the double-derivation into one pass).

- **2026-08-17 (phase 10 planning). No phase reshape.** Phase 9's carry-forwards are follow-ups,
  not criteria work. Reading the targets settled three things. (a) The `models.md`/`seeds.md`
  bullets criterion 7 names are **spec** files, not docs-site pages, and both use `smelt migrate`
  to mean a *config/seed-file rewrite assist* — a different verb from the shipped deployed-table
  migration verb. They are therefore reworded (naming what the shipped verb does not cover), not
  deleted, and `models.md`'s "(`smelt migrate` applies it)" fix-it parenthetical is corrected in
  the same pass since it now points at the wrong verb. (b) `smelt migrate` has **no** docs-site
  reference entry, so the rewritten guide would link into nothing; a `## smelt migrate` section in
  `docs-site/docs/reference/cli.md` is scoped in — mirroring `definition_deltas.md` §Surface and
  `cli.md` §"Exit codes" rather than inventing surface. (c) Hard constraint on the rewrite: the
  guide is under a doc-sync gate (`crates/smelt-logical/tests/backbuild_docs.rs`) whose sweep fails
  on any ```sql fence lacking a `backbuild-example` marker and whose `registry_matches_guide_markers`
  fails on a dropped or renamed id — so every existing marked block stays verbatim and new CLI
  examples use ```text/```console fences. A standing `migrate_verb_is_documented` ratchet lands
  beside `no_backbuild_verb_in_user_docs` to keep the "no CLI command yet" claim from returning.

## Blocked

_(empty)_
