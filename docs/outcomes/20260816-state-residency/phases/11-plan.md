# Phase 11 plan — close-out: gate sweep, live Spark leg, criteria judgment

## Objective

Prove criterion 6 by executing every standing gate this outcome touches, including one **live**
run of `maintenance_conformance_spark` (phase 9 edited it but could only compile-check it), then
judge success criteria 1–6 against the phase summaries and flip the outcome's `**Status:**`.
This phase ships no new feature behaviour; anything it finds red is either fixed here or recorded
truthfully rather than papered over.

## Spec delta

None. This phase changes no user-visible behaviour. The only spec/outcome text it may write is a
Known Divergences narrowing *if* a gate run contradicts a claim a prior phase committed (e.g. the
live Spark run shows the ledger-less downgrade path behaving differently than `state.md` says) —
in that case the spec edit comes before any code change, per the spec-first rule.

## Tests

No new test files. This phase **executes** existing gates; the "red-green" is on the gates
themselves, and every command's real output must be read before any claim is made
(`superpowers:verification-before-completion`). If a gate is red, apply the systematic-debugging
skill rather than adjusting the assertion.

The live-Spark leg is the one genuinely new *execution*:

- `cargo test -p smelt-cli --features smelt-cli/spark --test maintenance_conformance_spark` —
  the Spark twin, run against a real Delta-enabled Spark Connect server. Intent: prove the
  `bail!` arms phase 8 added for `DropStateDir`/`FreshClone` and phase 9's availability threading
  actually behave on the ledger-less backend, not just compile.

## Tasks

1. Confirm the working tree is clean and on `delta-signature-closure`; note the HEAD sha in the
   summary so the sweep's evidence is anchored to a commit.
2. Run the full bundled gate: `bash .claude/scripts/verify-phase.sh` (allow a 15-minute timeout;
   fmt-check + clippy zero-warnings + full `cargo test` + `example_diagnostics`). Capture the
   tail. If the `smelt-parser-compat` `prop_smelt_valid_implies_spark_valid` flake phase 10's
   summary named (`top`-as-identifier) reappears, re-run to confirm it is seed-dependent, do NOT
   commit a `.proptest-regressions` file, and record it as pre-existing and out of scope
   (unrelated crate, SQL-dialect divergence) under "## Out of scope" in `outcome.md`.
3. Run the named standing gates individually so each has its own recorded result:
   `cargo test -p smelt-cli --test maintenance_conformance`,
   `cargo test -p smelt-runtime --test statement_parity`,
   `cargo test -p smelt-logical --test walk_coverage`,
   `cargo test -p smelt-runtime --test execute_parity`,
   plus this outcome's own tests: `-p smelt-cli --test state_docs`, and the
   `state_deletion` / `frontier_residency` / `state_posture` legs.
4. Live Spark leg. Read `scripts/README-spark.md` first. Then, **from this worktree**
   (`/home/andrew/smelt-sql/.claude/worktrees/closure`): `bash scripts/spark-up.sh`,
   `source scripts/spark-env.sh`, run the Spark conformance test above, then
   `bash scripts/spark-down.sh`. The container is a singleton bound to whichever worktree last
   ran `spark-up.sh` — running it from anywhere else produces silent `read_parquet` path
   mismatches, so re-run `spark-up.sh` here even if a container is already up. Run every command
   in the foreground and read its output.
5. If the Spark leg cannot run (server won't start, Delta jars unavailable, OOM), do NOT claim it
   passed: record the exact failure output and the reason in the summary and as a dated line in
   the Decision log, and treat criterion 6's Spark half as evidenced-by-compile-check-only.
   A recorded reason is an acceptable outcome for this row per its own wording; a silent skip
   is not.
6. Criteria judgment. For each of success criteria 1–6, write one paragraph in the summary
   naming the phase summary/summaries that evidence it and the gate output that confirms it.
   Judge criterion 4's spec half against `phases/01-summary.md` (its content landed even though
   row 1 reads `blocked` — see the 2026-08-16 phase-10 Decision log entry), and criterion 5
   against `phases/08-summary.md`.
7. `/smelt:validate state` — run the actual slash-command flow this time (phase 10 only
   cross-checked by hand). Record the drift report verdict. Any drift it finds that this outcome
   claims closed must be fixed here, not deferred.
8. Verify criterion 6's last clause mechanically: for each Known Divergences bullet this outcome
   claimed to remove, `rg` the owning spec (`state.md`, `run_state.md`, `incremental_models.md`)
   to confirm it is actually gone or narrowed as phase 10's summary describes.
9. Write `phases/11-summary.md` (shipped / decisions / gates, with the real command outputs
   summarized).
10. Flip row 11 to `done`. Then set the outcome's `**Status:**` per the termination rule: all six
    criteria met → `done` plus a dated evidence line in the Decision log; any criterion genuinely
    unmet → leave `active` is NOT an option here (this is the last row) — record it under
    "## Blocked" and set `blocked`, naming what a human must decide.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be fully green before any status flip.
- The individual gates in task 3 — each read, not assumed from the bundled run.
- `cargo test -p smelt-cli --features smelt-cli/spark --test maintenance_conformance_spark`
  against a live server (or a recorded, specific reason it could not run).
- Timeless-oracle grep over any spec/docs file touched: no `Phase [A-Z0-9]` hits outside
  Known Divergences/References.

## Commit message

`outcome(20260816-state-residency): close out state residency with full gate sweep`
