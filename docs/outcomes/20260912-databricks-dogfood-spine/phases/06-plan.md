# Phase 6 plan — first live Databricks run: full refresh, refusals recorded

**Advances:** criterion 6 ("Databricks leg, live", the full-refresh half), and
criterion 9 (the first real entries in the findings ledger). Gates phases 7–9.

## Objective

Declare a `databricks` target on `examples/github_activity/` and take the whole
16-model set through one **full refresh** against `workspace.smelt_dogfood` —
the two days phase 5 landed. The deliverable is not a green run: it is a
precise, per-model characterisation of every compile refusal and every runtime
failure, written down rather than fixed. The one exception the outcome allows
stands: a fix is in scope only where *no* run completes at all without it.

## Spec delta

None expected. Phase 1 already specified the target shape
(`docs/specs/smelt_yml.md` §"Target shape", `docs/specs/multi_backend.md`
§Surface); this phase only *uses* it. If the live run shows the specified shape
is wrong (e.g. a key the session builder actually needs), stop and record it as
a finding for phase 10 — do not amend the spec inside a "record, don't fix"
phase.

## Tests

Offline, red-green, before any live call:

1. `github_activity_databricks::the_databricks_target_resolves_sources_to_the_loaded_tables`
   (new `crates/smelt-cli/tests/github_activity_databricks.rs`) — with the
   `databricks` target selected, `smelt.sources.raw.github_events` and
   `…github_events_arrival` resolve to `smelt_dogfood.github_events` /
   `…_arrival`, the physical names phase 5's loader actually wrote — not the
   default `<schema>.sources_raw_*` mapping. Mirrors
   `github_activity_bq_oracle::the_oracle_target_resolves_sources_to_the_shared_tables`.
2. `github_activity_databricks::adding_the_databricks_target_does_not_move_the_default`
   — `default_target` is still `dev` with the new target present (the
   alphabetical-fallback trap `target: dev` already pins; `databricks` sorts
   *first*, ahead of `bigquery`, so this is a live risk again).
3. `github_activity_databricks::the_databricks_target_parses_and_needs_no_credential_to_load`
   — the target block loads, resolves to `BackendType::Databricks`, and carries
   no `warehouse`/`format` key (config.rs refuses those); no network touched.

No live assertion is encoded as a test. Live results are evidence in the
summary, not a gate — phases 7–9 turn them into gates.

## Tasks

1. Add a `databricks` target to `examples/github_activity/smelt.yml`
   (`type: databricks`, `host: ${SMELT_DBX_HOST}`, `token: ${SMELT_DBX_TOKEN}`,
   `catalog: workspace`, `schema: smelt_dogfood`), with a comment stating it
   points at the long-lived dogfood schema and that the loader, not smelt,
   populates it. Leave `target: dev` pinned. Do **not** add the oracle target —
   that is phase 9's, with its own source-name entry.
2. Add `databricks: smelt_dogfood.github_events` /
   `databricks: smelt_dogfood.github_events_arrival` to the `name:` maps in
   `models/sources/raw/github_events.yml` and `github_events_arrival.yml`.
3. Write tests 1–3 red, then green.
4. Run `cargo test -p smelt-cli --test github_activity_replay --quiet` and the
   LSP/diagnostics example gates; a third real target has previously moved
   technique selection and diagnostics-path resolution (BigQuery phase 11
   needed two test files updated). Any such change is *reported in the summary
   with its cause*, never silently re-baselined.
5. **Live, credential first**: `bash scripts/dbx-auth.sh` (refresh — OAuth M2M
   tokens last ~1h), then `source scripts/dbx-dogfood-env.sh`, then
   `bash scripts/dbx-query.sh "SELECT current_user() AS u"`. If the workspace
   is unreachable or the token cannot be minted (gpg-agent TTL, `## Blocked`
   item (b)), **stop and emit `<<PHASE_BLOCKED>>`** — never skip green.
6. **Live, compile only first**: `smelt build --target databricks` (or
   `smelt run --target databricks --dry-run` if that is the available spelling —
   check `--help`, do not guess). Record every `UnsupportedOnBackend` and every
   other compile refusal with the model, the statement and the construct.
7. **Live, full refresh**: `smelt run --target databricks --full-refresh
   --allow-full-refresh`. Capture the run report from
   `examples/github_activity/.smelt/targets/databricks/`. On a runtime failure,
   record it, then continue with the remaining models (`--select` the rest) so
   one early failure does not hide the other fifteen.
8. Read back per-model row counts from Unity Catalog via `scripts/dbx-query.sh`
   for whatever materialised, and note which models produced no relation at all.
9. Write `phases/06-summary.md`: a numbered findings list (model → statement →
   refusal/failure → smallest reproducing SQL), what materialised and its row
   count, any fix made under the "no run completes at all" exception with its
   justification, and any new Free Edition fact for
   `free-edition-facts.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh` — the standing gate; must be green.
- `cargo test -p smelt-cli --test github_activity_databricks --quiet`
- `cargo test -p smelt-cli --test github_activity_replay --quiet`
- `cargo test -p smelt-cli --test example_diagnostics --quiet`
- Live evidence (not a gate): the compile transcript, the full-refresh run
  report, and the read-back row counts, all quoted in the summary.

## Commit message

`outcome(databricks-dogfood-spine): phase 6 takes the model set through a first full refresh on Databricks`
