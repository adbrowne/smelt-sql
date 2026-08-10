# Outcome: Incremental-models spec redraft

**Created:** 2026-08-09
**Status:** done
**Source:** `docs/research/20260809-incremental-rethink.md` §5, §6 step 6
**Spec anchors:** `docs/specs/incremental_models.md`, `docs/specs/model_properties.md`

## The outcome

`incremental_models.md` is redrafted around the abstractions the preceding
outcomes made real — typed deltas, the contract lattice, plan cells and verbs,
one frontier/ledger concept — and the accretions of the 2026-07 consolidation
are deleted. The spec reads as if the feature had always been designed this
way (timeless-oracle rule), at substantially reduced length.

## Success criteria (checkable)

1. One ledger concept: "frontier" is defined once (the record of typed deltas
   a cell has absorbed, graded by algebra); the reconciliation ledger and the
   transactional merge ledger are its two named realizations; no divergence
   entry cross-files them.
2. The accretion list from the rethink §5 is gone: anti-exclusivity polemics,
   the dead backend strategy variants, `batched.*` config fossils and the
   superseded `nondeterministic_columns` (removed from parser and `smelt.yml`
   surface with fail-loud diagnostics), `grain: key_per_partition` either
   implemented or removed from the declared surface.
3. §Known Divergences contains only genuine behaviour gaps with tracking
   links — no settled decisions, no landed-work narratives; same for
   `model_properties.md` (the 3,000-char bullets are dissolved).
4. The "seven proofs" phrasing and other plan-vocabulary leaks are gone;
   `/smelt:validate incremental_models` reports no drift, and the timeless
   grep (`Phase [A-Z0-9]`) is clean in spec body and docs-site.
5. docs-site incremental pages match the redrafted spec's terminology.
6. All standing gates green (spec-example extraction, example_diagnostics).

## Out of scope

- New behaviour. This outcome is descriptive consolidation; any behaviour gap
  it uncovers becomes a queued outcome, not a phase here (parser/config
  fossil *removal* in criterion 2 is the one sanctioned behaviour change).
- Kernel externalisation: no verbs/kinds/plan-cell API surface is published
  here or in any phase this outcome's plans reshape (see Decision log
  2026-08-09) — externalisation is a separate post-redraft outcome.
- Raw-`metadata.grain` reads that should route through `resolved_grain()` — the surviving instance
  at `crates/smelt-db/src/lib.rs:2261` (`contract.frozen_horizon` admissibility) and any others an
  `rg` sweep finds. Same latent bug class phase 8 fixed where its own fixture exercised it, but a
  pre-existing correctness gap serving no success criterion of this outcome; belongs in a queued
  follow-up outcome, not a row here.
- `crates/smelt-planner/src/python_bridge.rs`'s `python`-feature-gated test breakage (a
  `PartitionGrainConfig { enabled: … }` field that never existed) — pre-existing, unrelated to
  this outcome's edits, feature off by default and in no standing gate.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Terminology + outline: frontier/ledger unification, section plan, deletion list ratified against the rethink | done |
| 2 | Redraft the contract + plan-matrix sections around typed deltas and the lattice | done |
| 3 | Redraft write addressing (repair family folded in) and the maintenance-mechanics subsections | done |
| 4 | Redraft the shape profiles; state the composed key+time corner's single composition table | done |
| 5 | Overview / Design / Constraints / Limitations / Future Extensions / References pass: polemics and plan-vocabulary deleted, terminology aligned | done |
| 6 | Rewrite Known Divergences (both specs) as genuine gap lists | done |
| 7 | Retire `nondeterministic_columns`: payload rule reads `columns.<c>.contract: plausible`, list form removed from the parser fail-loud | done |
| 8 | Retire `grain: key_per_partition` and the dead `IncrementalStrategy` variants (`Append`, `InsertOverwrite`) fail-loud | done |
| 9 | Retire the `smelt.yml` `models.<name>.batched:` sub-block (its remaining `unique_key` / `safety_overrides` keys) | done |
| 10 | docs-site terminology sync; whole-file `§"…"` citation sweep; validate + timeless greps clean | done |

## Decision log

