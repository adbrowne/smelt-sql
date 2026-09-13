# Databricks Free Edition — measured facts

Provisioned by a human running `scripts/dbx-provision.sh` (phase 4b); the
demonstrable half of criterion 4 — reachability, grant scope, and this facts
sheet — was closed from a reachable session in phase 4c
(`docs/outcomes/20260912-databricks-dogfood-spine/phases/04c-summary.md`).
Free Edition carries no bill, so these measured quotas stand in for the
budget cap `docs/outcomes/20260906-bigquery-dogfood-spine/` records in
dollars (criterion 4 of
`docs/outcomes/20260912-databricks-dogfood-spine/outcome.md`).

| Quota | Value | Basis |
|---|---|---|
| Serverless concurrency | Max 5 concurrent job tasks per account; one SQL warehouse capped at `2X-Small` | cited — [Free Edition limitations](https://docs.databricks.com/aws/en/getting-started/free-edition-limitations) |
| Cold-start latency | ~4.4–4.7s per query round trip (`bash scripts/dbx-query.sh "SELECT 1"`), and this held equally for a call made ~2 minutes after the prior one and for calls made back-to-back — because each `dbx-query.sh` invocation opens a brand-new `DatabricksSession` rather than reusing a warm one, there is no separate "cold" vs "warm" number to report for this connection path | measured — 3 consecutive timed runs, 2026-09-12 |
| Storage limit | No fixed GB cap documented; storage is governed by the account-wide fair-usage policy (exceeding it suspends compute for the rest of the day, or month in extreme cases — data and settings are not deleted) | cited — [Free Edition limitations](https://docs.databricks.com/aws/en/getting-started/free-edition-limitations) |
| Session idle timeout | No published number for Databricks Connect serverless sessions. Databricks' own docs describe only a "default idle timeout" after which session *restoration* is best-effort, with preserved state expiring after two days of total inactivity. Directly observed in this worktree: during a single `dbx-verify.sh` run, two of four query calls (spaced ~4.5s apart) logged `INVALID_HANDLE.SESSION_CLOSED` when `adapter.close()` ran — the serverless session had already been torn down server-side by the time the client tried to release it, even though the query itself had already returned its result. This suggests serverless session teardown can happen on the order of single-digit seconds after the last statement, well under the 5-minute range a warehouse-backed SQL session typically allows, though this is inference from one reproduction, not a documented number | measured (partial) + cited — [Query interruptions with Databricks Connect](https://docs.databricks.com/aws/en/dev-tools/databricks-connect/queries) |
| Incremental-window wall time | ~2 min for W1 (load one day + a 14-model `smelt run`, including the one known `gold.events_enriched` failure), ~81s for W2 and W3 back-to-back once the venv/session paths were already warm. `run reports/*.json` cannot corroborate this directly — `completed_at`/`duration_ms` are unpopulated (`null`/`0`) even on completed runs, so this is start-to-start timing between consecutive `smelt run` invocations, not a value read from the report | measured — three consecutive windows, 2026-09-12 (phase 7) |
| Incremental-window wall time, 16/16 clean (phase 7b) | W4 (`2026-08-10`, `gold.events_enriched`'s first clean window, recomputing its whole pinned-since-`2026-08-07` region via the new `DeleteInsert` downgrade): 79.9s. W5 (`2026-08-11`, ordinary): 64.5s. W6 (`2026-08-12`, ordinary): 62.4s. Unlike the W1–W3 row above, these three reports' own `completed_at`/`duration_ms` fields ARE populated (a pre-existing gap that closed incidentally, not fixed by this phase) — the figures here are read directly from `reports/*.json`, not start-to-start timing | measured — three consecutive windows, 2026-09-12 (phase 7b) |

## Credential kind chosen

**Service-principal OAuth M2M.** The stored bearer token decodes as a JWT
(header `alg`/`typ` fields, three dot-separated base64url segments), not a
`dapi…`-prefixed personal access token, confirming Free Edition permits the
OAuth machine-to-machine path and the human run of phase 4b chose it over a
PAT. Observed token TTL is approximately one hour from mint time (see the
outcome's `## Blocked` item (b) for the unresolved question of how a
headless step refreshes this before phases 5–9 and 11 can run unattended).

## Operational notes

- **One-hour OAuth token lifetime.** `scripts/dbx-auth.sh` is the only
  refresher, and it is in `permissions.deny` (it needs a gpg passphrase to
  decrypt the stored secret). A headless live phase that runs longer than
  the human's last manual `dbx-auth.sh` invocation will find its token
  expired mid-run. This is tracked as unresolved in
  `docs/outcomes/20260912-databricks-dogfood-spine/outcome.md` §"Blocked",
  item (b) — phases 5–9 and 11 inherit this constraint and must check
  reachability before assuming a long-running window will complete.
- **Session-per-invocation, not session-per-script.** Every
  `scripts/dbx-query.sh` call opens and tears down its own
  `DatabricksSession`; nothing in this dogfood tooling keeps a warm session
  across calls. A live phase that needs many queries in a tight loop will
  pay the ~4.5s session-bootstrap cost on every single one.
