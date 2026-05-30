## Drift Report: incremental_models

**Spec**: docs/specs/incremental_models.md (last_reviewed: 2026-05-21)
**Date**: 2026-05-30
**Phase**: A2 (feature-sweep bug-hunt)

### Automated checks
- cargo fmt — PASS
- cargo clippy — PASS (zero warnings)
- cargo test — PASS (full workspace green at pre-flight and after)
- example_diagnostics — PASS (76 passed, 1 ignored)
- example_workspaces — PASS (21 passed; multi-project test now also sees the new `examples/incremental_idempotency/`)
- smelt-planner incremental unit tests — PASS (6)
- smelt-runtime parity — PASS (5)

### Surface drift
- ✅ `incremental:` block (`enabled`, `unique_key`, `safety_overrides`) — `crates/smelt-core/src/config.rs:366-377` (`IncrementalConfig`, `#[serde(deny_unknown_fields)]`).
- ✅ `safety_overrides` — code carries **six** flags (`allow_window_functions`, `allow_having`, `allow_limit`, `allow_subqueries`, `allow_nondeterministic`, `allow_distinct`) at `config.rs:310-323`. The spec's Surface YAML example (lines 28-32) lists only three, but this is an *illustrative* block ("# optional; bypass specific safety checks"); the spec's Semantics §"Safety checks" enumerates all six rejected constructs and the `allow_<check>` pattern, and `docs-site/docs/reference/smelt-yml.md:187-192` + `docs-site/docs/guide/incremental-models.md:231-237` document all six. **Not drift** — surface is fully specified in Semantics and complete in user docs.
- ✅ `timeseries:` is a **separate block** from `incremental:` — `config.rs:354-364` (`TimeseriesConfig`, factored out of `IncrementalConfig`). `ModelConfig` holds them as independent fields.
- ✅ `TimeseriesRequiredForIncremental` diagnostic — enforced at `crates/smelt-planner/src/rules/incremental.rs:138-143` (message differs from the spec's code name but intent matches).
- ✅ CLI `--event-time-start` / `--event-time-end` (both required, end exclusive) and `backbuild` — `crates/smelt-cli/src/main.rs:102-203`; exclusive end documented and now asserted end-to-end (new test).
- ✅ `IncrementalStrategy` enum (`DeleteInsert`/`Append`/`InsertOverwrite`) — `config.rs:335-341`.

### Semantics drift
- ✅ DELETE+INSERT execution — `crates/smelt-backend-duckdb/src/lib.rs` (`delete_partitions`, `insert_into_from_query`), `crates/smelt-runtime/src/transformer.rs` (`inject_time_filter`, exclusive `TimeRange`). Covered by `incremental_test.rs`, `incremental_run_window.rs`.
- ✅ **Idempotence under fixed input (Constraint #7)** and **per-partition equivalence with full refresh (Constraint #6)** — previously tested only at the backend-primitive level. **Now covered end-to-end** by the new `crates/smelt-cli/tests/incremental_idempotency.rs` (drives `smelt run` twice over the same window, then compares against a full-refresh per partition).
- ✅ Batch-safety classification (`FullyBatchSafe`/`BoundedSafe(n)`/`PerPartitionOnly`, 3× context clamped 7–90) — `incremental.rs:22-128`.
- ✅ Per-source bound derivation (`Bounded`/`Unbounded`/`NotDerivable`, Form A/Form B) — `crates/smelt-planner/src/analysis/source_bounds.rs:73-200`.
- ✅ Source-filter pushdown — `transformer.rs:46-99` (`inject_source_filters`).
- ✅ Partition-aligned window check (superset of `partition_column`; bare `OVER` rejected) — `incremental.rs:342-400`; tested by `incremental_refusal.rs`.
- ✅ Safety checks + per-check overrides — `incremental.rs:224-314`; tested by `incremental_refusal.rs`.
- ✅ `partition_column` in SELECT + GROUP BY — `incremental.rs:156-191`.
- ✅ NotDerivable refused at planning time, no silent downgrade (Constraint #10) — `incremental_not_derivable.rs`, `incremental_refusal.rs::test_no_full_refresh_fallback`; `--allow-downgrade` is the explicit opt-out.
- ✅ Misaligned run window rejected (Constraint #8) — `incremental_run_window.rs::test_misaligned_window_rejected` via `smelt_cli::validate_run_window_alignment`.
- ⚠️ **Execution-mechanism detail**: for the probe model the injected window filter compiles as `WHERE event_ts >= start AND event_ts < end` on the source (using the model's `event_time_column`), not the spec's idealized "outer WHERE on `partition_column` + separate source pushdown". The **output is correct** (idempotency + per-partition equivalence both pass), because `event_date = CAST(event_ts AS DATE)` is day-aligned. This sits inside the documented Known Divergence ("Bound derivation and source-filter pushdown run on the outer SQL body, not the expanded CST"); no separate finding.

### Invariant drift
- ✅ Invariants #1–#10 upheld by inspection + the new end-to-end test (#6, #7 now directly asserted).

### Timeless-oracle drift
- ✅ No phase-vocabulary leakage in the spec body. `grep -nE "Phase [A-Z0-9]+"` hits land only in §Known Divergences / §References, each paired with a `docs/plans/...` link — tolerated.

### Freshness
- last_reviewed: 2026-05-21
- One **stale** Known Divergence: line 304 states the time-dimension fields are still read from inside `incremental:` and the `timeseries:`-block migration is "the next plan". The migration is **implemented** (`config.rs`: `TimeseriesConfig` is a separate, factored-out block; `IncrementalConfig` has `deny_unknown_fields` and no time fields). Logged as **BUG-004 (needs-review)** — the loop never edits a spec autonomously.
- Verdict: **mostly fresh**; one stale divergence to retire via `/smelt:spec` in the human pass.

### Summary
- Drift items: 1 (freshness — stale Known Divergence, BUG-004). 0 surface, 0 semantics, 0 invariant.
- Code bugs found: 0 (incremental_models is mature and well-covered).
- Durable coverage added: `examples/incremental_idempotency/` + `crates/smelt-cli/tests/incremental_idempotency.rs` (Constraints #6, #7, exclusive end bound — end-to-end through the binary).
- `smelt_shop_min` re-verification (meta-plan flagged 3 "live" bugs): all three **fixed** — `smelt_shop_idempotency` test passes (idempotent rebuild, seed-as-ref resolution, no aggregate type-narrowing).
- Recommended next step: `/smelt:spec incremental_models` (human pass) to retire the stale line-304 divergence.