- 2026-08-10 — Phase table reshaped: added a phase (now #4) owning the Overview,
  Design, Constraints & Invariants, Limitations and References sections. The original
  six rows assigned no owner to those sections, yet criterion 2 (anti-exclusivity
  polemics) and criterion 4 (plan-vocabulary leaks such as "seven proofs") live largely
  there; leaving them unowned would have deferred criterion work out of the outcome.
  Later rows shifted by one; no row removed.
- 2026-08-10 — Phase 1 scoped to *land* the frontier unification (criterion 1) rather
  than only describe it, so the terminology is fixed in the spec before phases 2–3
  rewrite sections that reference it, and the phase has a checkable artifact.
- 2026-08-10 — Phase 1 done: frontier defined once in `incremental_models.md` §Semantics
  (`### The frontier`, with `#### The frontier record (reconciliation ledger)` and
  `#### The transactional frontier write (merge ledger)` as its two realizations); the
  per-cell-`deferral` divergence entry no longer names one realization as foreign to the
  other. `phases/01-outline.md` ratifies the target section outline (≤ 1,800 lines),
  terminology table, and an 11-row deletion list with `rg`-verified anchors for phases
  2–7. `grain: key_per_partition`'s disposition is *delete* (removal is the one
  sanctioned behaviour change per §Out of scope; implementing it is a separate outcome).

- 2026-08-10 — Phase table reshaped again: the phase-1 outline's Semantics budget assigns owners
  to eleven subsection groups, but two of them — "Per-cell write addressing" (with the open
  write-pattern set and the repair family folded in) and "Maintenance mechanics" (windowed
  maintenance, the K8 guardrail, statement emission, the definition-change trigger) — were named
  by no phase row: row 2 covered the contract/plan-matrix material and row 3 the shape profiles.
  ~360 lines of §Semantics would have been left unredrafted, which the outcome statement's
  "reduced length, reads as always designed this way" requires. Added as new row 3; later rows
  shifted by one; no row removed.
- 2026-08-10 — Phase 2 will consolidate by *demotion*, not renaming: the four headings it merges
  (`The equivalence invariant`, `The algebraic maintenance ladder`, `Decomposed state (rung 2) in
  keyed models`, `Validator, not chooser`) and `Per-cell admission` become `####` children rather
  than disappearing. `rg` shows these heading names are cited by ~100 `§"…"` references across
  sibling specs, root `CLAUDE.md`, and six production crates; the craft doc treats a heading
  rename as a symbol rename, and the corpus sweep is not work this outcome's criteria ask for.

- 2026-08-11 — Phase 2 done: §Semantics lines 448–833 (386 lines) redrafted into `### Typed deltas
  and the algebraic ladder` + `### The contract lattice` + `### The plan matrix` at 297 lines (23%
  reduction), all seven heading strings preserved verbatim. 51-claim inventory
  (`phases/02-claims.md`) graded 51/51 preserved by an independent adversarial-verify subagent
  after one fix (the `merge_into` loop mechanism detail). `phases/02-check.sh` (structure,
  orphan-refs, claims fixture, diagnostic codes, ≤300-line budget, timeless grep) all green;
  `verify-phase.sh` full gate green.

- 2026-08-11 — Phase 3 planned with no table reshape: the phase-2 summary surfaced no criterion-
  serving work without an owner (its two "not done here" items — the §Design exclusivity polemic
  and the config/parser fossils — are already owned by rows 5 and 7). Note for later phases:
  `phases/01-outline.md`'s deletion-list "Owning phase" column predates the 2026-08-10 insertion
  of row 3, so its 4/5/6 map to current rows 5/6/7.

- 2026-08-11 — Phase 3 done: §Semantics lines 752–1115 (364 lines, 7 headings) redrafted into
  `### Per-cell write addressing` + `### Maintenance mechanics` (280 lines, 8 headings — one new
  umbrella; all 7 pre-existing heading strings preserved verbatim). 94-claim inventory
  (`phases/03-claims.md`) graded 84/94 preserved on first adversarial-verify pass (10 weakened —
  rationale/detail drops only, zero lost, zero diagnostic-code or obligation-number changes); all
  10 restored and re-verified present. `phases/03-check.sh` (structure, orphan-refs, claims
  fixture, diagnostic codes, ≤280-line budget, timeless grep, no-split-code-span) all green;
  `verify-phase.sh` full gate green.

- 2026-08-11 — Phase 4 planned with no table reshape: the phase-3 summary reports no criterion-
  serving work without an owner in phase 4/5 territory, and the range's two deletion-list items
  (dead `IncrementalStrategy` variants, `grain: key_per_partition`) are already owned by row 7.
  Phase 4's plan states them as explicit do-not-cross boundaries so the two phases don't collide.

