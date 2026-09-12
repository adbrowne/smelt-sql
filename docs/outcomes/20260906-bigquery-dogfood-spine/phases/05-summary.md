# Phase 5 summary — the typed silver fan-out

**Note on scope:** the blocking half of this phase (`sample.sql`'s re-pin to project
`payload`, and the regenerated fixture) landed in a prior commit (`d8cbf875f`) before this
session started. This summary covers only the remainder: the four fan-out models, the
plumbing `payload` needed to reach them, and verification. `outcome.md` itself (status
rows, decision log) is owned by the orchestrator this session and is not edited here —
everything below that belongs in the decision log is called out explicitly.

**Shipped:**
- `examples/github_activity/models/silver/push_events.sql`,
  `.../pr_events.sql`, `.../issue_events.sql`, `.../star_events.sql` — the four models
  criterion 4 names. Each is a plain `WHERE type = ` filter over
  `silver.events_deduped` (Form A relative to it — `event_date` passes through the exact
  column that model already computes), extracting typed fields out of `payload` with
  `JSON_EXTRACT_TEXT` (the function-registry's canonical name; emits `JSON_EXTRACT_STRING`
  on DuckDB, `GET_JSON_OBJECT` on Spark, `JSON_VALUE` on BigQuery — all declared `Text`, so
  every numeric field is `CAST` explicitly rather than inferred from the JSON).
- `examples/github_activity/models/silver/events_deduped.sql` — grew a `payload` column,
  folded under the same `MIN` every other column already uses, so the fan-out has one
  deduped relation to read instead of re-deriving dedup itself.
- `examples/github_activity/models/sources/raw/github_events.yml` — declared the
  `payload VARCHAR` column (the fixture already carried it physically via `setup_sources.sql`'s
  `SELECT *`, but a source's `columns:` list is required and type-checked
  (`docs/specs/sources.md`/`guide/sources`), so nothing downstream could reference it
  without this). `github_events_arrival.yml` was **not** touched — nothing reads `payload`
  from the arrival twin.
