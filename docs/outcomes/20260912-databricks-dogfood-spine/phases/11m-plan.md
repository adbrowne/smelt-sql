# Phase 11m — Live: three scheduled runs under the `--skip-external-steps` carve-out

## Objective

Resume 11k's live legs, now unblocked by 11l's `--skip-external-steps` flag: redeploy the bundle
and wheels, confirm with one manual smoke run that `smelt_run` no longer tries to invoke
`sources.raw.github_loader`'s DuckDB-CLI dev-target loader and instead proceeds against what
`load_next_day` landed, then drive **three consecutive scheduled runs** to completion and compare
the resulting state against a full-refresh oracle. This closes criterion 11, the outcome's last
open criterion, and lands the evidence the three `github_activity_dbx_scheduled.rs` gates are
waiting on (criteria 4, 8 and 11).

## Spec delta

None. 11l already landed the user-visible change (`docs/specs/sources.md` §Semantics 12's named
carve-out, `docs/specs/run_state.md`) and its docs-site page. This phase is live execution and
evidence only — except task 8's `volume_probe` writeup, which is a docs-site addition, not a
behaviour change.

## Prerequisites (verify before any live task; do not skip green)

Re-measured at plan time **2026-09-14 22:36 local**. The picture has changed since the first
11m plan pass — record these three facts, they drive the whole phase:

- The credential is `oauth-m2m` and its minted bearer token lives **exactly one hour**
  (`scripts/dbx-auth.sh` writes `expires_in` from the OIDC exchange). A human re-minted it at
  21:28; it expired at 22:28, so `bash scripts/dbx-verify.sh` currently still fails every schema
  check with `PERMISSION_DENIED: Invalid Token`. **That is a stale token, not a stale secret.**
- **The gpg passphrase is cached in `gpg-agent` right now**, so re-minting is possible from this
  headless session with no prompt. Prove it non-interactively *before* invoking the auth script,
  because a cache miss would otherwise hang on pinentry:

      gpg --quiet --batch --pinentry-mode error --decrypt --output /dev/null \
        "${SMELT_DBX_CONFIG_DIR:-$HOME/.config/databricks-smelt-dogfood}/secret.gpg"

  Exit 0 ⇒ cached ⇒ `timeout 120 bash scripts/dbx-auth.sh < /dev/null` will complete silently.
  Non-zero ⇒ the cache lapsed ⇒ **stop**: emit `<<PHASE_BLOCKED>>` naming the gpg passphrase as
  the sole blocker (11m-summary.md's precedent). Never attempt a bare `dbx-auth.sh` on a cache
  miss, and never do partial live work on an unreachable workspace.
- **The one-hour token will expire mid-phase.** Three scheduled runs on a compressed cadence plus
  an oracle sweep exceed 60 minutes, so treat re-minting as a routine step, not an incident: run
  the probe-then-auth pair again before each long stage (tasks 4, 5, 7) and immediately on any
  `Invalid Token` from any `dbx-*.sh` script, then re-`source scripts/dbx-dogfood-env.sh`.
  `gpg-agent`'s `max-cache-ttl` is a hard wall (~2h from the human's 21:28 entry on stock
  settings); if the probe starts failing part-way through, block on the passphrase rather than
  abandoning a half-driven cadence — restore the committed daily cron first (task 9) so the
  workspace is not left on a compressed schedule.
- 11l merged (`--skip-external-steps` in `run_smelt.py`) and 11k's `sync.include: [dist/*.whl]`
  fix present in `databricks.yml` — both are committed on this branch.

## Tests

1. `github_activity_dbx_scheduled.rs::three_consecutive_scheduled_runs_completed` — flips from
   skip-when-missing to hard once `phases/11g-runs.json` is committed; asserts three
   `trigger: PERIODIC` runs with terminal success.
2. `github_activity_dbx_scheduled.rs::scheduled_run_compute_within_free_edition_quota` — hard once
   the same file lands; asserts criterion 4's quota accounting.
