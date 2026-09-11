# Phase 16 — Close the reopening: the live BigQuery run

**Outcome:** `docs/outcomes/20260906-bigquery-correctness/outcome.md`
**Row:** 16 — "Close the reopening: re-run `examples/github_activity` live on BigQuery,
regenerate coverage, move the ratchets, extend the findings handoff"
**Criteria served:** 9 (BigQuery runs the pipeline's real plan), 5 (cross-target agreement),
4 (ratchets move the right way), 7 (gates green), 6/3 as carried forward
**Spec anchors:** `docs/specs/state.md` §"Which dialects realise which structure";
`docs/specs/incremental_shapes.md` §"The tombstone ledger (hidden state)";
`docs/specs/multi_backend.md` §"Cross-engine emission audit"

**This phase reaches a real warehouse.** The outcome's preamble marks it human-gated; the
human gate has been given for this run. Everything below is bounded by that: verify what
phases 11–15 built, close the outcome, and do not start new feature work live.

## The run

```bash
source scripts/bq-dogfood-env.sh     # layers on bigquery-env.sh; unsets the expiry guard
smelt run --target bigquery          # from examples/github_activity/
```

Credentials are Application Default Credentials impersonating
`smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com`; ADC is present on this box.
Target is the **long-lived** `smelt_dogfood` dataset, not the ephemeral per-run integration
dataset — read the header of `scripts/bq-dogfood-env.sh` before touching any env var, in
particular why `SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS` is explicitly unset. The dataset
accumulates history across days and the equivalence invariant is checked against state that
must still be there tomorrow. **Do not re-run `scripts/bq-dogfood-loader.sh`** and do not touch
`githubarchive` — the dataset is populated out of band.

Cost discipline: prior live runs of this pipeline billed well under a cent, the heaviest read
being ~20 KB. Stay in that envelope. If a check would require a large scan, say so and skip it
rather than spending — an unverified item recorded honestly beats a surprise bill.

The baseline to beat: the 2026-09-11 run built **14 models** (14 success / 0 failed / 0 skipped,
38s), with `silver.actor_sessions` and `marts.daily_active_contributors` refused at compile time
for `LAG`/`MAX` window-frame limits. Phases 11–15 should not have changed that refusal, but
they have changed a great deal underneath it.

## The six inherited checks

Each was left explicitly unproven by an offline phase. Verify each, and record the verdict
either way — a check that fails is this phase's most valuable output, not a setback.

1. **The untyped `NULL` coercion in the domain union** (phase 15). The tombstone arm of
   `build_domain_cte`'s 3-arm `UNION ALL` emits a bare `NULL AS <payload col>`; its acceptance
   rests on documented GoogleSQL coercion rather than measurement.
2. **The patch `MERGE` as a whole** (phase 15) — it has never executed anywhere.
3. **The transactional rebuild is really rejected** (phase 15), and the unbound form leaves the
   presented table and the tombstone ledger consistent.
4. **The `already_reflected` sentinel survives the adapter's error envelope** (phase 14) —
   `SMELT_LEDGER_ALREADY_REFLECTED` must still be `contains`-matchable after the Python
   adapter wraps the error.
5. **`@@row_count = 0` on a repeat `MERGE`** (phase 14) — the zero-row outcome that *is* the
   never-fold-twice refusal. Prove a repeat fold refuses; a second run of the same window is
   the natural way to reach it.
6. **First-run DDL inside `write_with_bookkeeping_plan`'s transaction** (phases 13/14, resolved
   by phase 12's `BookkeepingAtomicity::NonAtomicCreatingWrite`) — confirm a creating write
   now takes the non-atomic path and is not rejected.

Plus phase 12's two: the **Arrow list shape** BigQuery's adapter actually returns for a
`REPEATED STRING` (now designed to fail loudly rather than decode empty — so a green run is
itself the evidence), and that the `ARRAY<STRING>` columns without `NOT NULL` accept a
fully-suppressed window landing one present-and-empty row.

**Run at least twice.** Checks 2, 5 and 6 are about the *second* run against existing state;
the first run only proves creation. Read state back from the warehouse (`scripts/
bq-dogfood-query.sh`) rather than trusting the console transcript — phase 12's headline finding
(3 partitions, not 5,797) came from reading back what smelt actually wrote.

## Closing work

- **Regenerate coverage.** `SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit
  the_coverage_table_matches_the_registry`, then confirm `docs/reference/dialect-coverage.md`
  is either unchanged or correctly updated (`git status` clean afterwards is the check the
  doc-sync gate wants).
- **Ratchets.** `.claude/dialect-gaps-baseline.txt` and `.claude/parser-gaps-baseline.txt`
  fall or hold; neither is raised (criterion 4). Phase 10 established that
  `dialect_gaps_bigquery 42` **holds** with a dated note, because giving the 42 no-verdict
  entries verdicts speculatively is forbidden by criterion 2. If this run's evidence retires
  any of them legitimately, move the number down and say which; otherwise add a dated hold note
  in the same style.
- **Extend the findings handoff.** `docs/handoffs/2026-09-08-github-activity-findings.md` has a
  `## Close-out (2026-09-08)` section from phase 10. Add a close-out for the reopening —
  one row per criterion with artifact + gate, the six checks with their verdicts, and the run
  report id. Note `github_activity_oracle.rs`'s `handoff_claimed_relations()` scans that file
  scoped to `## The registered divergences`; do not break that scoping.
- **Issue #179** gets a comment if this run verified anything about its entries — not a close.
- **The outcome itself.** Flip row 16, set `Status` to `done` (or record honestly what keeps it
  open), and add the decision-log entry.

## If the run finds a defect

That is the expected and useful case — the last live run found two defects every offline gate
had passed. The rule from that entry still applies: **fix it, gate it offline, re-run.** A fix
with only a manual sweep behind it violates criterion 3. If a defect is large enough to need
its own phase, say so plainly and record it rather than half-fixing it under a closing row;
criterion 2 forbids inventing scope, and the outcome's Out of scope list is already explicit
about what does not belong here.

Do not tolerate a divergence silently: criterion 5 requires every cross-target difference to be
fixed or promoted to a reasoned permanent entry naming the engines and the construct, with the
unexplained count zero.

## Gates

```
bash .claude/scripts/verify-phase.sh
cargo test -p smelt-db --test dialect_audit
cargo test -p smelt-runtime --test statement_parity --test state_guard_census
cargo check -p smelt-cli --features bigquery
bash .claude/scripts/large-file-check.sh
```

## Deliverables

- `docs/outcomes/20260906-bigquery-correctness/phases/16-summary.md` — the run transcript's
  substance: model count, run report id, cost, and the verdict on each of the eight checks.
- The handoff close-out, the regenerated coverage table, the ratchet notes.
- A decision-log entry at the **top** of the outcome's `## Decision log`, in the voice of the
  existing entries — and this one closes the reopening, so it should say plainly what is proven
  live, what is still only proven offline, and what the spine's phase 13 (dual-target *value*
  parity) still owns.
- Row 16 flipped; outcome `Status` updated.
- One commit on `bigquery-prod` with the session's trailers.
