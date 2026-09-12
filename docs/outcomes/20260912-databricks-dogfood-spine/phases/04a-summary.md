# Phase 4a summary — Provisioning tooling, offline

## Shipped

- `scripts/dbx-key.sh` — captures either credential kind (`oauth-m2m` or `pat`), gpg
  symmetric-encrypts it to `$CONFIG_DIR/secret.gpg` (0600), writes `config.env`, shreds the
  plaintext. `--self-test` validates config-dir permissions and the credential-kind dispatch
  with no prompt and no gpg.
- `scripts/dbx-auth.sh` — decrypts the secret, branches on `SMELT_DBX_CRED_KIND`: `oauth-m2m`
  exchanges client credentials at `$SMELT_DBX_HOST/oidc/v1/token` for a fresh token every run;
  `pat` decrypts the long-lived token as-is. Both land at `$CONFIG_DIR/token` (0600). Prints
  only the expiry, never the token. `--self-test` exercises the same dispatch with no secret.
- `scripts/dbx-provision.sh` — the `/wizard`-shaped stages: confirm the workspace host, mint
  the credential (calls `dbx-key.sh`), create `smelt_dogfood` + `smelt_dogfood_oracle` and
  their schema-scoped grants (never catalog-level, never `ALL PRIVILEGES`), hand off to
  `dbx-verify.sh`, then prompt for and record the four Free Edition quota facts.
- `scripts/dbx-verify.sh` — reachability leg (`SELECT 1` + `SHOW TABLES` against both schemas)
  and refusal leg (a `CREATE TABLE` in `workspace.default` must fail; the script's exit-status
  handling is inverted so a credential scoped too broadly is caught, not silently passed).
- `scripts/dbx-query.sh` + `scripts/dbx_dogfood_query.py` — the one allow-listed way a Claude
  session touches the workspace directly: a thin `SELECT`-runner over
  `smelt.databricks_adapter.DatabricksAdapter`, printing rows as newline-delimited JSON.
- `docs/outcomes/20260912-databricks-dogfood-spine/free-edition-facts.md` — quota-table
  skeleton, every value the literal `TBD (phase 4b)`.
- `.claude/settings.json` — `dbx-key.sh`/`dbx-auth.sh`/`dbx-provision.sh` plus the config dir's
  `Read(...)` added to `permissions.deny`; `dbx-verify.sh`/`dbx-query.sh`/`dbx-dogfood-loader.sh`
  added to a new `permissions.allow`.
- `scripts/dbx-dogfood-env.sh` now exports `SMELT_DBX_ORACLE_SCHEMA` (default
  `smelt_dogfood_oracle`), the gap phase 3's summary flagged.
- `crates/smelt-cli/tests/dbx_dogfood_provision.rs` — 8 tests, all offline (existence/shape,
  settings deny/allow split, no-unmasked-token, credential-kind dispatch, scoped-grant shape,
  inverted refusal-leg shape, oracle-schema export, facts-sheet placeholders).
- `examples/github_activity/README.md` — documents the provisioning sequence next to the
  existing loader section.

## Decisions

- Reused `bq-dogfood-provision.sh`'s wizard library block verbatim (per the plan); authored
  5 stages rather than 7 — Free Edition needs no API-enablement or budget stage, unlike
  BigQuery's reused-project path.
- `dbx-verify.sh` shells out through `dbx-query.sh` rather than reimplementing the Databricks
  Connect call, so the verification path and the one allow-listed query path are the same code.
- Fixed two real bugs the tests caught before they could hit a live workspace: (1) `dbx-key.sh`'s
  original `--self-test` used `trap ... EXIT` inside a function-local `tmp`, which threw an
  "unbound variable" at script exit because the trap fires after the local var's scope ends —
  replaced with direct `rm -rf` at the end of the function; (2) the wizard's own comment text
  contained the literal string `ALL PRIVILEGES` (in a "never do this" note), which its own test
  then correctly flagged as a false positive — reworded the comment.

## For the next planner

- **A `.env` file with a real-looking Databricks host** (`https://dbc-466c2133-...`) appeared in
  the worktree root partway through this session, with no test or script of mine having run
  `dbx-provision.sh`. This strongly suggests a human ran the wizard live in this same worktree
  while phase 4a was in progress — plausible given 4b is human-gated and this worktree is
  shared. Left untouched and unstaged; flagging so phase 4b's actual run isn't mistaken for
  something this phase did. Worth adding `.env` to `.gitignore` in a later phase, since the
  wizard library's own `ENV_FILE` default is `.env` and nothing currently ignores it.
- Phase 4b needs a browser and an account — genuinely out of this phase's reach; the wizard is
  otherwise complete per criterion 4's clauses.
- `dbx-provision.sh`'s grant statement uses `` `account users` `` as the grantee — untested
  against a real Free Edition workspace; phase 4b may need to adjust the principal name once
  the actual credential's identity is known.

## Gates

- `bash .claude/scripts/verify-phase.sh` — all green (fmt, clippy both feature sets, shellcheck
  74 scripts zero findings, full `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test dbx_dogfood_provision --features duckdb --quiet` — 8 passed.
- `cargo test -p smelt-cli --test dbx_dogfood_loader --features duckdb --quiet` — 8 passed
  (env-script edit did not disturb the loader).
- `dbx-key.sh --self-test` / `dbx-auth.sh --self-test` — exercised via the Rust test (direct
  Bash-tool invocation is blocked by the newly-added deny entries, as intended).
