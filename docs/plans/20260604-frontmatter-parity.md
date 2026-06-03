# Plan: Frontmatter Parity — malformed frontmatter surfaces a diagnostic, never a silent `table`→`view` drop

**Parent (master plan)**: `docs/plans/20260530-feature-sweep.md` — this is a **sub-plan** spawned from the feature sweep to remediate the "frontmatter fragility" cluster of its ledger findings: **BUG-016, BUG-023, BUG-025**. The autonomy loop works this sub-plan phase by phase and rolls up to the master only when it is exhausted.

**Date**: 2026-06-04
**Spec**: `docs/specs/architecture.md` §"Unified frontmatter rule" (the error-handling half of the shared parsing contract); `docs/specs/timeseries.md` §"MalformedTimeseries"; `docs/specs/functions.md` (function/extern key catalogue).
**Spec diff**: added the **error-handling invariant** to §"Unified frontmatter rule" — a frontmatter block that fails to parse (malformed YAML, unknown key, out-of-enum value) surfaces an `Error` diagnostic via `file_diagnostics` and is **never silently discarded** (no fallback to default materialization); a key valid on any declaration kind parses without error on every kind, and an inapplicable key does not reject the whole block.
**Tracking branch**: `worktree-test_features`
**Docs**: code+docs.

## Execution prompt (for a fresh session / autonomy iteration)

Read this file. Run the next `pending` phase in the Progress-tracking table (skip `done` and `blocked` rows) using the per-phase routine below (pre-flight → spec increment if listed → red-green `/smelt:implement` on the phase's tests, spec as oracle, implementer + reviewer → verification gates → update the table row → commit + push with the phase's commit message). Emit exactly one sentinel: `<<PHASE_COMPLETE>>` (phase done), `<<PHASE_BLOCKED>>` (decision/off-target-red recorded; see §"Block conditions"), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>` (sub-plan exhausted; see the loop's roll-up rule), or `<<ALL_DONE>>`. There is no hard-stop: a block is recorded and the loop continues to the next pending phase.

## Goal

Make a malformed or unknown-key frontmatter block **fail loudly** — an `Error` diagnostic through the shared `file_diagnostics` surface, which the build already gates on — instead of silently discarding the whole block and downgrading the declaration's materialization (`table` → `view`, exit 0, no diagnostic). Three findings, one root mechanism (serde error swallowed to `None` + two divergent parsers):

- **BUG-016** — `ModelMetadata` (`deny_unknown_fields`) rejects the function/extern planner keys (`deterministic`/`idempotent`/`append_only`/`backends`/`joins`/`provenance`) that the Unified-frontmatter rule makes valid on any declaration; a model carrying one drops its **whole** frontmatter (→ default `view`).
- **BUG-023** — a serde-level `timeseries:` error (`granularity: fortnight`, missing required key) silently discards the entire frontmatter (→ VIEW, exit 0, no diagnostic).
- **BUG-025** — `TimeseriesConfig` lacks `#[serde(deny_unknown_fields)]` (the only sibling frontmatter struct without it), so typo'd keys are silently accepted/lost.

## Design decisions (resolved — do not re-litigate; pre-decided from spec + precedent)

- **Tolerant model parser, not a full parser-unification refactor** (BUG-016). `ModelMetadata` gains the six function/extern keys as **optional, ignored** fields, so a model carrying one parses cleanly and keeps the rest of its block — satisfying the parser-sharing invariant at the surface (every kind accepts every valid key). A full structural merge of the two parser code paths (`smelt-core::metadata` ⇄ `smelt_planner::logical::parse_function_properties`) is a larger architectural effort tracked as a **separate** future sub-plan; it is **not** in scope here. *Rationale*: the spec mandates an identical parsing contract, which the tolerant fix achieves with low risk; structural deduplication is orthogonal and high-effort.
- **Surface as a diagnostic; do not fail-fast in discovery** (BUG-023). A frontmatter parse error becomes an `Error` diagnostic in `file_diagnostics` (collected, anchored at the declaration) rather than aborting workspace discovery. *Rationale*: the **Workspace-loading-parity rule** requires CLI and LSP to discover identically, and the LSP must keep giving partial results on a single bad block; the established pattern is "diagnostics surface, the gate refuses" — not discovery-abort.
- **Gating is already in place** (resolves the gating question). diag-parity **P2** wired the run pipeline (CLI + `execute_project`) to gate on **all** `severity == Error` diagnostics through one shared helper. So this sub-plan only has to make the frontmatter errors **surface** as `Error` in `file_diagnostics`; the build then refuses automatically. No new gate, no code allow-list.
- **No new diagnostic code is minted.** Model frontmatter serde/parse errors map to the existing `FrontmatterParseError`; `timeseries:` serde errors (unknown key, out-of-enum, missing required) map to the existing `MalformedTimeseries`.

