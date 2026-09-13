# Phase 4c — Close criterion 4 from a reachable session

## Objective

Phase 4b left its row `blocked`, but a probe from this worktree shows the workspace is
reachable and `workspace.smelt_dogfood` / `workspace.smelt_dogfood_oracle` both exist — so
the *demonstrable* half of success criterion 4 is no longer human-gated. This phase runs
`scripts/dbx-verify.sh` to green on both legs, confirms the credential's grants are scoped
to exactly those two schemas, and replaces every `TBD` in `free-edition-facts.md` with a
measured or cited value. It advances criterion 4 only; it loads no data (that is phase 5).

## Spec delta

None. No user-visible feature behaviour changes — this is provisioning verification and a
facts sheet. (`docs/specs/multi_backend.md`'s Databricks text landed in phase 1.)

## Tests

This phase's oracle is a live script, not a cargo test; the checks below are its red-green
list and must each be shown passing in the summary with pasted output.

- `bash scripts/dbx-verify.sh` — **reachability leg**: `SELECT 1` and `SHOW TABLES` against
  both dogfood schemas succeed.
- `bash scripts/dbx-verify.sh` — **refusal leg**: the out-of-scope probe is refused, proving
  the credential holds no catalog-wide rights.
- `bash scripts/dbx-query.sh "SHOW GRANTS ON SCHEMA workspace.smelt_dogfood"` — the grantee is
  the service principal's application ID, and the privilege set is exactly
  `USE SCHEMA, CREATE TABLE, SELECT, MODIFY`; no `ALL PRIVILEGES`, no account-wide group.
  Same for `smelt_dogfood_oracle`.
- `bash .claude/scripts/verify-phase.sh` — no regression from the doc/script edits.

## Tasks

1. Run `bash scripts/dbx-verify.sh`. Capture full output verbatim for the summary.
2. If the **refusal leg** fails because Free Edition grants catalog-level `CREATE SCHEMA` to
   every workspace user as a *platform default* (the same class of default the script's own
   header already documents for `workspace.default`), do **not** weaken the leg to a no-op:
   record the default as a Free Edition fact, and re-point the probe at an action the
   credential's own grants actually bound (e.g. `CREATE TABLE` in `workspace.information_schema`,
   or `SELECT` from a table in neither dogfood schema), with a header comment saying why the
   previous probe was not a scope test on this edition.
3. Run `SHOW GRANTS ON SCHEMA` for both schemas via `scripts/dbx-query.sh`. If the grantee is
   missing, or is an account-wide group rather than the service principal's application ID,
   re-apply the four privileges through `scripts/dbx-query.sh` (`scripts/dbx-provision.sh` is
   in `permissions.deny`) and re-check. Revoke anything broader that is present and revocable.
4. Fill `docs/outcomes/20260912-databricks-dogfood-spine/free-edition-facts.md`:
   - **Credential kind chosen** — service-principal OAuth M2M (the stored bearer token is a
     JWT, not a `dapi…` PAT); one line on why, and the observed token TTL.
   - **Cold-start latency** — time two `dbx-query.sh "SELECT 1"` calls: the first after an idle
     gap, the second immediately after, and record both (cold vs warm).
   - **Session idle timeout** — the interval after which a warm session returns
     `INVALID_HANDLE.SESSION_CLOSED` (already observed once in this worktree); if not
     reproducible within the phase, cite the edition's documented value and mark it *cited*.
   - **Serverless concurrency** and **storage limit** — measured where cheaply measurable,
     otherwise the value the workspace itself reports or the edition documents, each labelled
     `measured` or `cited`. No row may stay `TBD`.
5. Add a short "Operational notes" section to the facts sheet naming the one-hour OAuth token
   lifetime and pointing at the outcome's `## Blocked` item (b) as the unresolved refresh
   question — so phases 5–9 inherit the constraint in writing rather than by surprise.
6. Write `phases/04c-summary.md`: verbatim verify output, the grant rows, the filled facts
   table, and anything the live workspace contradicted about phases 1–4a's assumptions.
7. Flip row 4c to `done` in `outcome.md`. Leave row 4b `blocked` — its residue is now only the
   `04b-summary.md` the human owes and the token-refresh decision.

## Verification

- `bash scripts/dbx-verify.sh` → exit 0, both legs green.
- `bash .claude/scripts/verify-phase.sh` → green, no ratchet lowered.
- If `scripts/dbx-dogfood-env.sh` cannot reach the workspace (expired token, unreachable
  host), do **not** fabricate facts: emit `<<PHASE_BLOCKED>>` and record the reachability
  failure under the outcome's `## Blocked` item (b).

## Commit message

`outcome(databricks-dogfood-spine): phase 4c verifies credential scope and records Free Edition facts`