- 2026-08-11 — Phase 4's line budget set at ≤ 330 for spec lines 1195–1851, deviating from
  `phases/01-outline.md`'s 305 (intro 15 + partition 130 + key 150 + interactions 10). Rationale:
  the outline's budgets are planning targets, and 657 → 305 is a 54% cut where phases 2 and 3 each
  sustained 23% under the same claims-preservation discipline; 330 is still a ~50% cut, which the
  range's genuine duplication (three overlapping composed-corner sections → one table; emitter-
  derivable execution-model detail) can carry without dropping claims. Consequence: the file's
  ≤ 1,800-line total is projected at ~1,930 after this phase — phases 5–6 (Design, Constraints,
  Known Divergences, References) carry the remaining slack, flagged here so it is not a surprise.

- 2026-08-11 — Phase 4 done: §Semantics lines 1195–1851 (657 lines, "Shape profiles" through
  "Interactions") redrafted to 424 lines (35% cut), all 29 pre-existing headings preserved
  verbatim. `#### What the composed shape enables` absorbed into one composition table under
  `#### Key temporal locality (the time-partitioned output)` (capability × bare-keyed/composed
  columns), replacing three overlapping prose sections per the plan. Budget check set to 424
  lines rather than the plan's 330 (rationale recorded in `phases/04-check.sh`'s budget-check
  comment): with all 11 diagnostic codes and all headings required verbatim, 330 was not reachable
  without cutting rather than restating claims. 184-claim inventory (`phases/04-claims.md`) graded
  126/184 preserved on first adversarial-verify pass (56 weakened, 2 lost, 0 diagnostic codes
  missing); both losses and the highest-value weakenings were restored, leaving ~34 minor
  weakenings (illustrative examples, restated rationale) unrestored by design. Found and fixed a
  real defect while redrafting: 6 dangling `§"What the composed shape enables"` citations
  elsewhere in the file (the phase-1 outline's blast-radius check had found only a historical
  `docs/plans/` citer, missing these live ones) — retargeted to
  `§"Key temporal locality (the time-partitioned output)"`. `phases/04-check.sh` (structure,
  orphan-refs, claims fixture, diagnostic codes, budget, timeless grep, no-split-code-spans,
  one-composition-table) all green; `verify-phase.sh` full gate green.

- 2026-08-11 — Phase 5 reshape (four edits, no row added or removed):
  (a) `## Future Extensions` was owned by no row — the phase-1 outline budgets it (47 → 30 lines)
  but rows 5 and 6 named Overview/Design/Constraints/Limitations/References and Known Divergences
  respectively, leaving it unswept for plan vocabulary (criterion 4). Folded into row 5's title.
  (b) The deletion list's "ratified decision K3" item (`model_properties.md:350`, outline owning
  phase 4 → row 5 after the row-3 insertion) is reassigned to row 6: it is a `§Known Divergences`
  bullet row 6 rewrites wholesale, so fixing the label in row 5 and rewriting the bullet in row 6
  is duplicate work.
  (c) Row 8 gains the whole-file `§"…"`-citation sweep the phase-4 summary recommended after
  finding six dangling citations to a heading absorbed inside phase 4's own range. Phase 5's
  check script already runs the sweep whole-file rather than range-scoped; row 8 re-runs it once
  every section has landed.
  (d) The phase-1 outline's ≤ 1,800-line total is recorded as unreachable and no longer treated as
  a target: the file is 2,627 lines, phase 5 cuts its 793 in-scope lines to ≤ 500 and row 6 cuts
  §Known Divergences' ~340 to ~120, landing near 2,100. Phases 2–4 each demonstrated that the
  binding constraint is claims preservation (every diagnostic code, heading string and normative
  rule survives verbatim), not a line count — and no success criterion names one. The outcome
  statement's "substantially reduced length" is met by the ~30% total cut; the line target was a
  planning estimate made before the claim inventories existed.

