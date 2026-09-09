# Phase 11 summary — first live BigQuery run: full refresh, refusals recorded

**Deployed against:** project `smelt-bq-test-20260816`, dataset `smelt_dogfood` (the
long-lived dogfood dataset phase 10 loaded, **not** `smelt_test`), via ADC impersonation of
`smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com`. No dataset was created or
touched other than `smelt_dogfood`; no destructive operation was run against BigQuery (see
"What was NOT done" below).

The brief's emphasis is the point of this phase: get as far as possible, and characterise
every refusal precisely rather than fix it. Nothing under `crates/` was changed except
tests (two test files, justified below — both are consequences of *correctly* declaring a
second real target, not workarounds for a bug).

## Task 1 — the target and the source-name reconciliation

### The target

`examples/github_activity/smelt.yml` gained a `bigquery` target:

```yaml
target: dev
targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main
  bigquery:
    type: bigquery
    project: smelt-bq-test-20260816
    dataset: smelt_dogfood
    location: US
    schema: smelt_dogfood
```

**The explicit top-level `target: dev` is load-bearing, not decorative.** Before adding
it, `smelt-runtime`'s `default_target` (`crates/smelt-runtime/src/profile.rs:134-142`)
falls back to the alphabetically-first target name when `Config::target` is unset —
`"bigquery"` sorts before `"dev"`, so simply adding the target would have silently flipped
every no-`--target` invocation (including `crates/smelt-cli/tests/github_activity_replay.rs`,
which calls into `smelt-runtime` directly) onto a dialect with no transactional merge
ledger. Confirmed the hard way: adding the target without pinning `target: dev` broke three
`github_activity_replay` tests (`actor_naming_is_arrival_partitioned`,
`events_enriched_dimension_mutation_cell_technique`,
`repo_naming_is_recognised_as_the_succession_grain`) by silently downgrading
`ColumnScopedMerge`→`PerGroupRecompute` project-wide. Pinning `target: dev` fixed all
three — see "Two test files needed updating" below for the diagnostics-path corollary
this did **not** fix (a genuinely different resolution policy, not the same bug).

### The source-name reconciliation (phase 10 finding 1)

Read `crates/smelt-runtime/src/compile.rs`'s `make_path_ref_resolver_with_ephemerals`
(`compile.rs:1596-1627`) rather than guessing: without an override, `smelt.sources.raw.
github_events` resolves to `<target_schema>.<segs.join("_")>` — on the `bigquery` target
that is `smelt_dogfood.sources_raw_github_events`. Phase 10's loader landed the physical
tables at `smelt_dogfood.github_events` / `smelt_dogfood.github_events_arrival` (no
`sources_raw_` prefix), so the two would not have matched.

**This is not a stopgap.** smelt already has a first-class, spec'd mechanism for exactly
this: the **target-aware `name:` override** (`docs/specs/sources.md` §"Target-aware `name:`
override", `crates/smelt-core/src/sources.rs`'s `SourceInfo::db_name_for_target` /
`SourceNameOverride::PerTarget`). Added to both source YAMLs:

```yaml
# github_events.yml
name:
  bigquery: smelt_dogfood.github_events
# github_events_arrival.yml
name:
  bigquery: smelt_dogfood.github_events_arrival
```

`dev` is deliberately absent from the map — per the spec, an unnamed target falls back to
the default mapping, which already matches the DuckDB fixture's own table names
(`main.sources_raw_github_events`). This closes phase 10's finding 1 for real: the
per-target `name:` override is the intended long-term mechanism, not a placeholder a later
phase must replace. What phase 10 called "no home yet" was actually already-shipped smelt
functionality that just hadn't been wired into this project's source YAMLs.

### Two test files needed updating (both genuine, not workarounds)

