# Phase 06 summary — Whole-table recompute against a trimmed source

**Shipped:**
- `docs/specs/sources.md` §Semantics 5, `docs/specs/diagnostics.md`, `docs/specs/model_properties.md` §"Reach versus retained history": the whole-table-recompute retention gate is specified — refuse with stored output and no license, license (with a recorded loss) for a first build, a smelt-forced refresh, or `--allow-full-refresh`.
- `crates/smelt-logical/src/maintenance/retention.rs`: `RetainedSource`, `FullRefreshLicense{None,Explicit,Forced}`, `FullRefreshRetention{Admit,Licensed,Refuse}`, and the pure total `full_refresh_retention_verdict` (6 tests, `full_refresh_tests` module), re-exported from `maintenance`.
- `crates/smelt-runtime/src/execute/retention_admission.rs`: `model_source_retentions` (mirrors `key_addressed::build_maint_source_facts`'s ref→bare-name mapping, delegates to `smelt_db::queries::maintenance::build_source_retentions`), `FullRefreshRetentionError`, `full_refresh_loss_warning_message`, `check_full_refresh_retention`.
- One call site in `execute/project/mod.rs`, after the definition-change trigger settles `force_full_refresh` and before the keyed dispatch.
- `--allow-full-refresh` on `BuildArgs`/`RebuildArgs` (previously hardcoded `false`), wired through into `ExecuteRequest`; `smelt build`'s `ExecuteRequest` construction factored into a testable `build_request` fn (mirrors `rebuild.rs`'s existing `build_rebuild_request`).
- `crates/smelt-runtime/tests/retention_full_refresh.rs` — 4 real-DuckDB tests (refuse-and-leave-intact, licensed-reports-once, first-build-licensed, forward-only-unaffected).

**Decisions:**
- Gated on `plan.refresh == RefreshStrategy::Incremental`, **not** `plan.incremental.is_some()` as the plan's task 4 literally said. Discovered red: `build_model_plans`' own window-resolution fallback sets `plan.incremental: None` precisely when `request.full_refresh` is requested with no explicit `--start`/`--end` (the exact case this gate exists to catch) — `plan.incremental.is_some()` is true for an *ordinary bounded* incremental run, not for the whole-table-refresh shape. `plan.refresh` is refresh-strategy-derived and untouched by window resolution, matching the field's own doc comment ("materialized_view models... always land in the None arm of the plan.incremental match; this field is what that arm consults").
- `license` derivation: `Explicit` iff `request.allow_full_refresh`; `Forced` iff `force_full_refresh` (schema-evolution/definition-delta paths); otherwise `None`. `request.rebuild` (`smelt rebuild`) is *not* itself a license — an upstream-closure rebuild over a model with stored state and a retained source still needs `--allow-full-refresh`, matching the outcome's decision log ("reusing the existing `--allow-full-refresh` flag").
- `stored_state` read via `backend.table_exists`, defaulting to `false` on a query error (`unwrap_or(false)`) — errs toward licensing (never destroys anything unrecoverable on an inspection failure) rather than refusing a run that can't even check.
- Reused `SourceRetentionExceeded`/`SourceRetentionDowngraded` wording for the runtime-only error/warning text rather than minting new diagnostic codes — this gate is a run-time decision (depends on request flags and live table existence), never a plan-time `Diagnostic`, so no `DiagnosticCode` variant or `MetadataError` exhaustiveness entry was needed.

**For the next planner:**
- Audited every `--full-refresh` invocation over `examples/github_activity` (`github_activity_oracle.rs`, `github_activity_replay.rs`) — all stage a **fresh** workspace/db per call, so every one is a first build (licensed automatically); no fixture or test needed updating. Confirmed by full-suite green.
- Row 7 (keyed-grain coverage, `driving_source_granularity: None`) is untouched by this phase — this gate reads declared `retention:` sources directly, never through `derive_model_retention_plan`'s locality-refused-plan short-circuit, so it has no grain-shape gap of its own.
- Not covered here: `smelt-ui`'s `run_manager.rs` still hardcodes `allow_full_refresh: false` — the UI has no equivalent of `--allow-full-refresh` yet. Out of this phase's scope (CLI-only per the plan's test list); flagged for whoever next touches UI run requests.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test walk_coverage` — 14 passed.
- `cargo test -p smelt-runtime --test retention_full_refresh --test retention_admission --test execute_parity --test statement_parity --test availability_seam` — 6+4+3+4+41 passed.
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — passed.
- `cargo test -p smelt-cli --test example_diagnostics` — 128 passed, 1 ignored (pre-existing).
- `bash .claude/scripts/large-file-check.sh` — green after `--update` (execute/project/mod.rs baseline 4827→4874 lines; sign-off: the +47 lines are the one call site the plan's task 4 requires, no other growth).
