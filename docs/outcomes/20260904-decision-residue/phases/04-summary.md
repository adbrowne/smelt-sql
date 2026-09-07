# Phase 4 summary — `KeyedRecurrenceDeclarationMismatch` + order-independent key sets

**Shipped:**
- `LocalityRefusal::RecurrenceDeclarationMismatch` (`crates/smelt-logical/src/maintenance/locality.rs`) — fires in route 3's statically-derived branch when a declared `key_recurrence` (matching key) disagrees with the derived bound; falls through to admit the derived `Window` slice unaffected when the declaration agrees or names a different key.
- `key_sets_match` + `format_seconds` helpers in `locality.rs`, used by both route-3 sub-routes.
- `Refusal::KeyedRecurrenceDeclarationMismatch` + `recurrence_mismatch_plan` (`maintenance/mod.rs`), routed at the `establish_locality` call site in `crates/smelt-db/src/queries/maintenance.rs`.
- `DiagnosticCode::KeyedRecurrenceDeclarationMismatch` (Error), mapped in `smelt-db/src/lib.rs`; LSP slug `keyed-recurrence-declaration-mismatch` in `smelt-lsp/src/backend.rs`.
- `propagate.rs`'s `push_keyed_dirt` now compares `keys` as a set, not an ordered `Vec` — the one real order-sensitive site the audit (task 4 of the plan) found.
- Tests: 6 new `locality.rs` unit tests (mismatch/agree/different-key/underivable/permutation), 1 `propagate.rs` unit test, `crates/smelt-db/tests/keyed_recurrence_declaration_mismatch.rs` (2 tests), 1 LSP slug test, 1 `smelt-runtime` end-to-end test (`disagreeing_declaration_refuses_the_run`, full `execute_project` path).
- Spec bullets deleted: `diagnostics.md`'s "Specified, unimplemented" clause on the catalogue row and its Known Divergences bullet; `incremental_shapes.md`'s "Key-grain rule 16 … unimplemented" sentence.

**Decisions:**
- The mismatch fires only when derivation *succeeds* (route 3's static sub-route proves a bound) — an underivable model still takes the declared, checked route unchanged (plan task/test 4).
- An agreeing declaration admits the *derived* `Window{recurrence_bounded:true}`, not the checked `RecurrenceBounded` — the declaration is a check per rule 16, never the route.
- Routed through a new `Refusal`/`DiagnosticCode` variant rather than reusing `LocalityNotEstablished`/`KeyedForbidsTimeseries` — this is a value disagreement, not "no route applies", and the spec names its own code.
- `key_sets_match` normalizes case in addition to order, tightening the existing (order-independent but case-sensitive) comparisons at both route-3 sub-route sites to match rule 16's stated clause.

**For the next planner:**
- No residue surfaced beyond the plan's own scope. Phase 5 (retire `data_latency`) and phase 6 (append-only posture probe) are next per the phase table.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full workspace `cargo test`, `example_diagnostics`). One transient failure on the first attempt (`smelt-lsp --test example_workspaces` LSP-init timeout under concurrent build load) reproduced as green in isolation — confirmed environmental flake, not a regression.
- `cargo test -p smelt-logical --lib maintenance::locality maintenance::propagate` — green (42 + 37 passed).
- `cargo test -p smelt-db --test keyed_recurrence_declaration_mismatch` — green (2 passed).
- `cargo test -p smelt-runtime --test locality_route3_recurrence_check` — green (4 passed).
- `cargo test -p smelt-logical --test walk_coverage` — green.
- `cargo test -p smelt-cli --test partition_residue_probes --features duckdb` — green, count unchanged (2).
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb composed` — green (4 passed).
