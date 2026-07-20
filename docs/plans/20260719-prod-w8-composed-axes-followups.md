# Plan: Production W8 — composed-axes follow-ups (deferred-item sweep)

**Date**: 2026-07-19
**Spec**: [`docs/specs/models.md`](../specs/models.md) §"The Relation Contract", [`docs/specs/incremental_models.md`](../specs/incremental_models.md) (§Known Divergences "The partition grain" / "The key grain")
**Spec diff**: Phase 1 of this plan (sub-block retirement surface); Phases 5–6 close divergences against already-landed spec text
**Tracking PR / branch**: `worktree-production`
**Docs**: code+docs
**Master**: [`docs/plans/20260719-production-readiness.md`](20260719-production-readiness.md) (sub-plan W8)
**Source plan**: [`docs/plans/20260715-composed-axes-conditional-maintenance.md`](20260715-composed-axes-conditional-maintenance.md) — this plan works the actionable entries from its "Explicitly deferred" / "Deferred during implementation" sections and decision 9, and records a tracked home for everything that stays deferred.

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/models.md` §"The Relation Contract" and the `incremental_models.md` Known Divergences sections named above — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-production`. If not, ask the user before continuing.
3. **Pre-flight: the composed-axes work must be merged.** `docs/plans/20260715-composed-axes-conditional-maintenance.md` must exist in the tree and `rg -l 'KeyedRecurrenceBoundViolated' crates/` must be non-empty. If either fails, PR #163 has not been merged/rebased yet (master plan decision D2) — emit `<<PHASE_BLOCKED>>` naming D2; do not attempt the merge yourself.
4. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Red-green TDD; real-fixture tests where the phase has user-visible behavior.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call; failures-only output).
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Timeless-oracle rule: spec and docs-site edits carry no phase vocabulary.

---

## Context

The composed-axes + conditional-maintenance plan closed with all 40 phases done but left a small, explicit residue. Two items are user-surface debt from the Relation Contract cut: the `batched:` sub-block (`batched.unique_key` / `batched.safety_overrides` / `batched.nondeterministic_columns`) is still the live spelling for options whose top-level / `columns.<c>.contract` replacements the spec already names, and the pre-cut "batched" mode vocabulary survives in diagnostic codes and config type names. Two items are evidence debt: the C4 conformance story lacks its generative leg (no generated recipe can resolve `Suppressed` today, so the gate exercises only the `Unconditional` fallback — recorded honestly in the source plan's deferred section), and `build_forward_graph`'s driving-source granularity plumbing covers declared `sources.*` only, not the recursive case where the driving source is another maintained model's own composed output (source plan decision 9). This plan retires the surface debt and closes the evidence debt; everything else the source plan deferred is demand-gated or already tracked elsewhere and is listed under "Explicitly deferred" with its home.

## Scope

### In scope (spec coverage)
- `models.md` §"The Relation Contract" / refresh axis: top-level `safety_overrides:` parses; the `batched:` sub-block is retired with a fix-it naming the exact replacement keys; `nondeterministic_columns` retires in favour of the already-parsing `columns.<c>.contract: plausible`.
- `incremental_models.md` §Known Divergences "The partition grain": the "sub-block remains" and "pre-cut spellings" divergence entries close.
- `incremental_models.md` conformance posture: the change-suppressed column-scoped MERGE gains its generative equivalence leg (the C4 deferred item), narrowing G1's "the C4/E4 conformance legs are the proof" caveat to nothing.
- `incremental_models.md` §"Key temporal locality" / graph layer: a composed model driven by another maintained model's composed output reaches the same locality admission verdict through `build_forward_graph` that `smelt explain` already reports (decision 9's unplumbed recursive case).

