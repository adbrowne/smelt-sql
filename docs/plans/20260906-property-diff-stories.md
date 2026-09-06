# Plan: Property diff — reviewer-facing stories

**Date**: 2026-09-06
**Spec**: [`docs/specs/property_diff.md`](../specs/property_diff.md)
**Spec diff**: `eaf7065f..d71ea605 -- docs/specs/property_diff.md` (commit `d71ea605`)
**Tracking PR / branch**: PR #193 (`property-diff-narration`)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/property_diff.md` — it is the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `property-diff-narration`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/`.
- Red-green TDD: failing test before any implementation.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call: fmt + clippy + tests + example_diagnostics, failures-only output) — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` (e.g., `type_inference.rs` purity).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/<slug>.md` and `docs-site/docs/...` describe the feature as if it has always existed — no `### Phase A — …` headings, no `(Phase B)` inline labels, no `[deferred to Phase E1]` callouts in spec/user-doc body. If a phase ships an incomplete surface, the *spec* records the gap under **Known Divergences** in behavioural terms (not phase terms). The plan's Progress tracking table is where "what landed when" lives.

---

## Context

Dogfooding the property-diff PR comment on this repository's own pull requests (#191, #192) showed the verdict table is not something a reviewer can act on: a two-line SQL edit yields six rows of profile-field diffs with ISO-8601 durations and JSON blobs, a widened row key yields twelve, and two direction rulings (`cell_added` always an upgrade, grain widening an upgrade) are wrong for the reviewer's purposes. The spec now folds changes into severity-ranked **stories** every surface leads with (§"Stories"), derives severity from direction so `--fail-on`, `PropertyDowngrade`, and the stories agree (§Constraints items 10–11), exposes the trigger source and partition locality a story needs on `CellVerdict` (§"The property profile" item 2), and corrects the two direction rows (§"Direction", §Design "A new dependency is a cost, not an upgrade").

## Scope

### In scope (spec coverage)
- §"The property profile" item 2: `trigger_source` and partition locality on `CellVerdict`.
- §"Direction": the `cell_added`/`cell_removed` row and the `grain` row.
- §"Stories": `Story`, severity derivation, the folding rules, duration humanisation, headline, lens title.
- §Surface "Output forms": text, JSON (`headline`, `stories`), Markdown (headline, story bullets, collapsed `Verdict table`).
- §Surface "Editor" and "Diagnostics": lens title from stories, one `PropertyDowngrade` per risk/cost story anchored by story subject.
- §Constraints items 10 and 11, including the `story_coverage` gate.
- User docs: `docs-site/docs/reference/smelt-explain.md`, `docs-site/docs/reference/cli.md`, `docs-site/docs/guide/ci.md`, `docs-site/docs/guide/editor-features.md`, `docs-site/docs/reference/diagnostics.md`.

### Explicitly deferred
- Localisation / message catalogue for story text — spec §Known Divergences.
- Adding `examples/web_analytics` to this repository's dogfood workflow — carried by PR #192's own branch, not this plan.
- Cost estimates on downgrades — spec §Future Extensions.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | b25fc2fe | 2026-09-06 |
| 2     | done     | a8559476 | 2026-09-06 |
| 3     | done     | c546129b | 2026-09-06 |
| 4     | done     | 4c0a6960 | 2026-09-06 |
| 5     | done     |        | 2026-09-06 |

---

### Phase 1: Profile fields and direction-table corrections

**Goal.** `CellVerdict` carries the trigger source and partition locality; `cell_added`, `cell_removed`, and `grain` grade per the corrected §"Direction" rows.

