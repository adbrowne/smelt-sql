# Phase 10 plan — docs-site terminology sync, whole-file citation sweep, validate + timeless greps

## Objective

Close the outcome's remaining cross-cutting criteria: every `§"…"` citation in
`incremental_models.md` and `model_properties.md` resolves to a real heading in the file it
names (criterion 4's "no drift"), the docs-site incremental pages speak the redrafted spec's
frontier/ledger vocabulary (criterion 5), and `/smelt:validate incremental_models` plus the
timeless greps report clean (criterion 4). Also retires the three now-stale `keep` rows in
`phases/06-claims.md` whose gaps phases 7 and 9 actually closed, so `06-check.sh` is green
again.

## Boundaries (do not cross)

- **No behaviour change.** This phase edits `docs/specs/*.md`, `docs-site/docs/*.md`, and
  `phases/06-claims.md` only. If `/smelt:validate` reports a genuine spec-vs-code behaviour
  gap, record it as a §Known Divergences bullet with a tracking link — do not change code.
- **No section restructuring.** Phases 2–6 own the section shapes; a citation fix retargets a
  citation, it does not rename a heading (heading strings are load-bearing across ~100 sibling
  citations — outcome.md decision log, 2026-08-10).
- The `resolved_grain()` sweep and `python_bridge.rs` breakage stay under §Out of scope.

## Spec delta

No user-visible behaviour change, so no normative delta. The spec edits are citation
retargets in `docs/specs/incremental_models.md` and `docs/specs/model_properties.md`, plus
any §Known Divergences bullet a validate finding requires.

The seven currently-unresolvable citations (verified by an `rg`+heading-resolution sweep at
plan time — re-derive rather than trust these line numbers):

| file | line | citation | disposition |
|---|---|---|---|
| `incremental_models.md` | 357 (×2) | `§"Upstream model edges"` (line names `cli.md`) | find the real owning heading; retarget or qualify |
| `incremental_models.md` | 554 | `§"Two named carve-outs"` | retarget to the surviving heading |
| `incremental_models.md` | 983 | `§"The fingerprint sidecar"` | retarget to the surviving heading |
| `incremental_models.md` | 1092 | `§"Affected-key discovery"` | retarget to the surviving heading |
| `incremental_models.md` | 2358 | `§"Run pipeline parity rule (CLI ↔ LSP)"` → `architecture.md` | the real heading is `(CLI ↔ UI)`; fix the name |
| `model_properties.md` | 312 | `§"What this is"` (line names `incremental_models.md`) | retarget to the real heading |

## Tests

Red-green via `phases/10-check.sh` (new, same fixture style as `02`–`09`):

1. `orphan_refs_whole_file` — every `§"…"` in both specs resolves to a heading (substring
   match) in the file it names, defaulting to self when the line names none. Red today: 7 hits.
2. `citation_targets_are_files_that_exist` — every `.md` path a citation line names exists on
   disk (catches a retarget that renames the file instead of the heading).
3. `timeless_whole_file` — `Phase [A-Z0-9]|this phase|this outcome` absent from both spec
   bodies (excluding the Timeless-oracle boilerplate blockquote) and from
   `docs-site/docs/guide/incremental-models.md`, `reference/state.md`,
   `reference/timeseries.md`, `reference/smelt-yml.md`, `guide/materializations.md`.
4. `docs_site_frontier_terminology` — the docs-site pages that describe the ledger present it
   as a realization of the frontier: `reference/state.md` and `guide/incremental-models.md`
   each mention `frontier` alongside `reconciliation ledger`, and no docs-site page describes
   the reconciliation ledger and the merge ledger as unrelated mechanisms.
5. `docs_site_no_retired_surface` — `batched:` / `nondeterministic_columns` appear in
   docs-site only in a retirement paragraph that also names the replacement
   (`merge_key:` / `columns.<c>.contract`).
6. `prior_phase_checks` — `phases/0{2,3,4,5,6,7,8,9}-check.sh` all pass (06 is red today on
   IP-01 / IP-02 / MP-33).

## Tasks

1. Write `phases/10-check.sh` with checks 1–6; confirm it is red on 1 (7 hits) and 6 (three
   `06-check.sh` gap_claims failures).
2. Resolve each of the seven citations: locate the intended heading (`rg '^#{2,4} '` in the
   named file), retarget the citation text, never the heading.
3. Reclassify `phases/06-claims.md` rows IP-01, IP-02, MP-33 from `keep` to `drop` with a
   one-line note that phase 9 (`merge_key:` frontmatter home) and phase 7
   (`nondeterministic_columns` retirement) closed the gaps they tracked — the same treatment
   phase 8 applied to IC-21. Re-run `06-check.sh` green.
4. Sync docs-site terminology: in `reference/state.md` and `guide/incremental-models.md`,
   name the reconciliation ledger and the transactional merge write as the two realizations of
   the one frontier concept, matching `incremental_models.md` §"The frontier". Prose only — no
   new user-facing surface.
5. Sweep `docs-site/docs/` for any remaining terminology that the redraft changed (grain
   labels, typed deltas, contract lattice, plan cells/verbs) and align wording; leave correct
   `key_per_partition`-as-derived-label text alone.
6. Run `/smelt:validate incremental_models` (and `model_properties` if the command accepts
   it); triage each drift item — fix documentation-side drift here, record genuine
   behaviour gaps as §Known Divergences bullets with tracking links, and list anything
   deliberately left in the phase summary.
7. Write `phases/10-summary.md`: what the validate report said, per-item disposition, and the
   final judgement of each of the outcome's six success criteria against the phase summaries
   (this is the last row — the next planner terminates the outcome on it).

## Verification

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/10-check.sh` — all PASS.
- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/0{2,3,4,5,6,7,8,9}-check.sh` — all PASS.
- `bash .claude/scripts/verify-phase.sh` — fmt, clippy zero-warnings, full `cargo test`,
  `example_diagnostics`.
- `cargo test -p smelt-logical --test output_delta_spec` — the standing test that reads
  §Known Divergences prose (phase 6 found it load-bearing).
- `cargo test -p smelt-cli --test example_diagnostics --features smelt-cli/duckdb`.

## Commit message

`docs(incremental): sync docs-site terminology and repair whole-file spec citations`