- `crates/smelt-cli/tests/github_activity_replay.rs` — two new tests:
  `typed_fan_out_covers_every_event_of_its_type` (each fan-out model's row count equals
  `silver.events_deduped`'s row count for that `type`, over the full 30-day replay) and
  `push_events_extracts_typed_fields_from_payload` (every extracted column is non-NULL,
  and round-trips a direct `JSON_EXTRACT_STRING` over the source `payload`, proving the
  registry emission actually extracts the field it claims rather than a non-NULL
  placeholder). Also fixed an existing test broken by the new column:
  `recurrence_bound_violation_fails_the_run`'s hand-written `INSERT ... SELECT` named all 9
  pre-`payload` columns explicitly; it now names `payload` too (a fixture-shape update, not
  a behavior change to the test itself).
- `examples/github_activity/README.md` — replaced the "currently blocked" paragraph with a
  new "## The typed silver fan-out" section describing what was built, why the field lists
  are small, and the registry function's cross-dialect emission; also fixed two now-stale
  claims ("`payload` not projected" in "## The sample", and the phase-5-as-future-tense
  wording in "## The BigQuery loader").

**Design decisions:**
- Column naming avoids bare `action` as an alias (`pr_action`, `issue_action`,
  `star_action` instead) — `ACTION` is a DuckDB keyword in some grammar positions
  (confirmed directly: `SELECT ... AS action` fails to parse on the DuckDB CLI, `AS
  evt_action` doesn't). Also `git_ref` rather than bare `ref` for the same caution, though
  unconfirmed as an actual conflict — not worth the risk given `smelt.ref()` exists as a
  named construct elsewhere in the language.
- Field lists are deliberately small, chosen by probing the fixture directly (`duckdb ...
  json_keys(payload)`, per-type completeness counts) rather than assumed from GitHub's
  public archive schema docs. The trimmed BigQuery Archive payload in this fixture does
  not match that public schema exactly: `PushEvent` carries no `commits` array or `size`
  (only `repository_id, push_id, ref, head, before`, all 100% present across 60,643 rows),
  and `pull_request` carries no `title`/`user`/`merged` (only
  `url, id, number, head, base`, all 100% present across 366 rows). Every field actually
  extracted was confirmed present at 100% for its event type before being added — the
  fan-out extracts what the fixture reliably offers, not a wishlist.
- `star_events.sql` extracts `star_action` even though it is a fixture-wide constant
  (`"started"`, all 47 rows) — kept because the point of the model is the extraction shape
  itself, not that every column varies in this particular sample.
- `push_events.sql` drops `payload.repository_id` (present but redundant with the event's
  own more-trustworthy `repo_id`, which survives a rename `payload.repository_id` would
  not track).

**Measured:**
- Full 30-day `run_incremental.py` replay: exit 0, 68s, matching the pre-existing 30-day
  redelivery count (1,270 rows) unchanged — payload is inert with respect to the loader
  and dedup logic.
- Fan-out row counts against the fixture's per-type counts (`duckdb ... GROUP BY type`):
  `silver.push_events` 60,643, `silver.pr_events` 366, `silver.issue_events` 128,
  `silver.star_events` 47 — exact matches, no rows dropped or duplicated by the type
  filter.

**No new divergence, no new criterion-8 finding.** `cargo test -p smelt-cli --test
github_activity_oracle` (18 passed, 1 ignored — the measurement-only deep sweep) ran
`every_window_matches_the_full_refresh_oracle` and `no_relation_diverges_unexplained`
against a workspace that now includes all four new tables (the comparator discovers
relations from `information_schema.tables`, so they entered the sweep automatically with
no test-file change needed) and found **zero** divergence on every one of the 30 windows —
`DIVERGENCE_REGISTRY` stays empty, no entry was needed for any of the four new models.
`docs/handoffs/2026-09-08-github-activity-findings.md` needs no update: its four
already-recorded root causes are unrelated to this phase's models, and this phase adds no
fifth. The only thing handed to the next planner is the naming-collision caution above
(`ACTION` as a bare column alias) — not a smelt defect, a real DuckDB grammar restriction,
noted here only so a future model in this fixture doesn't rediscover it the hard way.

**Gates:**
- `cargo test -p smelt-cli --test example_diagnostics` — 128 passed, 1 ignored (0
  diagnostics on `github_activity`).
- `cargo test -p smelt-lsp --test example_workspaces` — 37 passed (`github_activity` among
  them).
- `python3 examples/github_activity/run_incremental.py` — 30-day replay, exit 0.
- `cargo test -p smelt-cli --test github_activity_oracle --features duckdb` — 18 passed, 1
  ignored, 213.70s.
- `cargo test -p smelt-cli --test github_activity_replay --features duckdb` — 21 passed
  (19 pre-existing + 2 new), 109.22s.
- `cargo test -p smelt-cli --test github_activity_loader` — 11 passed, unaffected.
- `bash .claude/scripts/verify-phase.sh` — clippy (both feature sets), the full workspace
  `cargo test` and `example_diagnostics` all PASS; the first run FAILED on `cargo fmt
  --check` alone (a `format!` call in `typed_fan_out_covers_every_event_of_its_type` that
  rustfmt collapses onto one line). Fixed with `cargo fmt --all`; `cargo fmt --all --
  --check` is clean and the test binary recompiles. No ratchet lowered.

**Orchestrator follow-up (not the implementer's work):** three comments referenced
`docs/outcomes/20260906-bigquery-dogfood-spine/phases/05-plan.md`, which does not exist —
this phase was dispatched directly rather than through the loop's plan step, so it has no
plan document. Retargeted to `outcome.md` in `pr_events.sql`, `github_events.yml` and
`github_activity_replay.rs`.