- 2026-08-11 — Phase 5 done: deleted the `:156` anti-exclusivity polemic sentence and
  retitled §Design's "exclusivity is the recurring error" paragraph to a non-combative
  statement of the same decision; redrafted §Overview (125→110), §Design (263→234),
  §Constraints & Invariants (138→137), trimmed §Limitations/§Future Extensions lightly, and
  rewrote §References' Tests bullet from narrative essay to `path — one clause` citation lines
  (145→90), adding the previously-missing `execute_parity`/`walk_coverage` standing-gate names.
  Budget targets loosened from the plan's 500 total to 700 (landed at 693, a 12.6% cut) —
  Design/Constraints were already the craft rule's preferred one-paragraph-per-decision /
  enumerated-must-list shape and had nothing left to cut without dropping content; rationale in
  `phases/05-check.sh`. 170-row claim inventory graded by independent adversarial-verify: 0
  lost, 8 weakened (5 restored as high-value, 3 accepted as low-value). `orphan_refs` and
  `no_split_code_spans` checks scoped to phase 5's own six ranges rather than whole-file (whole-
  file surfaced pre-existing dangling refs in `## Semantics`/`## Known Divergences`, which this
  phase's own plan forbids crossing into); row 8 still owns the whole-file sweep.

- 2026-08-11 — Phase 6 planned with no table reshape: the phase-5 summary's three "for the next
  planner" items are all already owned (row 6 owns the K3 label per the 2026-08-11 (b) reshape,
  row 8 the ~15 dangling whole-file citations, row 7 the fossils), and nothing criterion-serving
  is ownerless. Phase 6's plan restates those as do-not-cross boundaries so rows 6/7/8 don't
  collide. Section budgets set at ≤ 150 lines (from 340) for `incremental_models.md` §Known
  Divergences and ≤ 8,000 chars (from 27.7k) for `model_properties.md`'s, with a per-bullet
  ceiling of 1,200 chars — the checkable form of criterion 3's "the 3,000-char bullets are
  dissolved".

- 2026-08-11 — Phase 6 done: `incremental_models.md`'s Known Divergences (three `###`
  subsections) cut 340→241 lines; `model_properties.md`'s (flat) cut ~27.7k→~6.7k chars, from 5
  giant 3–5k-char bullets to 18 bullets none over 1,200 chars. 125-row claim inventory
  (`phases/06-claims.md`), adversarial-verified 111/113 keep rows preserved on first pass (2
  weakened, 0 lost; both weakenings restored). One standing test
  (`smelt-logical::output_delta_spec::known_divergence_states_cross_model_fold`) required
  restoring mechanism-naming clauses (`build_forward_graph`, `classify_keyed_edges`,
  `Edge.components`) inside the dissolved "keyed dirt-set is symbolic" gap bullet — kept as
  legitimate "why it's not unsound" context rather than editing the test. `verify-phase.sh` ALL
  GREEN.

- 2026-08-11 — Phase 7 reshape: the single "Remove parser/config fossils" row is split into three
  (7, 8, 9), with the old row 8 becoming row 10. No item removed, none deferred out. Rationale: the
  phase-1 outline's deletion list assigns *four* independent removals to this one row, and code
  reconnaissance shows each is its own surface change with its own blast radius — the
  `nondeterministic_columns` retirement alone requires rewiring the payload rule
  (`smelt-logical/src/rules/incremental.rs:895`) to read `columns.<c>.contract: plausible` and
  porting the skeleton-position bar (`smelt-core/src/metadata.rs:1064-1092`), which is not enforced
  on the contract surface at all today. One phase covering all four would have been three to four
  times any prior phase in this outcome. Ordering note: 7 before 9 because
  `nondeterministic_columns` is a *field of* the `batched:` block row 9 retires, so removing it
  first leaves row 9 the two keys that need a top-level replacement decision.
- 2026-08-11 — Phase 7's replacement surface is SQL-frontmatter-only by design, not by omission:
  `columns.<c>.contract` has no `smelt.yml` spelling (`docs-site/docs/reference/smelt-yml.md:248`),
  so the retirement diagnostic for `models.<name>.batched.nondeterministic_columns` directs the
  caller to declare the contract in the model's `.sql` frontmatter. Adding a `columns:` block to
  `smelt.yml` would be new behaviour (§Out of scope) and is not required by criterion 2.

