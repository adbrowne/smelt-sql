# Phase 16 summary — the live-BigQuery half is banked, and the outcome is closed

**No cloud was touched.** No credential was used, no `gcloud`/`bq` call was made, no
warehouse was queried. Every number, error string and `file:line` in the new sections was
copied from a committed phase summary; nothing was re-derived or re-measured here.

## What the handoff now says

`docs/handoffs/2026-09-08-github-activity-findings.md` is retitled "— both halves" and its
`**Status:**` now reads *complete*, naming 2026-09-12 as the live addendum's date and
"## The live BigQuery half" as the line the offline half ends at. The "Source of every
claim" line was extended from `0{2,3,4,6,8,9}-summary.md` to include `10-`, `11-`, `12-`,
`17-`, `13-` and `14-summary.md`.

Five new sections, in order:

1. **`## The live BigQuery half`** — a phase-ordered table (7, 10, 11, 12, 17, 13, 14) of
   what was provisioned, loaded, widened and run, each row carrying that phase's own
   measured cost; the whole live programme totals **≈ US$0.84**, of which US$0.45 is phase
   17's one-off widening scan. Then criteria 5, 6 and 7 stated as met **with what they rest
   on**: 14 of 16 models (two refused at compile time on GoogleSQL over an INTERVAL `RANGE`
   frame), 7 of 30 oracle checkpoints on BigQuery, against 16 models × 30 windows on
   DuckDB; six relations additionally exempt at the six intermediate checkpoints with a
   checkable proof each; no BigQuery CI tier, by standing decision.
2. **`## Live-BigQuery findings`** — nineteen rows,
   `| finding | provoking model | provoking statement | classification | owner |`. Findings
   `20260906-bigquery-correctness` has already closed (T5, the posture probe's two defects,
   `FILTER (WHERE …)`, the `VARCHAR` null placeholder) are recorded as **closed**, not as
   open work; so are the ones this outcome closed itself (the `stage_workspace` `.smelt/`
   leakage, the `--emit-ddl` `payload` omission, the `SMELT_BIN` de-featuring hazard, the
   `ArrivalLag` registration of `bronze_events`). The INTERVAL `RANGE` row is *partly*
   closed — the refusal landed, the lowering seam did not, which is exactly why the live
   half is 14 of 16.
3. **`## Recorded, not BigQuery findings`** — the backend-agnostic residue
   (`SourceRetentionExceeded` on a repeat `--full-refresh`, the posture-baseline
   granularity half being wrong on every backend, the inert `retention: '90 days'`, `smelt
   list --format json`, and the `bronze_events` arrival-order fact about the example).
4. **`## Operational notes for the next live run`** — the build/credential recipe (noting
   that ADC is already an impersonated `smelt-dogfood@` credential, so an explicit
   `--impersonate-service-account` is redundant), the measured costs, the live
   **2026-09-19** partition-expiration deadline, the pinned-binary and token-lifetime
   hazards, the `PARITY_CHECKPOINTS` / `PARITY_RESUME_FROM` levers, and the run-window
   -vs-partition-granularity note.
