# Phase 12 plan — three or more consecutive incremental windows on BigQuery

**Advances:** criterion 5 (BigQuery leg, live: a full refresh then at least three
consecutive incremental windows, with the run report captured for each). Feeds phase 13
(dual-target parity), which needs a BigQuery side that actually holds every model's output,
and phase 16 (the live half of the findings handoff), which harvests this phase's summary.

**Human-gated.** Runs live BigQuery against the dogfood dataset with a human-minted ADC
impersonation token. The block that stood over this row until 2026-09-11 is lifted:
`20260906-bigquery-correctness` phase 11 (`6158dc921`) turned the three T5 `bail!` sites
into recorded downgrades derived from `realisable_state_structures`. **Nothing is yet known
about whether the downgraded plan carries the model set past `silver.events_deduped`** —
that is task 3's question and everything after it is contingent on the answer.

## Spec delta

None. No user-visible feature behaviour changes are made here. This phase runs the product
as committed and records what it does. Per this outcome's "## Out of scope", a defect this
phase surfaces is characterised (model, statement, `file:line`, verbatim error) and handed
to `20260906-bigquery-correctness` — not fixed.

## The windows, derived from the table rather than assumed

Read back from `smelt_dogfood` this session (REST `queries`, `GROUP BY` over each
partition column) rather than taken from phase 10's summary:

| table | partition column | days present (rows) |
|---|---|---|
| `github_events` | `DATE(created_at)` (event time) | 2026-08-04 (75), 2026-08-05 (3,264), 2026-08-06 (2,714) |
| `github_events_arrival` | `ingested_date` (arrival time) | 2026-08-05 (3,276), 2026-08-06 (2,777) |

Three event-time days exist and no more: the loader ran twice (phase 10), on 08-05 and
08-06, and 08-04 exists only as day-05's 2% redelivery reach-back. **No further loader run
is made** — a loader run re-scans `githubarchive` and that is the only expensive thing in
this outcome; the whole phase runs over these ~6,053 already-loaded rows.

That bounds the window schedule to four consecutive days, so the split is:

| step | invocation | what it is |
|---|---|---|
| FR | `--full-refresh --start 2026-08-04 --end 2026-08-05` | clean full refresh over the smallest day (75 rows) — establishes the baseline state the incremental windows advance from |
| W1 | `--start 2026-08-05 --end 2026-08-06` | first incremental window, 3,264 new event rows + day-05's arrival partition |
| W2 | `--start 2026-08-06 --end 2026-08-07` | second incremental window, 2,714 new event rows + day-06's arrival partition |
| W3 | `--start 2026-08-07 --end 2026-08-08` | third incremental window over a day with **no** new source rows |

W3 being empty of new rows is deliberate rather than a shortfall of data. A window that
lands nothing is exactly the shape that drives a keyed merge to suppress a no-op write,
which is the path the T5 downgrade now sits on
(`crates/smelt-runtime/src/maintenance_driver/driver.rs`, the
`WriteSuppression::Suppressed` arm). If the downgrade is going to behave differently from
DuckDB anywhere, W3 is where it shows. Any additional window would also be empty — three
event days is all the data there is — so a fourth adds nothing W3 does not already cover,
and the phase stops at four steps.

`--end` is exclusive (`smelt run --help`: "End of range for backfill (exclusive)"), so each
row above is exactly one calendar day.

## Run reports: the project must opt into state

Phase 11 recorded, correctly, that **no run report exists** for this project: it declares
no `state:` key, so it runs at the default `state.mode: stateless`, under which "no
manifest, interval, snapshot, or environment record is written"
(`docs/specs/run_state.md` §Semantics, "Stateless writes nothing"). A run report is written
to `.smelt/targets/<target>/reports/<run_id>.json` only at a higher posture.

This phase's row requires run reports *and* "frontier and engine-resident state inspected
between runs", so the posture is the deliverable rather than an incidental. Task 2 adds
`state: { mode: intervals }` to `examples/github_activity/smelt.yml`. Two things this
must respect:

- **It is a project-wide key** (`docs/specs/smelt_yml.md`), not per-target, so the DuckDB
  leg starts writing `.smelt/targets/dev/` too. `examples/*/.smelt/` is gitignored, so
  nothing lands in the tree, but the DuckDB gates could still change behaviour (the
  interval ledger is what `--auto` and gap detection read). Task 7 runs them; if they go
  red, the config change is **reverted** and the fact recorded as a finding, rather than
  the gates being widened to accommodate it.