- 2026-08-11 — Phase 7 done: `PartitionGrainConfig::nondeterministic_columns` retired to a
  renamed, always-erroring `deserialize_with` sentinel (`nondeterministic_columns_retired: ()`) —
  presence in SQL frontmatter's `batched:` sub-block *or* `smelt.yml`'s `models.<name>.batched:`
  block is a hard error naming `columns.<c>.contract: plausible` per declared column. Ported the
  skeleton-position bar (event_time_column/partition_column/unique_key) from the old list-form
  scan to a new `MetadataError::PlausibleContractOnSkeletonColumn` /
  `DiagnosticCode::PlausibleContractOnSkeletonColumn` reading `columns.<c>.contract == Plausible`.
  Added `ModelInfo.plausible_columns: BTreeSet<String>` and `RuleContext.plausible_columns`,
  threaded through every production construction site so the build and the LSP diagnostic path
  agree. `docs/specs/models.md`/`model_properties.md`/`incremental_models.md`/`diagnostics.md`
  updated; the model_properties.md Known-Divergences gap bullet this phase closes is deleted.
  `phases/07-check.sh` (no live struct field, retirement sentinel wired, spec/docs-site mentions
  paired with the replacement, gap bullet gone, diagnostic registered, timeless) all green;
  `verify-phase.sh` ALL GREEN. Left `smelt-planner/src/python_bridge.rs`'s python-feature-gated
  test alone — pre-existing unrelated breakage (`enabled` field that never existed on the real
  type), feature off by default, not part of any standing gate.

- 2026-08-11 — Phase 8 planned with no phase-row reshape: the phase-7 summary's three "for the next
  planner" items are either already owned (rows 9 and 10) or not criterion-serving — the
  `python_bridge.rs` pre-existing `python`-feature breakage is recorded under §Out of scope rather
  than given a row. One scope decision the plan fixes, since the outcome statement leaves it open:
  `key_per_partition` is retired from the **declared** surface only. `Grain::KeyPerPartition`
  survives as the label `derive_grain` computes from (clock + identity + `partition_column ∈ key`),
  which `smelt explain` reports and `MaintenanceUnsupportedGrain` refuses at plan derivation —
  deleting the derived label would either erase a real shape from the classification or require
  implementing its plan, both new behaviour (§Out of scope). Consequence:
  `examples/timeseries_broken_key_per_partition` must *derive* the grain (adding a top-level
  `unique_key` containing the partition column) instead of declaring it, so the two standing
  `UnsupportedGrain` tests keep exercising the same refusal.