**Pre-conditions.** None.

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/profile.rs::tests::cell_verdict_carries_trigger_source_and_locality` — `render_cell_verdict` on a `NewData { source: "raw.devices" }` cell with `PartitionLocal::No { source, why }` yields `trigger_source == Some("raw.devices")`, `partition_local == false`, and `locality_reason == Some((source, why))`; a `Backfill` cell yields `trigger_source == None`.
- `crates/smelt-logical/src/analysis/diff.rs::tests::cell_added_not_partition_local_is_a_downgrade` — a `CellAdded` whose verdict is not partition-local on a still-maintained model grades `Downgrade`.
- `crates/smelt-logical/src/analysis/diff.rs::tests::cell_added_partition_local_is_neutral` — the partition-local case grades `Neutral`; a model going from zero cells to one still produces `maintenance_gained` as the only upgrade.
- `crates/smelt-logical/src/analysis/diff.rs::tests::cell_removed_is_a_downgrade_only_when_its_source_survives` — a removed cell whose trigger source is still read by another surviving cell grades `Downgrade`; a removed cell whose source no longer appears in any new-side cell grades `Neutral`.
- `crates/smelt-logical/src/analysis/diff.rs::tests::grain_widening_is_a_downgrade_and_narrowing_an_upgrade` — `Key([date, user])` → `Key([date, user, name])` grades `Downgrade`; the reverse grades `Upgrade`; `Key([a, b])` → `Key([a, c])` grades `Neutral`; keyed → unkeyed stays `Downgrade`. Replace the existing test that asserted the opposite.
- `crates/smelt-cli/tests/property_diff_cli.rs::new_unclocked_join_is_a_cell_added_downgrade` — real fixture: copy `examples/web_analytics` into a temp git repo, commit, add `JOIN smelt.sources.raw.devices d ON e.device_id = d.device_id` and `d.device_type` to `gold.eventstream_with_identity`; assert the JSON has a `cell_added` change with `direction == "downgrade"` whose `new.partition_local == false`, and no `upgrade` at all.
- `crates/smelt-cli/tests/property_diff_cli.rs::key_widening_join_is_a_grain_downgrade` — real fixture over `examples/timeseries`: the existing `daily_revenue` + `user_name` join edit (PR #191's shape); assert `grain` grades `downgrade`.
- `crates/smelt-cli/tests/property_profile_parity.rs` — stays green unchanged (the report renders cells from the profile, so the new fields flow through); if it needs an edit, the phase is reaching into the report, stop and ask.

**Implementation shape.**
- `profile.rs`: add `trigger_source: Option<String>` (via `cell_trigger_address`, mapping `"backfill"` back to `None`, or a dedicated match on `Trigger`), `partition_local: bool`, `locality_reason: Option<LocalityReason { source, why }>` to `CellVerdict`; populate in `render_cell_verdict` from `PlanCell.trigger` and `PlanCell.partition_local`.
- `diff.rs`: `ChangeKind::CellAdded` gains nothing (its `Box<CellVerdict>` now carries locality); `ChangeKind::CellRemoved` replaces `still_maintained: bool` with `source_survives: bool` computed in `diff_profile` from the new-side cell list (`still_maintained` remains derivable as "new side non-empty"; keep it if the `maintenance_lost` fold needs it). `direction()` rewritten for the three rows per the spec table; the grain branch flips `(false, true) => Downgrade` (widened) and `(true, false) => Upgrade` (narrowed), `(true, true) => Neutral`.
- Update every unit test in `diff.rs` and `diff_render.rs` that constructed a `CellVerdict` or asserted the old grain/`cell_added` grading.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/profile.rs` — `CellVerdict` fields, `render_cell_verdict`.
- `crates/smelt-logical/src/analysis/diff.rs` — `ChangeKind::CellRemoved`, `direction()`, `diff_profile`, tests.
- `crates/smelt-logical/src/analysis/diff_render.rs` — test fixtures only.
- `crates/smelt-runtime/src/**` — only if a `CellVerdict` constructor lives there.
- `crates/smelt-cli/tests/property_diff_cli.rs` — the two real-fixture tests.

**Docs touched.**
- `docs-site/docs/reference/smelt-explain.md` — the `--diff` direction description: a new non-partition-local cell and a widened grain are downgrades; `cell_added` never upgrades. Written as the feature description.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Spec rules from §"Direction" (three corrected rows) and §"The property profile" item 2 are satisfied
- [ ] Architectural invariants honored — profile single ownership; no renderer derives locality itself
- [ ] No scope creep into later phases (no story code)
- [ ] User docs updated to match Surface
- [ ] Spec + docs-site edits are timeless

