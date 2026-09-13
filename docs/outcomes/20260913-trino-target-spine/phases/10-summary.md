# Phase 10 summary — CI runs the Trino tier, gated like Spark's

## Shipped

- `crates/smelt-cli/tests/trino_ci_wiring.rs` — 5 standing gates, pure file
  assertions over `.github/workflows/compat.yml` and the Trino test sources:
  job gating (`needs: changes` + all four `if:` clauses), the `changes` job's
  `trino:` filter actually covering the six live paths (checked against the
  filesystem, not restated), the job's steps (tier up, env export, the three
  live-suite invocations, `if: always()` teardown), the no-skip guard
  (`set -o pipefail` + `grep -qi skipping`), and a census over every
  Trino-gated test file proving each skips through the shared `SMELT_TRINO_URL`
  gate with no fabricated default on that lookup.
- `.github/workflows/compat.yml` — `changes` job gains a `trino` output and
  filter block (mirrors `spark`'s shape, six glob paths covering the compose
  tier, the backend crate, the CLI test legs, and the shared runtime/dialect
  crates); a new `trino-integration` job standing up the tier via
  `scripts/trino-up.sh`, running `cargo test -p smelt-backend-trino` and
  `cargo test -p smelt-cli --test trino_smoke --test seed_parity --test
  materialization_parity` each under the no-skip guard, tearing down via
  `scripts/trino-down.sh` with `if: always()`.
- `scripts/README-trino.md` — new "In CI" section: which job, its three
  triggers, and the local-skip-is-green / CI-skip-is-a-failure asymmetry.

## Decisions

- The no-skip guard runs per test-invocation step (backend crate, then CLI
  legs) rather than as one job-wide post-step, so a skip is attributed to
  the specific step/log that produced it in the Actions UI.
- `SMELT_TRINO_*` env vars are exported into `$GITHUB_ENV` by sourcing
  `scripts/trino-env.sh` inside the tier-startup step (mirrors how
  `spark-parity` exports `SPARK_CONNECT_URL` — a sourced script's exports do
  not survive a step boundary on their own) rather than hardcoding the
  defaults into the workflow YAML, so a future `SMELT_TRINO_PORT` override
  in `trino-env.sh` is picked up automatically.
- `statement_client.rs` is in the plan's five-file census by name but never
  reads `SMELT_TRINO_URL` at all (confirmed: it only talks to a local `axum`
  stub, per its own header comment). The test treats that as the correct
  state for that one file — no live leg, so no gate to fake — rather than
  forcing it to read the var just to satisfy a uniform check.
- Skip-line detection is case-insensitive (`grep -qi`, `to_lowercase()`)
  because the four live-gated files don't agree on capitalization
  (`trino_smoke.rs` prints "SMELT_TRINO_URL unset — skipping ...", the other
  three print "Skipping ...").

## For the next planner

- Live-confirmed against the real tier (`trino-up.sh` → both cargo
  invocations with `SMELT_TRINO_URL` set → zero skip lines in either log →
  `trino-down.sh`), so phase 11's close can cite this as proven, not just
  file-asserted.
- Nothing new surfaced that changes phase 11's scope (docs-site page,
  hardening-baseline entry, divergences, handing the measured `✗` set
  forward). No reshape needed.

## Gates

- `cargo test -p smelt-cli --test trino_ci_wiring --test trino_tier_pins` — 10/10 pass.
- Live legs: `bash scripts/trino-up.sh` → `cargo test -p smelt-backend-trino`
  (60 tests, 0 skipped) → `cargo test -p smelt-cli --test trino_smoke --test
  seed_parity --test materialization_parity` (6 tests, 0 skipped) →
  `bash scripts/trino-down.sh` — all green, `grep -i skip` on both captured
  logs returned nothing.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both
  feature sets, shellcheck, full workspace `cargo test`, example_diagnostics).