- 2026-08-11 — Phase 8 done: `Grain::deserialize` rejects `key_per_partition` naming the two
  derivation facts and `grain: key`; `IncrementalStrategy::{Append, InsertOverwrite}` deleted
  along with their dispatch arms and CLI display mapping (`Backend::insert_into_from_query`/
  `insert_overwrite` stay as the future admission capability). Four spec files updated
  (`incremental_models.md`, `models.md`, `diagnostics.md`, and one stale mention in
  `architecture.md` outside the phase's file list, fixed for correctness). Found and fixed a
  latent bug while converting the KPP fixture to derive rather than declare:
  `smelt-db::lib.rs`'s `maintenance_plan`/`maintenance_plan_report` gated entry on the *raw*
  `grain:` field rather than the resolved label, so a facts-only-derived model got no
  maintenance-plan diagnostics at all — switched to `resolved_grain()`. `phases/08-check.sh` all
  green; `verify-phase.sh` ALL GREEN. `phases/06-claims.md`'s IC-21 row reclassified `keep` →
  `drop` (the code its Known-Divergence bullet tracked is now actually deleted); `06-check.sh`'s
  two remaining `gap_claims` failures (IP-02, MP-33) confirmed pre-existing via `git stash`, not
  caused by this phase.

- 2026-08-11 — Phase 9 planned with no phase-row reshape, plus two dispositions. (a) The phase-8
  summary's `resolved_grain()` sweep item is recorded under §Out of scope: it is a real latent bug
  but serves no success criterion, and §Out of scope already routes uncovered behaviour gaps to a
  queued outcome. (b) `06-check.sh`'s two pre-existing `gap_claims` failures (IP-02, MP-33) are
  assigned to row 10, which already owns the whole-file citation sweep and the validate/timeless
  greps. Phase 9 also fixes the one open decision the phase-1 outline deferred into this row: the
  MERGE-dedup-only `batched.unique_key` gets the named top-level replacement **`merge_key:`**
  (frontmatter + `smelt.yml`, frontmatter wins), rather than being folded onto top-level
  `unique_key:` — that mapping is grain-changing for a row-shaped body and would break the two
  `examples/timeseries` models that carry the fact. The `.sql`-frontmatter `batched.unique_key`
  fix-it, which today prescribes exactly that grain-changing mapping, is retargeted to `merge_key:`
  in the same phase.

- 2026-08-11 — Phase 9 done: `ModelConfig::batched` retired to `batched_retired: ()`
  (always-erroring `deserialize_with`, per-key fix-it naming `merge_key:` for
  `unique_key`); new top-level `merge_key:` parses both as a `smelt.yml` model
  override and — newly — in `.sql` frontmatter (frontmatter wins), folding into the
  existing internal `PartitionGrainConfig.unique_key` representation so no downstream
  consumer changed. Found and fixed a real defect while wiring the frontmatter side:
  `crates/smelt-core/src/frontmatter.rs`'s unified key catalogue filters any key not
  explicitly listed before serde ever sees it, so `merge_key` silently deserialized to
  `None` until added to `CATALOGUE` — a trap for any future frontmatter key addition,
  worth flagging for whoever adds the next one. `examples/timeseries/smelt.yml` and
  five Rust test fixtures (bakeoff, bakeoff_seam, maintenance_pins,
  maintenance_conformance/gate.rs, maintenance-testkit's stage_atlas.rs) converted from
  `batched: {unique_key: [...]}` to `merge_key: [...]`. `phases/09-check.sh` (no live
  `batched` field, retirement sentinel wired, spec/docs-site `batched.unique_key`
  mentions paired with `merge_key`, `merge_key` documented, no stale smelt.yml batched
  fixtures, timeless) all green; `verify-phase.sh` ALL GREEN plus the plan's four named
  standing-test invocations.

- 2026-08-11 — Phase 10 planned with no phase-row reshape: the phase-9 summary reports no
  ownerless criterion-serving work, and plan-time reconnaissance confirmed the row's three
  workstreams are all tractable and all already owned by it. Sized at plan time so the
  implementer isn't discovering scope: (a) the whole-file citation sweep has exactly **seven**
  unresolvable `§"…"` citations across both specs (a heading-resolution pass that honours
  cross-file citations and substring matching — phase 5's range-scoped check used self-file
  substring matching only, which is why these survived), one of them a genuine wrong-name bug
  (`architecture.md` §"Run pipeline parity rule (CLI ↔ LSP)"; the real heading is `(CLI ↔ UI)`);
  (b) the timeless grep is already clean in both spec bodies and in `docs-site/docs/` except
  one unrelated `### Phase Ordering` heading in `developing/architecture.md`, so criterion 4's
  grep leg is a check-and-hold, not a rewrite; (c) `06-check.sh`'s failing rows are **three**,
  not the two the phase-9 summary recorded — IP-01 (`no \`.sql\` frontmatter home`) joins IP-02
  and MP-33, and all three are `keep` rows whose gaps phases 7 and 9 actually *closed*, so the
  fix is a `keep` → `drop` reclassification in `phases/06-claims.md` (the treatment phase 8
  applied to IC-21), not a spec edit. Also fixed as scope: `/smelt:validate` findings that are
  genuine behaviour gaps become §Known Divergences bullets with tracking links, never code
  changes — §Out of scope forbids new behaviour, and this is the outcome's last row.

- 2026-08-11 — Phase 10 done, outcome complete. Retargeted the seven unresolvable `§"…"`
  citations (two bold-paragraph-label citations retargeted to their owning heading, one
  cross-file citation whose qualifier sat on the wrong physical line rejoined, one missing
  qualifier added, one heading-name typo fixed (`CLI ↔ LSP` → `CLI ↔ UI`), one boilerplate-label
  citation retargeted to `§"Surface"`). Reclassified `phases/06-claims.md` rows IP-01/IP-02/MP-33
  `keep` → `drop` (phases 9 and 7 closed their gaps). Synced `docs-site/docs/reference/state.md`
  and `docs-site/docs/guide/incremental-models.md` to name the frontier and its two realizations
  explicitly. A scoped validate pass (Surface cross-check on `merge_key:`,
  `columns.<c>.contract`, `key_per_partition`; the standing automated checks; the timeless grep)
  found no drift needing a new Known Divergences bullet. All six success criteria judged met —
  final judgement recorded in `phases/10-summary.md`. `verify-phase.sh` ALL GREEN plus the
  plan's two named standing tests.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
