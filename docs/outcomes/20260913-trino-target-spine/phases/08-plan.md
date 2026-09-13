# Phase 8 plan — the capability profile, established by execution

## Objective

Replace phase 6's provisional all-`false` `capabilities()` with a measured
`BackendCapabilities::trino_iceberg()`, populated by executing one probe per matrix flag
against the live coordinator, and write the spec's Trino column in the same commit. Also
measure the two `SqlDialect` *language* properties phase 2 landed conservatively `false`
(`supports_aggregate_filter_clause`, `supports_interval_range_frame`). Advances criterion 5
(and unblocks 7, which needs a non-degenerate profile).

**This phase requires the live tier.** Run `bash scripts/trino-up.sh && source
scripts/trino-env.sh` first. If `SMELT_TRINO_URL` cannot be reached, emit `<<PHASE_BLOCKED>>` —
never land a profile the probes did not produce, and never let the probe suite skip green on
the commit that writes the column.

## Spec delta (first, before code)

- `docs/specs/multi_backend.md` §Surface capability matrix — every `?` in the
  `Trino (Iceberg)` column replaced by the measured value (`✓` / `✗` / the
  `null_safe_equality` spelling). The paragraph beginning "Every Trino cell reads `?`" is
  replaced by one stating the column is measured, naming `scripts/trino-up.sh` as the tier the
  probes ran against and the probe suite as the mechanism, and recording where the prior
  ("Trino sits near Spark (Delta)") held and where it broke.
- `docs/specs/multi_backend.md` §Known Divergences — delete the "**The Trino capability column
  is unmeasured**" entry. The implicit-`Native` emission entry stays (owned by
  `20260913-trino-emission`).
- Two matrix rows — `supports_merge_not_matched_by_source` and
  `supports_staged_relation_group` — name no field on `BackendCapabilities` for *any* backend
  today. Measure and write their Trino cells like the rest, and mark them in the table as
  spec-only rows the constructor conformance gate cannot assert (that pre-existing drift is not
  this phase's to close).
- If a probe shows Trino accepting a construct the *dialect* refuses today, update
  `SqlDialect::supports_aggregate_filter_clause` / `supports_interval_range_frame` and their
  doc comments (which currently say "unmeasured; conservative `false`").
- Every `✗`: quote the measured Trino error verbatim in the outcome's `## Decision log`.

## Tests

- `capability_probes.rs::probe_matches_declared_profile` (new,
  `crates/smelt-backend-trino/tests/`, live-gated) — one probe fn per matrix flag executing the
  statement the flag names in an isolated schema; asserts each measured verdict equals the
  corresponding field of `BackendCapabilities::trino_iceberg()`. Fails naming the flag, the
  statement and the Trino error on divergence.
- `capability_probes.rs::every_capability_field_has_a_probe` — reflection-free totality check: a
  const list of `(flag_name, probe)` pairs is asserted to cover every field named in the spec's
  Trino column; a flag with no probe fails by name (never silently dropped).
- `capability_probes.rs::language_properties_are_measured` — probes `FILTER (WHERE …)` over an
  aggregate and a `RANGE BETWEEN INTERVAL … PRECEDING` frame; asserts the verdicts equal
  `SqlDialect::Trino.supports_aggregate_filter_clause()` / `…_interval_range_frame()`.
- `capability_conformance.rs::every_flag_matches_matrix` (extend, offline) — add the Trino
  column: `BackendCapabilities::trino_iceberg()` asserted cell-by-cell against the spec table,
  same `cell!` shape as the other five backends.
- `trino_spec_freshness.rs::trino_capability_column_is_measured` (replaces
  `trino_capability_column_exists_and_is_unmeasured`) — inverts the gate: **no** Trino cell may
  read `?`, and the Trino column must have exactly as many rows as the DuckDB column.
- `trino_spec_freshness.rs::trino_unmeasured_divergence_is_gone` — §Known Divergences no longer
  claims the column is unmeasured.
- `backend.rs::capabilities_are_provisionally_all_false` — deleted, replaced by
  `capabilities_are_the_measured_profile` (offline: `capabilities()` returns
  `BackendCapabilities::trino_iceberg()` and its `dialect` is `SqlDialect::Trino`).

## Tasks

1. `bash scripts/trino-up.sh && source scripts/trino-env.sh`; confirm reachable or
   `<<PHASE_BLOCKED>>`.
2. Write `crates/smelt-backend-trino/tests/capability_probes.rs` with one probe per flag,
   red against the provisional profile; capture each probe's raw Trino error text.
3. Add `BackendCapabilities::trino_iceberg()` in `crates/smelt-dialect/src/dialect.rs`, filled
   from the probe output only, with `dialect: SqlDialect::Trino`; doc-comment it as measured
   against the pinned tier.
4. Write the spec matrix column, the replacement paragraph, and the §Known Divergences deletion.
5. Update `SqlDialect::Trino`'s two language properties and their doc comments to the measured
   verdicts; update the existing assertions at `crates/smelt-dialect/src/dialect.rs` ~515–516.
6. Point `TrinoBackend::capabilities()` at `trino_iceberg()`; swap the provisional unit test.
7. Replace the `BackendType::Trino => unimplemented!(…)` arm in
   `crates/smelt-maintenance-testkit/src/s_tracker.rs` with
   `(SqlDialect::Trino, BackendCapabilities::trino_iceberg())`.
8. Extend `capability_conformance.rs`; invert the two `trino_spec_freshness.rs` gates.
9. Append the measured-`✗` error quotes and the prior-held/prior-broke verdict to the outcome's
   `## Decision log`; write `phases/08-summary.md`, flagging for phase 9 which flags
   `dialect_and_capabilities` will now see as `true`.

## Verification

- `bash scripts/trino-up.sh && source scripts/trino-env.sh && cargo test -p smelt-backend-trino
  --test capability_probes` — all live, **zero skipped** (paste the count in the summary).
- `cargo test -p smelt-backend-trino --lib`
- `cargo test -p smelt-dialect --test capability_conformance`
- `cargo test -p smelt-cli --test trino_spec_freshness`
- `cargo test -p smelt-dialect --test emission_ownership` (the new constructor must not have
  introduced a printer-side dialect branch)
- `bash .claude/scripts/verify-phase.sh` — run once with `SMELT_TRINO_URL` unset, confirming the
  live legs skip green and the offline conformance gates still pass.

## Commit message

`feat(trino): measure the Iceberg capability profile by execution and write the spec column`