## Per-phase routine
1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. If red, check **what** is red: if the failure is this phase's own acceptance target (the example/test this phase exists to make green), that is expected — **proceed**. If the red is unrelated breakage, treat it as a block (record + continue, per §"Block conditions"); do not build on a broken baseline.
2. **Spec increment** (only the phases that list one): edit the named spec section first; keep it timeless (no phase vocabulary in the spec body). *(The cross-cutting error-handling invariant is already landed in §"Unified frontmatter rule"; per-phase increments below are none unless listed.)*
3. **Red-green `/smelt:implement`.** Write the phase's failing test(s) first, then the implementation, with the spec as oracle. Implementer pass, then reviewer pass (material findings only).
4. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green; plus the phase's specific gates: `example_diagnostics`, `example_workspaces`, `smelt-runtime`. For `example_builds`: run it **scoped** to the workspace(s) this phase touches — `SMELT_EXAMPLE_BUILDS_ONLY="<ws1>,<ws2>" cargo test -p smelt-cli --test example_builds`. The full sweep runs only in F5 / CI.
5. **Record + commit.** Update the status-table row to `done` + date; commit and push tests + impl + spec + table together with the phase's commit message. Emit `<<PHASE_COMPLETE>>` (or `<<ALL_DONE>>` on the last green phase).

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue, no hard-stop)
When a phase hits one of the conditions below, **do not halt the loop**. Instead: (1) set the phase's status-table row to `blocked` with a one-line reason; (2) append a dated entry to §"Blocked phases" (phase id, the decision/reason, candidate options); (3) restore the tree to a clean committed state; (4) commit + push; (5) emit `<<PHASE_BLOCKED>>`. The next iteration skips the `blocked` row and picks the next `pending` phase.