Declaring a real second target with real ledger-requiring techniques (`silver.events_deduped`'s
`KeyedFold`/`ColumnScopedMerge`, `silver.repo_naming`/`actor_naming`'s `SuccessionPatch`)
makes `MaintenanceStateDowngraded` fire for real, because that diagnostic is **deliberately**
computed against the union of every declared target's backend type, not the resolved
default (`crates/smelt-db/src/queries/maintenance/diagnostics.rs:284-296`'s own comment:
"Checked against every declared backend... analysis time has no single declared target").
This is orthogonal to the `default_target` fix above — pinning `target: dev` has zero effect
on it, confirmed by reading `parse_active_backends` (`crates/smelt-core/src/config.rs`),
which iterates `config.targets.values()` regardless of `Config::target`.

BigQuery/Spark have no `MergeLedger`/`ReconciliationLedger`/`TombstoneLedger`
(`crates/smelt-logical/src/maintenance/availability/state_structure.rs::
realisable_state_structures`), so four cells legitimately downgrade to their recompute-family
equivalent the moment `bigquery` is declared as a target at all — independent of whether a
BigQuery run ever executes. This surfaced as two previously-green "zero diagnostics" gates
going red:

- `cargo test -p smelt-cli --test example_diagnostics` (`smoke_and_migration::
  github_activity_no_diagnostics`)
- `cargo test -p smelt-lsp --test example_workspaces` (`github_activity`)

Both asserted **zero** diagnostics ever, which no longer holds once a real second backend
with real technique gaps is declared — this is the diagnostic doing its job, not a bug to
chase away. Fixed as **test** changes (permitted by this phase's brief):

- Added `check_workspace_diagnostics_are_exactly` (`crates/smelt-cli/tests/
  example_diagnostics/support.rs`) and pointed `github_activity_no_diagnostics` at the
  four exact expected `MaintenanceStateDowngraded` messages, so a *fifth* diagnostic
  (a regression) or the disappearance of one of these four (an unnoticed behavior change)
  both still fail loudly — this does not degrade into `check_workspace_no_diagnostics`
  with a swallowed exception.
- Added `assert_example_workspace_clean_except` (`crates/smelt-lsp/tests/
  example_workspaces.rs`), filtering only the `maintenance-state-downgraded` code for the
  `github_activity` case, mirroring the pre-existing `property-downgrade` exclusion
  pattern already in that file for the identical "advisory, not a validity statement"
  reason.
- `.claude/large-file-baseline.txt` updated (`--update`) for `example_workspaces.rs`'s
  resulting growth (1485→1512 lines) — a mechanical ratchet consequence of the above, not
  a design change.

All four are genuinely expected static diagnostics, not compile-time refusals — they never
block a run.

## Task 2 — the full refresh

### Setup

Built `smelt` with `cargo build -p smelt-cli --features bigquery` (the default build has no
BigQuery backend at all — `bigquery` gates `smelt-backend-bigquery`). Created the pinned
Python client venv (`bash scripts/bigquery-venv.sh`) — did not exist in this worktree.
Authenticated via `source scripts/bq-dogfood-env.sh` plus
`SMELT_BQ_ACCESS_TOKEN=$(gcloud auth application-default print-access-token)` (the dogfood
path uses ADC impersonation; `python/smelt/bigquery_adapter.py` never falls back to ADC
itself, so the impersonated ADC token has to be minted explicitly and handed to the
adapter via `SMELT_BQ_ACCESS_TOKEN` — this is not written down anywhere else and is worth
recording for the next phase that needs it).

### What ran

```
smelt run --target bigquery --full-refresh --start 2026-08-04 --end 2026-08-07
```

(range chosen to cover every day phase 10 actually loaded: 2026-08-04's redelivery slice
through 2026-08-06).

**`--dry-run` and `--show-plan --dry-run` both refuse immediately**, before compiling
anything:

```
Error: ExternalStepNotInvocable: step 'sources.raw.github_loader' cannot be invoked this
run — this is a dry run — `smelt explain` is the non-refusing preview surface for a step
```

This is documented, expected behavior (`docs/handoffs/2026-09-08-github-activity-findings.md`
finding (c)) — recorded here as confirmation it reproduces exactly as described, not as a
new finding. `smelt explain` doesn't take `--target`, so there is currently no
target-scoped non-refusing preview; used `--show-plan` (without `--dry-run`) instead, which
does invoke the step for real.

**The external step ran, but wrote to the wrong place — a real, if low-severity, gap.**
`models/sources/raw/github_loader.yml`'s `command: ["bash", "load_day.sh", "--date",
"{run_date}"]` has no target-awareness: `StepRunContext` only carries `run_date`/`run_end`
(`crates/smelt-core/src/external_step.rs`), and `load_day.sh` hardcodes its output to
`target/dev.duckdb`. Running against the `bigquery` target still invoked this script,
which happily loaded (or no-op'd, via its own `main._loader_days` idempotency ledger) into
the **local DuckDB file**, achieving nothing for the BigQuery leg (whose `smelt_dogfood`
tables were already populated out-of-band by phase 10's separate loader). It did not error
or block the run — bronze/silver models still read the real `smelt_dogfood.github_events`
via the `name:` override — but it is silent, wasted work that looks like it did something
target-relevant and didn't. **Finding for `20260906-bigquery-correctness`**: an
`external_step:` has no way to know, or be told, which target a run is invoking it for;
a step whose command is inherently target/backend-specific (as this one is — it always
writes to a local DuckDB file) has no declared way to say "only run me for target X" or
receive the active target name as a placeholder.

### First refusal (clean state, fresh 3-day range) — the primary finding

`bronze.events`, and the first (table-creating) write of `silver.events_deduped`,
`silver.actor_naming`, `silver.repo_naming` all **succeeded** — confirmed by direct query
against `smelt_dogfood` (`bronze_events`, `silver_actor_naming`, `silver_events_deduped`,
`silver_repo_naming` all exist as of this writing). The run then failed on the model's
*second* batch (the multi-day range chunks into more than one write per model):

```
smelt: run failed at model 'silver.events_deduped': Feature not supported by BigQuery:
observed-delta recording for a change-suppressed keyed fold (T5)
```

**Classification: compile/runtime backend refusal — a genuine BigQuery capability gap, not
a smelt bug in the sense of incorrect behavior.** Traced to
`crates/smelt-runtime/src/maintenance_driver/driver.rs:632-641`:

```rust
(None, WriteSuppression::Suppressed { compared_columns }) => {
    if backend.dialect() != SqlDialect::DuckDB {
        bail!(
            "{}",
            BackendError::unsupported(
                backend.dialect().name(),
                "observed-delta recording for a change-suppressed keyed \
                 fold (T5)",
            )
        );
    }
    ...
```

This is `Grade::Idempotent`'s recording of a keyed fold's observed output delta (T5,
`docs/specs/incremental_models.md` §"The graph layer" — "Observed deltas on model edges")
past the model's first write, when the merge suppressed a would-be no-op write. The `bail!`
is unconditional on any non-DuckDB dialect — there is no BigQuery (or Spark) implementation
of this bookkeeping at all, so **any keyed model whose merge suppresses an unchanged row on
a second-or-later write blocks the entire run on BigQuery.** `silver.events_deduped` is
upstream of every gold/marts model in this pipeline (confirmed by grep —
`smelt.silver.events_deduped` is read directly or transitively by every model in
`gold/`, `marts/`, and the typed fan-out in `silver/`), so this refusal is a **hard stop for
the whole model set**, not a local one. No workaround exists within the example project:
the model's whole purpose is the dedup shape that trips this path, and rewriting it away
would defeat the point of dogfooding it. Recorded here rather than routed around, per the
brief.

### Second refusal (on retry, against already-populated state) — recorded, but *not* a
BigQuery finding

Retrying with a narrower single-day window (`--start 2026-08-04 --end 2026-08-05`, tables
already existing from the attempt above) hit a **different** refusal on all three silver
models:

```
smelt: run failed at model 'silver.events_deduped': SourceRetentionExceeded: a whole-table
recompute reaches past every finite bound, but stored output already exists and no license
was given for: 'raw.github_events' (retains 3888000 seconds)
```

Traced this to `crates/smelt-runtime/src/execute/retention_admission.rs`'s
`FullRefreshRetentionError` / `crates/smelt-logical/src/maintenance/retention.rs` — **this
is backend-agnostic maintenance-plan logic, not BigQuery-specific**, confirmed by the fact
that none of the touched code lives under a backend crate. It is the documented "Retention
refusal" (`docs/specs/sources.md` §Semantics 5): a keyed model with `allow_full_scan: true`
has an unbounded-by-construction reach on `--full-refresh`, and once **stored output
already exists**, a second `--full-refresh` over a retention-bounded source needs an
explicit license it wasn't given. This would reproduce identically on DuckDB given the same
sequence (a `--full-refresh` re-run over already-materialized output for a retention-bound
source) — **not attempted here to confirm** (a `--database` isolation attempt to check this
cheaply on DuckDB hit an unrelated harness mismatch: `load_day.sh` ignores `--database` and
always writes to `target/dev.duckdb`, so the isolated database saw no source tables at all;
not worth chasing further for this phase). Recorded as an operational fact for whoever runs
this pipeline live going forward: **a second `--full-refresh` against a target that already
has this project's output will refuse**, by design, not as a new bug. This is *not* filed as
a `bigquery-correctness` finding — it belongs nowhere near that outcome, since it is not
about BigQuery at all.

### Current live state (left as-is, not cleaned up)

`smelt_dogfood` now additionally holds `bronze_events`, `silver_actor_naming`,
`silver_events_deduped`, `silver_repo_naming` (all real, non-empty tables from the
successful first-write pass above). **Nothing was dropped** — deleting BigQuery tables was
explicitly out of scope for this phase and the harness refused the attempt when tried. Any
future `--full-refresh` against `bigquery` will hit the retention refusal above until either
these tables are dropped or a full run succeeds all the way through in one pass (which
requires the T5 gap to be fixed first). A **plain incremental `smelt run`** (not
`--full-refresh`) was not attempted this phase — out of the brief's scope, and its
behavior against this same T5 gap is untested; noting this as an open question rather than
assuming it would fare better (the `Grade::Idempotent` observed-delta recording path this
gap lives in is not spelled as full-refresh-only in the code).

### Everything downstream never ran

`gold.repo_dim`, `gold.events_enriched`, `gold.repo_activity_daily`, every model under
`silver/` reading `events_deduped` (`actor_sessions`, `issue_events`, `pr_events`,
`push_events`, `star_events`), and every `marts/` model never executed on BigQuery this
phase — all are downstream of `silver.events_deduped`/`repo_naming`/`actor_naming`, all
three of which now refuse on any further `--full-refresh`. **This is the honest stopping
point**: the BigQuery leg of `github_activity` gets through source ingestion and one
first-write pass of three of eight `silver/` models, then stops hard.

## Cost

Fetched BigQuery job history directly (`bigquery/v2/.../jobs`) rather than estimating.
Every job across both invocations of this phase:

| Job | `totalBytesBilled` |
|---|---|
| 8 query jobs (schema reads, first-write CREATEs, the two failed batches) | 7 × 10,485,760 + 1 × 20,971,520 |
| **Total** | **94,371,840 bytes (≈ 0.0944 GB)** |

At $5/TB on-demand: **≈ $0.00047** — under a tenth of a cent. All models read the already-
loaded ~6,053-row `smelt_dogfood.github_events`/`github_events_arrival`, never
`githubarchive` directly, so every job hit BigQuery's 10 MB per-table minimum-billing floor
rather than scanning real data volume. No dry-run ever approached the 5 GB stop threshold.
Well within the AUD 25/month budget.

## Run report

**None exists — expected, not a gap.** `examples/github_activity/smelt.yml` declares no
`state:` key, so the project runs at the default `state.mode: stateless`
(`docs/specs/run_state.md` §"Stateless writes nothing"): no `.smelt/` directory, no run
manifest, no run report is written for *any* target, DuckDB or BigQuery — confirmed by
`find examples/github_activity -iname '.smelt*'` finding nothing before or after this
phase's runs. The exact console transcripts quoted above (model, statement class, and
verbatim error) are the only run record this phase produces, which is what the brief's
"capture the run report" requirement resolves to for a stateless project. Opting into
`state.mode: intervals` to get real run manifests/reports is out of this phase's scope —
it wasn't needed to characterise the refusals, and doing so would itself be a pipeline
change requiring its own recording per the brief's own rule.

## Findings for `docs/outcomes/20260906-bigquery-correctness`

1. **(Primary) T5 observed-delta recording is DuckDB-only, unconditionally.**
   `crates/smelt-runtime/src/maintenance_driver/driver.rs:634-641` — any keyed model whose
   merge suppresses a no-op write on a second-or-later batch refuses outright on BigQuery
   (and Spark, by the same `!= SqlDialect::DuckDB` check). This is the hard stop for the
   entire `github_activity` model set on BigQuery today: `silver.events_deduped` sits
   upstream of everything. Needs either a BigQuery/Spark realisation of the observed-delta
   bookkeeping, or an explicit, surfaced downgrade path (parallel to the
   `MaintenanceStateDowngraded` mechanism that already exists for ledger-requiring
   techniques) rather than a hard `bail!`.
2. **`external_step:` has no target-awareness.** `StepRunContext` only carries
   `run_date`/`run_end` (`crates/smelt-core/src/external_step.rs`); there is no placeholder
   or env var conveying the active target/backend to a step's `command:`, and no way to
   scope a step to specific targets. `examples/github_activity`'s own loader step is a
   concrete instance: it always writes to a local DuckDB file regardless of which target a
   run is invoking it for, silently doing nothing useful on the BigQuery leg. Also
   confirmed: `smelt-cli` never exposes `ExecuteRequest::invoke_external_steps` as a flag
   (always `true`) and `--dry-run`/`--show-plan --dry-run` refuse outright the moment a
   step would need invoking — there is no "trust this source as already fresh, don't invoke
   the step" escape hatch for a source known to be populated out-of-band (this project's
   exact BigQuery situation). A future phase standing up real scheduling for the BigQuery
   leg will need one of: per-target step scoping, a `{target}` placeholder, or a CLI-level
   skip-with-trust flag.
3. **No non-refusing, target-scoped plan preview exists.** `smelt explain` doesn't take
   `--target`; `--dry-run` refuses the moment an external step is in the DAG. Confirms
   handoff finding (c) is still current, and sharpens it: there is currently no way to
   preview a `--target bigquery` plan without either invoking the loader step for real or
   getting refused.

4. **(Added by the orchestrator) The default-target fallback is alphabetical, so adding a
   target silently changes which backend everything runs against.**
   `crates/smelt-runtime/src/profile.rs`'s `default_target` falls back to the
   alphabetically-first target name when `Config::target` is unset. Adding a `bigquery`
   target to a project whose only target was `dev` therefore re-pointed **every**
   no-`--target` invocation at BigQuery — silently downgrading `ColumnScopedMerge` to
   `PerGroupRecompute` project-wide. It was caught only because three unrelated
   `github_activity_replay` tests failed; nothing announced the change.

   This is filed as a finding rather than left as a code comment because it is a
   fail-loud violation in the sense `CLAUDE.md` uses the term: a config edit that changes
   the execution backend for the whole project should not be inferable from sort order,
   and should not be silent. Candidate resolutions, none chosen here: require an explicit
   `target:` as soon as a project declares more than one; keep the first *declared* target
   rather than the alphabetically-first; or emit a diagnostic when the default is being
   inferred among several. The `target: dev` pin in this example is the local fix and does
   nothing for the next project to hit it.

## Not filed as BigQuery findings (recorded here for completeness only)

- `SourceRetentionExceeded` on a repeated `--full-refresh` against already-materialized
  output — backend-agnostic maintenance-plan behavior (`smelt-logical`/`smelt-runtime`,
  no backend crate involved), working as specced (`docs/specs/sources.md` §Semantics 5).
  Not a `bigquery-correctness` item.

## Gates

- `bash .claude/scripts/verify-phase.sh` — **ALL GREEN** (fmt, clippy both feature sets,
  full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test github_activity_oracle` — 18 passed, 1 ignored (DuckDB
  leg unaffected by this phase's smelt.yml/source-YAML changes).
- `cargo test -p smelt-cli --test github_activity_replay` — 21 passed (was 3 failed before
  pinning `target: dev`; see Task 1).
- `cargo test -p smelt-cli --test example_diagnostics` — including the rewritten
  `github_activity_no_diagnostics`.
- `cargo test -p smelt-lsp --test example_workspaces github_activity` — passes with the new
  narrow `maintenance-state-downgraded` exclusion.
- `.claude/large-file-baseline.txt` updated for `example_workspaces.rs`'s resulting growth.

## What was NOT done

- Nothing was fixed under `crates/` except the two test files above (both required by
  *correctly* declaring a second real target, not workarounds for the T5/retention
  refusals).
- No BigQuery table was dropped or reset — a delete attempt was made to get a clean-slate
  repro and was refused by the harness; the live `smelt_dogfood` state described above
  (six tables total) is what phase 12+ inherits.
- No plain incremental (non-`--full-refresh`) run was attempted.
- `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md` was **not edited** — decision-log
  material (the `target: dev` pinning requirement, the two test-file changes, the T5 finding)
  lives here instead, for the orchestrator to fold in.
- The tree is left dirty, as instructed — nothing was committed.

## Files touched

- `examples/github_activity/smelt.yml` — `target: dev` pin + `bigquery` target.
- `examples/github_activity/models/sources/raw/github_events.yml`,
  `github_events_arrival.yml` — target-aware `name:` overrides.
- `crates/smelt-cli/tests/example_diagnostics/support.rs` — new
  `check_workspace_diagnostics_are_exactly` helper.
- `crates/smelt-cli/tests/example_diagnostics/smoke_and_migration.rs` — rewritten
  `github_activity_no_diagnostics`.
- `crates/smelt-lsp/tests/example_workspaces.rs` — new
  `assert_example_workspace_clean_except` helper + updated `github_activity` test.
- `.claude/large-file-baseline.txt` — ratchet update for the file above.
