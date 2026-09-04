# Phase 02 plan — Re-point or delete every stale citation on `docs/TODO.md`'s list

## Objective

Close success criterion 3. Every `§"…"` citation named in `docs/TODO.md` §"Stale citations
flagged by the sweep" must resolve to a heading that exists today, or be deleted; then the TODO
bullet itself goes. Sites are spread across two specs and five Rust files — the Rust edits are
**doc comments only**, no behaviour change, consistent with the outcome's docs-only rule.

## Spec delta

None — no user-visible behaviour changes. The spec edits here are citation targets inside
`docs/specs/materialized_view.md`, `docs/specs/run_state.md`, `docs/specs/timeseries.md` and
`docs/specs/models.md`; each replaces a dangling `§"…"` anchor with the heading that now carries
the claim (or drops it). Nothing normative is added or removed.

## The site inventory (verified 2026-09-04; the TODO's line numbers are stale, the heading text is not)

| # | Site | Dangling anchor | Candidate target (verify before using) |
|---|------|-----------------|----------------------------------------|
| 1 | `docs/specs/materialized_view.md:20`, `:123` | `incremental_models.md` §"The composition contract" | `incremental_models.md` §"Shape profiles" (line 1687) |
| 2 | `docs/specs/run_state.md:139` | `incremental_models.md` §"Failure mode" | `incremental_shapes.md` §"First-run and backfill" (carries "halts at the first failed chunk", line 483) |
| 3 | `docs/specs/timeseries.md:119` | `incremental_models.md` §"Granularity values" | none — the enum is owned by `timeseries.md` itself; drop the circular parenthetical |
| 4 | `docs/specs/models.md:252`, `crates/smelt-core/src/metadata.rs:1232`, `crates/smelt-logical/src/rules/incremental.rs:457` | `incremental_models.md` §"Non-determinism and the payload rule" | `incremental_shapes.md` §"Batch safety classification" / §"Partition-grain declaration (`grain: partition`)" — whichever states the `contract: plausible` rule the sentence relies on (lines 149–152, 519–530) |
| 5 | `crates/smelt-cli/tests/maintenance_conformance/gate.rs:5189`, `:5400` | `incremental_models.md` §"Per-slice…" | `incremental_shapes.md` §"Key temporal locality (the time-partitioned output)" (line 830/872) |
| 6 | `crates/smelt-logical/src/maintenance/propagate.rs:411` | `incremental_models.md` §"Row movement" | `incremental_shapes.md` §"Key temporal locality…" — copy the form `locality.rs:1138` already uses |
| 7 | `crates/smelt-runtime/src/propagation.rs:1723` | `incremental_models.md` §"The clamp both directions" | `incremental_models.md` §"Windowed maintenance and the horizon" (line 1224) |
| 8 | `crates/smelt-core/tests/refresh_axis.rs:451` | `incremental_models.md` §"The declared shape axis" | `incremental_models.md` §"The declared shape" (line 252) |

**Rule when no target fits:** delete the `§"…"` anchor. If the sentence exists only to carry the
citation, delete the sentence. Never invent a heading, never leave a `§` naming a heading that no
`rg` hit confirms. Do **not** edit `docs/plans/` or `docs/research/` — they are historical records
(they cite these headings too; that is correct and out of scope).

## Tests

1. `citation_sweep_clean` (shell, not a cargo test) — over `git ls-files docs/specs crates`,
   extract every `` `<file>.md` §"<heading>" `` citation and assert each `<heading>` matches an
   `^#{1,6} ` line in that spec. Must be clean for the eight sites above; pre-existing unrelated
   failures elsewhere get listed in the summary, not fixed.
2. `no_dangling_anchors_in_touched_files` — re-run test 1 restricted to the files this phase
   edits; zero output.
3. `todo_bullet_removed` — `rg -n 'Stale citations flagged by the sweep' docs/TODO.md` exits 1.
4. `timeless_oracle_holds` — `rg -n 'Phase [A-Z0-9]' docs/specs/materialized_view.md
   docs/specs/run_state.md docs/specs/timeseries.md docs/specs/models.md` exits 1.
5. `verify_phase_green` — full gate (the Rust files are compiled/linted even for comment edits).

## Tasks

1. Write the sweep from test 1 as a throwaway shell one-liner; capture its baseline output so the
   summary can distinguish sites this phase fixed from pre-existing noise elsewhere.
2. For each of the eight sites: `rg` the candidate heading in its target spec, read the citing
   sentence, confirm the target actually carries the claim the sentence leans on, then edit.
3. Where no heading carries the claim (expected for site 3, possible for sites 2 and 7), delete
   the anchor per the rule above and note the deletion in the summary.
4. Delete the "Stale citations flagged by the sweep" bullet from `docs/TODO.md` (lines 27–33),
   leaving the surrounding bullets intact.
5. Re-run tests 1–4; then `cargo fmt --all` and the full gate.
6. Write `phases/02-summary.md`: per-site table of {anchor → resolution}, any site where the
   claim's home turned out to be a *behaviour* gap rather than a citation gap (record it as a new
   `docs/TODO.md` bullet per the outcome's Out-of-scope rule, do not fix it), and the residual
   sweep output for citations outside this phase's list.

## Verification

- `bash .claude/scripts/verify-phase.sh` → `VERIFY: ALL GREEN`
- Tests 1–4 above, output pasted into the summary.

## Commit message

`docs(programme-hygiene): re-point or drop every stale spec-heading citation on the TODO list`
