# Phase 1 summary — Spec: the lattice framing

## Shipped

- `docs/specs/incremental_models.md` §Semantics: new `### The contract lattice` after
  §"The equivalence invariant" — states the default point, the single-owner admission triple
  (declaration schema + pure oracle transform + probe emitter, owned in `smelt-logical`), and the
  two v1 points with their restated oracles and probes (`ContractLateArrivalOutsideHorizon`,
  `ContractDeferralExceeded`).
- §Surface: new `### Contract relaxations (`contract:`)` after §"Maintenance overrides" —
  the `contract:` block, `frozen_horizon`'s partition-grain-only restriction, `deferral`'s
  cell-refinement addressing, `ContractFrozenHorizonInvalid`/`ContractDeferralInvalid`.
- §"Windowed maintenance and the horizon": scoped the silent-late-arrival sentence to the
  default point, cross-referenced to the frozen-horizon relaxation.
- §Diagnostics: new "Contract-lattice codes" table (4 codes) in `incremental_models.md`;
  matching "Contract lattice" table + unimplemented note in `docs/specs/diagnostics.md`.
- §Constraints & Invariants: new bullet stating the lattice-point single-owner rule.
- §Known Divergences: new gap-first entry ("The contract lattice is specified and
  unimplemented"), linking this outcome.
- `CLAUDE.md`: new architectural-invariants bullet "Contract-lattice point single ownership".
- New standing gate `crates/smelt-logical/tests/contract_lattice_spec.rs` (8 tests, modeled on
  `output_delta_spec.rs`'s section-extraction pattern).

## Decisions

- Reused `maintenance.cells[]`'s addressing shape (`columns`/`on`) for `contract.cells[]`
  rather than inventing a new refinement grammar — keeps the two override blocks visually and
  mechanically parallel, per the plan's surface decision.
- `frozen_horizon` stays model-level only (no `cells[]` override) since it clamps write
  eligibility for the whole model, not a single cell; `deferral` is refinable per cell.

## For the next planner

- Nothing implemented beyond the spec + gate — phases 2-6 (declaration/validation/clamp wiring,
  the late-arrival diagnostic, deferral + subsumption, the parameterised conformance oracle,
  explain/docs-site surface) are all still `pending`, exactly as scoped.
- The local dev environment's DuckDB shared library lives at `~/.local/lib/duckdb`, not
  `/usr/local/lib` as CLAUDE.md's setup instructions suggest — `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH`/
  `LIBRARY_PATH` all need to point there for `verify-phase.sh` to link. Worth a CLAUDE.md fixup
  or a `.cargo/config.toml` note if this trips future phases too (out of scope for this phase).

## Gates

- `cargo test -p smelt-logical --test contract_lattice_spec` — 8/8 pass.
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — pass.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full workspace test,
  example_diagnostics).
