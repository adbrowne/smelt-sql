# Plan: Production W1 — fail-loud closure + CLI exit-code contract

**Date**: 2026-07-19
**Spec**: [`docs/specs/cli.md`](../specs/cli.md), [`docs/specs/materialized_view.md`](../specs/materialized_view.md), [`docs/specs/architecture.md`](../specs/architecture.md) §"Fail-loud discipline"
**Spec diff**: Phase 1 of this plan (exit-code contract formalization in `cli.md`); other phases close divergences against already-landed spec text
**Tracking PR / branch**: `worktree-production`
**Docs**: code+docs
**Master**: [`docs/plans/20260719-production-readiness.md`](20260719-production-readiness.md) (sub-plan W1)

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/cli.md` (§"Exit codes"), `docs/specs/materialized_view.md` (§"No silent fallback"), and `docs/specs/architecture.md` §"Fail-loud discipline" — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-production`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Red-green TDD; real-fixture tests where the phase has user-visible behavior.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call; failures-only output).
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Timeless-oracle rule: spec and docs-site edits carry no phase vocabulary.

---

## Context

Research basis: `docs/research/20260719-production-release-review.md` (blocker #7 and the "exit codes non-standardized" + "2 `error`-Unknown sites" secondary items). The fail-loud story is nearly closed already: `refresh: materialized_view` hard-errors at compile time (pinned by `crates/smelt-runtime/tests/materialized_view_hard_error.rs`), and both remaining `error`-classified `DataType::Unknown` census sites already emit their diagnostics (`TypeMismatch` for mixed-tz arithmetic, `NonPortableCollation` for non-binary `COLLATE`). What survives is residue: a dead, silently-falling-back backend trait default (`create_materialized_view_as` at `crates/smelt-backend/src/lib.rs:479` — zero callers), the un-pinned diagnostic pairing at the two census sites, and an exit-code surface that is real but scattered (four `std::process::exit(1)` call sites; config errors indistinguishable from detected failures). This plan deletes the residue, pins what exists, and formalizes the exit-code contract.

## Scope

### In scope (spec coverage)
- `cli.md` §"Exit codes": promote the informal "exit codes are meaningful" bullet (currently §Design line ~404) into a normative contract section: `0` success, `1` detected failure (build model failure / test failure / `error`-severity check violations / `diff` change), `2` usage or config/workspace error.
- `materialized_view.md` §"No silent fallback": remove the last warning-only fallback surface from the backend trait.
- `architecture.md` §"Fail-loud discipline": drive the Unknown-census `error` count to zero by pinning diagnostic pairing and reclassifying.

### Explicitly deferred
- `smelt init` (the `errors.rs` hint) — W3's scope (adoption surface).
- Run-report artifact / structured failure output — W2's scope (operability).
- Any new exit codes beyond `0/1/2` (e.g. distinct codes per failure family) — no consumer asks for them; revisit on demand.

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| 1     | done    | (this commit) | 2026-07-19 |
| 2     | done    | (this commit) | 2026-07-19 |
| 3     | pending |        |      |
| 4     | pending |        |      |

### Phase 1: Spec diff — normative exit-code contract in `cli.md`

**Goal.** One normative §"Exit codes" section in `docs/specs/cli.md` that the implementation phases (and external orchestrators) can cite.

**Pre-conditions.** None. Docs-only phase (no code).

**TDD tests to write first.** None (docs-only). The contract lands as spec text; Phase 4 writes the tests against it.

**Implementation shape.** Add §"Exit codes" near the top-level command semantics: `0` = success (including `warn`-severity check violations); `1` = detected failure — failed model build, failed `smelt test` case, `error`-severity `smelt check` violation, `smelt diff` detecting a change, unbuilt check target (`CheckTargetNotBuilt`); `2` = usage error (clap), config/workspace load error (bad `smelt.yml`, unresolvable project). Fold the existing scattered statements (the `smelt check` exit paragraph, the empty-output cases at ~§227, the Design bullet at ~§404) into references to the new section rather than restating. Record the current divergence (config errors exit `1` today) under Known Divergences, linked to this plan.

**Critical files (allowed to touch in this phase).**
- `docs/specs/cli.md` — new §"Exit codes"; cross-reference sweep of existing exit-code mentions.

**Docs touched.** *(timeless)*
- `docs/specs/cli.md` only; the docs-site page updates ride with Phase 4 when behavior actually changes.

**Review checklist** (material findings only):
- [ ] Contract covers every subcommand that can exit non-zero (`build`, `test`, `check`, `diff`, usage/config paths)
- [ ] Existing spec statements now reference, not restate, the contract
- [ ] Current `config-error → 1` behavior recorded under Known Divergences with a plan link
- [ ] No phase vocabulary in spec body

**Commit.** `docs(spec): normative exit-code contract in cli.md`

### Phase 2: Delete the silent materialized-view fallback trait surface

**Goal.** Remove the last warning-only fallback: the dead `create_materialized_view_as` / `drop_materialized_view_if_exists` trait defaults in `crates/smelt-backend/src/lib.rs` (zero call sites; the compile-time hard error makes them unreachable).

**Pre-conditions.** Confirm zero callers still holds: `rg -n 'create_materialized_view_as|drop_materialized_view_if_exists' crates/` must show only the trait definition. If a caller appeared since 2026-07-19, stop and reshape this phase (convert the default to `Err(BackendError::…)` instead of deleting) before proceeding.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/materialized_view_hard_error.rs::no_silent_fallback_surface_in_backend_crate` — source-scan assertion (same style as the `hardening_budget` gates) that `crates/smelt-backend/src/` contains no `falling back` warn-path and no `create_materialized_view_as` symbol. Red while the trait default exists.

**Implementation shape.** Delete both default methods from the `Backend` trait. If `drop_materialized_view_if_exists` turns out to have a live caller in a backend impl, keep it but remove the *silent* semantics doc and the `create_…` sibling; the test only pins the fallback path. No behavior change is possible for users — the surface is dead — so this is a pure fail-loud closure.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-backend/src/lib.rs` — remove the two trait defaults (and their `warn!`).
- `crates/smelt-runtime/tests/materialized_view_hard_error.rs` — the new pinning test.

**Docs touched.** *(timeless)*
- `docs/specs/materialized_view.md` — if §"No silent fallback" mentions the backend-trait fallback as a Known Divergence, delete that entry; otherwise no edit.

**Review checklist** (material findings only):
- [ ] `rg 'falling back' crates/smelt-backend/` is empty
- [ ] No backend impl or test lost behavior (workspace compiles; `verify-phase.sh` green)
- [ ] `materialized_view.md` Known Divergences no longer claims a fallback exists

**Commit.** `fix(backend): delete dead silent materialized-view fallback trait defaults`

### Phase 3: Pin diagnostic pairing at the two `error`-classified Unknown sites; drive census `error` count to zero

**Goal.** Prove (with standing tests) that the two `error`-classified `DataType::Unknown` construction sites never fire without their diagnostic, then reclassify both `legitimate` so `.claude/unknown-census.toml` has zero `error` entries.

**Pre-conditions.** The two sites are `crates/smelt-db/src/type_inference/binary.rs:496` (mixed-tz timestamp subtraction → `Unknown(Unresolved)`, diagnostic `TypeMismatch` via `check_mixed_tz_arithmetic_diagnostics`) and `crates/smelt-db/src/type_inference/collation.rs:90` (non-binary `COLLATE` → `Unknown(Unresolved)`, diagnostic `NonPortableCollation` via `check_collation_diagnostics`). Line numbers may have drifted; re-locate via `rg -n 'classification = "error"' -B6 .claude/unknown-census.toml`.

**TDD tests to write first.**
- `crates/smelt-db/tests/check_diagnostics.rs::mixed_tz_subtraction_unknown_is_paired_with_type_mismatch` — a model computing `ts_naive - ts_tz`: asserts the inferred column type is `Unknown` **and** a `TypeMismatch` diagnostic is present on the expression range (the pairing, not just each half).
- `crates/smelt-db/tests/check_diagnostics.rs::non_binary_collate_unknown_is_paired_with_non_portable_collation` — a model with `col COLLATE NOCASE` (or any non-binary collation): asserts `Unknown` inference **and** the `NonPortableCollation` diagnostic on the `COLLATE` range.
- `crates/smelt-types/tests/unknown_census.rs::no_error_classified_sites_remain` — asserts the parsed census contains zero `classification = "error"` entries. Red until the reclassification lands.

**Implementation shape.** No inference changes expected — the diagnostics already exist. The work is the two pairing tests, then editing `.claude/unknown-census.toml`: flip both entries to `classification = "legitimate"` with reasons stating the pairing test by name (the census header's rule that `error` ⇒ `discriminant = "unresolved"` no longer binds them; keep the discriminant field accurate). If a pairing test exposes a path where the Unknown escapes *without* the diagnostic (e.g. a nesting the checker misses), fix the checker to cover it before reclassifying — that is the point of the phase.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/tests/check_diagnostics.rs` — the two pairing tests.
- `crates/smelt-types/tests/unknown_census.rs` — the zero-`error` gate.
- `.claude/unknown-census.toml` — reclassification.
- `crates/smelt-db/src/type_inference/{binary.rs,collation.rs}` — only if a pairing test finds an uncovered path.

**Docs touched.** *(timeless)*
- None expected; if a checker fix changes diagnostic coverage, `docs/specs/diagnostics.md` gets the behavior note.

**Review checklist** (material findings only):
- [ ] Pairing tests assert both halves (Unknown type **and** diagnostic on the right range), not just diagnostic presence
- [ ] Census reclassification reasons name the pinning tests
- [ ] `every_unknown_site_is_classified` still green (no new unclassified sites)

**Commit.** `test(db): pin diagnostic pairing at the two error-classified Unknown sites; census error count to zero`

### Phase 4: Exit-code standardization across smelt-cli

**Goal.** Implement the Phase-1 contract: distinguish detected failures (`1`) from usage/config errors (`2`) and centralize the exit path.

**Pre-conditions.** Phase 1 merged (the contract is spec text). Current state: `async fn main() -> Result<()>` (anyhow ⇒ every error exits `1`), plus `std::process::exit(1)` in `commands/{build.rs:267,test.rs:557,check.rs:237,diff.rs:226}`; clap usage errors already exit `2`.

**TDD tests to write first.**
- `crates/smelt-cli/tests/exit_codes.rs::config_error_exits_two` — run the binary (assert_cmd or the existing e2e harness style under `crates/smelt-cli/tests/e2e/`) against a workspace with a malformed `smelt.yml`: exit code `2`, error on stderr.
- `crates/smelt-cli/tests/exit_codes.rs::missing_workspace_exits_two` — run in an empty temp dir: exit `2`.
- `crates/smelt-cli/tests/exit_codes.rs::failed_check_exits_one` — reuse a broken-check fixture (see `crates/smelt-cli/tests/check_command.rs` fixtures): exit `1`.
- `crates/smelt-cli/tests/exit_codes.rs::warn_severity_check_exits_zero` — pins the `warn` rule from the contract.

**Implementation shape.** Introduce a small exit-classification layer: `main` returns `std::process::ExitCode`; a `CliError` (or an enum over anyhow context) distinguishes `Usage/Config` (→ `2`) from `DetectedFailure` (→ `1`). Replace the four scattered `std::process::exit(1)` calls with returning the detected-failure variant so the exit path is single-owned. Config/workspace-load errors (`smelt.yml` parse, project resolution) map to `2`; everything already-diagnosed at runtime stays `1`. Remove the Known Divergences entry added in Phase 1.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/main.rs` — `ExitCode` return + classification.
- `crates/smelt-cli/src/commands/{build,test,check,diff}.rs` — replace `process::exit(1)` with the typed path.
- `crates/smelt-cli/src/errors.rs` — classification lives here if an error type already exists.
- `crates/smelt-cli/tests/exit_codes.rs` — new.

**Docs touched.** *(timeless)*
- `docs/specs/cli.md` — delete the Phase-1 Known Divergences entry.
- `docs-site/docs/reference/cli.md` — an "Exit codes" table for orchestrator authors (cron/Airflow), mirroring the contract.

**Review checklist** (material findings only):
- [ ] Zero `std::process::exit` calls remain outside the single exit path (`rg 'process::exit' crates/smelt-cli/src/`)
- [ ] `warn`-severity checks still exit `0` (spec rule preserved)
- [ ] Docs-site table matches the spec contract exactly
- [ ] Known Divergences entry removed

**Commit.** `feat(cli): standardized exit codes — 1 detected failure, 2 usage/config error`

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test exit_codes --quiet` and `cargo test -p smelt-runtime --test materialized_view_hard_error --quiet`
- `rg -n 'classification = "error"' .claude/unknown-census.toml` — empty
- `rg -n 'falling back' crates/smelt-backend/` — empty
- `/smelt:validate cli` reports zero drift on the exit-code section
