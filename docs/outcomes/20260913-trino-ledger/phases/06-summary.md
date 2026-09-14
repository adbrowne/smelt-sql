# Phase 6 summary — The three lattice points on Trino: `deferral` refuses, `frozen_horizon` and `retain_departed` proceed

**Shipped:**
- `docs/specs/state.md` §"Declarations stay fail-loud": the converse rule stated — a declaration
  the SQL upholds stays valid even where its *verification probe* has no emission on the target
  dialect; the run proceeds and the skip is a run-time warning naming the model, dialect and
  probe. `contract.frozen_horizon` named as the instance; `contract.deferral` restated as the
  sole refusing declaration. `docs/specs/multi_backend.md` §"Incremental & schema evolution per
  backend": matching one-liner for Trino.
- `crates/smelt-db/tests/maintenance_diagnostics/status_and_contract.rs`: 4 new tests —
  `deferral_on_trino_refuses_declared_contract_requires_state`,
  `cell_level_deferral_on_trino_refuses`, `frozen_horizon_on_trino_is_admitted`,
  `retain_departed_on_trino_is_admitted`. All green on arrival (the analysis-time
  `DeclaredContractRequiresState` derivation was already fully dialect-driven, as the plan
  predicted) — standing gates now, not just a prediction.
- `crates/smelt-logical/src/contract/point.rs`: `required_state_structure_is_dialect_free` test —
  the point → structure map takes no dialect argument, exhaustive over the 3 `ContractPoint`
  variants; no new lattice point for Trino.
