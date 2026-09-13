# Phase 11a plan — the Asset Bundle and its tooling, offline

## Objective

Land everything criterion 11 asks for that is provable with **no workspace**: the Databricks
CLI pinned and installed through `mise`, a committed Asset Bundle for `examples/github_activity`
declaring one daily-scheduled serverless job (loader task → `smelt run` task) that installs smelt
from a locally-built `bindings = "bin"` wheel and authenticates with the ambient session, the
Volume-resident project/state path, the `scripts/dbx-bundle.sh` wrapper, and a per-PR gate. This
is criterion 11's configuration half; 11b deploys it and proves the three scheduled runs.

## Spec delta

Criterion 11's ambient-credential delta is **already landed** (`docs/specs/smelt_yml.md`
§"Target shape": `token` absent ⇒ ambient credentials; implemented in
`python/smelt/databricks_adapter.py`). The remaining spec edit is one paragraph in
`docs/specs/multi_backend.md` §"Connection security", after the existing redaction rule: the
**in-workspace deployment form** of a `databricks` target — no `token` key at all, `host`
supplied by the job's own environment rather than a developer's config, and the statement that
this form carries no secret for a log line to leak, so the redaction rule is vacuous rather than
relied upon. No behaviour change; no new key. (Timeless-oracle rule: no phase vocabulary.)

## Tests

`crates/smelt-cli/tests/databricks_bundle.rs` (new; parses the committed YAML with `serde_yaml`):

- `bundle_declares_one_daily_scheduled_serverless_job` — exactly one job resource, with a
  `schedule` whose cron fires daily and an environment with no cluster/`node_type` key
  (serverless), so a paid-tier compute shape cannot creep in unnoticed.
- `bundle_tasks_are_loader_then_smelt_run_in_order` — two tasks; the `smelt run` task
  `depends_on` the loader task, and nothing else does.
- `bundle_smelt_comes_from_the_locally_built_wheel` — an `artifacts:` entry of type
  `whl` built by maturin from the repo root `pyproject.toml`, referenced by the job
  environment's dependencies, and **no** pinned `smelt-sql==` PyPI dependency (the swap is a
  named comment, asserted present so the placeholder cannot be forgotten).
- `bundle_job_target_is_ambient_and_carries_no_literal_credential` — the smelt target the job
  invokes declares no `token:` key, and no file in the bundle contains a `dapi`/`https://…token`
  literal or a `${ENV}`-interpolated secret.
- `bundle_project_and_state_live_on_a_unity_catalog_volume` — the `smelt run` task's working
  directory (and therefore `.smelt/`) is a `/Volumes/<catalog>/<schema>/…` path, and that path
  is derived from the bundle target's catalog/schema variables rather than hard-coded twice.
- `bundle_targets_name_the_dogfood_workspace_as_a_target_entry` — at least one `targets:` entry
  whose host comes from a bundle variable, so a second workspace is an entry, not a fork.
- `databricks_bundle_validate_is_clean` — shells out to `databricks bundle validate` in
  `examples/github_activity/`; asserts exit 0 and fails on any nonzero. Skips green **only**
  when `databricks` is absent from `PATH`, printing the `mise run setup-databricks` remedy
  (the gcloud precedent); the structural tests above always run, so the suite is never vacuous.

`crates/smelt-core/tests/databricks_docs_freshness.rs` (extend): the docs-site Databricks section
names the bundle deploy path (`databricks bundle deploy`/`run`) and the Volume state location, so
the guide cannot drift from the committed bundle.

## Tasks

1. Add `scripts/mise-setup-databricks.sh` + `scripts/mise-databricks-bin-dir.sh` (mirroring the
   gcloud pair), a pinned CLI version, `[tasks.setup-databricks]` and the `_.path` entry in
   `mise.toml`; run it and confirm `databricks --version` matches the pin.
2. Write `examples/github_activity/databricks.yml` (bundle name, variables for host/catalog/
   schema/Volume, `artifacts:` wheel, `targets:` with the dogfood workspace) and
   `examples/github_activity/resources/github_activity_job.yml` (one job: daily schedule,
   serverless environment, loader task then `smelt run` task, working directory on the Volume).
3. Add a `databricks_job` target (or reuse `databricks` with no `token:`) to
   `examples/github_activity/smelt.yml` for the ambient form, with a comment saying why; verify
   no existing offline test regresses on the new target name.
4. Write the tests above red, then make them pass.
5. Add `scripts/dbx-bundle.sh` — the only caller of `bundle validate`/`deploy`/`run`, refusing
   to run when the CLI is unpinned — and its two `.claude/settings.json` allow-list entries.
6. Make the spec edit named above, then the `### Databricks` docs-site subsection covering
   deploy, the ambient form, the Volume state path and the wheel placeholder.
7. Add the `mise run setup-databricks` line to `CLAUDE.md`'s setup block.
8. Write `phases/11a-summary.md`, naming for 11b: the exact deploy command, the Volume path, and
   the open live question of whether Volume FUSE supports `.smelt/lock` and rename-atomic writes.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, shellcheck, full test
  run, `example_diagnostics`) — all green, no ratchet lowered.
- `cargo test -p smelt-cli --test databricks_bundle` and
  `cargo test -p smelt-core --test databricks_docs_freshness`.
- `bash scripts/dbx-bundle.sh validate` from a clean shell with no credentials — proves the
  per-PR gate needs no workspace.
- `cargo test -p smelt-cli --test github_activity_dual_target` unchanged (new target name must
  not disturb the existing sweeps).

## Commit message

`outcome(databricks-dogfood-spine): phase 11a commits the Asset Bundle, mise-pinned CLI and offline validate gate`
