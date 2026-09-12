# Phase 16 plan — extend the findings handoff with the live-BigQuery half

**Advances:** criterion 8 (evidence banked), completing it — and with it the outcome.

**Supersedes the 2026-09-10 version of this plan**, which was written when phases 12–14 were
blocked on the T5 gap and the only live evidence was phases 10 and 11. All of that has since
happened: phases 12, 17, 13 and 14 ran, and every one of criteria 5, 6 and 7 is now met.
The handoff must therefore bank a *completed* live half, not a blocked one, and the old
plan's tasks 2–6 (which describe phases 12–14 as blocked, and a six-table stale-state
caveat) are void. Do not follow them.

**Not human-gated.** A pure harvest of committed summaries — no credential, no warehouse, no
`gcloud` call is needed and none may be made. Every number traces to a committed file.

## Spec delta

None. The doc is the deliverable, plus its string-level drift gates.

## What there now is to harvest

| source | what it carries |
|---|---|
| `phases/10-summary.md` | loader deployed; two days loaded; cost/run ≈ $0.018 |
| `phases/11-summary.md` | first live run; the T5 hard stop; `external_step:` target-blindness; `--emit-ddl`'s silent `payload` omission; alphabetical `default_target` |
| `phases/12-summary.md` | FR + three windows, 10 of 16 models; the probe-dispatch defects; the two emitted-SQL defects; the degradation contract being invisible at run time |
| `phases/17-summary.md` | the population widened to the fixture's 30 days; **`githubarchive` has not drifted from the committed fixture** (id sets, per-day counts, full-`payload` checksums, all 28 redelivery slices, zero differences); 90.5 GB / US$0.45 |
| `phases/13-summary.md` | thirty windows on both targets; 14/14 relations byte-equal at the final window; the `bronze_events` arrival-order divergence and its `--preload-source` attribution; the `stage_workspace` `.smelt/` leakage bug; the concurrent-rebuild hazard; US$0.29 |
| `phases/14-summary.md` | the BigQuery oracle leg; 14/14 byte-equal at w30; **a full refresh on BigQuery is not window-bounded against a static source** for six relations; the dataset-creation grant boundary; US$0.06 |

## The findings that are new since the old plan, and must not be lost

These are the phase's real product. Each needs a row naming the provoking model or mechanism,
the evidence, and the owner:

1. **`--event-time-end` does not bound a full refresh's source scans** (phase 14). Six
   relations' oracle refreshes over inputs the incremental leg had not seen; invisible on
   DuckDB, whose oracle stages a truncated source. Owner: `20260906-bigquery-correctness`.
   This is the single most consequential finding in the phase, because it means "full
   refresh" and "the oracle at window *k*" are not the same operation on a static source.
2. **Cost is jobs, not rows** — a model writing 2 rows costs 33 s (`silver.issue_events`,
   `deleteinsert`) while another writing 2 rows costs 5.5 s (`marts.star_growth`,
   `full_refresh`); ≈5.1 jobs/model/run and a ~5 s per-job floor derived from the 30 run
   reports. Record the derivation and that it is *derived*, not measured against
   `INFORMATION_SCHEMA.JOBS` (the dogfood SA lacks `bigquery.jobs.list`).