- **The real gap, found and fixed**: `crates/smelt-runtime/src/execute/project/mod.rs`'s batch
  loop resolved `smelt_backend::maintenance_dialect(backend.dialect())?` unconditionally at TWO
  sites before checking whether the model declared anything — the frozen-horizon probe dispatch
  (the plan's target) AND, discovered while making the RED test pass, the sibling
  `declared_model_probes` call for `timeseries.assert_monotonic`/`functional_dependencies`/
  `bounded_domain` (same unconditional shape, same batch loop, hit by literally any clocked
  model's batch write regardless of contract-lattice use at all). Both now gate lazily:
  - `crates/smelt-runtime/src/contract_probes.rs`: new `resolve_frozen_horizon_dialect(model,
    dialect, metadata) -> Option<MaintenanceDialect>` — `None` immediately if `contract.
    frozen_horizon` isn't declared; where declared but the dialect has none, logs `tracing::warn!`
    (model, dialect, probe) and returns `None`. `execute/project/mod.rs`'s frozen-horizon block
    now calls this instead of resolving the dialect inline.
  - `execute/project/mod.rs`'s batch-loop `declared_model_probes` call now gated by
    `crate::model_probes::any_declared_probe(...)`, mirroring the full-refresh site's existing
    gate a few hundred lines below (that site was already correct; the batch-loop counterpart was
    the miss).
- `crates/smelt-runtime/tests/trino_contract_points.rs` (new): 4 unit tests against
  `resolve_frozen_horizon_dialect` directly — undeclared on Trino/DuckDB both return `None`
  without ever asking `maintenance_dialect`; declared on DuckDB resolves normally
  (non-vacuity); declared on Trino returns `None` AND logs exactly one WARN naming the model,
  `frozen_horizon`, and `Trino` (captured via a global `tracing_subscriber::Layer`, mirroring
  `fingerprint_sidecar.rs`'s harness).
- `crates/smelt-cli/tests/trino_explain_downgrade.rs`: new
  `explain_on_trino_reports_the_deferral_refusal` — `smelt rebuild --dry-run` (which runs the
  diagnostic-parity gate) on a `contract.deferral`-declaring model against a `trino` target exits
  non-zero and names `DeclaredContractRequiresState`, `contract.deferral`, and `trino` in stderr.
- `.claude/large-file-baseline.txt`: `execute/project/mod.rs` 5064 → 5098, with a sign-off note
  (the two gate fixes above, grown in place — not a new abstraction).

**Decisions:**
- **Test 8 targets `smelt rebuild --dry-run`, not `smelt explain`, despite the plan's function
  name living in `trino_explain_downgrade.rs`.** Verified empirically: `smelt explain --json`
  builds the `MaintenancePlan` directly via `smelt-logical` and never runs the `smelt-db`
  diagnostic gate at all — a `contract.deferral`-declaring model on Trino shows its
  `contract_point` in the JSON with NO `DeclaredContractRequiresState` refusal anywhere,
  confirmed by hand before writing the test. `smelt rebuild --dry-run` DOES run the
  diagnostic-parity gate (the file's own `dry_run_on_trino_names_the_gap` docstring already notes
  "runs for dry_run too"), so it is the CLI entry point that actually proves the refusal reaches
  the CLI boundary, not just `smelt-db`. Kept the plan's file location (matches the file's
  existing Trino-explain fixture helpers) and function name.
- **Tests 6–7 test `resolve_frozen_horizon_dialect` directly, not a full live `execute_project`
  run**, contrary to the plan's "compiled/executed against a Trino target stub" framing. Found
  while making the RED test pass: Trino has NO `MaintenanceDialect` mapping at all yet
  (`crates/smelt-backend/src/lib.rs`'s `UnsupportedMaintenanceDialect` — `20260913-trino-
  incremental`'s subject), so ANY live incremental batch write on Trino — declared or not —
  currently hard-errors downstream at the actual DELETE+INSERT emission site
  (`execute_delete_insert_with_delta_restriction` and siblings), by design, not a bug. A full
  `execute_project` run therefore cannot complete for ANY clocked/incremental model on Trino
  today regardless of this phase's fix, making that style of test structurally unable to isolate
  the frozen-horizon gap specifically. Extracted the guard into `resolve_frozen_horizon_dialect`
  — a small, directly unit-testable pure/logging boundary — rather than force a live pipeline run
  through machinery this phase doesn't own.

**For the next planner:**
- **Sibling `maintenance_dialect(backend.dialect())?` sites audited, not all fixed.** Of the ~13
  occurrences in `execute/project/mod.rs`, only the two above were confirmed hit by this phase's
  test fixtures and fixed. The rest (source-posture probes ~L3499, succession probes ~L2581,
  fingerprint-sidecar diffing ~L3893/3990/4074, column-scoped merge ~L4399/4439) are all inside
  the ACTUAL incremental-write path, which cannot complete on Trino today regardless — they are
  `20260913-trino-incremental`'s subject (adding Trino's `MaintenanceDialect` variant and ~150 SQL
  spellings), not this phase's. Do not fix them piecemeal; T4 should audit and fix the whole set
  in one pass once the dialect exists to resolve them against.
- **Phase 7's "staged relation group without temp tables" and phase 8's "locking/versioning" both
  presuppose a working Trino write path** — worth flagging to that phase's planner: if T4
  (trino-incremental) hasn't landed yet when phase 7 starts, phase 7's live-write proof may hit
  the same `UnsupportedMaintenanceDialect` wall this phase did, and should test at whatever seam
  is actually reachable rather than forcing a full run.
- `retain_departed` needed NO code fix — it was never wired to `required_state_structure` at all
  (only `ContractPoint::Deferral` is), so "admitted on Trino" was true before this phase touched
  anything; the new test is non-regression coverage, not a fix.

**Gates:**
- `cargo test -p smelt-db --test maintenance_diagnostics` — 44 passed.
- `cargo test -p smelt-logical --test maintenance_availability` — 32 passed.
- `cargo test -p smelt-logical --lib contract::` — 50 passed.
- `cargo test -p smelt-runtime --test trino_contract_points --test availability_seam` — 14 passed.
- `cargo test -p smelt-cli --test trino_explain_downgrade --test trino_spec_freshness` — 10
  passed.
- `bash .claude/scripts/large-file-check.sh` — OK (baseline updated with sign-off note).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full `cargo test` workspace, `example_diagnostics`).