**Commit.** `feat(property-diff): cell locality on the profile; cell_added/cell_removed and grain-widening direction corrections`

---

### Phase 2: Stories, severity, headline, lens title

**Goal.** `smelt_logical::analysis::diff_stories::narrate` folds a `ModelDiff`'s changes into `Story` values per §"Stories"; `DiffReport` carries `headline`, `ModelDiff` carries `stories`; the `story_coverage` gate holds.

**Pre-conditions.** Phase 1 (locality and trigger source on `CellVerdict`).

**TDD tests to write first.**
- `crates/smelt-logical/tests/story_coverage.rs::every_change_is_folded_by_exactly_one_story` — proptest over generated `Vec<Change>` (every `Dimension`, every direction, random subjects/sources/keys): the union of `story.changes` over the model's stories is exactly `0..changes.len()` with no duplicates.
- `crates/smelt-logical/tests/story_coverage.rs::risk_and_cost_stories_fold_downgrades_and_vice_versa` — same generator: every `risk`/`cost` story folds ≥1 downgrade; every downgrade index is in a `risk`/`cost` story; `improvement` folds ≥1 upgrade and no downgrade.
- `crates/smelt-logical/tests/story_coverage.rs::narrate_never_panics` — same generator, plus adversarial: empty change list, only `other`-bound dimensions, `source_bound` with `Unbounded`/`NotDerivable` on either side, grain with empty keys.
- `crates/smelt-logical/src/analysis/diff_stories.rs::tests::widened_window_folds_into_one_reads_story` — the PR #192 change list (two `source_bound` P1D→P7D, `column_added` + `determinism` + `comparability` for `device_type`, one non-local `cell_added` for `raw.devices`) yields exactly three stories in order: `dependency` (cost, "New dependency read in full", detail names `raw.devices` and quotes the locality reason), `reads` (cost, detail "Each run now reads 7 days either side of the run window of gold.identity_forward_only, silver.sessions (was 1 day either side of the run window)"), `schema` (info, "Adds device_type.").
- `crates/smelt-logical/src/analysis/diff_stories.rs::tests::fan_out_join_folds_grain_identity_and_fds_into_rows_may_duplicate` — the PR #191 change list (grain widened, `row_identity` Key→WholeRow, `fan_out_join` false→true, five `fd_removed`, five `fd_added`, `column_added user_name` + its determinism/comparability) yields exactly two stories: `rows_may_duplicate` (risk, detail names `(revenue_date, user_id)`) and `schema`.
- `crates/smelt-logical/src/analysis/diff_stories.rs::tests::maintenance_lost_claims_cells_and_refusals` — `maintenance_lost` + three neutral `cell_removed` + one `refusal_added` yields one `risk` story whose detail ends with ` Reason: <refusal text>.`
- `crates/smelt-logical/src/analysis/diff_stories.rs::tests::row_key_widened_without_fan_out` — grain `[date,user]`→`[date,user,name]` alone yields `row_key` risk "Row key widened".
- `crates/smelt-logical/src/analysis/diff_stories.rs::tests::technique_story_folds_state_downgrade_reason` — `cell_technique` + `state_downgrade` on the same cell yields one `cost` story with ` Reason: …`.
- `crates/smelt-logical/src/analysis/diff_stories.rs::tests::humanise_seconds` — 86400 → "1 day", 604800 → "7 days", 3600 → "1 hour", 90 → "90 seconds", 5400 → "90 minutes".
- `crates/smelt-logical/src/analysis/diff_stories.rs::tests::headline_and_lens_title` — a report with one `maintenance_lost` model, one risk-only model, one cost-only model, one improvement-only model renders `4 models shifted · 1 lost incremental maintenance · 1 with correctness risks · 1 read more per run · 1 improved`; a zero-downgrade report ends with ` · no downgrades`; lens titles `1 risk vs main`, `1 costlier vs main`, `changed vs main`.
- `crates/smelt-logical/src/analysis/diff_render.rs::tests::report_json_matches_the_spec_schema_keys` — extend: top-level keys include `headline`; each model has `stories` with keys `kind, severity, subject, lead, detail, changes`.