Conditions:
- The phase needs a design decision **not** answered by this plan (the three above are settled) or the spec.
- Pre-flight is red on **unrelated** breakage (not this phase's own acceptance target — see routine step 1).
- The tree can't be returned to green after the phase.

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| F1 | `deny_unknown_fields` on `TimeseriesConfig` (unknown `timeseries:` keys become a serde error) | pending | BUG-025 | | |
| F2 | Tolerant `ModelMetadata`: accept the six function/extern keys as optional ignored fields (a model with a planner key keeps `materialization: table`) | pending | BUG-016 | | |
| F3 | Surface serde-level frontmatter parse errors as `Error` diagnostics in `file_diagnostics` (stop swallowing to `None` at both the discovery and smelt-db sites); model→`FrontmatterParseError`, timeseries→`MalformedTimeseries` | pending | BUG-023 (+ surfaces 025) | | |
| F4 | Example fixtures + gates: tolerated-planner-key model builds as a TABLE; invalid-granularity and unknown-timeseries-key models are build-refused with `MalformedTimeseries` | pending | BUG-016/023/025 (e2e) | | |
| F5 | Close-out: flip BUG-016/023/025 to `fixed` in the ledger with regression-test names; update master sub-plan table + ROADMAP; full `example_builds` + all gates green | pending | — | | |

**Status values**: `pending`, `done`, `blocked`. A phase is `done` only when its tests are red-green confirmed and all gates are green. A `blocked` phase has a dated §"Blocked phases" entry and returns to `pending` once a human resolves it.

## Blocked phases

Append-only log of phases the loop recorded as `blocked` and continued past. Each entry: date, phase id, reason/decision, candidate options. *(None yet.)*

## Phase detail

### F1 — `deny_unknown_fields` on `TimeseriesConfig` (BUG-025)
- **Change**: add `#[serde(deny_unknown_fields)]` to `TimeseriesConfig` (`crates/smelt-core/src/config.rs` ~line 353), matching every sibling frontmatter struct (`ModelMetadata`, `IncrementalConfig`, `SchemaEvolutionConfig`, `TestConfig`, `ColumnMetadata`).
- **Tests (red-green)**: `crates/smelt-core` unit test — a `timeseries:` block with a typo'd key (`partion_column`) now returns a serde `Err` (previously `Ok`, key silently dropped). Existing fixtures (no unknown keys) stay green.
- **Note**: this makes the error *exist*; F3 routes it to a diagnostic. Between F1 and F3 the error is still swallowed downstream — acceptable interim (the example fixtures land in F4 after F3).
- **Commit**: `fix(core): deny unknown keys in TimeseriesConfig frontmatter (closes BUG-025)`

### F2 — Tolerant `ModelMetadata` for function/extern keys (BUG-016)
- **Change**: add `deterministic`, `idempotent`, `append_only`, `backends`, `joins`, `provenance` to `ModelMetadata` (`crates/smelt-core/src/metadata.rs` ~line 117) as `#[serde(default, skip_serializing_if = "…")]` optional fields, ignored by the model path (they carry meaning only to the planner). Keeps `deny_unknown_fields` (now these keys are *known*).
- **Tests (red-green)**: `crates/smelt-core` unit test — `extract_file_metadata` on a model with `materialization: table` + `deterministic: true` returns `Some(metadata)` with `materialization == table` (previously `Err` → dropped → default `view`).
- **Commit**: `fix(core): accept function/extern frontmatter keys on models (closes BUG-016)`

### F3 — Surface frontmatter serde errors as diagnostics (BUG-023; also surfaces 025)
- **Change**: stop swallowing a frontmatter parse `Err` to `None`. Two sites:
  - `crates/smelt-core/src/discovery.rs` (~264–270): the `Err(e) => { eprintln!…; None }` swallow on `extract_file_metadata`.
  - `crates/smelt-db/src/lib.rs` (~1170–1200): the generic-serde fall-through (`_ => None`) where only `TimeseriesRequiredForIncremental` / `MalformedTimeseries` validation errors are bridged today.
  Route the error into `file_diagnostics` as an `Error`-severity diagnostic anchored at the declaration: a `timeseries:` serde error → `MalformedTimeseries`; any other model-frontmatter serde error → `FrontmatterParseError`. Keep discovery resilient (collect + continue; do not abort load) per the resolved decision.
- **Tests (red-green)**: smelt-db unit/integration — `file_diagnostics` on a file with `granularity: fortnight` emits one `MalformedTimeseries`; on a file with an unknown `timeseries:` key emits one `MalformedTimeseries`; on a model with a genuinely malformed block emits `FrontmatterParseError`. With diag-parity's gate in place, the corresponding `smelt build` now exits non-zero (asserted in F4).
- **Commit**: `fix(db,core): surface malformed-frontmatter serde errors as diagnostics instead of dropping the block (closes BUG-023)`

### F4 — Example fixtures + end-to-end gates (BUG-016/023/025)
- **Fixtures** (under `examples/`):
  - `frontmatter_planner_key_on_model` — a model with `materialization: table` + `deterministic: true`. **Positive**: builds clean and materialises as a **TABLE** (asserts the tolerant parse + retained materialization; this is *not* a `*_broken_*` fixture).
  - `timeseries_broken_invalid_granularity` — `granularity: fortnight`. Build-refused with `MalformedTimeseries`.
  - `timeseries_broken_unknown_key` — an unknown key in the `timeseries:` block. Build-refused with `MalformedTimeseries`.
- **Gates**: assertions in `crates/smelt-cli/tests/example_diagnostics.rs` (the analyzer surfaces the codes / the positive fixture is clean) and `crates/smelt-cli/tests/example_builds.rs` (the broken fixtures are build-refused naming the code; the positive fixture builds + executes as a table). Confirms analysis↔build parity end-to-end, mirroring the diag-parity disposition.
- **Commit**: `test(examples): frontmatter-parity fixtures — tolerated planner key builds, malformed timeseries refused (closes BUG-016/023/025 e2e)`

### F5 — Close-out
- **Tests**: full `example_builds` (var unset) + full suite + all gates green.
- **Docs**: flip BUG-016/023/025 to `fixed` in `docs/bug-hunt/2026-05-30-findings.md` with their regression-test names; update the master plan's §"Spawned sub-plans" row to `done`; update `docs/ROADMAP.md`. Note the deferred follow-up (full parser-code-path unification) as an explicit Known item.
- **Commit**: `docs(frontmatter-parity): close out — ledger + roadmap updated, parser-unification deferred`

## Verification
- Every table row `done` (or `blocked` with a recorded entry).
- `cargo fmt --all -- --check`, `cargo clippy --all-targets` (no warnings), `cargo test`, `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-cli --test example_builds`, `cargo test -p smelt-lsp --test example_workspaces` all green.
- Each `Closes` bug has a red-green regression test, confirmed red on `git stash` of the fix (a `*_broken_*` build-gate assertion or a materialization-survival assertion).
