# Outcome: Wire the definition-delta synthesis layer (plan-and-approve migration)

**Created:** 2026-08-15
**Status:** active
**Source:** `docs/research/20260811-delta-signatures-and-definition-deltas.md` §6 step 2
**Spec anchors:** `docs/specs/definition_deltas.md`, `docs/specs/incremental_models.md`,
`docs/specs/incremental_shapes.md`

## The outcome

The classification and emission machinery for definition changes
(`crates/smelt-logical/src/backbuild/`: diff factoring, per-group verdicts, the technique
catalogue, script assembly) stops being dead code and becomes the `smelt migrate` verb that
`docs/specs/definition_deltas.md` already specifies: a definition change (a redefined column, a
changed grain-adjacent field) is classified per column group into a verdict (eclipsed / backfill
in place / re-derive / skeleton change), a plan is printed naming the technique per group, and
`--apply` only executes a plan whose hash matches what was printed and approved — giving CI a
gate against unreviewed migrations. The ranged-rebuild verb ships under its spec name
(`smelt rebuild`, not `smelt backbuild`). The generative conformance suite exercises definition
edits, not only data deltas, so the equivalence invariant is checked for this mechanism the way
it already is for the maintenance ladder. This outcome closes the gap the incremental-spec
redraft (`docs/outcomes/20260809-incremental-spec-redraft/outcome.md`) deliberately left open:
that outcome specified this mechanism and recorded it as unwired; this one wires it.

**Scope statement, revised 2026-08-15.** `docs/outcomes/` is the only currently-live tracking
layer, and every other outcome in it is `done`; the `docs/plans/*` files that predate it are
themselves done, superseded, or citing spec sections that no longer exist. That means every
still-live bullet in `definition_deltas.md`, `incremental_models.md`, and `incremental_shapes.md`
§Known Divergences has, in practice, no current owner but this outcome. This outcome therefore
covers not just the migrate/rebuild mechanism but closing out the remaining Known Divergences of
all three anchor specs — implementation gaps get a phase that closes them; open questions where
intent itself is undecided get a phase that decides them (recording the decision in the spec, the
same pattern phase 1 already used) wherever the decision is small enough to make in-outcome.

