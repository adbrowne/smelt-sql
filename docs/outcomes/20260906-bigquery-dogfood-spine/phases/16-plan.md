# Phase 16 plan — extend the findings handoff with the live-BigQuery half

**Advances:** criterion 8 (evidence banked), completing it. It is the last workable row:
criteria 5, 6 and 7 stay unmet behind blocked phases 12-14, and this phase's job is to
make *that* legible to the three downstream outcomes rather than to close it.

**Not human-gated any more.** The row was written when no live run had happened. Phases 10
and 11 executed live and their summaries carry verbatim job output, costs, error strings
and code references — this phase is a pure harvest of committed summaries, exactly as phase
15 was for the DuckDB half. No credential, no warehouse, no `gcloud` call is needed, and
none may be made.

## Spec delta

None. No user-visible feature behaviour changes; this is a handoff document plus its
string-level drift gates. (`Docs: code-only` does not apply — the doc *is* the deliverable.)

## Tests

All in `crates/smelt-cli/tests/github_activity_oracle.rs`, alongside phase 15's tests 6-8,
string-level over the `include_str!`'d handoff — no DuckDB, no new test target.

1. `findings_handoff_declares_both_halves_landed` — **replaces test 8**
   (`findings_handoff_declares_its_interim_status`). Asserts the handoff no longer carries
   the `DuckDB half only` interim marker and does declare the live-BigQuery half landed.
   RED against today's committed doc.
2. `live_findings_each_name_a_provoking_model_and_statement` — parses the new
   `## Live-BigQuery findings` table (its own scoped scan, mirroring
   `claimed_relations_in`'s section-bounded shape) and asserts every row's
   provoking-model and provoking-statement cells are non-empty. This is criterion 8's
   literal wording ("each with the model and statement that provoked it") made checkable.
3. `live_findings_scan_is_scoped_and_non_vacuous` — two synthetic documents, per phases
   10's pattern: a backtick row outside the live-findings section is not collected, and a
   row inside it is. Keeps test 2 from silently passing on an empty scan.
4. `findings_handoff_records_the_blocked_live_phases` — asserts the handoff names phases
   12, 13 and 14 as blocked and names the T5 observed-delta gap as their gate, so a reader
   cannot mistake "the live half landed" for "the live run completed".
5. `every_registry_entry_is_named_in_the_findings_handoff`,
   `findings_handoff_names_no_unknown_relation`,
   `the_handoff_scan_is_scoped_to_the_divergence_section`,
   `the_handoff_scan_still_catches_a_stale_claim_in_its_own_section` — existing, must stay
   green unchanged after the new sections are appended (the scoped scan must not read the
   new tables as divergence claims).

## Tasks

1. Rewrite the handoff's title and `**Status:**` block: title drops "— DuckDB half";
   status becomes complete, naming both halves and dating the live addendum 2026-09-10.
   Extend the "Source of every claim" line to include `phases/1{0,1}-summary.md`.
2. Add `## The live BigQuery half (2026-09-10)` — what was provisioned and loaded (phase
   10: `smelt_dogfood`, day-partitioned `github_events`/`github_events_arrival`, retention
   read back from table metadata, redelivery slice present), what ran (phase 11: the
   `bigquery` target, the target-aware `name:` overrides, the exact
   `smelt run --target bigquery --full-refresh --start … --end …` invocation), and how far
   it got — `bronze.events` plus the first write of three `silver/` models, then a hard
   stop; everything under `gold/` and `marts/` never executed.
3. Add `## Live-BigQuery findings` as a table with columns
   `| finding | provoking model | provoking statement | classification | owner |`, one row
   per finding harvested verbatim from `phases/10-summary.md` §Findings and
   `phases/11-summary.md` §Findings: T5 observed-delta recording (`silver.events_deduped`,
   second-batch suppressed keyed-fold merge, `driver.rs`'s `!= SqlDialect::DuckDB` bail);
   `external_step:` target-blindness (`sources.raw.github_loader`); no target-scoped
   non-refusing preview; alphabetical `default_target` silently re-pointing the project;
   `--emit-ddl`'s silent `payload` omission; loader load-order untested across two
   invocations. Record phase 10's `raw.` → dataset mapping finding as **closed by phase
   11** (the shipped per-target `name:` override), not as open work.
4. Add `## Recorded, not BigQuery findings` — `SourceRetentionExceeded` on a repeated
   `--full-refresh` over already-materialised output (backend-agnostic, working as
   specced), and "no run report exists because the project is `state.mode: stateless`".
5. Add `## Operational notes for the next live run` — build with
   `cargo build -p smelt-cli --features bigquery`; `scripts/bigquery-venv.sh`;
   `source scripts/bq-dogfood-env.sh` **plus** an explicitly minted
   `SMELT_BQ_ACCESS_TOKEN=$(gcloud auth application-default print-access-token)` because
   the adapter never falls back to ADC itself; measured cost (loader ≈ $0.018/run, the
   whole live model run ≈ $0.0005); and the live-state caveat (six tables left in
   `smelt_dogfood`, so the next `--full-refresh` refuses on retention until they are
   dropped or a run completes in one pass).
6. Add `## Final punch-list` — the ordered list `20260906-bigquery-correctness` consumes,
   each item naming its owner and, for the T5 item, that it is what unblocks this
   outcome's phases 12-14. Do not restate the already-`Done` DuckDB punch-list items;
   reference them.
7. Update the `FINDINGS_HANDOFF` doc comment in `github_activity_oracle.rs` (it currently
   says "interim ... banked by phase 15") and implement tests 1-4.
8. Append a dated decision-log pointer to `docs/outcomes/20260906-bigquery-correctness`'s
   `outcome.md` naming the completed handoff and the T5 item as its primary input;
   likewise a one-liner to `20260906-trimmed-history-sources` only if the live half
   changed anything it consumes (the retention bound was read back live — say so).
9. Update `examples/github_activity/README.md:257-260` — drop the "interim / DuckDB-half"
   framing, point at the complete handoff.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be ALL GREEN.
- `cargo test -p smelt-cli --test github_activity_oracle` — the four new/changed tests plus
  the four existing handoff gates.
- `bash .claude/scripts/large-file-check.sh` — OK.
- Manual: every number, error string and file:line in the new sections traced to
  `phases/10-summary.md` or `phases/11-summary.md`. Nothing re-derived, nothing measured
  here, no live call made.

## Commit message

`outcome(bigquery-dogfood-spine): phase 16 — bank the live-BigQuery findings`
