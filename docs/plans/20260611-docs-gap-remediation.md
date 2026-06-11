# Plan: Docs-gap remediation — close the six deferred ledger findings

**Date**: 2026-06-11
**Spec**: cross-spec — [`incremental_models.md`](../specs/incremental_models.md), [`seeds.md`](../specs/seeds.md), [`cumulative_aggregate.md`](../specs/cumulative_aggregate.md), [`models.md`](../specs/models.md), [`smelt_yml.md`](../specs/smelt_yml.md), [`diagnostics.md`](../specs/diagnostics.md), [`lsp.md`](../specs/lsp.md)
**Spec diff**: none — this plan remediates documentation gaps, not behaviour. The change description for each phase is its ledger entry in [`docs/bug-hunt/2026-05-30-findings.md`](../bug-hunt/2026-05-30-findings.md) (BUG-022, 031, 052, 058, 062, 071). All six are `Resolution: docs-gap`; code and spec semantics are already agreed.
**Tracking PR / branch**: `worktree-test_features`
**Docs**: docs-only (the docs *are* the deliverable; Phase 6 adds one coverage test so the diagnostics catalogue cannot rot again)

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read each phase's ledger entry in `docs/bug-hunt/2026-05-30-findings.md` — it is the change description. The named spec sections are the correctness oracle; do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-test_features`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- A ledger entry's claim turns out to be wrong against current code (verify the cited code path first; if behaviour changed since the sweep, update the ledger entry instead of documenting stale behaviour).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Docs phases have no red-green loop; instead every phase's review checklist carries explicit verification greps, and Phase 6 has a real failing-first coverage test.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/*.md` and `docs-site/docs/...` describe the feature as if it has always existed — no `### Phase A — …` headings, no `(Phase B)` inline labels, no plan vocabulary. The diagnostics catalogue in particular must not group codes by "meta-language Phase A–F" — group by owning feature.

---

## Context

The 2026-05-30 feature-sweep ledger closed with 70 findings fixed and six deliberately deferred as `docs-gap` (each ">one-line edit"). This plan batches them: five are prose additions/moves across specs and `docs-site/`, and one (BUG-052) is the diagnostics back-catalogue — ~195 `DiagnosticCode` variants exist in `crates/smelt-db/src/diagnostics_types.rs` but `docs/specs/diagnostics.md` documents only the silent-failures-hardening codes.

## Scope

### In scope (ledger coverage)
- BUG-062 — `incremental_models.md` §Known Divergences: add the window-function safety check as a third non-expanding site.
- BUG-031 — `seeds.md` §Type inference + `docs-site/docs/guide/seeds.md`: document the shape-valid/calendar-invalid DATE interaction and the `VARCHAR`-pin escape hatch.
- BUG-071 — `cumulative_aggregate.md` + `docs-site/docs/reference/cumulative-aggregate.md`: document the day/week-only granularity limitation.
- BUG-022 — `docs-site/docs/guide/materializations.md`: add the `test` materialization section; strike the corresponding `models.md` Known Divergence.
- BUG-058 — `docs-site/docs/reference/smelt-yml.md`: relocate the Schema Evolution Configuration section to the SQL-frontmatter docs; clarify the smelt.yml-vs-frontmatter field split.
- BUG-052 — `docs/specs/diagnostics.md`: catalogue every `DiagnosticCode` variant, with a coverage test; update `lsp.md` Known Divergences and `docs-site/docs/reference/diagnostics.md`.
- Ledger + master-plan close-out.