**Boundary, re-affirmed 2026-08-15 (see decision log).** A bullet only gets a phase — here or in a
spawned outcome — when the spec has already decided the target behaviour and only the
implementation is missing, or when the decision needed is small enough to make in the same sitting
(the open-questions phase's pattern: record the call, drop the "(Open Question)" tag, move on). A bullet the
spec itself flags as undecided (an explicit `(Open Question)` tag, or a `§Future Extensions` entry
stating "not decided ... via its own spec diff") is **not** implemented here or in any spawned
outcome — deciding it is a product call this program does not make unilaterally. Every such bullet
is named below with the reason it's excluded, not silently dropped.
`docs/outcomes/20260815-incremental-spec-closure-confirm` is the final audit confirming that
boundary was actually honoured: every closeable bullet closed, every excluded bullet still
accurately described as open.

## Success criteria (checkable)

1. `smelt migrate` exists as a CLI verb: given a model whose definition changed, it invokes the
   backbuild synthesis layer (diff → classify → emit) and prints a plan (per-group verdict +
   technique), without executing anything.
2. `smelt migrate --apply` executes only a plan whose stored hash matches the plan just
   re-derived; a stale or unapproved plan refuses with a distinct CI exit code. An approval
   store persists the hash (§"No approval store exists" divergence closed). The open question
   "plan-hash scope" (`definition_deltas.md` §Known Divergences) is resolved and the decision is
   recorded in the spec, not left implicit in the code.
3. The ranged-rebuild verb is named `smelt rebuild` (renamed from `smelt backbuild`) end to end:
   CLI, `--help`, docs-site, examples, tests.
4. The generative maintenance-conformance suite (`cargo test -p smelt-cli --test
   maintenance_conformance`) gains a definition-edit step kind — staged definition changes mid-
   history, asserted against the full-refresh-on-new-definition oracle — closing "The
   conformance harness has no definition-edit step kind yet".
5. The atomicity divergence is resolved one way or the other, not left "conditional in
   practice": either the `schema_evolution: strategy: full_refresh` escape routes through the
   same migration gate as every other backfill-in-place field, or it gets a real repair path.
   Whichever is chosen is stated in the spec, and the divergence bullet is removed.
6. `MaintenanceSkeletonColumnAdded` is renamed or split per the spec's own noted decision
   (`MaintenanceSkeletonChanged` or a split add/changed pair), and the definition-change
   diagnostic is surfaced ahead of a run (LSP + `smelt explain`), not only reachable via the
   maintenance driver's own I/O path.
7. A docs-site migration guide page ships (`definition_deltas.md` §References currently says
   "none yet — lands with the wiring plan"). `docs-site/docs/guide/backbuild-synthesis.md` —
   today's "no CLI command for this yet" placeholder for this exact mechanism — is rewritten in
   place (not left beside a new page) to document `smelt migrate`/`--apply` end to end, and its
   "Naming: two things called 'backbuild'" callout is removed now that the two verbs have
   disjoint names.
8. `/smelt:validate definition_deltas` reports no drift; every Known Divergences bullet this
   outcome claims to close is actually removed from `definition_deltas.md`, not just addressed
   in code. In addition, every other spec that names `smelt backbuild`, references
   `MaintenanceSkeletonColumnAdded`, or records "no `smelt migrate` command exists" as a
   divergence is swept so it matches the shipped surface — not a full clean `/smelt:validate`
   of those files (they carry unrelated divergences this outcome doesn't own), but these
   specific bullets:
   - `docs/specs/cli.md` — verb table (line 25), `smelt run` vs `smelt backbuild` section (336,
     340), `--dry-run` behaviour (356, 364, 367, 510), and the `incremental_models.md`
     cross-reference (582) all rename to `smelt rebuild`.
   - `docs/specs/model_selection.md` — the positional-selector callout (line 54) renames to
     `smelt rebuild`.
   - `docs/specs/architecture.md` — the `walk_coverage` module-path reference and the backbuild
     emitter-parity paragraphs (415, 424, 484, 513) update their prose to the `smelt rebuild`
     name where they mean the CLI verb (the `backbuild/` crate module path and "backbuild
     synthesis" as the definition-delta mechanism's name may stay, since those aren't the
     renamed verb — resolve which is which per bullet, don't blanket-replace).
   - `docs/specs/models.md` (line 244, 346) and `docs/specs/seeds.md` (line 180) — both record
     "the `smelt migrate` assist does not exist" / "no `smelt migrate` command exists" as an
     open divergence; both bullets are removed once `smelt migrate` ships (phase 2) and rechecked
     against what it actually does for a retired-`refresh:`-value fix-it and a seed-migration
     assist respectively — if `smelt migrate` doesn't cover those cases, the bullets are
     reworded to say so precisely rather than deleted wholesale.
   - The diagnostic-rename sibling sweep already named in the decision log above
     (`model_transforms.md`, `model_properties.md`, `incremental_models.md`,
     `schema_evolution.md`, `diagnostics.md`) lands in phase 7 as previously decided.
9. All standing gates green, including the new/extended conformance suite, `statement_parity`,
   and `walk_coverage`.
10. The orphaned scheduler-dispatch gap closes: a `KeyedUpsert` upstream feeding a
    `grain: partition` downstream derives a key-addressed repair cell today (`output-delta-typing`
    shipped the derivation), but the run loop dispatches it only inside the `grain: key` run
    branch — the `grain: partition` branch falls back to the ordinary (correct, non-incremental)
    run route. Wire dispatch so a `grain: partition` downstream also takes the key-addressed
    route when one is derivable, and remove `incremental_models.md` §Known Divergences' "The
    scheduler does not yet consume delta signatures end to end" bullet (narrowing or deleting it
    to whatever residue remains — the clockless-cross-model-watermark and value-level-discovery
    clauses are a different, still-open piece of that bullet and stay).
11. Per-cell frontier addressing lands, closing two divergences that both name it as their
    blocker: **per-cell `deferral` is scheduled** (not just parsed/validated/printed —
    `incremental_models.md` "Per-cell `deferral` is not yet scheduled"), and **`diff_patch` over
    the region `DeleteInsert` default gets a runtime lowering** rather than failing loud by name
    unreached (`incremental_models.md` "`diff_patch` over the region `DeleteInsert` default has
    no runtime lowering"). Both bullets' cited trackers (`contract-lattice-v1`, `repair-family`)
    are `done` outcomes that left exactly this residue.
12. The write-pin equivalence factor stops being structural-only: the per-cell equivalence hook
    threads real column-comparability (or a suppression-specific proof) instead of always
    accepting, and an inadmissible write-*variant* pin gets a pre-execution gate — forcing
    `technique: suppress` on a refusing cell refuses rather than silently falling back to full
    recompute, and `smelt explain` shows the refusal. Closes both `incremental_models.md`
    "write-pin equivalence factor is structural only" and "inadmissible write-*variant* pin has no
    pre-execution gate".
13. Observed-delta consumption stops being partial: `--since-upstream` reads the recorded delta
    table live; backward resolution, the keyed-fold write family, and the staged-candidate write
    family all record and consume observed deltas; the settle-bound × observed-delta composition
    gets its "delta empty" leg. Closes `incremental_models.md` "Observed-delta consumption is
    partial".
14. The maintained-model-creation cell gets a real execution technique (not the ordinary run
    loop), closing "No execution technique keys off a maintained-model creation cell". The
    frontmatter-time grain-checking gap closes too: a `grain: key` model deriving identity from
    `GROUP BY` (no top-level `unique_key:`) is checked at frontmatter validation, not only plan
    derivation (cross-ref `models.md`).
15. The plan-consumer and graph-layer gaps close: the horizon-clamped partition-local mutation
    quadrant is reachable from a real workspace fixture; dispatch distinguishes "a mutation
    genuinely happened" from re-derivation; the `prefer` soft-bias ladder and
    `scan_bounds.on_violation: warn` are consumed (not every refusal is `Error`); `AppendOnly`
    sources get an `UpstreamMutation` cell; bare `grain: key` nodes with no admitted locality get
    a real fixture past `MaintenanceGraphUnsupportedNode`; time-unrolled self-edges are built; a
    key-level dirt representation exists in the graph layer (not only intervals);
    `examples/web_analytics` is fully `--since-upstream`-compatible end to end; `--select` scoping
    exists. Closes `incremental_models.md`'s "Plan-consumer gaps" and "Graph-layer gaps" bullets.
    The cost model between two admissible techniques (also named in "Plan-consumer gaps") is
    explicitly **not** required here — building a real cost model is its own body of work; this
    criterion only requires that where no cost model exists, the choice is principled and
    documented (e.g. a fixed preference order), not left as "unbuilt" with no fallback stated.
16. The maintenance-plan proof residues close: a locality-admitted keyed model's clamps carry a
    derived (not assumed) write-footprint mirror into propagation; column-group-scoped dirt no
    longer coarsens to whole-partition where the finer grain is derivable; hour-granularity
    propagation matches its declared surface (not day-ordinal); `INTERSECT`/`EXCEPT` get a real
    per-arm-cardinality classification (not blanket whole-model mutation-sensitivity). Closes
    `incremental_models.md`'s "Locality and diagnostic residues" and "`INTERSECT`/`EXCEPT` are
    unclassified set operations" bullets (cross-ref `model_properties.md` §Known Divergences,
    swept where it names the same residue).
17. The conditional-maintenance gaps close: `smelt explain --show-sql` renders the suppressed
    form a live run actually executes (not only the unconditional matched arm); the region
    DELETE+INSERT family gets a conditional variant; the whole-row (keyless) staged-candidate
    realisation exists; a `write:` pin selects between keyed MERGE and staged-candidate;
    delta-restriction admission consumes an external `mutable_snapshot` source's
    fingerprint-sidecar delta. Non-DuckDB targets keeping the widened-scan recompute is
    acceptable to leave as a stated backend-capability gap, not a defect, provided it's declared
    via the capability struct rather than silently falling back. Closes `incremental_models.md`'s
    "Conditional-maintenance gaps" bullet.
18. The remaining decidable Open Questions across all three specs are resolved and the decisions
    recorded in the owning spec (§Design + §Known Divergences update), following the phase-1
    precedent — each decision below is small enough to make without further product input:
    - **No out-of-band-edit tripwire**: decided not worth a digest tripwire in v1 (cost exceeds
      benefit for a rare, self-inflicted failure mode) — `incremental_models.md` records the
      decision and drops the "(Open Question)" tag, replacing it with a stated non-goal.
    - **`on_column_add` policy knob**: superseded by `smelt migrate`'s per-group verdict (backfill
      in place / re-derive / skeleton change already answers "what happens when a column is
      added"); the proposed standalone knob is dropped from `incremental_models.md` as redundant
      once phase 2 ships, not left as a parallel undecided proposal.
    - **docs-site CLI-surface coverage audit**: the residue is enumerated (a checklist, not a
      re-audit each time) and either documented or explicitly dropped; folds into phase 8's
      docs-site pass rather than staying an open-ended "(Open Question)".
    - **Group-merge-provenance policy** (a group merged across two mutable inputs): decided to
      force region recompute — the conservative, always-correct default every other
      mutation-sensitivity rule in this spec already takes; recorded in `model_properties.md`
      and/or `incremental_models.md` wherever the provenance rule lives.
    - **`change_feed` sources and `UpstreamMutation`**: decided to give `change_feed` sources an
      `UpstreamMutation` cell like every other mutation-sensitive source (consistency with the
      existing rule), while leaving "only full-input re-derivation is admitted" as the honestly
      still-open residue (live fold machinery for a change feed's delta shape is materially larger
      work — Future Extensions territory, not decided here).
    Genuinely large product calls are deliberately **not** decided here and are named instead in
    §"Out of scope — needs explicit sign-off" below.
19. Two small, decidable `incremental_shapes.md` key-grain correctness gaps close: a window-forward
    keyed run started with only one (or neither) of `--event-time-start`/`--event-time-end` refuses
    with a named diagnostic instead of silently dropping and recreating the target from a
    whole-source SELECT ("A window-forward keyed run with no event-time window silently
    full-refreshes instead of refusing"); and `safety_overrides:` on a key-addressed model becomes
    a hard configuration error at frontmatter validation, matching what §"Key-grain declaration
    (`grain: key`)" already states rather than parsing silently and being ignored ("`safety_overrides:`
    on a key-addressed model is not a hard error"). Both bullets are removed from
    `incremental_shapes.md` §Known Divergences.
20. `/smelt:validate incremental_models` and `/smelt:validate incremental_shapes` are run and
    every Known Divergences bullet this outcome's phases 11–29 close is actually removed from the
    respective spec (not just addressed in code) — the same discipline success criterion 8
    already applies to `definition_deltas.md`. Bullets this outcome deliberately does not close
    (the "Out of scope" list) stay, worded accurately rather than pointing at a done outcome as if
    it still owned them.

## Out of scope

`docs/outcomes/` is the only currently-live tracking layer — every `docs/plans/*` file predating
it is either fully done, superseded by a later outcome/spec, or (rarely) genuinely orphaned.
Genuinely orphaned bullets that are within the three anchor specs, and closeable *without* a new
product decision, are pulled into scope (phases 10–23 above, plus the two spawned outcomes below)
— not excluded. What's left here is either (a) material each spec itself frames as
deliberately-undecided future work (its own `§Future Extensions`, or a bullet naming a not-yet-
scoped next ladder rung / lattice point), which "implemented" cannot honestly mean for ideas the
author declined to commit to, or (b) an `(Open Question)` bullet whose resolution would widen
admission or add new surface the spec doesn't currently describe.

**2026-08-15 revision, reversed same day.** An earlier revision of this section queued nine new
outcomes to *build* everything below, including the deliberately-future material. That was reverted
after review: several of those items are explicitly undecided by the specs' own words (`(Open
Question)` tags, or `§Future Extensions`' "not decided ... via its own spec diff"), and building
against an undecided intent means inventing the decision on the spot — exactly what the spec-first
rule and this outcome's original discipline exist to prevent. Two outcomes survive from that
revision because their content turned out to be genuine known-divergences (spec already decided,
only implementation missing) once re-checked bullet-by-bullet against the spec text:
`docs/outcomes/20260815-keyed-grain-residue` and `docs/outcomes/20260815-partition-grain-residue`.
`docs/outcomes/20260815-incremental-spec-closure-confirm` still runs last, but now confirms only
that every closeable bullet is closed and every excluded bullet is still honestly described as
open — not that the excluded bullets themselves are gone.

**Deliberately future, per the specs' own framing — no spec diff exists to implement against:**

- **Lattice v2** (retention, reconciliation points, declared indifference, per-column-group
  freshness) — `incremental_models.md` §Future Extensions; research doc §6 step 3, sequenced
  after this step because it consumes this step's approved-destructive-legs machinery. No
  outcome exists yet.
- **Proofs as product** (`smelt prove`, `must_hold:`, proof-diff in CI, `smelt explain`'s
  guarantee-summary rewrite, the delta-signature headline and per-column guarantee summary
  `incremental_models.md` and `incremental_shapes.md` both flag as unprinted) —
  `incremental_models.md` §Future Extensions; research doc §6 step 4, deliberately last because
  steps 1–3 (including this outcome) change what the proofs say.
- **Smelt-maintained SCD2 via succession-pattern recognition**, **automatic watermark-diffed
  `--since-upstream`**, and **the observer / prefix-consistency contract for non-replayable
  combinations** — all three named explicitly in `incremental_models.md` §Future Extensions as
  "not decided ... may not be relied on or implemented against until it graduates ... via its own
  spec diff". The cross-model-runs-need-an-explicit-`--landed`-flag residue of the
  scheduler-currency bullet (phase 10) is this same automatic-watermark item under another name —
  left here, not duplicated as a gap.
- **Eclipse-detection breadth** (algebraic identities, join reorderings) and **row-local
  derivation for mid-catch-up groups** — `definition_deltas.md` §Future Extensions.
- **Retraction handling / change-feed consumption as a first-class delta shape** —
  `definition_deltas.md` §"What stays data-side". (Phase 21 does give `change_feed` sources an
  `UpstreamMutation` cell for consistency with every other mutation-sensitive source — that's a
  small, decidable admission-rule fix, not building retraction/change-feed folding.)
- **Ladder rungs 3–4** (group-rung retraction, the bounded-domain multiset) —
  `incremental_shapes.md` "Ladder rungs 3–4 remain specified ahead of this profile's use of
  them", explicitly deferred by the (`done`) `rung2-state-shapes` outcome's own §"Out of scope".
  Rung 2 is what shipped; rungs 3–4 are the next rung, not a rung-2 gap. `KeyedRetractableContribution`'s
  classifier/diagnostic plumbing is a genuine known-divergence closed in `20260815-keyed-grain-residue`
  — the retraction *fold path itself* still needs rung 3, which stays here.
- **Locality open questions** (slice pruning under snapshot-reconcile, relaxing the
  granularity-equality precondition, slice-scoped deletion), **key deletion beyond retention**
  (tombstones, opt-in hard delete), key temporal locality route 2's declared-FD-only admission
  (explicitly tagged `(Open Question)` — the key-derived-expression sub-route the spec never
  commits to), the wider locality-machinery gaps (route 3's DuckDB-binder-limited slice predicate,
  and granularity-determination/recurrence-precedence choices the spec text itself says are
  "underdetermined"), and `key_per_partition`'s missing plan derivation (the shape is *named* in
  §Overview but has no described execution model — designing one from scratch, not filling a gap
  in an existing one) — `incremental_shapes.md`, deferred to
  `docs/research/20260705-keyed-collapse-application.md` §5 as a design surface not yet drafted.

**Genuinely large product calls — flagged for explicit sign-off, not decided here:**

- **`docs/plans/20260704-model-updates.md`'s still-pending rows** (`C3`/`C4`: group-rung
  retraction and the bounded-domain multiset — same as ladder rungs 3–4 above; `D1`/`D2`/`D3`:
  `refresh: latest_value`, `refresh: versioned`, `refresh: materialized_view` classifiers and
  execution) target spec files that were deleted and consolidated into
  `incremental_shapes.md`/`incremental_models.md`. Whether D1–D3's scope survived that
  consolidation under a different name, was dropped as a decided non-surface, or is still
  genuinely wanted is unclear without checking each mode's fate individually — a real question,
  not a mechanical rename sweep.
- **`g_run >= g_part` auto-coarsening vs. reject-with-suggestion** (partition grain — explicitly
  "(Open Question) ... whether to auto-coarsen or reject-with-suggestion instead is undecided"),
  **snapshot-reconcile multi-unclocked-source admission**, **once-write nullability route for a
  key-derived expression (not just a bare column — widens the catalogue's four fixed spellings)**,
  **pattern functions (`smelt.latest`/`smelt.once`/`smelt.current`) as built-ins vs. a shipped
  template**, **driver granularity below `day`/`week`**, **`--auto` staleness fidelity beyond
  conservative v1**, **self-referential keyed models**, and **run-pinning alignment for
  `NOW()`/`CURRENT_*` in keyed models (today a deliberate hard refusal,
  `KeyedForbidsNondeterministic`, not merely an unfilled gap)** — all real design decisions with
  behavioural consequences (admission width, correctness posture, or a new surface keyword), not
  a small in-spec call like phase 21's list. Recommend scoping these as their own outcome(s) once
  a product owner is ready to decide them, rather than folding a design pass into an
  implementation-only outcome.

- **Broader backbuild statement-parity coverage** (the D2/B5/B6/E1/E2/E4/F1/F2/C1 techniques
  phase 30 left unexercised by real-fixture parity tests). The single-ownership rule is enforced
  structurally for every technique by the scan; per-technique executed-vs-emitted fixtures are
  breadth, not a success criterion, and each needs its own staged workspace.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Resolve the two open design questions (plan-hash scope, diagnostic rename/split) and land the decisions in `definition_deltas.md` before wiring against them | done |
| 2 | Wire `smelt migrate` (plan-only): CLI verb invokes the backbuild synthesis layer end to end and prints the per-group verdict/technique plan | done |
| 3 | Approval store + `--apply` + `--json`: plan-hash persistence, hash-mismatch/staleness refusal, machine-readable plan and the CI exit-code contract | done |
| 3b | `smelt run` refuses to fold data deltas over a pending non-eclipsed definition delta (spec §Detection), and the delta is reported by `smelt explain`/plan paths | done |
| 4 | Rename `smelt backbuild` → `smelt rebuild` across CLI, docs-site, examples, tests, and the spec sweep (`cli.md`, `model_selection.md`, `architecture.md` prose) named in success criterion 8 | done |
| 5 | Conformance harness gains a definition-edit step kind; wire into the generative equivalence suite | done |
| 6 | Close the atomicity divergence (unify the `schema_evolution` full-refresh escape with the migration gate, or land its repair path) | done |
| 7 | Diagnostic rename lands in code (`MaintenanceSkeletonChanged`) plus the sibling-spec sweep | done |
| 8 | docs-site migration guide: rewrite `guide/backbuild-synthesis.md` in place around `smelt migrate`/`--apply`, drop its stale "no CLI command yet" and naming-collision callouts; update `models.md`/`seeds.md`'s "no `smelt migrate`" bullets | done |
| 9 | Surface the definition-change diagnostic ahead of a run: plumb the deployed-schema snapshot into `smelt-db` as a Salsa world-fact input (CLI + LSP parity), so `MaintenanceSkeletonChanged` reaches LSP diagnostics and `smelt explain` without a run | done |
| 10 | Validate + close out: `/smelt:validate definition_deltas` clean, Known Divergences bullets removed (including the sibling-spec sweep in success criterion 8), full standing-gate sweep | done |
| 11 | Wire run-loop dispatch for a `KeyedUpsert`→`grain: partition` key-addressed repair cell (today derived but never dispatched outside the `grain: key` branch); narrow/remove the corresponding clause of `incremental_models.md`'s scheduler-currency divergence bullet | done |
| 12 | Per-cell frontier addressing: schedule per-cell `deferral`; runtime-lower `diff_patch` over the region `DeleteInsert` default | done |
| 13 | Write-pin equivalence: thread real column-comparability into the per-cell equivalence hook; pre-execution refusal gate for an inadmissible write-variant pin | done |
| 14 | Per-cell `deferral` dispatch: wire `deferral_cell_decisions` into the plain `Trigger::NewData` incremental fold dispatch (the only trigger family where `contract.cells[].deferral` is validly declarable), populating `deferred_cells`/`cell_frontiers` and narrowing the remaining half of the per-cell-deferral divergence | done |
| 15 | Observed-delta consumption (read side): live `--since-upstream` read of the recorded `_smelt_observed_delta` table; decide and record the backward-resolution clause (existence is not a change question — currency belongs to the ledger/`--auto`) | done |
| 16 | Observed-delta consumption (write side): keyed-fold and staged-candidate write families record their observed delta; the settle-bound × observed-delta composition gets its live "delta empty" leg | done |
| 17 | Maintained-model-creation execution technique; frontmatter-time grain check for `GROUP BY`-derived `grain: key` identity; fix the empty-key derivation `group_by_unique_key` returns for a `GROUP BY` column named `order_id` (phase 13 summary: confirmed keyword/substring collision on `ORDER`, silently breaks `grain: key` admission) | done |
| 18 | Consume the declared guardrail/preference config: `scan_bounds.on_violation: warn` admits the plan and reports a Warning; the `prefer`/`technique` choice ladder is consulted by the ordinary region path (`resolve_incremental_strategy`); the absent-cost-model fallback preference order is stated in the spec | done |
| 19 | Mutation-cell reachability: trigger derivation moves to a pure `smelt-logical` function and covers clocked explicitly-mutable sources plus aggregate-sensitive `AppendOnly` sources, so the horizon-clamped `PartitionLocal::Yes` corner is reachable from `examples/timeseries` through the production wrapper | done |
| 20 | Mutation-happened discrimination: a recorded per-source fingerprint baseline decides whether an `UpstreamMutation` cell dispatches or is recorded as a no-op, so dispatch distinguishes a genuine mutation from re-derivation | done |
| 21 | Keyed dirt cascades and is consumed: `propagate` walks a node dirtied only through the keyed channel, and `plan_since_upstream` schedules and reports keyed dirt — a bare `grain: key` model with readers propagates end to end past `MaintenanceGraphUnsupportedNode` | done |
| 22 | Time-unrolled self-edges: a backward-bounded self-referential model (`examples/web_analytics`'s `silver.sessions_chained`) builds a day-unrolled self-edge instead of refusing the whole-workspace graph | done |
| 23 | `--select` scoping for `--since-upstream`: the propagated plan intersects with the selector instead of ignoring it | done |
| 24 | `examples/web_analytics` end-to-end under `--since-upstream`: an open-ended propagated window (`start: Some(_), end: None`, phase 22's self-edge frontier) is resolved to a finite run window before `execute_project` instead of dying on `parse_run_window`'s "both or neither" guard, so the whole-workspace run completes | done |
| 24b | Bare-keyed→clocked-reader model-edge admission: `silver.device_user_edges`'s `RepairKeysNotDiscoverable` refusal (a grain column absent from a `KeyedUpsert` delta's row shape, discoverable by a key-projected lookup back into the upstream through a plain `FROM`), so every maintained `examples/web_analytics` model is scheduled; remove `incremental_models.md`'s "Graph-layer gaps" bullet | done |
| 25 | Reconcile the pre-execution maintenance-plan gate's admission posture with `smelt-runtime`'s narrower dispatch so real `deployed_column_names` can be threaded outside the maintenance driver | done |
| 26a | Derived (not assumed) write-footprint mirror: a `ScanClamp` carries the derived footprint or none; a keyed output's footprint is posed against its declared time axis, and propagation stops mirroring an underived clamp | done |
| 26b | `INTERSECT`/`EXCEPT` per-arm classification: a real per-arm-cardinality verdict instead of blanket whole-model mutation-sensitivity | done |
| 26c | Hour-granularity propagation: the graph layer's intervals match the declared `timeseries.granularity` surface instead of being day-ordinal, and edge grains derive from the model's declaration rather than the caller's | done |
| 26d | Finer-than-partition column-group dirt: dirt scoped to a column group stops coarsening to whole-partition where the finer grain is derivable | done |
| 27a | `smelt explain --show-sql` renders the write form a live run would execute: the change-suppressed matched arm for a `Suppressed` cell (column-scoped `MERGE` and keyed fold), resolved through the same `choice::resolve_write_suppression`/`resolve_write_variant` the driver uses, never the unconditional arm | done |
| 27b | Region DELETE+INSERT conditional variant: the recompute family's staged, change-suppressed route and its admission (`emit_staged_candidate_conditional_recompute` exists — establish what is actually unwired and close it) | done |
| 27c | Keyless (whole-row `EXCEPT ALL`) staged-candidate realisation for a region with no declared/proven key | done |
| 27d | `write:` pin selecting between the keyed `MERGE` and the staged-candidate mechanism: the pure selection layer (`resolve_keyed_write_mechanism` consults the pin, fail-loud) plus the folded staged-candidate select the merge-less keyed-fold realisation needs | done |
| 27e | Delta-restriction admission consumes an external `mutable_snapshot` source's fingerprint-sidecar delta | done |
| 27f | `window_independence`'s `Ordered` verdict must require `before > 0` for a same-partition self-read, matching the graph layer's refusal | done |
| 27g | Runtime dispatch for the 27d selection: thread the matching `write:` pin into the live keyed-fold write path (`cumulative.rs`), execute the staged-candidate group where pinned, extend `statement_parity`, and narrow the `incremental_models.md` Known Divergences bullet | done |
| 28a | Record the already-taken decisions in their owning specs (out-of-band-edit non-goal cross-reference, `on_column_add` supersession) and close the docs-site CLI-surface audit with a standing coverage gate | done |
| 28b | Pin the merged-group region-recompute rule: a column group whose sensitivity spans two or more mutation-sensitive inputs takes region recompute — audited, checked, fixture-pinned; bullet removed | done |
| 28c | `change_feed` sources get an `UpstreamMutation` cell like every other mutation-sensitive posture (plan-layer `MutationProfile` gains the kind); the Known Divergences bullet narrows to the still-open full-input-re-derivation residue | done |
| 29 | Close two key-grain frontmatter/CLI validation gaps: refuse a window-forward keyed run started with an incomplete event-time window instead of silently full-refreshing (`--full-refresh` stays the rebuild escape); give `safety_overrides:` on a key-addressed model its own hard frontmatter error instead of the misdirecting `PartitionGrainRequiresRefreshIncremental`, routed through the *resolved* grain so a derived partition shape stops being over-refused | done |
| 30 | Extend `statement_parity`'s byte-identical structural leg to the backbuild emitter family; remove the correspondingly narrowed `architecture.md` Known Divergences bullet | done |
| 30b | Schema-evolution DDL second author: `smelt-state`'s `ddl_duckdb.rs` builds model-table `ALTER TABLE … ADD/DROP COLUMN` text beside `backbuild::emit`'s `emit_alter_add_column`/`emit_alter_drop_column`; route it through the single-owner emitters (or record a justified per-dialect exception) and widen the structural scan to cover it | done |
| 31 | Validate + close out (extended): `/smelt:validate incremental_models` and `/smelt:validate incremental_shapes` clean for every bullet phases 11–29 close, alongside the existing `definition_deltas` validate in phase 10 | pending |

## Decision log

- **2026-09-03, phase 30b — schema-evolution DDL declared a separate single-owner family,
  scan widened to `smelt-state`.** `smelt-state`'s `schema_tracking.rs` safe-path loop and the
  nested-change fast path built their own `ALTER TABLE`/`UPDATE` text beside `ddl_duckdb.rs`'s
  `generate_duckdb_ddl`. Extracted six renderers (`render_add_column`, `render_drop_column`,
  `render_alter_column_type`, `render_set_not_null`, `render_drop_not_null`,
  `render_backfill_update`) in `ddl_duckdb.rs` and routed both authors through them; widened
  `statement_parity`'s structural scan to `smelt-state/src` with the three `ddl_<dialect>.rs`
  modules as declared per-dialect exclusions. A quoting bug surfaced as a side effect: the safe
  path previously interpolated column names unquoted, so a keyword-named column (`order`) would
  have produced invalid SQL; delegation fixes it for free.

- **2026-09-03, phase 30 — backbuild joins `statement_parity`; B3 fixtures need an explicit
  key pull-through column and no `SELECT *`.** Three new tests drive `definition_delta::
  {derive_plan, apply_migration}` directly (B1 in-place backfill, the `SkeletonChange` →
  `FullRefresh` fallback, B3 upstream backfill), each proving executed SQL byte-identical to a
  direct `backbuild::emit` call plus result multiset-equality to a full refresh. Discovered along
  the way: `infer_deployed_columns` returns zero columns for a `SELECT *`-projected VALUES model,
  so schema tracking silently skips saving its deployed schema — any fixture leaning on an
  upstream's declared/inferred NOT NULL facts (B3's grain-link proof) needs an explicit column
  list. `smelt-state/ddl_duckdb.rs`'s own `ALTER TABLE` DDL authoring (phase 30b's scope) was
  confirmed still present and untouched.

- **2026-09-03, phase 29 — windowless window-forward keyed runs refuse; `safety_overrides:`
  on a keyed model gets its own diagnostic.** `execute.rs`'s windowless keyed dispatch now
  `bail!`s (mirroring the pre-existing snapshot-reconcile arm) unless `request.full_refresh` is
  set. `validate_timeseries` splits its `batched.is_some()` check: a resolved `grain: key` shape
  (via `resolved_grain()`, not the literal field, so a derived-partition model isn't
  over-refused) gets the new `KeyedForbidsSafetyOverrides`; kept `refresh: incremental` as an
  explicit precondition alongside the derived-grain check — dropping it (as a literal reading of
  the plan's phrasing would) silently admitted a non-incremental model's folded
  `safety_overrides:`, breaking two pre-existing tests. `smelt build` had no `--full-refresh`
  flag at all, so 12 of `example_web_analytics.rs`'s own fixture calls needed it added
  end-to-end (CLI flag → `ExecuteRequest`) before they could pass one. See
  `phases/29-summary.md`.
- **2026-09-03, phase 28c — `change_feed` sources get an `UpstreamMutation` cell, clamped to
  full-input re-derivation.** Added `MutationProfile::ChangeFeed` to the plan layer with a
  single-owner `is_mutable()` predicate; `derive_triggers` derives the cell unconditionally from
  the declaration (no `explicitly_mutable` gate needed, unlike `mutable_snapshot`); the cell's
  technique is clamped to `RecomputeRegion`/`DeleteInsert` (no column-scoped merge); the
  fold-repair narrowing branch refuses fail-loud for a `ChangeFeed` posture rather than attempting
  a fingerprint-sidecar discovery that doesn't exist for a feed. `repair::discovery_posture` now
  returns `Option` so that non-existence is a typed, checked case, not a silent default. The Known
  Divergences bullet narrows from "does not yet get a cell" to "always re-derives from the full
  input" — the honestly-open residue (live fold over the feed's delta shape) stays in §Future
  Extensions, unchanged.
- **2026-09-03, phase 28b — merged-group region-recompute rule enforced, not just declared.**
  `derive_mutation` was calling the corner/technique choice per source alone, so a column group
  value-sensitive to two mutation-capable inputs got two independent `ColumnScopedMerge` cells —
  the exact shape §"The plan matrix" forbids. Added a guard: a group whose mutation-capable
  input count (sources actually deriving an `UpstreamMutation` trigger, read off the same
  `covered_by_mutation` set `derive_triggers` already computes) is ≥ 2 forces region recompute,
  same as the pre-existing membership-sensitivity branch. Pinned with 4 hand-built-`ModelInputs`
  unit tests plus one real-derivation-path fixture; the "unverified in the implementation"
  Known Divergences bullet is removed.
- **2026-09-03, phase 28a — recorded two taken decisions; closed the docs-site CLI-coverage
  divergence with a standing gate.** `incremental_models.md`'s "no out-of-band-edit tripwire"
  cross-reference now points at §"Other deliberate boundaries" (the non-goal) instead of a
  stale "Open Question, §Known Divergences" framing. `definition_deltas.md` §Design gained a
  paragraph recording that a per-model `on_column_add: backfill | leave_null | recompute` knob
  was considered and dropped — the per-column-group verdict already answers the question
  case-by-case. New `crates/smelt-cli/tests/cli_docs_coverage.rs` walks `Commands`/
  `DocsCommands` and every `*Args` struct's long flags in `main.rs` and asserts each is
  documented verbatim in `docs-site/docs/reference/cli.md` (or listed in an
  `UNDOCUMENTED_BY_DESIGN` allowlist, checked two-sided like the hardening baseline). The audit
  found **zero residue** — cli.md already documented every command and flag — so the "docs-site
  coverage … is partial" Known Divergences bullet was deleted outright rather than narrowed.
- **2026-09-03, phase 27g — keyed-fold `write:` pin dispatch wired at runtime.** `WindowedKeyedRule`
  gained `write_group`, resolving `KeyedWriteMechanism` (27d) into an actual `StatementGroup`;
  `run_windowed_keyed_maintenance` resolves the mechanism once, up front, from the driving
  source's matching `cells[].write` pin (`smelt_db::queries::maintenance::keyed_fold_write_pin`).
  The `Grade::Additive` ledger branch refuses fail-loud on a multi-statement `action_group`
  (`fold_ledger_delta` only wraps one action statement) — not reachable by any current fixture,
  but guarded rather than assumed impossible. `incremental_models.md`'s Known Divergences
  "Conditional-maintenance gaps" bullet dropped its "no `write:` pin selects…" clause.
- **2026-09-03, phase 27f — same-partition self-read refused as non-convergent.**
  `self_edge_bound_days` now requires `before.0 > 0` (not just `after == 0`) to admit a
  self-edge as `Ordered`; a zero-backward self-read is circular, refused with a reason naming
  the model. `build_forward_graph` now refuses this shape itself at the `self_edge_clamp` call
  site rather than deferring to `propagate.rs`'s later generic `before_seconds <= 0` gate — the
  two layers refuse at one place with one reason. Two pre-existing fixtures encoded the old
  two-layer behavior and were updated in place (`since_upstream.rs`'s
  `self_referential_node_refuses_fail_loud`, `since_upstream_propagation.rs`'s
  `same_partition_self_referential_model_refuses`).
- **2026-09-03, phase 27e — delta-restriction admission consumes an external `mutable_snapshot`
  source's fingerprint-sidecar delta.** `RestrictionDeltaSource` enum generalizes
  `execute_delete_insert_with_delta_restriction` over the model-edge and external-source routes;
  `resolve_live_external_delta_restriction_facts` resolves the live `UpstreamMutation` cell for an
  explicitly-mutable external source, gated on the new `BackendCapabilities::
  supports_fingerprint_sidecar` (DuckDB only). Found and fixed two bugs blocking this phase's own
  end-to-end test: `emit_count_preservation_probe_from_body` never unwrapped `inject_time_filter`'s
  output-clamp wrap (silently defeating the declared-`referential_integrity` probe — and therefore
  delta restriction — on every real, time-filtered run for BOTH this route and the pre-existing
  model-edge route, not just this phase's own); and the probe's `enrichment_source` must be the
  join's physical table text, not the closure's bare logical address (they only coincide for a
  model edge). See `phases/27e-summary.md` — flags an untested adjacent gap: whether the
  model-edge route's `DeclaredReferentialIntegrity` closure now actually restricts on a real
  `execute_project` run post-fix (no test added for that route specifically).
- **2026-09-03, phase 27a — `--show-sql` previews resolve write suppression, matching a live run.**
  A new shared resolver (`smelt_logical::maintenance::choice::resolve_cell_write_suppression`)
  folds the P2/P3 proof + override ladder that `maintenance_driver.rs`'s live `ColumnScopedMerge`
  path already ran inline; the `smelt-runtime::diagnostics` preview builder now calls it (and, for
  `KeyedFold`, mirrors `cumulative.rs`'s own raw-proof-only live resolution — deliberately NOT the
  override-folding one, since that live path doesn't fold overrides today). Discovered gap for a
  future phase: KeyedFold's live write-suppression resolution ignores override pins and first-build
  posture entirely, unlike `ColumnScopedMerge`'s. See `phases/27a-summary.md`.
- **2026-09-03, phase 27b — the region DELETE+INSERT family gains its change-suppressed conditional
  variant, scoped to model-edge-sourced creation cells.** `choice::resolve_region_write_variant`
  composes the existing P2/P3 proof (no new proof logic); `build_delete_insert_group_dispatched`
  gained a third dispatch arm calling `emit_diff_patch` over the region's own slice predicate.
  Wired only inside `DeltaRestrictionFacts`/`resolve_live_delta_restriction_facts` — an
  external-source-only region `Trigger::NewData` cell executes via a different `Backend` trait
  method entirely (`execute_model_incremental`) and never reaches `build_delete_insert_group_
  dispatched`, so it is unaffected and still unconditional (recorded honestly in both specs'
  Known Divergences, not silently narrowed). `smelt explain --show-sql` is also NOT wired to this
  dimension for region cells (confirmed by grep — no reference in `explain.rs`), unlike
  `ColumnScopedMerge`/`KeyedFold`. See `phases/27b-summary.md`.

- **2026-09-03, phase 26b — per-arm `INTERSECT`/`EXCEPT` classification lands.**
  `grouping::derive_column_groups` no longer collapses a set-op model whole-model: a chain of one
  repeated operator gets a real per-arm verdict (value provenance unions/first-arm-only per output
  position, membership sensitivity couples every arm's referenced sources for every operator except
  `UNION ALL`), with fail-closed fallback for a mixed-operator chain, a nested compound arm, an
  arity mismatch, or an arm with its own unresolvable reference. Caught and fixed a real regression
  along the way: the single-`SELECT` path's refactored column accumulator briefly lost the
  alphabetical-by-alias column ordering a repair-recipe write pin exact-matches against — only
  `cargo test --workspace` surfaced it, not any of the phase's own named test targets. See
  `phases/26b-summary.md`.

- **2026-09-03, phase 26c — propagation intervals move from day ordinals to exact seconds.**
  `PartitionInterval` (renamed from `DayInterval`) now stores exact seconds since the civil
  epoch, not day ordinals; `PartitionGrain` gained `Hour`, `Week { start_dow }`, `Quarter`, and
  `Year` alongside `Day`/`Month`, each with real civil-boundary `align_outward`. The margin/
  footprint split moved with it: an `Edge`'s `before_seconds`/`after_seconds`/`footprint_seconds`
  (renamed from the `_days` fields) now carry the clamp's EXACT margin, never pre-ceiled to whole
  days — ceiling to the receiving axis's own partition boundary happens once, in
  `align_outward`, not twice (once at margin capture, once at alignment). `smelt-runtime`'s
  `granularity_grain` is now total over every `Granularity` variant (no more
  `MaintenanceGraphUnsupportedNode` for `hour`/`week`/`quarter`/`year`), threading the declared
  `week_start` through a new `smelt_core::config::Weekday` → `chrono::Weekday` conversion point.
  A rendering seam (`iso_floor`/`iso_ceil` in `smelt-runtime::propagation`) aligns a propagated
  interval outward to whole days only at the CLI-facing boundary (`smelt run` windows are
  date-valued even when the underlying dirt is sub-day). See `phases/26c-summary.md`.

- **2026-09-03, phase 26d planning — group-scoped dirt is licensed by the existing
  closure-prune proof, not by a new one.** `grouping.rs` already proves, per enrichment source,
  that a join is `Closed` (row-preserving) and prunes its membership contribution; that is exactly
  the "this source's delta can revise values but never add or remove rows" fact a narrower dirt
  scope needs. 26d surfaces it as `GroupingResult::value_only_sources` rather than deriving a
  second creation-reaching classifier. Everything else — a creation-reaching source, a degenerate
  collapse, an upstream naming no group, an untyped outbound edge, a node also dirtied by an
  unscoped edge — stays whole-model (widen-never-narrow). No work leaves the outcome.

- **2026-09-03, phase 26d planning — the residual "grain-alignment check validates only the
  declaration" clause is posture, not a defect.** Same call as 26c's: §"Granularity is declared,
  not derived" is the normative rule, so once 26a/26c/26d have deleted their clauses the whole
  "Locality and diagnostic residues" bullet goes, closing success criterion 16 without a
  derived-granularity classifier.

- **2026-09-03, phase 26c planning — the "graph edges take the declaration directly" clause is
  posture, not a defect.** `incremental_models.md` §"Granularity is declared, not derived" is the
  normative rule (deriving the grain from a `date_trunc` projection would let a refactor silently
  change scheduling semantics), so 26c closes that clause by making `Edge`'s constructor *require*
  both declared grains rather than defaulting to Day, and deletes the clause from Known
  Divergences instead of building SQL-derived grains. No work leaves the outcome.

- **2026-09-03, phase 27d — `write:` pin selects the keyed-fold mechanism (`MERGE` vs.
  staged-candidate), plan-layer only.** `choice::resolve_keyed_write_mechanism` now consults an
  optional `write:` pin: `keyed`/`keyed_conditional` pin `MERGE` (fail-closed refusal on a
  merge-less backend, defence-in-depth behind the registry's own capability gate);
  `staged_candidate` pins the staged conditional shape even on a `MERGE`-capable backend and
  refuses (never substitutes) over an `Unconditional` suppression verdict. New
  `emit::keyed_fold_candidate_select` builds the post-fold candidate rows
  (`combiner(stored, delta)` per matched key, raw delta value per delta-only key) the
  staged-candidate mechanism needs to realise a keyed fold. Deliberately not wired into any live
  path — `resolve_keyed_write_mechanism` has no production call site yet; phase 27g threads it
  into `cumulative.rs`'s live write path. See `phases/27d-summary.md`.

- **2026-09-03, phase 26 reshape — split into 26a–26d (one residue each).** The single row
  bundled four independent proof residues touching four different layers (clamp derivation,
  set-operation classification, the propagation interval unit, column-group dirt scoping); each
  carries its own spec delta and its own regression surface, and the hour-granularity change
  rewrites the interval type the column-group work builds on. Ordered
  footprint → set-ops → granularity → column-group dirt so the interval-type change lands before
  the dirt scoping that composes with it. No work leaves the outcome: success criterion 16 still
  requires all four, and the "Locality and diagnostic residues" bullet is narrowed clause by
  clause (26a, 26c, 26d) rather than deleted early.

- **2026-09-03, phase 24 — open-ended run windows resolve; whole-workspace
  `examples/web_analytics` under `--since-upstream` completes.**
  `resolve_run_window` (`crates/smelt-runtime/src/propagation.rs`) resolves a
  `(start: Some, end: None)` propagated run (a time-unrolled self-edge's
  frontier) to `[start, today + 1 day)` against the same `now` the
  propagation planner already takes; a `start` on/after that resolved end
  refuses fail-loud naming the model. Wired into `run_since_upstream`'s
  per-run loop ahead of `ExecuteRequest` construction. The whole-workspace,
  unfiltered `--since-upstream --dry-run` run over `examples/web_analytics`
  now completes (7 `RUN` lines, exit 0) rather than dying on
  `parse_run_window`'s "Both start and end" guard. `docs/specs/
  incremental_models.md` §"Time-unrolled self-edges" and `docs-site/docs/
  reference/cli.md` state the resolution rule. See `phases/24-summary.md`.

- **2026-09-03, phase 23 — `--select` scoping lands.** `scope_plan_to_selection`
  (`crates/smelt-runtime/src/propagation.rs`) intersects a computed `SinceUpstreamPlan` with the
  ordinary CLI selector: propagation stays whole-workspace, only execution narrows. Reuses
  `smelt_runtime::select::select_executable_models` for the selection pass and
  `DependencyGraph::get_upstream` for the direct-upstream refusal check, rather than a bespoke
  model-name filter — keeps `--select`/`--exclude` semantics identical to the ordinary run path.
  `incremental_models.md`'s "Graph-layer gaps" bullet no longer names missing `--select` scoping.

- **2026-09-03, phase 22 — time-unrolled self-edges land.** A strictly time-backward
  self-referential model (`after_days == 0`, `before_days > 0`, day/month axis) is admitted into
  the propagation graph as a day-unrolled self-edge instead of refusing the whole-workspace
  graph; forward dirt widens to the frontier, backward requirements reach the model's own basis
  once. A same-partition self-read (`before_days == 0`) is still refused — not "strictly
  backward". `examples/web_analytics`'s `silver.sessions_chained` now builds in the unfiltered
  whole-workspace graph. Surfaced but left open: `window_independence`'s own `Ordered` verdict
  doesn't check `before > 0` (a narrower pre-existing gap in the ordered-backfill execution path,
  not the graph layer); and an open-ended `PropagatedRun` (`start: Some, end: None`) has no
  `execute_project` wiring yet (`parse_run_window` still requires both-or-neither) — see
  `phases/22-summary.md`.

- **2026-09-03, phase 20 — mutation-happened discrimination lands.** Closed the "Plan-consumer
  gaps" bullet entirely: every live `UpstreamMutation` dispatch site (keyed column-scoped-merge,
  keyed membership-recompute, non-keyed column-scoped-merge) now compares a recorded per-source
  whole-source fingerprint against the source's current state before dispatching, recording a
  no-op when unchanged. This closed a real, previously-undocumented-as-such behavior change in
  several existing tests that had encoded the old "fires every run regardless" divergence as
  their expected assertion (`technique_lowering.rs`, `maintenance_conformance/gate.rs`) — all
  updated; see `phases/20-summary.md` for the full list.

- **2026-09-03, phase-19 planning — table reshape.** Row 19 bundled three independent clauses of
  the same `incremental_models.md` "Plan-consumer gaps" bullet, two of which are plan-derivation
  work in `smelt-logical`/`smelt-db` (which sources get an `UpstreamMutation` trigger at all) and
  one of which is run-time state work in `smelt-runtime`/`smelt-state` (has the source actually
  changed since the last run). Split into 19 (trigger derivation + real-workspace reachability)
  and 20 (mutation-happened discrimination). Nothing left the outcome — both rows close clauses
  of success criterion 15, and the bullet is only fully removed once phase 20 lands.

- **2026-09-03, phase-18 planning — table reshape.** The single row 18 bundled ~10 independent
  items spanning two distinct `incremental_models.md` §Known Divergences bullets (Plan-consumer
  gaps, Graph-layer gaps) plus an unrelated `deployed_column_names` threading item. Split into
  four rows — 18 (declared guardrail/preference config consumption), 19 (mutation quadrant),
  20 (graph-layer gaps), 21 (`deployed_column_names` / gate-posture reconciliation) — and
  renumbered the trailing rows 19–24 to 22–27. Nothing left the outcome; success criterion 15
  is still covered in full by 18+19+20.
- **2026-09-03, phase 18.** Success criterion 15 explicitly does not require building a cost
  model, only that the choice between two admissible techniques be principled and documented
  where none exists. Decision: the fixed preference order is the one `resolve_cell_choice`
  already implements — validated `cells[].write` pin > hard `cells[].technique` pin (refuses
  loudly if unadmitted) > soft `prefer` > the cell's own admitted-and-live technique > region
  recompute — and it is stated in `incremental_models.md` §Design rather than left implicit in
  the resolver. The cost model itself moves to §Future Extensions.
- **2026-09-03, phase 18.** `scan_bounds.on_violation: warn` means the guardrail **admits** the
  plan and reports a Warning, rather than refusing with a downgraded-severity diagnostic — the
  spec already calls the guardrail "check-only: never modifies a derived clamp, only refuses
  (or warns)", and a warning that still refuses the cell would make `warn` indistinguishable
  from `error`.

- **2026-09-02, phase 14.** Per-cell `deferral` skip on the plain `Trigger::NewData` fold
  requires FULL coverage: a skip fires only when every one of the fold's own column groups is
  fully covered (union of matching declaring cells' columns) by cells that are ALL
  skip-licensed. A run that actually folds (partial coverage, an unlicensed covering cell, or
  measured lag past `D`) always advances every one of the model's own declaring cells'
  frontiers together — never a partial advance — since the plain fold's write is whole-row.
  "Fold groups" are the model's `derive_column_groups` output paired with the first
  `Trigger::NewData` cell's source, mirroring `resolve_incremental_strategy`'s existing
  single-creation-cell assumption.

- **2026-09-02, phase-15 planning — table reshape.** Renumbered every lettered phase row to a
  plain integer (`13b` → 14, `20b` → 23, everything after shifted): the loop's own row scanner
  (`.claude/scripts/outcome-loop.sh`, `next_step`) skips any row whose number does not match
  `^[0-9]+$`, so `13b` was invisible and its already-written plan would never have been
  implemented. `phases/13b-plan.md` renamed to `phases/14-plan.md`. Future rows use plain
  integers only. Also split the old observed-delta row in two: read side (15) and write side
  (16) — the read leg is CLI/propagation wiring, the write leg is `maintenance_driver` recording
  plus the settle-bound composition, and they share no code seam.

- **2026-09-02, phase 15.** Backward resolution (`smelt build --include-upstreams`) will NOT
  consume observed deltas, and that clause of the divergence bullet is closed as a stated
  non-goal rather than as unbuilt work. Backward resolution answers "what must **exist** over
  this period" — an existence question a change record cannot soundly narrow: a present-and-empty
  observed delta says a past run changed nothing, not that the region is current with respect to
  inputs that landed since. Currency is the reconciliation ledger's question (§"The frontier
  record", `smelt run --auto`), and skipping a required build on delta evidence alone would
  under-cover the resolved period, violating `forward(backward(P)) ⊇ P`.

- **2026-08-15, phase 1.** Plan-hash scope: hash the plan data structure the emitters consume
  (verdicts, techniques, input facts — source declarations, backend capabilities), not only
  rendered SQL text; exclude region *enumeration*, which is resolved at apply time from the
  frontier so `--apply` stays reachable on an actively-loading warehouse. Diagnostic
  rename/split: rename `MaintenanceSkeletonColumnAdded` to `MaintenanceSkeletonChanged` (one
  code, not a split add/changed pair) — add and change trigger identical refusal and
  remediation, and every other `Maintenance*` code names the refused condition, not its
  trigger. Both landed in `docs/specs/definition_deltas.md` §Design and §Known Divergences;
  §Surface and body prose now use the target names. The code-side rename and the sibling-spec
  sweep (`model_transforms.md`, `model_properties.md`, `incremental_models.md`,
  `schema_evolution.md`, `diagnostics.md`) are deferred to phase 7, since renaming a
  diagnostic code is itself a code change out of scope for this docs-only phase.
- **2026-09-02, phase 9.** `DeployedSchemaInput` world-fact input threads only `model_sql` into
  `maintenance_plan`/`maintenance_plan_report`, not `deployed_column_names` (kept `&[]`).
  Threading real column names too would widen the pre-execution diagnostic gate to derive a live
  `Trigger::ColumnAdded` cell whose own admission can refuse `MaintenanceScanUnbounded` for a
  column add that `smelt-runtime`'s narrower live-cell resolution still executes safely —
  confirmed via two real e2e regressions before scoping the fix down to the skeleton-clause
  check alone.

- **2026-09-02, phase 11.** Factored the key-addressed model-edge cell's resolve-then-execute
  body into a shared `resolve_and_dispatch_key_addressed_edge_cell` helper, then wired it into
  the non-keyed (window-forward) incremental branch's `Some(inc_plan)` arm alongside the
  existing keyed-branch call site, short-circuiting past that branch's self-ref bootstrap and
  batch loop via a labeled block when the cell resolves live. A `grain: partition` downstream
  fed by a clockless `KeyedUpsert` upstream now dispatches `Technique::PerGroupRecompute`
  instead of falling back to the ordinary window-forward batch loop.

- **2026-09-02, phase 12.** Shipped the `diff_patch` region-`DeleteInsert` runtime lowering
  (scoped to `resolve_live_membership_recompute_cell`, the one dispatch site the plan named) and
  the full per-cell-deferral pure/data-layer stack (`cell_address`, `IntervalStore.
  cell_frontiers`, `deferral_cell_decisions`, `ModelRunRecord.deferred_cells`). Did NOT wire
  per-cell deferral scheduling to any live dispatch site: `contract.cells[].deferral` is validly
  declarable only over a clocked, interval-representable `on:`, but every currently-wired
  per-cell resolver serves an `UpstreamMutation` trigger over a mutable/unclocked source — a
  structural mismatch discovered mid-phase, not a design question the plan left open. A real
  wiring needs the plain windowed-incremental/cumulative-fold dispatch (the `Trigger::NewData`
  family), a materially bigger change deferred to a future phase (see phase 12's summary).

- **2026-08-30, phase 8.** `backbuild-synthesis.md` rewritten around the shipped `smelt migrate`
  verb; corrected the "enumerates options; it does not yet choose" claim to state precisely what
  it does (first-admissible-option-per-group, no cost model) rather than dropping the caveat.
  Criterion 18's docs-site CLI-surface audit found no gap — every subcommand and `smelt run` flag
  is already documented in `cli.md` (see `phases/08-summary.md`).

- **2026-08-15, scope decision.** User directive: close every remaining Known Divergences /
  Open Question bullet across the three anchor specs to zero, choosing "build everything" over
  "decide and record a non-goal" for the items previously held as "needs explicit sign-off" or
  "deliberately future." Nine new outcomes were scaffolded (`20260815-keyed-open-questions-buildout`,
  `20260815-partition-grain-residue`, `20260815-key-locality-and-deletion`,
  `20260815-ladder-rungs-3-4`, `20260815-lattice-v2`, `20260815-scd2-watermark-observer-contract`,
  `20260815-retraction-and-changefeed`, `20260815-refresh-mode-consolidation-audit`,
  `20260815-proofs-as-product`), each owning one cluster of the previously-deferred bullets, plus
  a closing `20260815-incremental-spec-closure-confirm` outcome that audits zero-divergence once
  all of them (and this one) reach `done`. All ten appended to `.claude/outcome-backlog`
  immediately after this outcome. This outcome's own phase table (1–19) is unchanged by this
  decision — it still owns only the migrate/rebuild mechanism plus the sweep already scoped into
  it.

- **2026-08-15, scope decision reversed.** User pushback: "build everything" conflated
  implementing already-decided spec text with inventing decisions the specs themselves mark
  undecided (`(Open Question)` tags, `§Future Extensions`' explicit "not decided ... via its own
  spec diff"). Re-triaged every bullet in the nine spawned outcomes against the actual spec
  wording rather than the outcome's own framing. Result: `20260815-lattice-v2`,
  `20260815-proofs-as-product`, `20260815-scd2-watermark-observer-contract`,
  `20260815-retraction-and-changefeed`, `20260815-ladder-rungs-3-4`, and
  `20260815-refresh-mode-consolidation-audit` deleted outright — every bullet they owned is
  genuinely undecided-by-spec and returns to the "Out of scope" lists above.
  `20260815-key-locality-and-deletion` deleted too — on closer reading its surviving candidate
  items (route 2's key-derived sub-route, `key_per_partition`'s execution model) turned out to be
  explicitly open or undesigned rather than implementation gaps, leaving nothing genuinely
  build-only in it. `20260815-keyed-open-questions-buildout` (renamed `keyed-grain-residue`) and
  `20260815-partition-grain-residue` survive, trimmed to only the bullets where the spec already
  states the target behaviour in normative prose elsewhere (e.g. `KeyedRetractableContribution`'s
  semantics are already fully stated in §"Enrichment joins" and the Diagnostics table; the merge
  ledger's "every window-forward keyed model" and "transactional with the write it describes"
  are already unqualified normative statements the implementation just doesn't meet yet).
  `20260815-incremental-spec-closure-confirm` retargeted from "confirm zero Open Questions" to
  "confirm every closeable bullet is closed and every excluded bullet is still honestly open."
  `.claude/outcome-backlog` and this section rewritten to match.

- **2026-08-28, phase 2 planning.** Two reshapes, both narrow. (a) `--json` and the CI
  exit-code contract move from phase 2 into phase 3's row: the exit codes are defined by the
  approved/unapproved distinction, which does not exist until the approval store does, so
  splitting them across two phases would ship a `--json` whose contract is unrepresentable.
  Phase 2 keeps human-readable plan output only. (b) The plan **hash derivation** (a pure
  function over the plan data structure, per phase 1's decision) lands in phase 2 so the
  printed plan carries the hash it will later be approved by; phase 3 owns only its
  *persistence* and matching. Nothing left the outcome. Also noted while planning: the
  spec's §Detection refusal (`smelt run` must refuse to fold data deltas over a pending
  non-eclipsed definition delta) is behaviour no current phase row owns; it is not a
  Success-criteria item, and it is left for the phase 9 validate sweep to raise if
  `/smelt:validate definition_deltas` flags it as drift.

- **2026-08-29, phase 3 planning.** One reshape and two decisions. (a) Added row **3b**: the
  spec's §Detection rule — `smelt run` must refuse to fold data deltas over a pending
  non-eclipsed definition delta — is normative surface no phase row owned; phase 2's summary
  flagged it and `definition_deltas.md` §Known Divergences names phase 3 as its tracker. It
  serves success criterion 8 (`/smelt:validate definition_deltas` clean), so it gets a row
  rather than leaving the outcome. It is split out of phase 3 because it lands in the run loop,
  not the migrate verb. (b) **Exit-code contract**: a derived-but-unapproved non-trivial
  migration (and a stale/mismatched approval on `--apply`) exits **3** — a new code in
  `cli.md` §"Exit codes" meaning "the command ran correctly and found a state requiring human
  approval", deliberately distinct from `1` (found a problem in data/models) and `2` (bad
  invocation). (c) **Resume** is marker-based in this phase: the approval record carries an
  in-progress marker, and re-invoking `--apply` re-runs the identical (unchanged-hash) script;
  frontier-region-scoped resume per §"Frontier semantics" stays a stated divergence rather than
  being silently claimed.

- **2026-08-29, phase 3 implementation.** "Approved" is defined operationally: the plan step
  records `{plan_hash, in_progress: false}` unconditionally on every invocation, and the exit
  code reflects whether the store *already held this exact hash before this call* — so a second
  identical plan-step invocation (human review having happened out of band) exits `0`, while the
  first sighting of a given plan always exits `3`. This makes "the human ran `smelt migrate
  <model>` and it printed this exact plan" the approval act itself, with no separate confirmation
  step. `--apply` never writes an approval on refusal (absent/stale hash) — only a plan-step
  invocation can move the recorded hash forward, so a stale `--apply` cannot accidentally
  self-approve by retrying. `MigrationPlan::statements` (task 3) is assembled once via the
  existing `assemble(&BackbuildOptions, Selection::Targeted{atom_choices: all-zero})` — no new
  statement authoring. `MigrationPlan::all_rerun_safe()` was added (not in the original task
  list) to answer test 14's "chosen option not rerun_safe" check without re-deriving
  `BackbuildOptions` a second time in the CLI. Updated `migrate_plan.rs`'s two existing
  success-exit assertions to the new exit-`3`-for-unapproved contract (a legitimate contract
  change, not a definition-of-done drift — `docs/specs/cli.md` §"Exit codes" states it
  normatively). Hardening baseline updated (`smelt-cli` expect 41→42, println 169→171: one
  `serde_json` `.expect` and two `println!` in the new JSON/apply rendering paths, same pattern
  as `commands::diff.rs`'s existing `print_json`).

- **2026-08-29 (phase 3b planning).** The phase-3 summary flagged one untested `--apply`
  leg (an `in_progress` approval that is also `all_rerun_safe()` — should resume, not refuse).
  It serves success criterion 2 and the phase-3b gate reads the same `in_progress` flag, so it is
  folded into phase 3b as a test rather than becoming its own row. No other reshape: the summary
  surfaced nothing else outside an existing row.
- **2026-08-29 (phase 3b planning).** Single-owner call: the definition-delta derivation moves out
  of `commands/migrate.rs` into `smelt-runtime`'s new `definition_delta.rs`, so the run gate,
  `smelt explain`, and `smelt migrate` all read one derivation. Putting it in `smelt-runtime`
  (not `smelt-cli`) keeps the run-pipeline-parity rule intact — the UI gets the same refusal
  through `execute_project` without a CLI-only pre-flight check.
- **2026-08-29 (phase 3b planning).** The run refusal exits `3`, not `1`: per `cli.md`
  §"Exit codes", `3` means "a correctly-derived state a human has not yet reviewed", which is
  exactly a pending migration. A `--full-refresh` run is not a fold and is not gated.

- **2026-08-29 (phase 4 planning).** No reshape — the phase-3 summary's open items were
  already folded into rows 3b and 8. Two scoping calls recorded here: (a) **no compatibility
  alias** for `smelt backbuild` — it is removed outright rather than kept as a hidden or
  deprecated alias, since an alias preserves the exact naming collision the rename exists to
  end, and the project carries no back-compat constraint; (b) the rename is a **per-mention
  pass, not a blanket replace** — `crates/smelt-logical/src/backbuild/`, "backbuild option
  catalogue", "backbuild synthesis" and the docs-site page filename all name the
  *definition-delta mechanism*, not the verb, and stay. Only
  `docs-site/docs/guide/backbuild-synthesis.md`'s now-false "two things called 'backbuild'"
  callout is removed in phase 4; that page's narrative rewrite remains phase 8's.

- **2026-08-29 (phase 3b implementation).** Two discoveries, both resolved in-phase. (a) `smelt
  run` had no `--full-refresh` flag at all — `ExecuteRequest::full_refresh` was already wired
  from `smelt-ui` and doc-commented "CLI as `--full-refresh`" but no `RunArgs` field or mapping
  existed; this phase's own gate needs it to be exemptable, so the flag was added to `RunArgs`
  and threaded through both `ExecuteRequest` construction sites in `commands/run.rs`, matching
  the pre-existing doc comment's stated intent rather than inventing new behavior. (b) The
  generative `maintenance_conformance` suite's `pure_backfill_column_add_executes_in_place_update`
  failed red against the new gate: it drives the maintenance driver's own live
  `Trigger::ColumnAdded` → `Technique::InPlaceUpdate` dispatch (the "narrower third mechanism"
  `definition_deltas.md` §Known Divergences already documents as coexisting with `smelt migrate`)
  through an ordinary windowed run with no `smelt migrate` step — exactly the shape the gate would
  otherwise block. Rather than forcing every column addition through `smelt migrate` and breaking
  that mechanism's ergonomics, added `DefinitionDiff::is_pure_column_addition` and a
  `pure_column_addition` field on `DefinitionDeltaStatus::Pending`; the run gate skips refusal
  when it's `true` (`smelt explain`/`smelt migrate` still report and offer the delta). Recorded
  in `definition_deltas.md` §"Detection" as "Pure column addition is exempt."
- **2026-08-30 (phase 5 planning).** No reshape — phase 4's summary left nothing outside an
  existing row. Two scoping calls. (a) The new `MigrateModel` step is added *alongside*
  `RewriteModel`, not in place of it: `RewriteModel` asserts the pre-migrate contract (a later
  run compiles whatever is on disk) and that behaviour is still real and still worth covering;
  `MigrateModel` asserts the spec's own oracle — the new definition holds immediately after the
  migration. (b) Generation goes into a NEW `arb_schedule_with_definition_edit`, leaving
  `arb_schedule_for` byte-identical, because the Spark and BigQuery conformance twins consume the
  same generator and changing it would silently reshape their (nightly/manual) samples inside a
  per-PR phase. The cross-engine `families/gate.rs` still gets a real arm for the new variant, via
  the same shared driver helper — not a skip.

- **2026-08-29 (phase 4 implementation).** Renamed the CLI verb `smelt backbuild` → `smelt
  rebuild` with no compatibility alias (`smelt backbuild` now exits 2, unrecognised
  subcommand). The `smelt-logical/src/backbuild/` crate module, the "backbuild synthesis"
  mechanism name, and the `docs-site/docs/guide/backbuild-synthesis.md` page filename/nav
  were deliberately left untouched — only the CLI verb renamed, per the plan's scope note.
  The page's full narrative rewrite around `smelt migrate`/`--apply` remains phase 8's job.

- **2026-08-30 (phase 5 implementation).** `ConformanceStep::MigrateModel` drives the real
  `smelt migrate` backbuild path (`derive_plan` → `apply_migration`-or-full-refresh) via a
  new shared driver (`smelt-maintenance-testkit/src/migrate_step.rs`), consumed identically
  by both `maintenance_conformance/gate.rs` and its target-parametrized `families/gate.rs`
  twin — deliberately distinct from the pre-existing `RewriteModel` step, which still
  exercises the live `Trigger::ColumnAdded` maintenance-driver path (`definition_deltas.md`'s
  "narrower third mechanism"). The generative gate (`definition_edit_pool_upholds_new_
  definition_equivalence`) surfaced two real `smelt_logical::backbuild` bugs — an
  aggregate-shaped column-add wrongly admitted `SelfDerivedColumnAdd`, and `try_b5`'s
  re-aggregation subquery spliced unresolved `smelt.<path>` ref syntax — both fixed in-phase
  (`classify.rs`, new `requalify::requalify_source_refs`) since the outcome's own success
  criteria already require the migrate plan to be genuinely executable, not just derivable.
  The spec bullet "The conformance harness has no definition-edit step kind yet" is removed
  from `definition_deltas.md`. Full detail in `phases/05-summary.md`.

- **2026-08-30 (phase 6 implementation).** The atomicity rule is now unconditional: a
  `schema_evolution: strategy: full_refresh` model rebuilds when its schema changes
  (`schema_evolution::full_refresh_escape_requires_rebuild`), and a migration group is made
  rerun-safe on any backend by reconciling its `ADD COLUMN` statements against the target's
  physical columns before executing (`schema_evolution::reconcile_add_columns`), reading them
  via `information_schema.columns` rather than the plan's originally-suggested `SELECT * ...
  LIMIT 0` — DuckDB's Arrow bridge returns zero record batches for a zero-row result, so that
  probe shape cannot report a schema on an empty table. The standalone
  `maintenance_driver::execute_in_place_update` fallback dispatch is deleted; a not-yet-folded
  backfill assignment now forces a full refresh instead. `definition_deltas.md` §"The atomicity
  rule" and §"Boundary with schema_evolution.md" updated; the "conditional in practice" Known
  Divergences bullet removed. Full detail in `phases/06-summary.md`.

- **2026-08-30 (phase 7 planning).** One reshape: row 7 is split into **7** (the diagnostic
  rename in code + the sibling-spec sweep) and **7b** (surfacing it ahead of a run). Nothing left
  the outcome — both halves of success criterion 6 keep a row. The split is because the two halves
  are unlike work: the rename is a mechanical cross-crate API sweep whose oracle is the existing
  diagnostics-catalogue gate, whereas surfacing requires a genuinely architectural change —
  `smelt-db`'s maintenance query derives an empty trigger set because it has no deployed-schema
  snapshot and does no I/O (Salsa purity), so 7b must add a deployed-schema world-fact Salsa input
  fed by `workspace_ingest` (the `register_loader_files_from_disk` precedent) from **both** the CLI's
  `init_db` and the LSP's `initialize`, under the workspace-loading-parity rule. Scoping note
  recorded now so 7b's planner does not rediscover it: the deployed snapshot carries `model_sql`
  as well as column names, so 7b can surface a skeleton *change* (not only a skeleton *add*) via
  `backbuild::definition_diff`'s existing skeleton-clause diff — which is what the renamed code
  name promises. The two "not yet surfaced ahead of a run" divergence bullets
  (`model_properties.md`, `incremental_models.md`) stay open through phase 7 and close in 7b.

- **2026-08-30 (phase 7 implementation).** `Refusal::SkeletonColumnAdded` /
  `MaintenanceRefusal::SkeletonColumnAdded` / `DiagnosticCode::MaintenanceSkeletonColumnAdded` /
  the LSP `"maintenance-skeleton-column-added"` wire string all renamed to
  `SkeletonChanged`/`MaintenanceSkeletonChanged`/`"maintenance-skeleton-changed"`. The
  ~220-arm `DbCode → &str` match inline in `Backend::to_lsp_diagnostic` (using no `self`) was
  extracted to a standalone `pub(crate) fn diagnostic_code_str` so the rename's LSP leg is
  directly unit-testable without constructing a `Backend`/`Client` — the only code-shape change
  beyond the mechanical rename. The new grep gate's needle is built via string concatenation
  (`["Skeleton", "Column", "Added"].concat()`) rather than a literal, since the plan's own
  verification step requires zero matches for the stale spelling across `crates/` and
  `docs/specs/` with no carve-out for the guard test's own source. `definition_deltas.md`'s
  "one code, not a split pair" design paragraph was reworded to describe the decision without
  naming the retired identifier, both for the grep gate and because `docs/specs/CLAUDE.md`'s
  timeless-oracle rule already forbids historical/pre-rename identifiers in spec prose. Full
  detail in `phases/07-summary.md`.

- **2026-08-30 (phase 7b planning).** No reshape — phase 7's summary surfaced nothing outside an
  existing row. Three scoping calls recorded so the implementer does not re-litigate them.
  (a) The deployed-schema reader lives in `smelt-db`'s `workspace_ingest` (adding a `smelt-state`
  dependency, acyclic) rather than being written twice in the CLI and the LSP — the
  workspace-loading-parity rule's single-owner shape, mirroring
  `register_loader_files_from_disk`. (b) A skeleton *clause* change (changed `GROUP BY`/FROM,
  no column add) is surfaced too, via the snapshot's `model_sql` and `backbuild::definition_diff`,
  as a new `Refusal::SkeletonClauseChanged` mapped to the **same**
  `DiagnosticCode::MaintenanceSkeletonChanged` — phase 1's "one code, not a split pair" decision
  constrains the diagnostic code, not the number of refusal shapes feeding it. (c) The effective
  target for the schemas directory is the `smelt.yml` `target:` else `"dev"` (the CLI's own
  `--target` default); a command carrying an explicit `--target` re-registers with the shared
  reader rather than getting its own.

- **2026-08-30 (phase 8 planning).** No reshape needed: phase 7's summary surfaced nothing
  outside its own scope. Two clarifications folded into the phase-8 plan rather than the table:
  the "two things called 'backbuild'" callout named in success criterion 7 is already absent from
  the guide (phase 4's rename removed it), so phase 8 only verifies its absence; and criterion 18's
  docs-site CLI-surface coverage audit lands as a checklist in phase 8's summary, per that
  criterion's own "folds into phase 8's docs-site pass" wording.

- **2026-08-30 (phase 9 planning) — reshape: letter-suffixed rows renumbered.** Phase `7b`
  (surface the definition-change diagnostic ahead of a run) was planned on 2026-08-30 but never
  implemented: `.claude/scripts/outcome-loop.sh`'s `next_step()` scanner skips any row whose
  number is not purely numeric (`if (n !~ /^[0-9]+$/) next`), so rows `3b`/`7b` are invisible to
  the wrapper and the loop advanced straight to phase 8. Since success criterion 6 depends on that
  work — and the close-out validate must not run before it — row `7b` is renumbered to **9** (its
  plan file renamed `phases/07b-plan.md` → `phases/09-plan.md`, unchanged in substance) and the
  former rows 9–20 shift to 10–21. Row `3b` is `done` and left alone. The loop-script bug itself is
  *not* patched here: editing a bash script while the loop process is executing it risks corrupting
  the running interpreter's read offset. A human should change that regex to `/^[0-9]+[a-z]?$/`
  between runs; until then, never use letter-suffixed phase numbers.
- **2026-08-30 (phase 9 planning) — residue routed to phase 10.** Phase 8's summary flagged one
  unchecked item: whether `docs-site/docs/models.md` / `docs-site/docs/seeds.md` (distinct from
  `docs/specs/`) still carry stale "no `smelt migrate` command" wording — phase 8's grep gate
  scoped only `docs/specs/`. That is a success-criterion-8 close-out check, so it belongs to phase
  10's validate sweep rather than a new row. Phase 8 also recorded criterion 18's docs-site
  CLI-surface audit as complete with no gap found; phase 19 need only note that.

- **2026-09-02 (phase 10 planning) — one reshape, one routing.** Phase 9's summary left a residue
  with no owner: `deployed_column_names` is still hardcoded `&[]` everywhere outside
  `smelt-runtime`'s maintenance driver, because the pre-execution gate's admission posture can
  refuse (`MaintenanceScanUnbounded`) for a column add the runtime driver executes safely. That is
  the same "dispatch distinguishes what actually happened" question success criterion 15 already
  owns, so it is folded into **phase 16**'s row rather than becoming a new one — no new criteria,
  no deferral. No other rows changed. Phase 10 also picks up the two known spec corrections
  (`definition_deltas.md`'s divergence bullet still cites the pre-renumber "phase 11" for per-cell
  frontier addressing, now phase 12; `last_reviewed` bump) and adds the first standing grep guard
  against the criterion-8 rename regressing.

- **2026-09-02 (phase 10 implementation).** The validate sweep found two more drift items beyond
  the two already-known corrections, both wording gaps in sibling specs (not `definition_deltas.md`
  itself), both fixed in-phase: (a) `diagnostics.md`'s Known Divergences said
  `MaintenanceSkeletonChanged` "only reaches its own `file_diagnostics()` mapping from a caller
  that plumbs one in (today, none does...)" — stale since phase 9 wired the `DeployedSchemaInput`
  world fact into exactly that caller; (b) `architecture.md`'s Known Divergences said "no
  CLI/runtime consumer drives a backbuild script through a real backend yet" — stale since phase
  2/3 shipped `smelt migrate --apply`, proven end to end by
  `crates/smelt-cli/tests/migrate_apply.rs` against a real DuckDB. Both rewritten to state what's
  actually still missing (the `statement_parity` byte-identical structural leg has not been
  extended to backbuild specifically). No behavioral gap was found — everything else validate
  checked (Surface, Semantics, invariants, cli.md/model_selection.md/models.md/seeds.md sibling
  sweep, docs-site) was already correct from phases 1–9.

- **2026-09-02 (phase 11 planning)** — reshape: added row 20b. Phase 10's summary recorded the narrowed `architecture.md` backbuild statement-parity gap as an honest *untracked* residue with no owning phase. Success criterion 9 requires the standing gates (which name `statement_parity`) green for this outcome's own mechanism, so extending that gate's structural leg to the backbuild emitter family serves a success criterion and is not deferred out. Placed before the final validate row so phase 21 can confirm the bullet is actually removed.

- **2026-09-02 (phase 13 planning)** — reshape: added row 13b. Phase 12's summary proved that the per-cell deferral data layer (address, ledger frontier, decision builder, manifest field) landed but that no *validly declarable* cell reaches any wired dispatch site — `contract.cells[].deferral` is admissible only on a clocked `on:`, i.e. the ordinary `Trigger::NewData` fold cell, which phase 12 did not touch. Success criterion 11 requires per-cell `deferral` to be **scheduled**, so this residue serves a success criterion and gets its own row rather than being deferred out. Placed after 13 so the write-pin equivalence work (already pre-scanned) proceeds first.
- **2026-09-02, phase 13.** `cell_equivalence_proof` classifies compare-based patterns by registry
  `pattern.name` (`diff_patch`/`keyed_conditional`/`staged_candidate`), not `WriteSelection` (which
  collapses `keyed`/`keyed_conditional`/`staged_candidate` onto the same `Technique::KeyedFold`
  selection and would blur which ones actually carry a comparability obligation).
  `MaintenancePlanResult::comparability` is populated by one new `model_property_vector` call in
  `derive_model_maintenance_plan`'s success path. Discovered (not fixed, out of scope): a real bug
  in `group_by_unique_key`/`analyze_select` derives an empty `GROUP BY` key whenever the grouping
  column is literally named `order_id` (confirmed via isolated probe; `customer_id`/`orderid` both
  work) — smells like a keyword/lexer collision on the `ORDER` substring, worth its own ticket.

- **2026-09-02 (phase 13b planning)** — reshape: merged phase 13's discovered `order_id` empty-key bug into row 15, which already owns the `GROUP BY`-derived `grain: key` identity check — same code path (`group_by_unique_key`/`analyze_select`), so a separate row would split one fix across two phases. It silently breaks `grain: key` admission, so it stays inside the outcome rather than moving to Out of scope. Phase 13b's own design call, recorded here so the implementer does not re-litigate it: the plain fold's write is whole-row, so a per-cell skip is licensed only when **every** `Trigger::NewData` column group the fold serves is covered by a skip-licensed declaring cell; partial coverage falls through to the normal path (declining unlicensed work would violate the deferral oracle), and the residue is stated in the spec rather than silently accepted.

- **2026-09-03, phase 15 implementation.** `--since-upstream`'s CLI wiring now reads
  `_smelt_observed_delta` live via a new `propagation::load_observed_delta_lookup` before calling
  `plan_since_upstream_with_observed_deltas`; `plan_since_upstream` (empty-lookup) is now only the
  testkit/conformance harness's wrapper, not a CLI-reachable path. `maintenance_driver::
  read_observed_delta` is the new shared decoder for both `changed_keys` and `partitions`;
  `read_observed_delta_changed_keys` is re-expressed over it. Backward resolution's non-consumption
  landed in the spec verbatim (already decided in this log above). Found and fixed a real bug
  while wiring the CLI: the lookup's backend connection was kept alive across the whole
  `run_since_upstream` function, overlapping with the run loop's own connection to the same DuckDB
  file — two live connections to one file from the same process silently lost writes (3 tests
  failed, 2 of them pre-existing and unmodified by this phase, with "table gold does not exist").
  Fixed with an explicit `drop(lookup_backend)` before the run loop. Full detail in
  `phases/15-summary.md`.

- **2026-09-03 (phase 16 planning) — no reshape.** Phase 15's summary surfaced no
  success-criterion work without an owner: its one forward-looking caution (never hold two live
  DuckDB connections to the same target across the run loop) is a constraint on future pre-run
  probes, and phase 16 adds no pre-run probe — its recording happens inside the existing write
  path's own connection. The remaining rows stand as written. Phase 16 scopes the settle-bound ×
  observed-delta composition as a **reporting** leg: an empty recorded delta whose window is
  provably behind the derived settle bound is a *settled* no-op rather than an empty-this-run one.
  It does not prune further work, because the empty-delta arm of
  `plan_since_upstream_with_observed_deltas` already propagates nothing — the settle bound supplies
  the provable horizon the spec claims (§"Observed deltas on model edges"), not extra skipping.
  `Grade::Additive` keyed folds (ledger-interleaved via `fold_ledger_delta`) stay unrecorded this
  phase; if that leaves a real gap it belongs to phase 19's proof-residue sweep, not a new row.

- **2026-09-03, phase-17 planning.** No table reshape: phase 16's summary surfaced only the
  `Grade::Additive` unrecorded-delta residue, already assigned to phase 19 by the 2026-09-03
  (phase-16 planning) entry above. Phase 17 keeps its three-part scope. Scoping call for its
  first part: "no execution technique keys off a maintained-model creation cell" is narrowed to
  the **partition-addressed** model-edge creation cell — the key-addressed (`PerGroupRecompute`)
  edge cell already dispatches (phase 11), and `resolve_live_delta_restriction_facts` already
  consumes the partition-addressed cell's *closure/identity* facts but never its *technique*.
  The phase makes `resolve_incremental_strategy` edge-aware (reading the driving edge's own
  creation cell, derived via the existing `derive_model_maintenance_plan_with_edges` — never a
  second derivation) and turns a `ReachNotDerivable`-refused driving edge with no other creation
  cell into a fail-loud run refusal instead of a silent region recompute. If a real fixture would
  newly refuse, the refusal condition is narrowed further and the residue restated honestly in
  the spec rather than a fixture being edited.

- **2026-09-03, phase 17.** `IncrementalStrategy` has exactly one variant (`DeleteInsert`), so
  no fixture can show the driving edge's cell and a plain source's cell resolving to visibly
  different `IncrementalStrategy` values — the edge-aware dispatch is real at the
  `MaintenancePlan` level (`plan.cell_for` vs. first-`NewData`-match) but collapses to the same
  return value either way today. `crates/smelt-runtime/tests/model_edge_creation_cell.rs`'s
  first test was narrowed to what's actually observable: an edge-only model (no plain
  `sources:`) resolves via the edge's own cell instead of falling back to `backend_default` for
  lack of any cell. Two real fixtures (`maintenance_diagnostics.rs::grain_mismatch_is_error_
  never_silent`, `explain_maintenance.rs::degenerate_plan_visibly_reported`) had `grain: key` +
  no `unique_key:` + no `GROUP BY` — genuinely underivable identity, now caught by the new
  frontmatter check earlier than the paths they were written to exercise. Updated rather than
  relaxed: the first now asserts `GrainAssertionMismatch`; the second's fixture gained a
  `GROUP BY o.order_id` (with `MIN(...)` folds) to keep a derivable identity while still
  reproducing the ambiguous-join column-group collapse it actually tests.

- **2026-09-03, phase-20 planning — renumber `19b` to `20`; no new plan written this turn.**
  The outcome-loop wrapper's row selector (`.claude/scripts/outcome-loop.sh::next_step`) skips any
  table row whose `#` column is not purely numeric (`if (n !~ /^[0-9]+$/) next`). The lettered row
  `19b` added by the previous plan step was therefore invisible to the loop: it sat `planned` with
  a written plan that no IMPLEMENT step would ever be dispatched for, and the pre-scan hint pointed
  past it to the next numeric row. Since that row closes a real clause of success criterion 15
  ("dispatch distinguishes a genuine mutation from re-derivation"), dropping or skipping it is not
  permitted by this outcome's never-defer rule. Fixed by renumbering: `19b` -> `20` (plan file
  renamed `phases/19b-plan.md` -> `phases/20-plan.md`, its `incremental_models.md` §Known
  Divergences tracker link updated), and every subsequent row shifted +1 (old 20..27 -> 21..28).
  No plan file for a new phase was written this turn — row 20's plan already exists and is
  unchanged in content; the next iteration dispatches its IMPLEMENT step. Lettered row ids must not
  be used in this table again.

- **2026-09-03, phase-21 planning — split the omnibus graph-layer row into four numeric rows.**
  Old row 21 bundled five independent items (bare-keyed node admission, key-level dirt, time-unrolled
  self-edges, whole-workspace `examples/web_analytics`, `--select` scoping); reading the code showed
  they are not one change. `KeyedDirt`/`keyed_dirty`/`per_edge_keys` already exist in
  `smelt-logical::maintenance::propagate` (so the divergence bullet's "no key-level dirt
  representation exists" is stale as written), but the representation is consumed by nothing: a node
  dirtied only through the keyed channel gets no `dirty` entry, so `propagate` never walks its own
  outbound edges and `plan_since_upstream` never schedules or reports it. That cascade+consumption
  bug is now row 21 on its own. The self-edge refusal (`examples/web_analytics`'s
  `silver.sessions_chained`, a backward-bounded `Ordered` self-read) is row 22; `--select` scoping —
  the flag is parsed on `RunArgs` but `run_since_upstream` ignores it — is row 23; and the
  whole-workspace `examples/web_analytics` end-to-end leg plus the divergence-bullet removal, which
  depends on 21 and 22 landing, is row 24. Old rows 22–28 shift to 25–31. Nothing left the outcome;
  all five items remain, each with its own row (success criterion 15). Numeric ids only, per the
  phase-20 planning entry.
- 2026-09-03 (phase 22 plan): no reshape of the remaining rows. Phase 21's "for the next
  planner" note — a `grain: partition` downstream's key-addressed model-edge admission when the
  bare-keyed upstream is reached only through a plain `FROM` — serves success criterion 15
  (`examples/web_analytics` fully `--since-upstream`-compatible end to end), so it is folded
  into phase 24's row text rather than deferred out or given a row of its own.
- 2026-09-03 (phase 22 plan): a self-edge is unrolled as a *time* edge, not admitted into the
  table graph — `topo_order` excludes it, forward dirt widens open-ended to the frontier
  (`[a, →)`, scheduled as `start: Some(_), end: None`), and the backward requirement applies
  the clamp once against the model's stored basis. Rejected: a fixed-point unrolling over the
  day axis (unbounded work for no extra precision — forward dirt reaches the frontier either
  way) and admitting the self-edge into `topo_order` with a tie-break (would silently make a
  genuine table-graph cycle orderable).

- 2026-09-03 (phase 23 plan): phase 24 extended to name the open-ended `(Some, None)` run window `parse_run_window` still rejects (surfaced by phase 22's summary); phase 27 extended to cover `window_independence`'s same-partition `Ordered` gap. Both serve the outcome's end-to-end criterion, so neither is deferred out.

- **2026-09-03 (plan 24)** — Split phase 24 in two. A live `smelt run --since-upstream --source sources.raw.events` over the whole unfiltered `examples/web_analytics` workspace now builds and propagates with **no** graph refusal (phases 21–23 closed that); the single remaining hard error is `parse_run_window`'s "both or neither" guard rejecting the open-ended `silver.sessions_chained`/`silver.events_enriched` runs. Separately, `smelt explain silver.device_user_edges` still reports `RepairKeysNotDiscoverable { source: "silver.events_deduped", why: "grain expression reads column 'device_id' absent from the delta's own row shape" }`, so that model is silently unscheduled. The two gaps share no machinery (one is a runtime window-encoding boundary, the other a P7 affected-key proof extension), so they get one row each: 24 (window) and 24b (admission). 24b keeps the "Graph-layer gaps" divergence-bullet removal, since it is the later of the two to close the criterion.

- **2026-09-03 (plan 24b)** — Spec correction folded into this phase rather than deferred.
  §"Upstream model edges" pins the key-addressed discovery at the *upstream's* key columns and
  then projects changed keys forward over the post-change upstream. For a downstream whose grain
  differs from the upstream's key (`silver.device_user_edges`: grain `(device_id, user_id)` over
  an `event_id`-keyed upstream), a grain value that **moves** between downstream groups surfaces
  the arriving group and never the vacated one — an under-approximation the equivalence invariant
  forbids, and the `maintenance_conformance` gate would catch it. Decided: when the two key sets
  differ, group the fingerprint sidecar at the **downstream's grain projected over the upstream
  relation**, so the diff's own `delta_key` is the affected downstream key and both groups' XOR
  digests flip. The existing upstream-keyed route stays verbatim for the equal-key case, where no
  move is representable. Rejected: shipping the spec's literal post-state projection with the
  residue recorded as a divergence (knowingly unsound maintenance), and extending the sidecar to
  store pre-image grain *values* (new sidecar schema + migration for no extra precision over
  regrouping). The admission leg is corrected in the same edit — the obligation is that the
  downstream's grain resolve against the upstream *relation*, not that the downstream carry the
  upstream's key columns.

- **2026-09-03 (phase 25 planning)** — no table reshape. Phase 24's summary left one residue
  (`window_independence`'s `Ordered` verdict not checking `before > 0` for a same-partition
  self-read), which row 27 already owns explicitly — no new row needed. Design call recorded so
  the implementer does not re-litigate it: the pre-execution gate's posture is reconciled by
  giving the definition-change trigger's non-skeleton refusals their own variant
  (`DefinitionChangeNotBackfillable`) mapped to a **Warning**, because `execute.rs`'s
  definition-delta run gate already exempts a pure column addition outright — the gate must not
  report as an Error what a run does not refuse. A skeleton-position add keeps its
  `MaintenanceSkeletonChanged` Error, and a `schema_evolution: full_refresh` model derives no
  definition-change trigger in the gate at all (fact assembly in the Salsa wrapper, not a new
  branch in the pure derivation). With that posture, real `deployed_column_names` are threaded
  from the existing `DeployedSchemaInput` world fact.

- **2026-09-03 (phase 25 done)** — implemented per the recorded design call. Reclassifying the
  four non-skeleton `Trigger::ColumnAdded` refusal pushes broke two pre-existing non-regression
  unit tests outside the phase's own listed test files (`maintenance_tracer.rs`,
  `maintenance_tracer_evolution.rs`) — both asserted the old `NoAdmissibleTechnique`/
  `ScanUnbounded` variant for the exact shape this phase retargets; fixed in place after a full
  `cargo test --workspace` sweep caught them.

- **2026-09-03 (phase 26b planning)** — no table reshape; phase 25's summary named no residue rows
  26+ do not already own. Two things recorded instead. (1) Tooling: `.claude/scripts/outcome-loop.sh`'s
  row scanner skipped every lettered row (`if (n !~ /^[0-9]+$/) next`), so 3b/24b/26a–26d were
  invisible to the wrapper and it dispatched a PLAN step for row 27 while row 26a sat `planned` and
  unimplemented — an infinite-plan stall. Regex widened to `^[0-9]+[a-z]?$`; the table, as always,
  wins over the hint. (2) Design call for 26b so the implementer does not re-litigate it: the
  per-arm verdict splits *value* arms from *cardinality-deciding* arms — `EXCEPT`'s subtrahend
  contributes membership sensitivity only, `INTERSECT`/`UNION`-distinct couple every arm into
  membership (dedup/matching make a row's existence depend on the other arms), and `UNION ALL` adds
  nothing beyond each arm's own membership set. The membership leg is *not* filtered by mutation
  profile: an append-only insert into an `EXCEPT` right arm still deletes an output row. A
  mixed-op chain, a nested compound arm and an arity mismatch keep today's whole-model collapse,
  each with its own named reason.

- **2026-09-03 (phase 27a planning)** — reshape: row 27 was a six-clause grab-bag sweep, too wide
  for one implement step and spanning unrelated code (preview rendering, a new emitter family, a
  new frontmatter pin, source-delta admission, a property-walk verdict). Split into rows 27a–27f,
  one clause each, in dependency order — 27a first because it makes the forms the later rows add
  inspectable, and because it is independent of rows 26a–26d. Nothing left the outcome; the
  bullet's remaining "non-DuckDB targets keep the widened-scan recompute" clause is a backend-
  capability fact, not a gap, and each sub-row trims only its own clause from
  `incremental_models.md` §Known Divergences "Conditional-maintenance gaps" and
  `model_transforms.md`'s matching bullets. Also noted for 27b: `emit_staged_candidate_
  conditional_recompute` already exists in `smelt-logical::maintenance::emit`, so that row's real
  scope is establishing which half (admission vs. emission) is unwired rather than assuming both.

- **2026-09-03 (phase 27b planning)** — no table reshape; phase 25's summary named no residue rows
  26+ do not already own, and 27a's split already covers this bullet clause by clause. Two design
  calls recorded so the implementer does not re-litigate them. (1) The gap is admission +
  dispatch, not emission: `emit_staged_candidate_conditional{,_recompute}` both exist and the
  recompute sibling is live-dispatched for `UpstreamMutation` membership cells; the ordinary
  `Trigger::NewData` region path simply has no conditional route. (2) The region variant is
  realised with `emit_diff_patch` under the region's own slice predicate and `DeleteLeg::Complete`
  — `emit_staged_candidate_conditional` unmodified is unsound here (its keyed `DELETE` is not
  region-bounded and it has no departed-row leg, so a key leaving the region would go stale), and
  a fourth staged emitter would duplicate `emit_diff_patch`'s legs for no semantic difference.
  Delta restriction keeps precedence over suppression when both admit; everything unproven falls
  closed to today's byte-identical widened scan.

- **2026-09-03 (phase 27c planning)** — no table reshape. Two calls recorded. (1) **Keyless
  suppression is region-grained, not row-grained.** Without a row address a multiset difference
  cannot be deleted with multiplicity in portable SQL, so the whole-row realisation materialises a
  two-way `EXCEPT ALL` diff sentinel once and guards an otherwise byte-identical region
  `DELETE`+`INSERT` with it: empty diff ⇒ no write at all, non-empty ⇒ exactly today's
  unconditional statements. The spec must state this grain rather than let "EXCEPT ALL both ways"
  imply per-row suppression. (2) **The keyless path records no observed delta** — the
  observed-delta table is keyed by the identity's key columns and a keyless write has none;
  it records nothing rather than synthesising a key.

- **2026-09-03 (loop hygiene, noted during 27c planning)** — rows 26a, 26b, 26c, 26d, 27a and 27b
  are `planned` with plan files committed but **no summaries and no implementation commits**; the
  last feature commit is phase 25's. The wrapper's own pre-scan for this run also disagreed with
  the table (it hinted phase 28 while `next_step()` over the committed file returns
  `implement 26a`). Whoever picks this outcome up next should run the IMPLEMENT step against 26a
  first — this planning run deliberately did not renumber or re-flip those rows.

- **2026-09-03, phase 27d planning — split the pin-selection row in two.** Closing "no `write:` pin
  selects between keyed MERGE and staged-candidate" is two separable jobs: a pure selection layer
  (`choice::resolve_keyed_write_mechanism` today takes no pin and is called only from its own unit
  tests) and a live runtime dispatch (`cumulative.rs::build_cumulative_merge_sql` emits `MERGE`
  unconditionally and never sees the model's overrides). The second also needs a *folded* candidate
  select — a keyed fold's staged candidate is `combiner(stored, delta)`, not the delta, so
  substituting `emit_staged_candidate_conditional` over the fold's delta SQL would be silently
  wrong. 27d owns the selection layer and the folded-candidate emitter; new row 27g owns the live
  dispatch and the Known Divergences narrowing. Neither half leaves the outcome.

- **2026-09-03 (phase 27e planning)** — no table reshape. Recorded design calls so the
  implementer does not re-litigate them: (1) the non-DuckDB widened-scan fallback becomes a
  declared `BackendCapabilities::supports_fingerprint_sidecar` flag rather than the inline
  dialect equality check `diff_fingerprint_sidecar_changed_keys` uses today — success
  criterion 17 admits the gap only when it is declared via the capability struct; (2) the
  external route reuses `execute_delete_insert_with_delta_restriction` through a
  `RestrictionDeltaSource` enum rather than gaining a parallel executor, so the
  count-preservation probe dispatch and the emitter path keep one owner. Rejected: a second
  `execute_delete_insert_with_external_delta_restriction` entry point (duplicates the probe
  obligation, which `statement_parity` would then have to police twice).

- **2026-09-03 (loop hygiene, second observation)** — the running `outcome-loop.sh` process
  (PID 1367419, started 03:51) predates commit 518e317e's fix to `next_step()`'s
  `^[0-9]+$` row-number filter, so it still cannot see the lettered rows and has emitted a
  plan step every iteration since phase 25 — eight plan commits, no implementation. The
  committed script is already correct; this planning run terminated the stale process so
  `outcome-loop-forever.sh` restarts it with the fixed scanner, which should pick rows
  26a–27e up as IMPLEMENT steps.

- **2026-09-03 (phase 27f planning)** — no table reshape. Phase 25's summary named no
  residue rows 26+ do not already own, and 27f is the last clause of phase 22's surfaced
  gap, already owned by its own row. One design call recorded so the implementer does not
  re-litigate it: the fix goes in the **shared** `self_edge_bound_days` derivation, not in
  `windowing.rs` or `propagate.rs` separately — `self_edge_clamp`'s doc comment already
  promises the ordered-execution verdict and the propagation-graph admission cannot
  diverge, and duplicating a `before > 0` check at the two call sites would re-create the
  divergence it exists to prevent. The refusal text is the derivation's, so the graph
  layer's `MaintenanceGraphUnsupportedNode` names the same reason ordered backfill does.

- **2026-09-03 (loop hygiene, third observation)** — the stale `outcome-loop.sh`
  (PID 1367419, started 03:51, predating 518e317e's `next_step()` fix) was **still
  running** at this iteration: the previous run's scheduled kill did not survive its own
  session, so the hint again disagreed with the table (hinted phase 28; `next_step()` over
  the committed file returns `implement 26a`). Terminated for real this run via a detached
  delayed kill, so `outcome-loop-forever.sh` restarts with the fixed scanner. Rows
  26a–27e remain `planned` with committed plans and no implementation; whoever picks this
  outcome up next should let the IMPLEMENT step drain them starting at 26a. This run
  deliberately did not renumber or re-flip those rows.

- **2026-09-03, phase 26a implemented — a locality-admitted composed node is a clocked
  node, not `Keyed`.** `smelt-runtime::propagation::model_grain` returns the model's own
  declared granularity (not `PartitionGrain::Keyed`) once key temporal locality is
  admitted, so its inbound key→partition margin edge genuinely runs through
  `Edge::reflect`'s interval math — it is not exempt the way a bare keyed node is. Missed
  on the first pass (assumed `Keyed` unconditionally) and caught by
  `since_upstream_composed_web_analytics`'s e2e test (a 3-day date shift). Every other
  non-target production fold site (self-edge margin, the `PartitionLocal::No` fallback)
  keeps mirroring its pre-phase read-margin behaviour exactly, out of 26a's scope.

- **2026-09-03, phase 26d — column-group-scoped dirt lands; fixed a real
  `output_delta::SourceFacts` plumbing gap along the way.** `dirt_scope` (`smelt-logical`
  `grouping.rs`) narrows a propagated edge to the downstream's own column groups a
  closure-pruned enrichment source can actually touch; `propagate`'s forward walk gates an
  outbound typed edge whose components name only groups outside the node's own dirty scope.
  The end-to-end test exposed that `output_delta::SourceFacts` (used by `smelt-runtime`'s real
  per-workspace graph assembly) never carried a source's declared `unique_key` — it mapped
  `unique_key` from `delta_identity`, a change-feed-only fact — so closure-pruning was
  structurally unreachable from `build_forward_graph` even though the underlying proof and
  `smelt-db`'s plan-layer adapter were both already correct. Added the missing field; full
  `cargo test --workspace` sweep green. See `phases/26d-summary.md`.

- **2026-09-03, phase 28 reshaped into 28a/28b/28c (plan step).** The five "small Open
  Questions" were already *decided* in `docs/research/20260816-open-questions-triage.md` and
  two of them are already recorded in the specs — what is actually left is not one recording
  pass but three differently-shaped jobs: a doc-only recording + CLI-audit close-out (28a), an
  implementation audit with a pinning check for the merged-group rule (28b, `incremental_models.md`
  says explicitly "unverified in the implementation … no check or fixture pins the rule"), and
  real derivation work for `change_feed` (28c — the plan-layer `MutationProfile` in
  `smelt-logical/src/maintenance/derive.rs` has only `AppendOnly`/`MutableSnapshot` variants, so
  no `UpstreamMutation` cell can be derived for a feed today). Nothing left the outcome; success
  criteria 18 and 20 are still covered in full by the three rows.

- **2026-09-03, phase 28b planned — the merged-group rule is violated, not merely
  unchecked.** Planning-time audit of `derive_mutation`
  (`crates/smelt-logical/src/maintenance/derive.rs`) found the `(Corner, Technique)` choice
  keys on `membership_sensitive` alone, per source: a group value-sensitive to two mutable
  inputs gets two `ColumnScopedMerge` cells, exactly what `incremental_models.md` §"The plan
  matrix" forbids. 28b is therefore a code-change phase (guard + fixture pin + bullet
  removal), not a docs-only confirmation. No rows added or removed.

- **2026-09-03, phase 29 planned — one divergence bullet's premise is wrong, and the same
  predicate over-refuses in the other direction.** `safety_overrides:` on a keyed model does
  *not* silently parse: the top-level key folds into `metadata.batched` and
  `validate_timeseries` already refuses it — but as `PartitionGrainRequiresRefreshIncremental`,
  which tells a keyed author to add `grain: partition`. The gap is a correctly-named refusal,
  not a missing one. The audit also found the check reads the *declared* `grain:` rather than
  `resolved_grain()`, so a partition-shaped model that writes no `grain:` is wrongly refused
  for declaring `safety_overrides:`. Row 29 is rewritten to cover both directions; no rows
  added or removed.

- **2026-09-03, phase 30 planned — a second author of model-table `ALTER TABLE` DDL exists
  outside the scanned crates.** Planning-time audit for the structural-leg widening found
  `crates/smelt-state/src/ddl_duckdb.rs` constructing `ALTER TABLE … ADD COLUMN` /
  `DROP COLUMN` text for *model* tables (consumed by `smelt-runtime`'s `schema_evolution`
  `MigrationAction::AlterTable` path), duplicating `backbuild::emit`'s
  `emit_alter_add_column`/`emit_alter_drop_column`. That is the two-authors bug class item 12
  names, not the ledger-bookkeeping exclusion the spec carves out for `smelt-state`, so it
  serves success criterion 9 and is not deferred out: added row 30b. Phase 30 keeps the
  structural scan's crate list unchanged (`smelt-backend*`, `smelt-runtime`, `smelt-logical`)
  and widens only its *shape* list to the backbuild families; 30b owns the crate-list widening
  plus the unification, so a large refactor cannot silently expand phase 30.

**2026-09-03 (phase 30b planning).** Resolved 30b's stated either/or in favour of the
*justified per-dialect exception*, not routing through `backbuild::emit`:
`incremental_models.md` §"Statement emission (single owner)" already places schema-evolution DDL
outside the maintenance/backbuild emitter rule, and `smelt-state`'s DDL layer is multi-dialect
(DuckDB/Spark/BigQuery) with struct-field, nested-widening and nullability operations the
DuckDB-test-grade backbuild emitters have no forms for. The residual defect is real but *inside*
`smelt-state`: `schema_tracking::generate_migration_sql` authors its own inline DuckDB DDL beside
`ddl_duckdb`, so 30b collapses that second author and widens the structural scan to
`smelt-state/src` with the three `ddl_<dialect>.rs` modules as declared owners. No phase rows
added or removed; phase 30's suggestion of per-technique backbuild parity fixtures declined and
recorded under "## Out of scope".

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->