3. **The shared `_smelt_ledger` serialises every model's bookkeeping** — filed as
   [#203](https://github.com/adbrowne/smelt-sql/issues/203) with the evidence that every
   ledger access is already model-scoped. Reference the issue; do not restate its argument.
4. **The run window need not match partition granularity** (`incremental_shapes.md:555`),
   so the thirty daily windows were a schedule choice, not a requirement — with the caveat
   that the saving is batch-safety-class-dependent. Record it as an operational note.
5. **`stage_workspace` copied the gitignored `.smelt/`** into staged workspaces, so a staged
   run inherited a developer's local posture baseline. Reproduces nowhere in CI. Fixed in
   phase 13 — record as *closed*, with the mechanism, because the class of bug recurs.
6. **A concurrent `cargo test` rebuilt `target/debug/smelt` without `--features bigquery`**
   mid-run and killed a live leg at window 3. Operational note for any future live run.
7. **Dataset creation is outside the dogfood SA's grant** (phase 14) — deliberate, from
   phase 7. Record the boundary and the script that works within it.

## Tasks

1. Rewrite the handoff's title and `**Status:**`: drop "DuckDB half"/interim framing; state
   that both halves have landed and date the live addendum 2026-09-12. Extend the "Source of
   every claim" line to every summary in the table above.
2. `## The live BigQuery half` — what was provisioned, loaded, widened and run, in phase
   order, with the costs. State plainly that criteria 5, 6 and 7 are met **and what they
   rest on**: 14 of 16 models (two refused at compile time on GoogleSQL), 7 of 30 oracle
   checkpoints on BigQuery against all 30 windows on DuckDB.
3. `## Live-BigQuery findings` — a table `| finding | provoking model | provoking statement |
   classification | owner |`, one row per finding harvested verbatim from the summaries,
   including the seven above. Findings phases 13–16 of `20260906-bigquery-correctness`
   already closed are recorded as **closed**, not as open work.
4. `## Recorded, not BigQuery findings` — keep the backend-agnostic items.
5. `## Operational notes for the next live run` — build and credential recipe (note that ADC
   is already an impersonated `smelt-dogfood@` credential, so the explicit
   `--impersonate-service-account` flag is redundant), measured costs, the
   **2026-09-19 partition-expiration deadline**, findings 4 and 6 above, and the
   `PARITY_CHECKPOINTS` / `PARITY_RESUME_FROM` levers.
6. `## Final punch-list` — the ordered list `20260906-bigquery-correctness` consumes, each
   item naming its owner. Reference, do not restate, the already-closed DuckDB items.
7. Update the `FINDINGS_HANDOFF` doc comment in `github_activity_oracle.rs` and implement
   the tests below.
8. Append a dated decision-log pointer to `20260906-bigquery-correctness`'s `outcome.md`
   naming the completed handoff and finding 1 as its primary input; a one-liner to
   `20260906-trimmed-history-sources` only if the live half changed what it consumes (the
   retention bound *was* read back live — say so).
9. Update `examples/github_activity/README.md`'s interim framing.
10. Flip the phase-16 row and the outcome's `**Status:**` to `done`, with a closing
    decision-log entry stating which criteria are met and on what evidence, and naming
    honestly what is *not* covered (the two compile-refused models; 7 of 30 checkpoints; no
    CI tier, by standing decision).

## Tests

All in `crates/smelt-cli/tests/github_activity_oracle.rs`, string-level over the
`include_str!`'d handoff — no DuckDB, no new test target.

1. `findings_handoff_declares_both_halves_landed` — replaces
   `findings_handoff_declares_its_interim_status`; RED against today's committed doc.
2. `live_findings_each_name_a_provoking_model_and_statement` — section-scoped scan over the
   new table; every row's provoking-model and provoking-statement cells non-empty.
3. `live_findings_scan_is_scoped_and_non_vacuous` — two synthetic documents: a row outside
   the section is not collected, one inside it is.
4. `findings_handoff_records_what_the_criteria_rest_on` — **replaces** the old plan's
   `findings_handoff_records_the_blocked_live_phases`, which is now false. Asserts the
   handoff names the 14-of-16 model coverage and the 7-of-30 checkpoint coverage, so a
   reader cannot mistake "criteria met" for "everything covered".
5. The four existing handoff gates stay green unchanged after the new sections are appended
   (the scoped divergence scan must not read the new tables as divergence claims).

## Verification

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN.
- `cargo test -p smelt-cli --test github_activity_oracle` — new and existing gates.
- `bash .claude/scripts/large-file-check.sh` — OK.
- Manual: every number, error string and `file:line` traced to a committed summary. Nothing
  re-derived, nothing measured here, no live call made.

## Commit message

`outcome(bigquery-dogfood-spine): phase 16 — bank the live-BigQuery findings and close the outcome`