### Explicitly deferred
- Any behaviour change. If a documented behaviour looks wrong while writing it down, log a new ledger finding — do not fix code in this plan.
- `ModelConfig` `deny_unknown_fields` (the silent-vanish half of BUG-058's symptom) — that is a code change; raise a new ledger finding if still reproducible.
- Per-code *prose depth* in the diagnostics catalogue: one row per code (name, severity, one-line trigger) is the bar; long-form examples stay with the owning feature spec.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | d8330311 | 2026-06-11 |
| 2     | done     | 8e92002f | 2026-06-11 |
| 3     | done     | 255d6a97 | 2026-06-11 |
| 4     | done     | 9f804903 | 2026-06-11 |
| 5     | done     | 97f2ef10 | 2026-06-11 |
| 6     | done     | 1e945114 | 2026-06-11 |
| 7     | done     |        | 2026-06-11 |

## Blocked phases

Append-only log of phases the loop recorded as `blocked` and continued past. Each entry: date, phase id, reason/decision, candidate options. *(None yet.)*

---

### Phase 1: BUG-062 — third non-expanding site in incremental Known Divergences

**Goal.** `incremental_models.md` §Known Divergences enumerates the window-function safety check alongside `derive_model_source_bounds` and `compute_backbuild_plans` as sites that classify on unexpanded outer SQL.

**Pre-conditions.** Verify the claim still holds: `detect_builtin_rules` is called with stripped (unexpanded) SQL in `crates/smelt-db/src/lib.rs` and `planner.plan()` with raw `model.sql` in the CLI run path. If the pushdown/migration work since 2026-06-08 changed this, update the ledger entry instead and re-scope.

**TDD tests to write first.** n/a (prose). Verification: `rg -n "window" docs/specs/incremental_models.md` shows the new Known Divergences line naming the safety check as non-expanding.

**Implementation shape.** One bullet in §Known Divergences, behavioural wording: a non-partition-aligned `OVER` inside a `smelt.define` body is not seen by the safety check; link the tracking plan that owns expansion-before-analysis.

**Critical files (allowed to touch in this phase).**
- `docs/specs/incremental_models.md` — Known Divergences bullet.

**Docs touched.** Spec only — no user-doc change (the divergence list is spec-level).

**Review checklist** (material findings only):
- [ ] Cited code paths re-verified against current HEAD (not the 2026-06-08 line numbers)
- [ ] Wording is behavioural ("the safety check runs before function expansion"), not phase vocabulary
- [ ] No scope creep into later phases

**Commit.** `docs(incremental): record window-function safety check as a non-expanding site (BUG-062)`

### Phase 2: BUG-031 — shape-valid / calendar-invalid date interaction in seeds docs

**Goal.** `seeds.md` §Type inference and the seeds user guide explain that a column of shape-valid but calendar-invalid dates (`2025-02-30`) infers DATE, hard-fails at load per coercion semantics, and is recovered by pinning the column to `VARCHAR` in the sidecar.

**Pre-conditions.** None beyond Phase ordering being irrelevant here.

**TDD tests to write first.** n/a (prose). Verification: `rg -n "calendar" docs/specs/seeds.md docs-site/docs/guide/seeds.md` hits both files.

**Implementation shape.** Extend the existing "Feb 30 passes shape" note in `seeds.md` §Type inference with the downstream consequence and the sidecar-pin remedy; mirror a short admonition in the user guide near sidecar/type-pinning docs.

**Critical files (allowed to touch in this phase).**
- `docs/specs/seeds.md` — §Type inference note extension.
- `docs-site/docs/guide/seeds.md` — admonition with the `VARCHAR` pin example.

**Review checklist** (material findings only):
- [ ] Both inference behaviour and load behaviour described, plus the remedy
- [ ] Spec wording stays normative (this is documented behaviour, not a bug apology)
- [ ] Timeless — no plan/ledger vocabulary in spec or guide body

**Commit.** `docs(seeds): document calendar-invalid DATE inference/load interaction + sidecar remedy (BUG-031)`

### Phase 3: BUG-071 — cumulative_aggregate granularity limitation

**Goal.** `cumulative_aggregate.md` states (Surface or Constraints, plus Known Divergences) that cumulative aggregation supports `day` and `week` granularity only, with `Month`/`Quarter`/`Year` rejected at runtime; the user-facing reference page warns the same.

**Pre-conditions.** Re-verify against `crates/smelt-runtime/src/cumulative.rs` (the bail site was lines 286–290 at sweep time) — confirm which granularities are accepted today and the exact error text, and document that.

**TDD tests to write first.** n/a (prose). Verification: `rg -n "Month" docs/specs/cumulative_aggregate.md docs-site/docs/reference/cumulative-aggregate.md` hits both files.

**Implementation shape.** Add the granularity restriction where the spec defines the timeseries requirement for cumulative models; add a Known Divergences entry framing coarser granularities as not-yet-supported behaviour; add a matching note/admonition in the reference page near the partition-step description.

**Critical files (allowed to touch in this phase).**
- `docs/specs/cumulative_aggregate.md` — granularity restriction + Known Divergences entry.
- `docs-site/docs/reference/cumulative-aggregate.md` — user-facing warning.

**Review checklist** (material findings only):
- [ ] Documented set of accepted granularities matches `cumulative.rs` at HEAD, not the sweep-era snapshot
- [ ] Runtime error message quoted/paraphrased accurately
- [ ] Timeless wording; divergence framed behaviourally

**Commit.** `docs(cumulative): document day/week-only granularity limitation (BUG-071)`

### Phase 4: BUG-022 — `test` materialization in the materializations guide

**Goal.** `docs-site/docs/guide/materializations.md` documents all six materialization modes; the now-stale `models.md` Known Divergence recording the gap is struck.

**Pre-conditions.** Read `docs/specs/models.md` §Materialization modes and `docs/specs/testing.md` for the authoritative `test` semantics; read the existing testing guide section so the new section links rather than duplicates.

**TDD tests to write first.** n/a (prose). Verification: `rg -n "^## .*test|^### .*test" docs-site/docs/guide/materializations.md` shows the new section; `rg -n "materializations guide" docs/specs/models.md` no longer reports the divergence.

**Implementation shape.** New `test` section in the guide following the established per-mode shape (what it is, frontmatter example, when to use, link to the testing guide). Remove the corresponding entry from `models.md` §Known Divergences.

**Critical files (allowed to touch in this phase).**
- `docs-site/docs/guide/materializations.md` — new `test` section.
- `docs/specs/models.md` — strike the satisfied Known Divergence.

**Review checklist** (material findings only):
- [ ] Section shape matches the five existing mode sections
- [ ] Example frontmatter is valid against current parser (`materialization: test`)
- [ ] Links to the testing guide instead of restating its content
- [ ] Stale Known Divergence removed, others untouched

**Commit.** `docs(materializations): add test mode section; strike satisfied models.md divergence (BUG-022)`

### Phase 5: BUG-058 — schema-evolution config out of the smelt.yml reference, into frontmatter docs

**Goal.** The smelt.yml reference no longer presents `schema_evolution` (or other frontmatter-only fields) as smelt.yml model config; the SQL-frontmatter docs gain the schema-evolution configuration content, and the layer split (smelt.yml model fields vs SQL frontmatter fields) is stated explicitly.

**Pre-conditions.** `docs/specs/smelt_yml.md` §Model-config shape is the oracle (`materialization`, `incremental`, `tags`, `target`, `timeseries` only). The Model Fields table in the reference is already correct; the offending remnant is the §Schema Evolution Configuration section (`docs-site/docs/reference/smelt-yml.md` ~line 205). The frontmatter homes are `docs-site/docs/guide/sql-models.md` §Supported metadata fields and `docs-site/docs/guide/schema-evolution.md`.

**TDD tests to write first.** n/a (prose). Verification: `rg -n "schema_evolution" docs-site/docs/reference/smelt-yml.md` returns nothing; `rg -n "schema_evolution" docs-site/docs/guide/schema-evolution.md docs-site/docs/guide/sql-models.md` shows the relocated config docs.

**Implementation shape.** Move the §Schema Evolution Configuration YAML content into `guide/schema-evolution.md` (as frontmatter config, with a frontmatter example) and list `schema_evolution`/`format`/`columns` in `sql-models.md` §Supported metadata fields if absent. In the smelt.yml reference, add one sentence under §Model Fields noting that frontmatter-only fields (with a link) are not valid smelt.yml model config.

**Critical files (allowed to touch in this phase).**
- `docs-site/docs/reference/smelt-yml.md` — remove §Schema Evolution Configuration; add the layer-split sentence.
- `docs-site/docs/guide/schema-evolution.md` — receive the configuration content as frontmatter docs.
- `docs-site/docs/guide/sql-models.md` — §Supported metadata fields additions if missing.

**Review checklist** (material findings only):
- [ ] No frontmatter-only field is presented as smelt.yml model config anywhere on the reference page
- [ ] Relocated content shows the fields in SQL frontmatter syntax, not under `models.<name>:`
- [ ] Field lists verified against `ModelMetadata` (`crates/smelt-core/src/metadata.rs`) and `ModelConfig` at HEAD
- [ ] In-site links between the two pages updated (no orphaned anchors)

**Commit.** `docs(smelt-yml): move schema-evolution config to frontmatter docs; state the layer split (BUG-058)`

### Phase 6: BUG-052 — full diagnostics catalogue with coverage test

**Goal.** `docs/specs/diagnostics.md` catalogues every `DiagnosticCode` variant (one row per code: name, severity, owning feature spec, one-line trigger), a test fails if a variant is ever added without a catalogue entry, and `lsp.md`'s "many undocumented diagnostic codes" Known Divergence collapses to a pointer at the catalogue.

**Pre-conditions.** Phases 1–5 are independent of this one; no ordering requirement beyond the close-out phase following it.

**TDD tests to write first.**
- `crates/smelt-db/tests/diagnostics_catalogue.rs::every_diagnostic_code_is_catalogued` — parses the `DiagnosticCode` enum variants out of `crates/smelt-db/src/diagnostics_types.rs` (source-text regex, same style as the `hardening_budget` gates), reads `docs/specs/diagnostics.md` via `CARGO_MANIFEST_DIR`-relative path, and asserts every variant name appears in the catalogue. Must be red first (~190 missing), green when the catalogue lands.

**Implementation shape.** Generate the initial table mechanically from `diagnostics_types.rs` (variant name + severity where derivable), then hand-fill the one-line trigger column from each code's producer; group rows by owning feature (models, seeds, sources, timeseries, types, scoping, functions/expansion, meta-language, records/maps/loaders, python, cumulative/incremental, LSP/infra) — never by plan phase. Replace `lsp.md`'s undocumented-codes divergence bullet with a pointer to the catalogue. Update `docs-site/docs/reference/diagnostics.md` to point at (or render a user-level summary of) the catalogue. Remove the diagnostics.md Known Divergences "full back-catalogue" entry.

**Critical files (allowed to touch in this phase).**
- `docs/specs/diagnostics.md` — the catalogue.
- `crates/smelt-db/tests/diagnostics_catalogue.rs` — new coverage test.
- `docs/specs/lsp.md` — collapse the Known Divergence bullet to a pointer.
- `docs-site/docs/reference/diagnostics.md` — user-facing update.

**Review checklist** (material findings only):
- [ ] Coverage test was red before the catalogue and is green after; it reads the real enum source, not a hardcoded list
- [ ] Spot-check 10 random rows: trigger description matches the actual producer site
- [ ] Catalogue grouped by feature, zero plan-phase vocabulary (the `lsp.md` bullet's "Phase A–F" framing must not be copied in)
- [ ] `lsp.md` and `diagnostics.md` Known Divergences entries for this gap removed/replaced
- [ ] No scope creep: no diagnostic behaviour or severity changed

**Commit.** `docs(diagnostics): catalogue all DiagnosticCode variants + coverage gate (BUG-052)`

### Phase 7: Close-out — ledger and master registry

**Goal.** The six ledger entries flip `deferred` → `fixed` with dates and commit references, the ledger summary table reads 76 fixed / 0 deferred, and the master plan registry row for this sub-plan flips to `done`.

**Pre-conditions.** Phases 1–6 all `done`.

**TDD tests to write first.** n/a. Verification: `rg -c "Status\*?\*?: deferred" docs/bug-hunt/2026-05-30-findings.md` returns 0 matches among BUG-022/031/052/058/062/071.

**Implementation shape.** Per-entry `Status:` line updates in the ledger; summary-table count update; flip this plan's row in `docs/plans/20260530-feature-sweep.md` §Spawned sub-plans to `done (2026-06-…)`; mark this plan's Progress table complete.

**Critical files (allowed to touch in this phase).**
- `docs/bug-hunt/2026-05-30-findings.md` — six status lines + summary table.
- `docs/plans/20260530-feature-sweep.md` — registry row.
- `docs/plans/20260611-docs-gap-remediation.md` — progress table.

**Review checklist** (material findings only):
- [ ] All six entries cite the phase commit that resolved them
- [ ] Summary table counts re-derived, not decremented blindly

**Commit.** `chore(ledger): close BUG-022/031/052/058/062/071 — docs-gap remediation complete`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **Stale doc-comment in `crates/smelt-runtime/src/cumulative.rs` (Phase 3).** The comment on `generate_partitions` (~lines 252–255) says "Coarser granularities are passed through but produce only the start value" — stale: the current implementation bails on any granularity other than `day`/`week`. A code-comment cleanup, out of scope for this docs-only plan; raise a new ledger finding if it bites.

## Verification

How to confirm the plan is satisfied at the end:
- `cargo test -p smelt-db --test diagnostics_catalogue` — every `DiagnosticCode` variant is catalogued.
- `rg -n "Status\*?\*?: deferred" docs/bug-hunt/2026-05-30-findings.md` — no remaining deferred entries.
- `cargo test --quiet 2>&1 | tail -40` — full suite green (docs phases must not break gates).
- `/smelt:validate models`, `/smelt:validate seeds`, `/smelt:validate cumulative_aggregate`, `/smelt:validate smelt_yml`, `/smelt:validate lsp` — zero drift on the touched sections.
