## Drift Report: planner_integration

**Spec**: docs/specs/planner_integration.md (last_reviewed: 2026-05-05)
**Date**: 2026-05-30
**Phase**: B6 (feature-sweep)

### Automated checks
- cargo fmt — not re-run here (no code changed in this phase); pre-flight suite green
- cargo clippy — not re-run (no code changed)
- cargo test — PASS (pre-flight full suite exit 0)
- example_diagnostics — PASS (87 passed, 1 ignored)
- example_workspaces — PASS (27 passed)
- smelt-planner + show_plan + provenance_validator gates — PASS (planner suite; show_plan 7/7; provenance_validator 5/5)

### Surface drift
- ✅ `show_plan_rules()` order matches spec (PushFilter → Expand → ElideEmpty → EliminateUnusedLeftJoin) — `crates/smelt-planner/src/logical_plan_rules.rs:131`.
- ✅ `smelt build --show-plan` requires a positional file (`crates/smelt-cli/src/commands/build.rs:53`); errors without one (verified: exit 1, "requires a model file path").
- ✅ `--show-plan` is read-only (verified: fs hash unchanged across a run) and deterministic (verified: two runs byte-identical) — Constraints 3 & 4 hold.
- ✅ Four diagnostic codes exist and are wired into `file_diagnostics`: `ProvenanceMismatch`, `JoinsMismatch`, `DeclaredCardinalityUnverifiable`, `MissingProvenancePushdownAdvisory` — emission sites in `crates/smelt-db/src/provenance_validator.rs` + `crates/smelt-db/src/queries/function_diagnostics.rs`; gated on `unstable_schema: true` at `crates/smelt-db/src/lib.rs:1134,1147`.
- ❌ **Frontmatter keys `deterministic` / `idempotent` / `append_only` / `backends` / `joins` / `provenance` are rejected by the model frontmatter parser.** Spec Surface line 37 + architecture.md "Unified frontmatter rule" (line 152: *"The frontmatter parser is shared across all four declaration kinds; the parsing contract is identical"*) say these keys are valid frontmatter on **any** declaration including a model `SELECT`. But `ModelMetadata` (`crates/smelt-core/src/metadata.rs:117`, `#[serde(deny_unknown_fields)]`) does not list them, so a declaration carrying one is rejected wholesale. → **BUG-016** (needs-review).
  - On a **model**: the entire frontmatter block is dropped silently, reverting e.g. `materialization: table` to the project default (`view`) with no diagnostic and exit 0 (verified: model with `materialization: table` + `deterministic: true` reports `materialization: "view"` in `explain --json`).
  - On a **function file**: produces spurious `Warning: …: YAML parse error: unknown field 'provenance'/'backends'/'deterministic'` noise on every CLI invocation (verified on `examples/functions_demo`); harmless to function semantics (planner props are parsed separately by `parse_function_properties`) but a UX defect and the same root cause.

### Semantics drift
- ✅ Rule 6 (provenance validation): `provenance_validator::validate_provenance` — covered by `crates/smelt-db/tests/provenance_validator.rs::{provenance_matches_body_projection, provenance_extra_column_errors, provenance_missing_column_errors}`.
- ✅ Rule 7 (joins validation + cardinality warning): `validate_joins` — covered by `joins_declared_but_body_has_different_join_set`, `joins_cardinality_unverifiable_warning`.
- ✅ Rule 9 (pushdown advisory): `missing_provenance_advisory_for_file` — covered by `crates/smelt-db/tests/phase52_lints.rs` and `broken_function_diagnostics.rs`.
- ✅ Rule 10 (fixed-point loop): `apply_rules_to_fixed_point` — covered by `combined_rule_set_reaches_fixed_point` in `pushdown_tests.rs`.
- ⚠️ The four validation diagnostics surface only through the smelt-db/LSP `file_diagnostics` path; they do **not** surface through `smelt run`/`build`/`type` (verified: a `ProvenanceMismatch` fixture builds clean via CLI). This is the already-logged systemic class (BUG-006/011/015 — run/build pipeline skips `file_diagnostics`); not re-logged.

### Invariant drift
- ✅ Constraint 3 (`format_plan` deterministic) — verified by re-run byte-equality.
- ✅ Constraint 4 (`--show-plan` read-only) — verified by fs-hash equality.
- ✅ Constraint 5 (cycle pre-pass upstream) — `FunctionCallCycle` runs before logical_plan; not re-audited this phase.
- ❌ architecture.md "Unified frontmatter rule" (shared parser, identical contract) — **violated**: two parsers exist (`ModelMetadata` serde vs `smelt_planner::logical::parse_function_properties`) with divergent key sets. See BUG-016.

### Timeless-oracle drift
- ✅ Spec body uses phase vocabulary only inside §Known Divergences (paired with `docs/plans/...` links) and §References → Plans (history). No leakage in Surface/Semantics/Design/Constraints. (Code comments reference "Phase NN" — that's code, not spec/user-doc body; out of scope for the timeless-oracle rule.)

### Freshness
- last_reviewed: 2026-05-05
- The L1/L2/L3 + diagnostic surfaces described are present and wired. The one drift (BUG-016) is a cross-cutting frontmatter-parser issue owned by architecture.md, not a staleness of this spec.
- Verdict: spec is **fresh** for its own surface; BUG-016 is an architecture-invariant violation tracked in the ledger.

### Summary
- Drift items: 1 (1 surface/invariant — BUG-016, the shared-frontmatter-parser violation; needs-review).
- Recommended next step: human review of BUG-016 (resolution options in the ledger). No spec edit; no autonomous fix (touches the Unified frontmatter rule architectural invariant).