3. `github_activity_dbx_scheduled.rs::scheduled_state_matches_full_refresh_oracle` — hard once
   `phases/11g-equivalence.json` is committed; asserts criterion 8's comparator verdict.
4. `cargo test -p smelt-cli --test databricks_bundle` — the 24-case structural suite stays green
   across the cadence compression and its restoration (the committed default must end daily).

Keep the `11g-*` evidence filenames the three gates already point at — renaming per resume attempt
costs a red/green cycle and gains nothing (11i plan's ruling, re-affirmed).

## Tasks

1. Re-mint the credential: run the gpg cache probe, then
   `timeout 120 bash scripts/dbx-auth.sh < /dev/null`, then `source scripts/dbx-dogfood-env.sh`,
   then `bash scripts/dbx-verify.sh` to green on both legs (see Prerequisites). Re-run this pair
   whenever the hour lapses.
2. `bash scripts/dbx-bundle.sh deploy` — picks up 11l's `run_smelt.py` and 11k's `sync.include`
   fix. Confirm via `databricks workspace list` that **both** wheels (`_x86_64`, `_aarch64`) are
   present at the path `smelt_env.dependencies` references, since that silent-drop bug is recent.
3. `bash scripts/dbx-bundle.sh seed` (re-seeds `smelt.yml`/`models/`; never touches `.smelt/`, so
   11k's uploaded `intervals.json` survives). Verify with `databricks fs cat` that
   `.../project/.smelt/targets/databricks_job/intervals.json` still holds 11k's seeded window.
4. **One manual smoke run** (`bash scripts/dbx-bundle.sh run`), not yet on the compressed cadence.
   From the run's own task logs confirm three things in order: `load_next_day` advanced a fixture
   day; `smelt_run`'s `--auto` derived a non-empty window; and `sources.raw.github_loader` is
   reported **skipped** (`RunOutcomeKind::Skipped`), not invoked and not refused. If the step is
   still invoked, the deployed wheel or `run_smelt.py` predates 11l — re-check task 2 before
   suspecting the flag.
5. **Compress the cadence** (`--var schedule_cron=...`, the mechanism 11c/11e/11g/11i/11k used)
   and let **three consecutive scheduled runs** (`trigger: PERIODIC`, never manual) complete.
6. **Pull the three run reports from the Volume**; write `phases/11g-runs.json` with each run's
   id, trigger type, terminal state, duration and the loader's advanced fixture day.
7. **Compare against the full-refresh oracle** exactly as criterion 8's comparator does
   (`scripts/dbx-dogfood-oracle.sh` / `dbx-dogfood-parity.sh`), accounting for every fixture day
   now loaded — 11k measured 21 distinct days, `min=2026-08-05`, `max=2026-08-25`, plus one day
   per run since. Write the verdict to `phases/11g-equivalence.json`.
8. **Record compute consumed** against criterion 4's Free Edition quotas (into
   `phases/11g-runs.json`), and write up the `volume_probe` verdict (committed in 11c,
   `phases/11c-volume-probe.md`) in `docs-site/` if no prior phase already published it.
9. **Restore the committed daily cadence** (`0 0 6 * * ?`) and redeploy, so the committed bundle
   default is daily again; re-run the `databricks_bundle` suite to confirm.
10. Write `phases/11m-summary.md` with all the above evidence. If a genuinely new infra defect
    appears, fix what is small and root-caused in-pass (11k's `sync.include` precedent) and record
    any design question in `## Blocked` for the next planner rather than improvising a semantics
    change.
11. If criterion 11 is met: flip row 11m to `done`, mark criterion 11 met in the outcome body,
    set the outcome `**Status:**` to the bare word `done`, and append a dated evidence line to
    `## Decision log`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test databricks_bundle --quiet`
- `cargo test -p smelt-cli --test github_activity_dbx_scheduled --quiet` — the three gates must
  now run hard (evidence present), not print their skip message.
- No ratchet lowered.

## Commit message

`feat(databricks): complete three scheduled runs under the fresh-sources carve-out; close criterion 11`
