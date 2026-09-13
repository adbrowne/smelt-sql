# Phase 4a plan — Provisioning tooling, offline

## Objective

Author, and gate with no workspace, the whole credential + provisioning mechanism success
criterion 4 asks for: a wizard that creates the two schemas and mints a scoped credential,
a `dbx-key.sh`/`dbx-auth.sh` pair that stores it gpg-encrypted and decrypts it into the
`token` file `scripts/dbx-dogfood-env.sh` already reads, a `dbx-verify.sh` that demonstrates
reachability *and* the out-of-scope-write refusal, the `.claude/settings.json` deny/allow
split, and the Free-Edition facts sheet phase 4b fills in. After this phase the human step
is "run one wizard and answer its prompts" — nothing else in criterion 4 remains unbuilt.

## Spec delta

None. No user-visible smelt behaviour changes; these are operator scripts outside the
product surface, exactly like `scripts/bigquery-key.sh`. Phase 1 already specified the
`databricks` target shape and the token-never-logged rule these scripts obey.

## Design decisions this phase must honour

- **Credential-agnostic.** Whether Free Edition permits a service principal with an OAuth
  M2M secret, or only a personal access token, is an empirical fact 4b discovers. The
  scripts branch on a `SMELT_DBX_CRED_KIND` (`oauth-m2m` | `pat`) recorded in `config.env`;
  both branches end at the same artefact — `$SMELT_DBX_CONFIG_DIR/token`, a bearer token
  the env script exports. `oauth-m2m` mints a short-lived token from the encrypted
  client-secret on every `dbx-auth.sh` run (the `bigquery-auth.sh` shape); `pat` decrypts a
  long-lived one. `dbx-auth.sh` prints the expiry it knows and never the token.
- **Config dir and key names are phase 3's, not new.** `${SMELT_DBX_CONFIG_DIR:-$HOME/.config/databricks-smelt-dogfood}`,
  `config.env` carrying `SMELT_DBX_HOST`, and a plaintext `token` file. `dbx-dogfood-env.sh`
  is not modified except to also export `SMELT_DBX_ORACLE_SCHEMA` (default
  `smelt_dogfood_oracle`), which phase 3's summary flagged as wired nowhere yet.
- **The wizard is `/wizard`-shaped**, reusing the library block verbatim from
  `scripts/bq-dogfood-provision.sh` (copy above the `STAGES` marker, author stages below).
- **Settings split.** Secret-touching scripts (`dbx-key.sh`, `dbx-auth.sh`,
  `dbx-provision.sh`) go in `permissions.deny` alongside the `bigquery-*` ones, together with
  `Read(//home/andrew/.config/databricks-smelt-dogfood/**)`. Read-only wrappers
  (`dbx-verify.sh`, `dbx-dogfood-loader.sh`, `dbx-query.sh`) go in `permissions.allow`.

## Tests

New `crates/smelt-cli/tests/dbx_dogfood_provision.rs` — all offline, no workspace, no gpg:

1. `every_dbx_script_exists_and_is_shellcheck_clean_shape` — the five scripts exist, start
   with `#!/usr/bin/env bash` and `set -euo pipefail` (the sourced env script excepted).
2. `secret_scripts_are_denied_and_read_only_wrappers_allowed` — parses `.claude/settings.json`
   and asserts each of `dbx-key.sh`/`dbx-auth.sh`/`dbx-provision.sh` matches a `deny` entry,
   each of `dbx-verify.sh`/`dbx-query.sh` a non-denied `allow` entry, and the config dir is
   under `Read(...)` deny. Fails naming the missing side.
3. `no_dbx_script_echoes_the_token` — no `dbx-*` script contains an `echo`/`printf` whose
   argument expands `SMELT_DBX_TOKEN`, `DATABRICKS_TOKEN` or `CLIENT_SECRET` unmasked
   (the `${VAR:+SET}` form is the only permitted mention).
4. `auth_script_supports_both_credential_kinds` — `dbx-auth.sh` dispatches on both
   `oauth-m2m` and `pat`, and errors with a named message on any third value.
5. `provision_wizard_emits_both_schemas_and_scoped_grants` — the wizard's generated SQL
   mentions `smelt_dogfood` and `smelt_dogfood_oracle` and issues `GRANT` only on those two,
   never on `CATALOG workspace` or `ALL PRIVILEGES`.
