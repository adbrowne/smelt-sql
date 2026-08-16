# Outcome: Wire the definition-delta synthesis layer (plan-and-approve migration)

**Created:** 2026-08-15
**Status:** superseded
**Superseded by:** the delta-signature closure programme — `docs/handoffs/2026-08-16-delta-signature-closure-programme.md`. This outcome will never be run as written; its content remains reusable by the successor outcomes named there.
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
(phase 18/19's pattern: record the call, drop the "(Open Question)" tag, move on). A bullet the
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
    every Known Divergences bullet this outcome's phases 10–19 close is actually removed from the
    respective spec (not just addressed in code) — the same discipline success criterion 8
    already applies to `definition_deltas.md`. Bullets this outcome deliberately does not close
    (the "Out of scope" list) stay, worded accurately rather than pointing at a done outcome as if
    it still owned them.

## Out of scope

`docs/outcomes/` is the only currently-live tracking layer — every `docs/plans/*` file predating
it is either fully done, superseded by a later outcome/spec, or (rarely) genuinely orphaned.
Genuinely orphaned bullets that are within the three anchor specs, and closeable *without* a new
product decision, are pulled into scope (phases 10–19 above, plus the two spawned outcomes below)
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
  `definition_deltas.md` §"What stays data-side". (Phase 18 does give `change_feed` sources an
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
  a small in-spec call like phase 18's list. Recommend scoping these as their own outcome(s) once
  a product owner is ready to decide them, rather than folding a design pass into an
  implementation-only outcome.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Resolve the two open design questions (plan-hash scope, diagnostic rename/split) and land the decisions in `definition_deltas.md` before wiring against them | done |
| 2 | Wire `smelt migrate` (plan-only): CLI verb invokes the backbuild synthesis layer end to end and prints the per-group verdict/technique plan | pending |
| 3 | Approval store + `--apply`: plan-hash persistence, hash-mismatch/staleness refusal, CI exit codes | pending |
| 4 | Rename `smelt backbuild` → `smelt rebuild` across CLI, docs-site, examples, tests, and the spec sweep (`cli.md`, `model_selection.md`, `architecture.md` prose) named in success criterion 8 | pending |
| 5 | Conformance harness gains a definition-edit step kind; wire into the generative equivalence suite | pending |
| 6 | Close the atomicity divergence (unify the `schema_evolution` full-refresh escape with the migration gate, or land its repair path) | pending |
| 7 | Diagnostic rename/split lands in code; surface ahead of a run via LSP and `smelt explain` | pending |
| 8 | docs-site migration guide: rewrite `guide/backbuild-synthesis.md` in place around `smelt migrate`/`--apply`, drop its stale "no CLI command yet" and naming-collision callouts; update `models.md`/`seeds.md`'s "no `smelt migrate`" bullets | pending |
| 9 | Validate + close out: `/smelt:validate definition_deltas` clean, Known Divergences bullets removed (including the sibling-spec sweep in success criterion 8), full standing-gate sweep | pending |
| 10 | Wire run-loop dispatch for a `KeyedUpsert`→`grain: partition` key-addressed repair cell (today derived but never dispatched outside the `grain: key` branch); narrow/remove the corresponding clause of `incremental_models.md`'s scheduler-currency divergence bullet | pending |
| 11 | Per-cell frontier addressing: schedule per-cell `deferral`; runtime-lower `diff_patch` over the region `DeleteInsert` default | pending |
| 12 | Write-pin equivalence: thread real column-comparability into the per-cell equivalence hook; pre-execution refusal gate for an inadmissible write-variant pin | pending |
| 13 | Observed-delta consumption: live `--since-upstream` read, backward resolution, keyed-fold + staged-candidate recording, settle-bound × observed-delta "delta empty" leg | pending |
| 14 | Maintained-model-creation execution technique; frontmatter-time grain check for `GROUP BY`-derived `grain: key` identity | pending |
| 15 | Plan-consumer + graph-layer gap sweep: horizon-clamped quadrant fixture, mutation-vs-rederivation dispatch distinction, `prefer`/`scan_bounds.on_violation` consumption, `AppendOnly` `UpstreamMutation` cell, bare-keyed-node fixture, time-unrolled self-edges, key-level graph dirt, full `--since-upstream` web_analytics compatibility, `--select` scoping | pending |
| 16 | Maintenance-plan proof residues: derived (not assumed) keyed-locality write-footprint mirror, finer-than-partition column-group dirt, hour-granularity propagation, `INTERSECT`/`EXCEPT` per-arm classification | pending |
| 17 | Conditional-maintenance gap sweep: `--show-sql` suppressed-form rendering, region DELETE+INSERT conditional variant, keyless staged-candidate realisation, `write:` pin, external `mutable_snapshot` fingerprint-sidecar consumption | pending |
| 18 | Decide and record the small Open Questions (out-of-band-edit tripwire, `on_column_add` supersession, docs-site CLI-coverage audit, group-merge-provenance, `change_feed` `UpstreamMutation`) in their owning specs | pending |
| 19 | Close two key-grain frontmatter/CLI validation gaps: refuse a window-forward keyed run started with an incomplete event-time window instead of silently full-refreshing; make `safety_overrides:` on a key-addressed model a hard frontmatter error | pending |
| 20 | Validate + close out (extended): `/smelt:validate incremental_models` and `/smelt:validate incremental_shapes` clean for every bullet phases 10–19 close, alongside the existing `definition_deltas` validate in phase 9 | pending |

## Decision log

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

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
