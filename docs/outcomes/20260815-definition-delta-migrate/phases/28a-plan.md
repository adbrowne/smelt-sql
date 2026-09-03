# Phase 28a — record the taken decisions; close the docs-site CLI-surface audit

## Objective

Land the two already-taken-but-not-fully-recorded decisions from
`docs/research/20260816-open-questions-triage.md` (items 14 and 16) in their owning specs, and
close the "docs-site coverage of the plan's CLI surface is partial" Known Divergence by turning
the one-time audit into a **standing checklist gate** rather than a re-audit. Advances success
criteria 18 (decidable Open Questions recorded in the owning spec) and 20 (the bullets are
actually removed, not just addressed). Doc + test only — no production behaviour changes.

## Spec delta

No user-visible behaviour changes; the edits are recording/cleanup and come first.

1. `docs/specs/incremental_models.md`, §"Observed output delta" (the trust-boundary sentence,
   ~line 1598): the clause "there is no out-of-band-edit tripwire … (an explicit Open Question,
   §Known Divergences)" is stale — the decision is already recorded under §Non-goals ("No
   out-of-band-edit detection … a stated non-goal, not an open question"). Re-point the
   cross-reference at the non-goal and drop the "Open Question" wording.
2. `docs/specs/definition_deltas.md`, §Design: add one short paragraph recording that a per-model
   `on_column_add: backfill | leave_null | recompute` policy knob was **considered and dropped**
   — `smelt migrate`'s per-column-group verdict (`PureBackfill` / `UpstreamRederive` /
   `SkeletonAdd`) already answers "what happens when a column is added" case-by-case, and a
   standalone knob would be a second, drifting answer. (Rationale-in-spec rule; no live spec or
   docs-site page mentions the knob today — verify before writing, and record the rejection so
   the question stops reopening.)
3. `docs/specs/incremental_models.md` §Known Divergences: delete the bullet "**docs-site coverage
   of the plan's CLI surface is partial**" once the gate below is green and the residue is
   documented or explicitly dropped.
4. `docs-site/docs/reference/cli.md`: document each residue item the audit finds worth
   documenting (expected: maintenance-adjacent flags on `smelt run`/`explain`/`rebuild` that the
   page does not name today).

## Tests

New test file `crates/smelt-cli/tests/cli_docs_coverage.rs`:

- `every_command_is_documented` — walk `Cli::command()`'s subcommands (recursively, including
  `docs <sub>`) and assert each has a `## smelt <name>` (or `### `) heading in
  `docs-site/docs/reference/cli.md`; a command may instead appear in an explicit
  `UNDOCUMENTED_BY_DESIGN` allowlist with a one-line reason.
- `every_long_flag_is_documented` — for each subcommand, every long flag (`--foo`) must appear
  verbatim in `cli.md`, or be listed in the same allowlist. This is the checklist: the residue is
  enumerated once, in code, and a newly-added undocumented flag fails.
- `allowlist_has_no_stale_entries` — two-sided: an allowlist entry naming a command/flag that no
  longer exists, or that *is* now documented, fails and tells the reader to delete the entry
  (same two-sided discipline as the hardening baseline).

Red first: run the two coverage tests with an empty allowlist to produce the actual residue list,
then either document each item in `cli.md` or move it to the allowlist with its reason.

## Tasks

1. Add `crates/smelt-cli/tests/cli_docs_coverage.rs` with the three tests and an empty
   `UNDOCUMENTED_BY_DESIGN` allowlist; resolve `cli.md` via `CARGO_MANIFEST_DIR/../../docs-site`.
2. Run it red; capture the enumerated residue (commands + long flags absent from `cli.md`).
3. For each residue item, decide document-or-drop: document in `docs-site/docs/reference/cli.md`
   under the owning `## smelt <cmd>` section; drop → allowlist entry with a one-line reason
   (dev/internal-only flags, aliases, `--help`/`--version`-style clap builtins).
4. Make the two coverage tests green; confirm `allowlist_has_no_stale_entries` passes.
5. Apply spec edits 1 and 2; verify `on_column_add` appears in no live spec or docs-site page
   (`rg` over `docs/specs` + `docs-site/docs`) before writing the rejection paragraph.
6. Delete the docs-site-CLI-coverage bullet from `incremental_models.md` §Known Divergences.
7. Bump `last_reviewed` on both edited specs if the file header carries one.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test cli_docs_coverage`
- `rg -n "on_column_add|out-of-band" docs/specs docs-site/docs` — no surviving "(Open Question)"
  framing for either item.
- Spot-check the mkdocs build is unaffected (markdown-only edits; no nav change expected).

## Commit message

`docs(cli): record the out-of-band and on_column_add decisions; gate docs-site CLI coverage`
