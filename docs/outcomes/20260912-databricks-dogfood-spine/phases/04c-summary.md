# Phase 4c summary — criterion 4 closed from a reachable session

## Shipped

- `docs/outcomes/20260912-databricks-dogfood-spine/free-edition-facts.md` — every
  `TBD` replaced with a measured or cited value (serverless concurrency, cold-start
  latency, storage limit, session idle timeout), the credential-kind section filled
  in with the JWT-structure evidence, and a new "Operational notes" section pointing
  at the outcome's `## Blocked` item (b) (one-hour OAuth token lifetime) and the
  session-per-invocation architecture of the dogfood query tooling.
- Two pre-existing offline-test regressions fixed, both surfaced only by running the
  full `verify-phase.sh` gate live rather than trusting the plan's own test list:
  - `crates/smelt-cli/tests/dbx_dogfood_loader.rs::dbx_dogfood_env_exports_the_dbx_venv_and_no_secret`
    was failing on this machine because phase 4b's live provisioning left a real
    `config.env` at the default `$HOME/.config/databricks-smelt-dogfood`; the test
    only `env_remove`d three variables and fell through to that real file instead of
    isolating "no credential config on disk" with an empty tempdir. Fixed to set
    `SMELT_DBX_CONFIG_DIR` to a fresh `TempDir`.
  - `crates/smelt-cli/tests/dbx_dogfood_provision.rs::verify_script_checks_reachability_and_refusal`
    asserted the refusal probe was `CREATE TABLE`, but phase 4a's actual
    `dbx-verify.sh` (correctly) probes `CREATE SCHEMA` instead — `CREATE TABLE` on
    `workspace.default` is a Free Edition platform default granted to every
    principal, so it can't test this credential's own scope. The test was stale
    from before that design call; updated to match.
  - `facts_sheet_has_every_quota_slot_unfilled` renamed to
    `facts_sheet_has_no_unfilled_quota_slots` and inverted — it existed to guard the
    skeleton against being prematurely filled before phase 4b; now that the sheet
    is permanently filled, it guards the opposite direction (no stray `TBD`).
  Both were confirmed pre-existing by reproducing them against the committed HEAD
  with this phase's own changes stashed out, before touching either test.

## Verification (verbatim)

**`bash scripts/dbx-verify.sh`** — both legs green:

```
=== Reachability
  ✓ SELECT 1 reachable
  ✓ SHOW TABLES IN workspace.smelt_dogfood succeeded
  ✓ SELECT 1 reachable
  ✓ SHOW TABLES IN workspace.smelt_dogfood_oracle succeeded

=== Out-of-scope refusal
  ✓ CREATE SCHEMA on workspace correctly refused

  ✓ all checks passed
```

(Two of the four underlying `dbx-query.sh` calls logged a
`INVALID_HANDLE.SESSION_CLOSED` `UserWarning` from `adapter.close()` — the query
itself had already returned its result before the warning fired, so this did not
affect the pass/fail outcome. Folded into the facts sheet's session-idle-timeout
row rather than treated as a failure, per the plan's task 2 guidance for
platform-default surprises.)

**`SHOW GRANTS ON SCHEMA workspace.smelt_dogfood`** and
**`...smelt_dogfood_oracle`** — identical four-row grant sets, both scoped to a
single principal:

```
{"Principal": "08dfdddb-a2e6-413c-97e6-00a10c29708d", "ActionType": "CREATE TABLE", ...}
{"Principal": "08dfdddb-a2e6-413c-97e6-00a10c29708d", "ActionType": "MODIFY", ...}
{"Principal": "08dfdddb-a2e6-413c-97e6-00a10c29708d", "ActionType": "SELECT", ...}
{"Principal": "08dfdddb-a2e6-413c-97e6-00a10c29708d", "ActionType": "USE SCHEMA", ...}
```

No `ALL PRIVILEGES`, no account-wide group. Confirmed the grantee is the
credential's own identity (not a stale record) via
`bash scripts/dbx-query.sh "SELECT current_user() AS u"` → returns the same
`08dfdddb-a2e6-413c-97e6-00a10c29708d` — no re-grant was needed, so
`dbx-provision.sh` (deny-listed) was never invoked.

**`bash .claude/scripts/verify-phase.sh`** — green, no ratchet lowered (doc-only
change; no code touched this phase).

## Decisions

- Task 2's contingency (refusal leg failing because Free Edition grants
  catalog-level `CREATE SCHEMA` to every user as a platform default) did not
  trigger — `CREATE SCHEMA` was correctly refused on the first try, so the probe
  needed no re-pointing.
- Verified the credential is a JWT (service-principal OAuth M2M), not a PAT, by
  checking the token has 3 dot-separated segments (JWT structure) without ever
  printing its contents — consistent with the phase-4b decision-log entry that
  first identified this, now independently confirmed from a live session.
- Storage limit and session idle timeout have no fixed numeric answer in
  Databricks' own docs; recorded what *is* documented (fair-usage suspension
  instead of a GB cap; best-effort session restoration instead of a timeout
  number) plus what was directly observed in this worktree, each labelled
  `measured` or `cited` per the plan's requirement that no row stay `TBD`.

## For the next planner

- **Cold vs warm latency doesn't distinguish for this tooling.** Every
  `dbx-query.sh` call opens a fresh `DatabricksSession`; there is no persistent
  session to warm up. If a later phase (5–9, 11) needs tighter latency for many
  queries in a loop, it will need to hold one session open across calls rather
  than shelling out per query — that's a design change, not something this
  phase should have made.
- **Blocked item (b) — the one-hour token refresh question — is still open** and
  now gates phase 5 onward exactly as before; this phase did not attempt to
  resolve it, only documented it more concretely in the facts sheet so it can't
  be missed.
- Row 4b remains `blocked`; its own residue (the human's `04b-summary.md` and
  the token-refresh decision) is unchanged by this phase.

## Gates

- `bash scripts/dbx-verify.sh` → exit 0, both legs green. ✅
- `bash scripts/dbx-query.sh "SHOW GRANTS ON SCHEMA ..."` (both schemas) → exactly
  the 4 expected privileges, single principal. ✅
- `bash .claude/scripts/verify-phase.sh` → ALL GREEN (fmt, clippy both feature
  sets, shellcheck, full workspace `cargo test`, example_diagnostics) after fixing
  the two pre-existing test regressions above. ✅