### Explicitly deferred
Each remaining source-plan deferred item, with its tracked home — none is silently dropped:
- **`key_per_partition` trajectory profile** (backfill-cascade discipline, lateness truncation) — demand-gated; gets its own plan when a real workspace needs it (A0's refusal stands).
- **Granularity relaxation, snapshot-reconcile locality, slice-scoped deletion (`NOT MATCHED BY SOURCE`)** — spec Open Questions, unchanged.
- **Keyed dirt-sets / time-unrolled self-edges; sub-day propagation axes; hourly driver granularity** — graph/driver generality with no current consumer; revisit on demand.
- **Automatic watermark-diffed `--since-upstream`** — spec Future Extensions; explicit `--source`/`--landed` stays v1.
- **`versioning: interval`** — own divergence entries; unrelated to this diff.
- **Statistics-fed cost model for technique choice** (G1's open question: region-level change-ratio statistics from observed deltas) — belongs with the measurement machinery; W7 (`smelt bakeoff`) is the natural home, recorded in its decisions when it gets there.
- **Spark emitters for the merge ledger, observed-delta store, and fingerprint sidecar** (fail-loud `UnsupportedFeature` today) — Spark surface parity is W4's evidence brief's call; the fail-loud posture is the accepted divergence until then.
- **`smelt migrate` assist** — no `smelt migrate` command exists for any surface; a three-key mechanical rename does not justify inventing one. Phase 3's fix-it prints the exact replacement YAML instead. Revisit if a future retirement is non-mechanical.
- **Row-identity proof through joins** (the wider alternative for the C4 leg) — Phase 5 takes the narrower declared-key recipe cut; proving grain keys through joins remains open, tracked by the P2 divergence in `incremental_models.md`.

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| 1     | done    | (this commit) | 2026-07-20 |
| 2     | done    | 42147964 | 2026-07-20 |
| 3     | done    | (this commit) | 2026-07-20 |
| 4     | done    | (this commit) | 2026-07-20 |
| 5a    | pending |        |      |
| 5b    | pending |        |      |
| 6     | done    | 31e297e6 | 2026-07-20 |

*(Phase 5 was reshaped 2026-07-20 into 5a + 5b after the original "no production code expected" scoping was proven unsatisfiable — see "## Blocked phases". 5a is the production dispatch change that makes `Suppressed` reachable for a generatable recipe; 5b is the original generative conformance leg, now satisfiable on top of 5a.)*

### Phase 1: Spec diff — sub-block retirement surface

**Goal.** `models.md` states the post-retirement surface normatively: top-level `unique_key:` (already live) and `safety_overrides:` beside it; column contracts via `columns.<c>.contract`; the `batched:` sub-block refused with a fix-it. `incremental_models.md` divergence entries updated to point at this plan.

**Pre-conditions.** Pre-flight check from the execution prompt passed (composed-axes merged). Docs-only phase.

**TDD tests to write first.** None (docs-only). Phases 2–3 write the tests against this text.

**Implementation shape.** In `models.md`, move `safety_overrides` from the `batched:` sub-block description to the top-level key list (same precedence sentence as `unique_key:`: frontmatter wins over `smelt.yml` model overrides); state that `nondeterministic_columns` has no top-level form — its replacement is `columns.<c>.contract: plausible` (semantics unchanged, owned where they already are). State the refusal: a `batched:` sub-block is a hard error whose fix-it names each replacement key with the caller's own values. In `incremental_models.md` §Known Divergences "The partition grain", rewrite the "mode value is cut; the sub-block remains" entry to describe the target state as pending with this plan as tracker (behavioural terms, no phase vocabulary).

**Critical files (allowed to touch in this phase).**
- `docs/specs/models.md` — surface section.
- `docs/specs/incremental_models.md` — the two divergence entries.

**Docs touched.** *(timeless)*
- Spec files only; docs-site rides with Phases 2–3 when behavior changes.

**Review checklist** (material findings only):
- [ ] Top-level `safety_overrides:` precedence matches `unique_key:`'s existing rule exactly
- [ ] The fix-it contract (exact replacement keys, caller's values) is stated, not implied
- [ ] Divergence entries describe behaviour and link this plan; no phase vocabulary in spec body

**Commit.** `docs(spec): batched sub-block retirement surface — top-level safety_overrides, contract-key replacement, fix-it refusal`

### Phase 2: Top-level `safety_overrides:` parses

**Goal.** `safety_overrides:` is accepted at top level (`.sql` frontmatter and `smelt.yml` model overrides, frontmatter wins), feeding the same checks the sub-block form feeds today. Both spellings coexist this phase; declaring both is a conflict error.

**Pre-conditions.** Phase 1 merged.

**TDD tests to write first.**
- `crates/smelt-core/tests/` (metadata unit) — top-level `safety_overrides:` in frontmatter parses into `ModelMetadata` identically to the sub-block form; declared in both places ⇒ a conflict `MetadataError` (never silent precedence between old and new spellings).
- `crates/smelt-cli/tests/example_diagnostics.rs` — an `examples/` model migrated to the top-level spelling stays diagnostic-clean and its maintenance plan is unchanged (compare `smelt explain` output before/after the spelling flip).

**Implementation shape.** Mirror S1's top-level `unique_key:` landing: `ModelMetadata` reads the top-level key with the existing merge precedence; the safety-check consumers already take the parsed struct, so the work is extraction + the conflict diagnostic (new `MetadataError` variant — the exhaustiveness gate forces the `smelt-db` mapping arm).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/metadata.rs` — extraction + conflict variant.
- `crates/smelt-db/src/lib.rs` — the forced mapping arm.
- One `examples/` model flipped to the new spelling as the real fixture.

**Docs touched.** *(timeless)*
- `docs-site/docs/reference/smelt-yml.md` (and the models reference page if it lists frontmatter keys) — `safety_overrides` documented at top level.

**Review checklist** (material findings only):
- [ ] Conflict error, not precedence, between sub-block and top-level spellings
- [ ] No safety-check consumer changed behaviour (same overrides reach the same checks)
- [ ] Reference docs show the top-level spelling only

**Commit.** `feat(metadata): top-level safety_overrides — sub-block replacement parses, dual-declaration refuses`

### Phase 3: Retire the `batched:` sub-block

**Goal.** A `batched:` sub-block is a hard error with a fix-it naming each replacement (`unique_key` → top-level `unique_key:`, `safety_overrides` → top-level `safety_overrides:`, `nondeterministic_columns: [c]` → `columns.c.contract: plausible`) carrying the caller's own values, mirroring the `refresh: batched` retirement style. All in-repo fixtures migrate.

**Pre-conditions.** Phase 2 merged (the replacement spellings all parse).

**TDD tests to write first.**
- `crates/smelt-core/tests/` (metadata unit) — each sub-block key produces the hard error and the fix-it text contains the caller's actual values under the replacement spelling.
- `crates/smelt-cli/tests/example_diagnostics.rs` + `crates/smelt-lsp/tests/example_workspaces.rs` — green after every `examples/` workspace is migrated off the sub-block (`rg -l 'batched:' examples/` empty).
- `crates/smelt-cli/tests/maintenance_conformance` — conformance recipes that staged sub-block frontmatter are migrated and still equal the full-refresh oracle.

**Implementation shape.** Convert the sub-block acceptance in `crates/smelt-core/src/metadata.rs` into the refusal; delete the now-unreachable `KeyedForbidsBatched` guard (a `grain: key` model can no longer declare the sub-block at all — record the code's removal in `docs/specs/diagnostics.md`). Migrate every fixture, example, and doc snippet in the same commit so the gates stay green atomically.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/metadata.rs` — refusal + fix-it.
- `examples/**` + test fixtures — migration.
- `docs/specs/diagnostics.md` — code removal note.

**Docs touched.** *(timeless)*
- `docs-site/docs/**` — sweep every `batched:` sub-block snippet to the replacement spellings (`rg -l 'batched:' docs-site/docs/` empty).
- `docs/specs/incremental_models.md` — close the "sub-block remains" divergence entry.

**Review checklist** (material findings only):
- [ ] Fix-it carries values, not just key names
- [ ] `rg -l 'batched:' examples/ docs-site/docs/` both empty
- [ ] Divergence entry closed, not restated

**Commit.** `feat(metadata): retire the batched sub-block — hard error with per-key fix-it; fixtures and docs migrated`

### Phase 4: Rename the surviving pre-cut "batched" spellings

**Goal.** Pure internal rename: `BatchedConfig`/`BatchedSafetyOverrides` and the surviving diagnostic codes (`TimeseriesRequiredForBatched`, `BatchedNotSafe`) drop the retired mode vocabulary (e.g. `PartitionGrainConfig`, `TimeseriesRequiredForPartitionGrain`, `PartitionGrainNotSafe`); `crates/smelt-logical/src/rules/incremental.rs` module naming reviewed in the same sweep. Diagnostic *codes* are user-visible strings, so the catalogue moves with them.

**Pre-conditions.** Phase 3 merged (dead codes already deleted; only live spellings remain to rename).

**TDD tests to write first.**
- Existing suites are the net (rename must be behaviour-preserving); add one guard: a source-scan test in the `hardening_budget` style asserting `rg -c 'Batched'` over production `crates/**/src` is zero (allowing the term only in historical plan/research docs).

**Implementation shape.** Mechanical rename with `cargo fix`-assisted sweep; update `docs/specs/diagnostics.md` code entries and any spec prose citing the old code names. No semantics change of any kind — the reviewer's whole job is confirming that.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/config.rs`, `crates/smelt-core/src/metadata.rs`, `crates/smelt-logical/src/**`, `crates/smelt-db/src/**` — the rename ripple.
- `docs/specs/diagnostics.md`, spec prose citing old codes.

**Docs touched.** *(timeless)*
- `docs/specs/incremental_models.md` — close the "pre-cut spellings" divergence entry.

**Review checklist** (material findings only):
- [ ] Zero behaviour change (no test assertion edited except code-name strings)
- [ ] Diagnostics catalogue matches the new codes; no orphaned old names in specs or docs-site
- [ ] Source-scan guard green

**Commit.** `refactor: rename surviving pre-cut batched spellings to partition-grain vocabulary`

### Phase 5a: Dispatch change-suppressed column-scoped MERGE on the keyed run path

**Goal.** A composed clock-and-identity **keyed** model (`grain: key`) that enriches from an
`explicitly_mutable` dimension declared `allow_full_scan` maintains its dimension-driven column
group via the change-suppressed column-scoped `MERGE` **at runtime**, reaching `Suppressed` when P2
(the model's declared `unique_key` → `RowIdentity::Key`) and P3 (per-column change comparability)
both hold. This closes the runtime dispatch gap that makes the original Phase 5 unsatisfiable: the
derivation already produces the `Technique::ColumnScopedMerge` cell carrying `RowIdentity::Key` for
this shape, but `execute.rs`'s keyed branch returns before the suppression path is ever consulted.
The spec (`incremental_models.md` §"Per-cell write addressing", §"What the composed shape uniquely
enables") already describes this behaviour as live — this phase is a **Known-Divergence closure**,
not new normative surface.

**Root cause (verified 2026-07-20, see "## Blocked phases").**
- `execute.rs` `plan_is_keyed` branch (~L1471–1616) routes every `grain: key` model to
  `cumulative::execute_cumulative_aggregate` and returns (~L1610) **before** the incremental branch's
  `resolve_live_column_scoped_cell` call (`maintenance_driver.rs:769`, invoked from `execute.rs` ~L1788).
- So a keyed model's `UpstreamMutation` `ColumnScopedMerge` cell — which *does* carry
  `RowIdentity::Key` (via `ModelInputs::declared_unique_key`, `derive.rs:466`) and survives derivation
  when the mutable source is declared `allow_full_scan` (`derive_mutation`, `derive.rs:802`+, pushes the
  cell at ~L873–884) — is never dispatched, and `Suppressed` is never reached generatively.
- The partition path *does* dispatch column-scoped MERGE (tested by `technique_lowering.rs::column_scoped_merge_e2e`),
  but there P2 is structurally `RowIdentity::WholeRow` (empty `JoinContext` in `row_identity`,
  `derive.rs:41`/573), so `resolve_write_suppression` (`choice.rs:385`) forces `Unconditional`.

**Chosen reshape — Candidate (b), runtime-only.** Consult `resolve_live_column_scoped_cell` for the
keyed model's `explicitly_mutable` sources inside the keyed branch and dispatch the resolved
column-scoped `MERGE` (with its `WriteSuppression`) alongside the cumulative fold, reusing the
existing resolver and the single-owner `smelt-logical::maintenance::emit` column-scoped-merge
emitters. Explicitly **not** touching derivation (`derive.rs`) — the cell is already produced — and
**not** the alternative (Candidate a: threading `SourceFacts`/`JoinContext` into P2 for
`Grain::Partition` enrichment joins), which perturbs the single shared P2 identity project-wide and
needs a new source-level `unique_key` surface. Rejected as larger blast radius for the same one-shape goal.

**Pre-conditions.** W8 phases 1–4 done (they are). DuckDB backend only — no Spark needed for this phase.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/technique_lowering.rs` (a sibling of `column_scoped_merge_e2e`) — a keyed
  (`grain: key`) model enriching from a mutable dimension declared `allow_full_scan`: after a dimension
  mutation that genuinely changes a compared column, the dimension-driven column is column-scoped-merged
  and the **`Suppressed`** arm (`IS DISTINCT FROM`) executes; a no-change redelivery writes nothing.
  Assert the technique reached is `ColumnScopedMerge` + `Suppressed` (not the cumulative fold, not
  `Unconditional`). This is the red test — it fails today because the keyed branch returns early.

**Implementation shape.**
- `crates/smelt-runtime/src/execute.rs`, `plan_is_keyed` branch: before the unconditional return
  (~L1610), for the keyed model's `explicitly_mutable` sources, call `resolve_live_column_scoped_cell`
  exactly as the incremental branch does (~L1788) and dispatch the column-scoped `MERGE` (with its
  resolved `WriteSuppression`) when a live mutation cell resolves and the target table exists. The
  cumulative fold still owns the creation/append (`NewData`) trigger; the column-scoped merge owns the
  `UpstreamMutation` trigger — both can run in one keyed run.
- `crates/smelt-runtime/src/cumulative.rs` — a dispatch helper if cleaner, or keep dispatch in
  `execute.rs`. **No new emit code** — reuse the existing single-owner emitters (statement-parity gate).

**Critical files (allowed to touch).** `crates/smelt-runtime/src/execute.rs`,
`crates/smelt-runtime/src/cumulative.rs`, `crates/smelt-runtime/src/maintenance_driver.rs` (only if a
small shared helper is needed), the runtime test above. **Not** `derive.rs`, **not** the emit layer.

**Spec increment (pre-authorized).** `docs/specs/incremental_models.md` §Known Divergences — the entry
that scopes the live column-scoped-`MERGE` dispatch to the partition/incremental path (around "The
regular incremental run loop … dispatches into the column-scoped `MERGE` automatically … the one
currently reachable") is **narrowed**: the keyed run path now dispatches it too, so `Suppressed` is
reachable on a generatable keyed shape. Locate the exact entry with one targeted read; edit its text
only (timeless — describe behaviour, not this phase).

**Review checklist** (material findings only):
- [ ] The keyed model + mutable dim + `allow_full_scan` reaches `ColumnScopedMerge` + `Suppressed` at runtime (the e2e Suppressed assertion)
- [ ] No-change redelivery writes nothing (suppression actually suppresses)
- [ ] The creation/append fold still runs on the keyed path — standing keyed conformance legs stay green
- [ ] `statement_parity` + `technique_lowering` standing gates green (single-owner emission preserved)
- [ ] No change to `derive.rs` or the emit layer

**Commit.** `feat(runtime): dispatch change-suppressed column-scoped MERGE on the keyed run path`

### Phase 5b: Generative conformance leg for change-suppressed column-scoped MERGE

**Goal.** Close the source plan's C4 deferred item: at least one generated conformance recipe resolves `RowIdentity` to a proven grain key, so `resolve_write_suppression` genuinely admits `Suppressed` inside `maintenance_conformance`, and the suppressed-vs-full-refresh equivalence is proven generatively — not only on the hand-built `statement_parity`/`technique_lowering` fixtures. Satisfiable **on top of Phase 5a** (which makes `Suppressed` reachable at runtime for a keyed recipe); this phase adds no production code.

**Pre-conditions.** **Phase 5a done** (the runtime dispatch is what makes the suppressed arm reachable for a generated recipe). Phases 2–3 done (top-level `unique_key:` frontmatter staged by the generator).

**TDD tests to write first.**
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs` — a structural leg asserting the recipe pool contains at least one recipe whose derived plan admits `Technique::ColumnScopedMerge` with `Suppressed` resolved (guards against the leg silently degrading back to `Unconditional`-only, the exact failure mode the source plan recorded).
- `crates/smelt-cli/tests/maintenance_conformance` — the equivalence run over the new recipe family: after every step, including an unchanged-input redelivery step (the zero-write case), state equals the full-refresh oracle.

**Implementation shape.** Add a recipe (or extend the pool) whose model is the **keyed** shape Phase 5a dispatches: `grain: key` (top-level `unique_key:`) enriching from a mutable-snapshot dimension declared `allow_full_scan`, so P2 gets `RowIdentity::Key` for free and the runtime reaches the suppressed column-scoped MERGE. The redelivery step must be a genuine no-change delta so the suppression arm executes. Runs on DuckDB (and, via W9's backend seam, dual-backend where Spark is live).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/tests/maintenance_conformance/**` — recipe + gate legs only. No production code expected (Phase 5a already landed it); if admission still falls short, that is a new finding against 5a, not new scope here.

**Docs touched.** *(timeless)*
- `docs/specs/incremental_models.md` — the conformance-posture caveat that scoped the C4/E4 evidence to hand-built fixtures is narrowed: the change-suppressed column-scoped MERGE now has its generative equivalence leg.

**Review checklist** (material findings only):
- [ ] The structural leg fails if no recipe admits `Suppressed` (verified by temporarily breaking admission)
- [ ] The equivalence leg includes the zero-write redelivery step
- [ ] No production code changed in this phase

**Commit.** `test(conformance): generative suppressed-MERGE equivalence leg via keyed declared-key recipe`

### Phase 6: Recursive composed driving source in `build_forward_graph`

**Goal.** Close decision 9's unplumbed case: a `grain: key` model whose driving source is another maintained model's own composed (locality-admitted) output reaches the same admission verdict through `build_forward_graph` that `smelt explain` already reports for it — instead of silently yielding no edge.

**Pre-conditions.** None beyond the pre-flight (the A5/B1 machinery this extends is merged).

**TDD tests to write first.**
- `crates/smelt-runtime/tests/` (propagation) — real fixture: extend the `examples/timeseries` composed chain with a second `grain: key` + `timeseries:` model reading the first composed model's output as its driving ref; assert `build_forward_graph` constructs the edge at the declared granularity and the node's locality verdict matches the `smelt explain` verdict for the same model (parity assertion, not a re-derivation).
- `crates/smelt-logical/tests/maintenance_propagation_adjoint` — the adjointness law holds over the new two-composed-stage chain.

**Implementation shape.** Replicate `smelt-db::lib.rs`'s `model_source_granularities` handling at the `build_forward_graph` call site: the clocked-granularity candidate set for a keyed model's driving-source resolution includes upstream maintained models' admitted composed outputs (A5's output-as-clocked-source), not only declared `sources.*` entries; the "exactly one clocked candidate, else undecided" rule is unchanged. Pure plumbing to an existing verdict — no new admission logic, upholding the property-composition-walk rule (verdicts come from the walk; this only routes them).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/propagation.rs` — candidate plumbing.
- `examples/timeseries/**` — the second composed stage fixture (kept diagnostic-clean; `example_diagnostics`/`example_workspaces` stay green).

**Docs touched.** *(timeless)*
- `docs/specs/incremental_models.md` — if the graph-layer text records the runtime gap as a divergence, close it; otherwise no edit.

**Review checklist** (material findings only):
- [ ] Verdict parity with `smelt explain` asserted, not re-derived logic
- [ ] No key→partition projection semantics changed (B2's math untouched)
- [ ] Fixture is a real diagnostic-clean example, not a synthetic unit-only case

**Commit.** `feat(propagation): composed driving-source resolution reaches model outputs in build_forward_graph`

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Blocked phases

**2026-07-20 — Phase 5** ("Generative conformance leg for change-suppressed column-scoped MERGE"): no recipe can reach a live, suppression-capable `Technique::ColumnScopedMerge` cell with a proven/declared row key over a joined external source through the *current production pipeline*, so the phase's own "no production code expected" constraint is unsatisfiable as scoped. Two independent, exhaustive gaps, both requiring real production changes with their own admission-safety implications:
- `Grain::Partition` recipes (e.g. `MutableEnrichedRecipe`) can never get `RowIdentity::Key` for an `UpstreamMutation` cell driven by a joined external source: `ModelInputs::declared_unique_key()` (`crates/smelt-logical/src/maintenance/derive.rs:464`) returns `&[]` for `Grain::Partition` (top-level `unique_key:` only threads into `Grain::Key`), and the proven-key fallback (`row_identity_with_context`) is always called with an empty `JoinContext` for external sources in the general derivation path (`derive_maintenance_plan_impl`, `derive.rs:573`) — verified empirically, `row_identity` returns `WholeRow` for enrichment-join bodies against a mutable dimension either way. `SourceFacts::unique_key` is hardcoded to `vec![]` at its one call site (`crates/smelt-db/src/queries/maintenance.rs::source_facts:82`).
- `Grain::Key` recipes get `RowIdentity::Key` for free (even through a join) but their maintenance is never dispatched through the `ColumnScopedMerge` suppression path at runtime: `execute.rs`'s `plan_is_keyed` branch (~line 1471) routes every `grain: key` model to `cumulative::execute_cumulative_aggregate` and returns before reaching `resolve_live_column_scoped_cell` (`maintenance_driver.rs:769`); `cumulative.rs` has no handling of `UpstreamMutation`/`ColumnScopedMerge`/mutable dimensions at all.

Candidate reshapes (neither attempted — both are production changes, not test-only): (a) thread `SourceFacts`/`JoinContext` facts into row-identity derivation for external-source enrichment joins on `Grain::Partition`; (b) wire `ColumnScopedMerge` dispatch into `execute_cumulative_aggregate` for `Grain::Key` models. Filed as a new finding for human triage; Phase 5 stays `blocked` until reshaped by a human (new plan phase or a follow-up sub-plan).

**RESOLVED 2026-07-20 — reshaped into Phase 5a + 5b.** A follow-up code investigation confirmed both gaps and one additional favourable fact the original block note did not establish: derivation **already produces** a `Technique::ColumnScopedMerge` cell carrying `RowIdentity::Key` for a `Grain::Key` model enriching from a mutable dimension declared `allow_full_scan` (`derive_mutation` at `derive.rs:802`+ is grain-agnostic; the cell survives because `allow_full_scan` skips the `ScanUnbounded` refusal, and `declared_unique_key` at `derive.rs:466` supplies the key). So candidate (b) is **runtime-only** — no derivation change — and candidate (a)'s shared-P2 blast radius is avoided. Phase 5a lands the `execute.rs` keyed-branch dispatch that consumes that cell (making `Suppressed` reachable), and Phase 5b is the original generative conformance leg, now satisfiable on top of 5a. Rows 5a/5b are `pending`.

## Verification

How to confirm the spec is satisfied at the end:
- `bash .claude/scripts/verify-phase.sh`
- `rg -l 'batched:' examples/ docs-site/docs/` — empty; `rg -c 'Batched' crates/**/src` — zero (production)
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — including the structural `Suppressed`-admission leg
- `cargo test -p smelt-logical --test maintenance_propagation_adjoint --quiet` — green over the two-composed-stage chain
- `cargo test -p smelt-lsp --test example_workspaces --quiet` — green
- `/smelt:validate incremental_models` and `/smelt:validate models` — the four divergence entries this plan closes are gone; no new unexplained findings