5. **`## Final punch-list`** — nine ordered items with an owner each. Item 1 is
   `--event-time-end` not bounding a full refresh's source scans; item 2 is
   [#203](https://github.com/adbrowne/smelt-sql/issues/203), referenced rather than
   restated; items 3-5 are `bigquery-correctness`'s, 6-7 `external-dag-steps`', 8
   `trimmed-history-sources`', 9 `bigquery-unattended`'s.

## The gates

Four new string-level tests, all RED before the document was written (`3 failed` on the
first run: the interim marker was still present, the live-findings table did not exist, and
the coverage numbers were unstated; the fourth test is the scan's own scoping control and
passes by construction). They sit beside the four phase-15 gates.

- `findings_handoff_declares_both_halves_landed` — replaces
  `findings_handoff_declares_its_interim_status`, which asserted the opposite and would
  have been false the moment this phase landed.
- `live_findings_each_name_a_provoking_model_and_statement` — a section-scoped scan over
  the new table; every row's provoking-model and provoking-statement cells must be
  non-empty (an em dash or `n/a` fails), with a classification and an owner.
- `live_findings_scan_is_scoped_and_non_vacuous` — two synthetic documents: a five-cell row
  outside the section is not collected, one inside it is.
- `findings_handoff_records_what_the_criteria_rest_on` — the handoff must itself state
  "14 of 16" and "7 of 30", so a reader cannot mistake "criteria met" for "everything
  covered". It caught a real defect in the first draft: "7 of 30" was split across a line
  break and the gate failed until the sentence was rewrapped.

**The eight gates moved to their own module file.** Adding the four tests took
`github_activity_oracle.rs` from 1,223 to 1,369 lines, past its large-file baseline. Rather
than raise the ratchet, the whole handoff-gate block (phase 15's four and phase 16's four)
moved to `crates/smelt-cli/tests/github_activity_handoff/mod.rs`, declared as `mod
github_activity_handoff;` from the oracle suite so it keeps reading that suite's own
`DIVERGENCE_REGISTRY` through `super::`. Same test binary, no new test target (the plan's
constraint), oracle file back to **1,069** lines, and the ratchet is unmoved.

## Other files

- `examples/github_activity/README.md` — the closing "it is interim" paragraph replaced
  with the finished claim plus the two compile-refused models.
- `docs/outcomes/20260906-bigquery-correctness/outcome.md` — a dated inbound entry naming
  the completed handoff and punch-list item 1 as its primary input, and the four other
  items that are its own.
- `docs/outcomes/20260906-trimmed-history-sources/outcome.md` — a one-liner: the 45-day
  bound it must reconcile was read back live (`expirationMs = 3888000000`, phases 10 and
  17) and is operationally live to 2026-09-19; the requirement itself is unchanged.
- `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md` — phase-16 row and
  `**Status:**` flipped to `done`, the stale driver-line clause corrected (phase 16 harvests
  phases 10–14 and 17, not 10 and 11), a lead "nothing is blocked" entry on the `## Blocked`
  section, and a closing 2026-09-12 decision-log entry naming which criteria are met on what
  evidence and what is not covered.

## Verification

```
$ bash .claude/scripts/verify-phase.sh
PASS  cargo fmt --check
PASS  cargo clippy (zero warnings, both feature sets)
PASS  shellcheck (scripts/, .claude/scripts/)
PASS  cargo test (workspace)
PASS  example_diagnostics
VERIFY: ALL GREEN

$ cargo test -p smelt-cli --test github_activity_oracle
test result: ok. 21 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 307.62s

$ cargo test -p smelt-cli --test github_activity_oracle handoff
test github_activity_handoff::every_registry_entry_is_named_in_the_findings_handoff ... ok
test github_activity_handoff::findings_handoff_names_no_unknown_relation ... ok
test github_activity_handoff::findings_handoff_declares_both_halves_landed ... ok
test github_activity_handoff::live_findings_scan_is_scoped_and_non_vacuous ... ok
test github_activity_handoff::live_findings_each_name_a_provoking_model_and_statement ... ok
test github_activity_handoff::the_handoff_scan_is_scoped_to_the_divergence_section ... ok
test github_activity_handoff::the_handoff_scan_still_catches_a_stale_claim_in_its_own_section ... ok
test github_activity_handoff::findings_handoff_records_what_the_criteria_rest_on ... ok
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 14 filtered out

$ bash .claude/scripts/large-file-check.sh
Large-file ratchet OK — no tracked file exceeds its baseline, no new file exceeds 1500 lines.
```

The RED run, before the document was written (same command, four new tests):

```
test result: FAILED. 1 passed; 3 failed
  findings_handoff_declares_both_halves_landed
    -> findings handoff still declares itself the DuckDB half only
  findings_handoff_records_what_the_criteria_rest_on
    -> findings handoff must state the live half's model coverage (14 of 16)
  live_findings_each_name_a_provoking_model_and_statement
    -> expected the live-findings table to carry at least the seven findings phase 16
       harvested, got 0
```

## Not done

- Nothing was blocked and nothing was deferred.
- **No defect was fixed.** Every open finding is handed on with an owner; that is this
  phase's whole job, and this outcome's "Out of scope" assigns the fixes elsewhere.
- The live evidence was not re-run or re-verified against the warehouse — it could not be,
  and did not need to be: the phases that produced it committed their own gated reports.