**Implementation shape.**
- New `crates/smelt-logical/src/analysis/diff_stories.rs`: `pub enum Severity { Risk, Cost, Improvement, Info }`, `pub enum StoryKind { … 13 variants … }`, `pub struct Story { kind, severity, subject, lead, detail, changes: Vec<usize> }`, `pub fn narrate(model: &ModelDiff) -> Vec<Story>`, `pub fn headline(report: &DiffReport) -> String`, `pub fn lens_title(model: &ModelDiff, baseline: &BaselineInfo) -> String`, `pub fn humanise_seconds(s: u64) -> String`, `pub fn severity_label(Severity) -> &'static str` (`risk`/`cost`/`improved`/`info`) and `severity_glyph` (🔴 ⚠️ 🟢 ℹ️). Rules implemented as an ordered list of claim functions over a `claimed: Vec<bool>` mask, reading `Change.kind` (the typed `ChangeKind`, never re-parsing `old`/`new` JSON). Dimension class tables (`GUARANTEE_DIMENSIONS`, `COST_DIMENSIONS`) as `const` slices; an exhaustive `match` over `Dimension` asserts every dimension is classified or explicitly always-neutral.
- `diff.rs`: `ModelDiff` gains `stories: Vec<Story>` (serialized), `DiffReport` gains `headline: String`; both populated at the end of `diff_profiles` (and re-derived after `--select` narrows the reported set — the headline counts the reported set). `Change` keeps `kind` (`#[serde(skip)]`) for the fold.
- `diff_render::lens_title` becomes a re-export of `diff_stories::lens_title` (Phase 4 rewires the LSP; keep the old signature compiling).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/diff_stories.rs` — new.
- `crates/smelt-logical/src/analysis/mod.rs` — module registration.
- `crates/smelt-logical/src/analysis/diff.rs` — `ModelDiff.stories`, `DiffReport.headline`, population in `diff_profiles`.
- `crates/smelt-logical/src/analysis/diff_render.rs` — `lens_title` delegation; JSON-keys test.
- `crates/smelt-logical/tests/story_coverage.rs` — new gate.
- `crates/smelt-logical/Cargo.toml` — `proptest` dev-dependency if absent.
- `crates/smelt-cli/src/commands/explain_diff.rs` — only if `--select` narrowing needs the headline re-derived there.

**Docs touched.**
- `docs-site/docs/reference/smelt-explain.md` — the JSON schema block gains `headline` and `stories`; a short "Stories" subsection describing severity and the story kinds in reviewer terms.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Spec rules from §"Stories" (severity derivation, rule order, sentence templates, headline, lens title) are satisfied
- [ ] Architectural invariants honored — narration single ownership (§Constraints item 10): no SQL knowledge, reads `ChangeKind` not JSON
- [ ] No scope creep into later phases (text/Markdown/LSP surfaces untouched)
- [ ] User docs updated to match Surface
- [ ] Spec + docs-site edits are timeless

**Commit.** `feat(property-diff): stories — severity-ranked narration, headline, lens title, story_coverage gate`

---

### Phase 3: Text and Markdown forms render from stories

**Goal.** `smelt explain --diff` text and `--markdown` output match §Surface "Output forms": stories first, verdicts under, headline last; the Markdown comment leads with the headline and story bullets and collapses the verdict table.

**Pre-conditions.** Phase 2.

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/diff_render.rs::tests::text_block_lists_stories_then_verdicts` — a model with one risk and one info story renders `[risk] <lead>: <detail>` then `[info] …` then an indented `verdicts:` line followed by the `▼`/`●` change lines; the report's last line is the headline, not `N downgrades, …`.
- `crates/smelt-logical/src/analysis/diff_render.rs::tests::markdown_leads_with_headline_and_story_bullets` — heading `### property diff vs main @ abc1234`, bold headline on the next line, `**staging.orders** (edited)`, `- 🔴 **Rows may be duplicated.** …`, then `<details>\n<summary>Verdict table</summary>` (never `<details open>`), the six-column table, marker last.
- `crates/smelt-logical/src/analysis/diff_render.rs::tests::markdown_values_match_the_text_form` — extend: story lead/detail strings in Markdown are byte-equal to the text form's.
- `crates/smelt-logical/src/analysis/diff_render.rs::tests::markdown_body_of_a_large_diff_stays_under_the_comment_limit` — still holds with story bullets.
- `crates/smelt-cli/tests/property_diff_cli.rs::markdown_comment_for_the_web_analytics_edit_reads_as_stories` — real fixture (Phase 1's `web_analytics` temp repo): `--markdown` output contains `⚠️ **New dependency read in full.** raw.devices`, `⚠️ **Reads more per run.**`, `ℹ️ **Schema.** Adds device_type.`, exactly one `<details>` per model, and contains no `P7D`.
- `crates/smelt-cli/tests/property_diff_cli.rs` — update the existing text/Markdown assertions for the new shape.
- `crates/smelt-cli/tests/property_diff_ci_docs.rs` — still green (marker literal unchanged).

**Implementation shape.**
- `diff_render.rs`: `model_block` emits story lines via `severity_label` + `lead` + `: ` + `detail`, then `verdicts:` and the existing `change_line`s; `text_report` ends with `report.headline`; `markdown_model_block` emits the bold name/cause line, one bullet per story via `severity_glyph`, then a collapsed `<details><summary>Verdict table</summary>` around the existing table; `markdown_report` emits the new heading and bold headline. Remove `<details open>` logic and the `N downgrades, M upgrades, K neutral` summary strings.
- `explain_diff.rs`: no logic change beyond consuming the renderer.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/diff_render.rs`
- `crates/smelt-cli/src/commands/explain_diff.rs`
- `crates/smelt-cli/tests/property_diff_cli.rs`

**Docs touched.**
- `docs-site/docs/reference/smelt-explain.md` — replace the text and Markdown samples with the story-led output (use the `user_daily_spend` example already on the page).
- `docs-site/docs/reference/cli.md` — the `explain --diff` output description.
- `docs-site/docs/guide/ci.md` — the sample comment body.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Spec rules from §Surface "Output forms" (text, Markdown) are satisfied
- [ ] Architectural invariants honored — surface parity: every Markdown value comes from the shared primitives
- [ ] No scope creep into later phases (LSP untouched)
- [ ] User docs updated to match Surface
- [ ] Spec + docs-site edits are timeless

**Commit.** `feat(property-diff): text and Markdown forms lead with stories; verdict table collapsed`

---

### Phase 4: Editor lens and diagnostics from stories

**Goal.** The code lens title is the story-derived lens title; one `PropertyDowngrade` per risk/cost story, anchored by the story's subject; the `property_diff_parity` gate asserts both against the CLI JSON's `stories`.

**Pre-conditions.** Phase 3.

**TDD tests to write first.**
- `crates/smelt-lsp/src/property_diff.rs::tests::one_diagnostic_per_risk_or_cost_story` — a `ModelDiff` with one risk story folding three downgrades yields exactly one `PropertyDowngrade` whose message is `<lead>: <detail>`.
- `crates/smelt-lsp/src/property_diff.rs::tests::story_subject_anchors_column_source_or_first_token` — a `schema`-kind story is not a diagnostic; a `column_semantics` story with a column subject anchors at the SELECT item; a `reads` story whose subject is a source anchors at the `FROM`/`JOIN` item; a `rows_may_duplicate` story (empty subject) anchors at the first SQL token.
- `crates/smelt-lsp/tests/property_diff_parity.rs` — update: lens title equals `diff_stories::lens_title` computed from the CLI JSON's `stories`; the `PropertyDowngrade` set equals the set of `risk`/`cost` stories per model.
- `crates/smelt-lsp/tests/property_diff_refresh.rs` / `property_diff_overlay.rs` — update any assertion pinned to `N downgrades, M upgrades vs`.

**Implementation shape.**
- `smelt-lsp/src/property_diff.rs`: `diagnostics_for_model` iterates `model.stories` filtered to `Risk | Cost`; `anchor_for` takes `(StoryKind, subject)` — column-subject kinds (`column_semantics`, `schema` is never a diagnostic), source-subject kinds (`reads`, `dependency`), everything else first token; `lens_title_for` calls `diff_stories::lens_title`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-lsp/src/property_diff.rs`
- `crates/smelt-lsp/tests/property_diff_parity.rs`, `property_diff_refresh.rs`, `property_diff_overlay.rs`, `property_diff_coalescing.rs` (assertion updates only)

**Docs touched.**
- `docs-site/docs/guide/editor-features.md` — lens text sample (`1 risk, 1 costlier vs main`), one warning per risk/cost story, hover shows stories then verdicts.
- `docs-site/docs/reference/diagnostics.md` — `PropertyDowngrade` entry: one per risk/cost story, message shape.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Spec rules from §Surface "Editor" and "Diagnostics" are satisfied
- [ ] Architectural invariants honored — surface parity (§Constraints item 5): the LSP renders `Story` values, composes nothing
- [ ] No scope creep into later phases
- [ ] User docs updated to match Surface
- [ ] Spec + docs-site edits are timeless

**Commit.** `feat(property-diff): editor lens and PropertyDowngrade render from stories`

---

### Phase 5: Closure — validate, ROADMAP, outcome cross-reference

**Goal.** `/smelt:validate property_diff` reports zero drift; ROADMAP records completion; the property-diff outcome's decision log notes the two direction reversals.

**Pre-conditions.** Phases 1–4.

**TDD tests to write first.**
- None new; the full gate set runs: `verify-phase.sh`, `cargo test -p smelt-logical --test story_coverage`, `cargo test -p smelt-cli --test property_profile_parity`, `cargo test -p smelt-lsp --test property_diff_parity`, `cargo test -p smelt-cli --test property_diff_ci_docs`, `cargo test -p smelt-cli --test cli_docs_coverage`.

**Implementation shape.**
- Run `/smelt:validate property_diff`; fix any drift in code or docs (not by weakening the spec).
- `docs/ROADMAP.md`: entry dated September 6, 2026.
- `docs/outcomes/20260905-property-diff/outcome.md` Decision log: one dated entry per direction reversal (`cell_added`, grain widening) pointing at the spec §Design paragraphs.

**Critical files (allowed to touch in this phase).**
- `docs/ROADMAP.md`, `docs/outcomes/20260905-property-diff/outcome.md`, and whatever `/smelt:validate` names.

**Docs touched.**
- As named by the drift report only.

**Review checklist** (material findings only):
- [ ] `/smelt:validate property_diff` zero drift
- [ ] All standing gates green
- [ ] Spec + docs-site edits are timeless

**Commit.** `docs(property-diff): ROADMAP + outcome decision log for the stories surface`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- 2026-09-06 (Phase 1): `crates/smelt-runtime/src/python.rs` test module has an intermittent isolation flake — concurrent Python-model tests (`python_discovery_runs_in_runtime`, `python_multimodel_delimiter_not_a_section`, `python_name_mismatch_blocks_and_retains_other_keys`) collide on a shared tmp path and read each other's `unstable.py` counter. Reproduces on `main`; unrelated to this plan. Fix separately.
- 2026-09-06 (Phase 5): `crates/smelt-cli/tests/property_diff_cli.rs::a_join_induced_downgrade_propagates_to_the_named_downstream_model` failed once under parallel execution during validation and passed on rerun — same test-isolation class; the temp-repo fixtures may share state. Investigate separately.
- 2026-09-06 (Phase 5): lens hover is unimplemented (pre-existing; now recorded in the spec's Known Divergences and the editor guide).

## Verification

How to confirm the spec is satisfied at the end:
- On this branch with PR #192's `web_analytics` edit applied to a scratch copy: `smelt explain --diff main --markdown --project-dir examples/web_analytics` prints the three-story block from spec §Design ("New dependency read in full", "Reads more per run", "Schema"), no `P7D`, one collapsed `Verdict table`.
- With PR #191's `daily_revenue` edit: `smelt explain --diff main --project-dir examples/timeseries` prints `[risk] Rows may be duplicated: …` and `[info] Schema: …` and nothing else for that model.
- `cargo test -p smelt-logical --test story_coverage`
- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-lsp --test property_diff_parity`
- `/smelt:validate property_diff` reports zero drift