6. `verify_script_checks_reachability_and_refusal` — `dbx-verify.sh` contains both legs: a
   SELECT against each of the two schemas, and a write outside them that must FAIL for the
   script to exit 0 (assert the inverted exit-status handling is present, not just the SQL).
7. `env_script_exports_the_oracle_schema` — sourcing `scripts/dbx-dogfood-env.sh` in a
   subshell with `SMELT_DBX_CONFIG_DIR` pointed at an empty temp dir exports
   `SMELT_DBX_ORACLE_SCHEMA=smelt_dogfood_oracle` and leaves `SMELT_DBX_HOST` unset.
8. `facts_sheet_has_every_quota_slot_unfilled` — `docs/outcomes/.../free-edition-facts.md`
   exists and every row's value is the literal `TBD (phase 4b)`, so 4b cannot silently skip one.

`dbx-key.sh` and `dbx-auth.sh` additionally get a `--self-test` mode (no workspace, no gpg
prompt) that validates argument handling and config-dir permissions against a temp dir; test
1 invokes it for both.

## Tasks

1. Add `SMELT_DBX_ORACLE_SCHEMA` to `scripts/dbx-dogfood-env.sh` (default `smelt_dogfood_oracle`).
2. Write `scripts/dbx-key.sh` — prompt for credential kind, capture host + secret, gpg
   symmetric-encrypt to `$CONFIG_DIR/secret.gpg` (0600), write `config.env`
   (`SMELT_DBX_HOST`, `SMELT_DBX_CRED_KIND`, `SMELT_DBX_CLIENT_ID` when oauth), shred the
   plaintext on EXIT. `--self-test` short-circuits before any prompt.
3. Write `scripts/dbx-auth.sh` — decrypt, branch on `SMELT_DBX_CRED_KIND`, produce
   `$CONFIG_DIR/token` (0600) plus an expiry stamp; for `oauth-m2m` exchange the client
   credentials at `$SMELT_DBX_HOST/oidc/v1/token` with `curl`; never print the token.
4. Write `scripts/dbx-provision.sh` — the `/wizard` library block copied from
   `bq-dogfood-provision.sh`, then stages: create/confirm the Free Edition workspace and
   capture the host; choose and mint the credential (calls `dbx-key.sh`); create the two
   schemas and the scoped grants via the dbx venv; hand off to `dbx-verify.sh`; prompt the
   human for the four quota facts and write them into the facts sheet.
5. Write `scripts/dbx-query.sh` — a thin read-only `SELECT`-runner over the dbx venv
   (`source dbx-dogfood-env.sh`, one SQL argument, prints rows), the only allow-listed way a
   Claude session touches the workspace.
6. Write `scripts/dbx-verify.sh` — reachability leg (`SELECT 1` + `SHOW TABLES` in both
   schemas) and refusal leg (`CREATE TABLE workspace.default.smelt_probe_<ts>` must fail;
   exit non-zero if it *succeeds*), printing a PASS/FAIL line per leg.
7. Create `docs/outcomes/20260912-databricks-dogfood-spine/free-edition-facts.md` with the
   quota table skeleton (serverless concurrency, cold-start latency, storage limit, session
   idle timeout), every value `TBD (phase 4b)`.
8. Add the deny/allow entries to `.claude/settings.json`.
9. Write `crates/smelt-cli/tests/dbx_dogfood_provision.rs` red-green against tasks 1–8.
10. Document the provisioning sequence in `examples/github_activity/README.md` next to the
    existing Databricks loader section: `dbx-provision.sh` once, `dbx-auth.sh` per session.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be all green (shellcheck covers the five new
  scripts at zero findings).
- `cargo test -p smelt-cli --test dbx_dogfood_provision --features duckdb --quiet` — 8 passed.
- `cargo test -p smelt-cli --test dbx_dogfood_loader --features duckdb --quiet` — still 8
  passed (the env-script edit must not disturb the loader).
- `bash scripts/dbx-key.sh --self-test` and `bash scripts/dbx-auth.sh --self-test` — exit 0
  with no workspace and no prompt. (Run these *before* adding the deny entries, or via the
  deny-exempt path; the deny list is a Claude-session guard, not a shell one.)
- Do **not** run `dbx-provision.sh` or `dbx-verify.sh` for real — they need phase 4b's human.

## Commit message

`outcome(databricks-dogfood-spine): phase 4a builds the provisioning and credential tooling offline`
