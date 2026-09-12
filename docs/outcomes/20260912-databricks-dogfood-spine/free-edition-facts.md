# Databricks Free Edition — measured facts

Filled in by `scripts/dbx-provision.sh` (phase 4b, human-gated). Free Edition
carries no bill, so these measured quotas stand in for the budget cap
`docs/outcomes/20260906-bigquery-dogfood-spine/` records in dollars
(criterion 4 of `docs/outcomes/20260912-databricks-dogfood-spine/outcome.md`).

| Quota | Value |
|---|---|
| Serverless concurrency | TBD (phase 4b) |
| Cold-start latency | TBD (phase 4b) |
| Storage limit | TBD (phase 4b) |
| Session idle timeout | TBD (phase 4b) |

## Credential kind chosen

TBD (phase 4b) — service principal with an OAuth M2M secret if Free Edition
permits one, else a personal access token. `scripts/dbx-key.sh` and
`scripts/dbx-auth.sh` are credential-agnostic over this choice; record which
one and why here once `scripts/dbx-provision.sh` has run.