- The reconciliation ledger is **not** what `state.mode` controls. It is engine-resident
  and required correctness structure independent of the posture
  (`docs/specs/run_state.md` §"Relationship to the reconciliation ledger"), and on BigQuery
  it is declared unrealisable (`docs/specs/state.md` §"Which dialects realise which
  structure" — BigQuery is "not yet" on all five rows). So "the frontier" this phase can
  inspect between runs is the `.smelt/` interval ledger plus the landed-delta record, and
  the engine-resident side is inspected as *the absence it is declared to be*: the
  dataset's table list must show no ledger/sidecar/tombstone sibling tables, and the
  downgrade must be visible in the diagnostics rather than in a refusal.

## Clearing the ground

Phase 11 left four derived tables (`bronze_events`, `silver_events_deduped`,
`silver_repo_naming`, `silver_actor_naming`), and recorded that any further
`--full-refresh` refuses while they exist (`SourceRetentionExceeded`, from
`crates/smelt-runtime/src/execute/retention_admission.rs` — backend-agnostic and specced,
`docs/specs/sources.md` §Semantics 5). The human has authorised dropping exactly those
four, plus any ledger/sidecar/state table smelt itself created in `smelt_dogfood`. Read
back this session, the dataset holds exactly six tables — the four above plus
`github_events` and `github_events_arrival` — so there is no smelt-created state table to
drop, and the two loaded source tables are untouchable and stay.

## Tasks

1. **Baseline the live state.** Record the dataset's table list, row counts and creation
   times, and the per-day partition counts of both source tables, from the API. (Done while
   planning; re-confirmed immediately before the drop so the summary's before/after is
   honest.)
2. **Opt into `state.mode: intervals`** in `examples/github_activity/smelt.yml`, with a
   comment saying why (run reports and the interval frontier are this phase's evidence).
3. **Drop the four derived tables** and re-verify the dataset holds only the two source
   tables plus nothing else. Then run FR
   (`smelt run --target bigquery --full-refresh --start 2026-08-04 --end 2026-08-05`).
   **This is the phase's pivot.** Three outcomes, all recorded either way:
   - it completes the whole model set → the rest of the plan runs as written;
   - it stops at a *new* refusal → characterise it exactly (`file:line`, verbatim text),
     and the phase's honest status is `blocked` with that gate named;
   - it stops at the T5 site again → the phase-11 fix did not reach this path; record the
     reached line and hand it back to `bigquery-correctness` phase 11's row.
4. **Run W1, W2, W3 in order.** After each: capture the console transcript verbatim, the
   run report JSON (`.smelt/targets/bigquery/reports/<run_id>.json`), the interval ledger
   (`intervals.json`) and the landed-delta record, and the dataset's table list with row
   counts. Re-mint the ADC token between runs if more than ~40 minutes have passed (a
   minted token lasts ~1 hour and a run dying mid-way is worse than a spare `curl`).
5. **Inspect between runs, and say what was inspected.** Per window: which models the
   report records `success`/`skipped`/`failed`, how the interval ledger's coverage advanced,
   and whether the dataset grew the way the window's row counts predict. Assert the
   engine-resident absence explicitly (no `*__tombstones`, no ledger/sidecar tables), since
   that absence is what the degradation contract promises here rather than an oversight.
6. **Cost.** Read `totalBytesBilled` per job from the BigQuery job history for the whole
   phase (REST `.../jobs`), not from dry-run estimates. No statement in this phase touches
   `githubarchive`; if any projection exceeds ~US$1, stop and report instead of running.
7. **Gates.** `cargo fmt --all` (nothing under `crates/` is expected to change, so this is
   a no-op check rather than a claim), then the gates the config change actually touches:
   `cargo test -p smelt-cli --test github_activity_replay`,
   `--test github_activity_oracle`, `--test example_diagnostics`. The full workspace suite
   is not run — no production code is edited — and that is stated rather than skipped
   silently.
8. **Write `phases/12-summary.md`**, self-contained per finding (phase 16 harvests it),
   and update `outcome.md`: the phase 12 row's status and a dated decision-log entry. If
   the phase stops short, mark it `blocked` with a "## Blocked" entry naming the gate and
   its owner.
9. **Commit and push** to `bigquery-prod`.

## What this phase does not do

- No loader run, no `githubarchive` scan, no dataset create or delete, nothing outside
  `smelt_dogfood`, and no touch of `github_events` / `github_events_arrival`.
- No fix under `crates/`. If a run cannot complete at all without one, that is the single
  exception the brief allows and it is stated explicitly in the summary with the change
  kept minimal — not slipped in as incidental.
- No parity comparison against DuckDB (phase 13) and no full-refresh oracle (phase 14),
  beyond leaving the BigQuery side in a state those phases can read.
