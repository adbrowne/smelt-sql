# Plan: Frontmatter Parity — malformed frontmatter surfaces a diagnostic, never a silent `table`→`view` drop

**Parent (master plan)**: `docs/plans/20260530-feature-sweep.md` — this is a **sub-plan** spawned from the feature sweep to remediate the "frontmatter fragility" cluster of its ledger findings: **BUG-016, BUG-023, BUG-025**. The autonomy loop works this sub-plan phase by phase and rolls up to the master only when it is exhausted.

**Date**: 2026-06-04
**Spec**: `docs/specs/architecture.md` §"Unified frontmatter rule" (one parser over a key catalogue; the error-handling contract); `docs/specs/timeseries.md` §"MalformedTimeseries"; `docs/specs/functions.md` (function/extern key set).
**Spec diff**: sharpened §"Unified frontmatter rule" — "shared" now means *literally one* parser over a **key catalogue** (the sole authority on which keys exist, open so a rule may contribute entries); and the error-handling contract: an **unknown** key / malformed YAML / out-of-shape value is an **`Error`**, a key **known but inapplicable to the declaration kind** is a **`Warning`** (block retained), and a block is **never silently discarded**.
**Design**: `docs/research/2026-06-04-frontmatter-parser-unification.md` (the catalogue-seam architecture and why it is extensibility-ready).
**Tracking branch**: `worktree-test_features`
**Docs**: code+docs.

## Execution prompt (for a fresh session / autonomy iteration)

Read this file. Run the next `pending` phase in the Progress-tracking table (skip `done` and `blocked` rows) using the per-phase routine below (pre-flight → spec increment if listed → red-green `/smelt:implement` on the phase's tests, spec as oracle, implementer + reviewer → verification gates → update the table row → commit + push with the phase's commit message). Emit exactly one sentinel: `<<PHASE_COMPLETE>>` (phase done), `<<PHASE_BLOCKED>>` (decision/off-target-red recorded; see §"Block conditions"), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>` (sub-plan exhausted; see the loop's roll-up rule), or `<<ALL_DONE>>`. There is no hard-stop: a block is recorded and the loop continues to the next pending phase.

## Goal

**Collapse the two frontmatter parsers into one** (over a key catalogue), and make a malformed or unknown-key block **fail loudly** — an `Error` diagnostic through the shared `file_diagnostics` surface the build already gates on — instead of silently discarding the whole block and downgrading materialization (`table` → `view`, exit 0, no diagnostic). The unification removes the root cause (two divergent parsers) rather than patching around it, and lands the catalogue seam that user-extensible planner rules will later contribute schemas to. Three findings, one root mechanism (serde error swallowed to `None` + two divergent parsers):

- **BUG-016** — `ModelMetadata` (`deny_unknown_fields`) rejects the function/extern planner keys (`deterministic`/`idempotent`/`append_only`/`backends`/`joins`/`provenance`) that the Unified-frontmatter rule makes valid on any declaration; a model carrying one drops its **whole** frontmatter (→ default `view`).
- **BUG-023** — a serde-level `timeseries:` error (`granularity: fortnight`, missing required key) silently discards the entire frontmatter (→ VIEW, exit 0, no diagnostic).
- **BUG-025** — `TimeseriesConfig` lacks `#[serde(deny_unknown_fields)]` (the only sibling frontmatter struct without it), so typo'd keys are silently accepted/lost.

## Design decisions (resolved — do not re-litigate; see the design doc + spec)

- **Full unification via a frontmatter catalogue seam** (BUG-016), *not* a tolerant patch and *not* a closed superset struct. One parser in **smelt-core** over a `FrontmatterCatalogue` (`key → {value-shape, applicable declaration kinds, owning feature}`); the built-in features are the only registrants; each consumer deserializes its typed slice from the validated map (model → `ModelMetadata`, planner → `RawFunctionProperties`); the hand-rolled `parse_function_properties` is **deleted**. *Rationale*: frontmatter keys must stay **open** for user-extensible planner rules (`planner_rule_api_design.md`), so a closed `deny_unknown_fields` superset would paint into a corner; the catalogue is the seam the extensibility API plugs into later. **Deferred**: the public/dynamic registration API for non-built-in rules — out of scope here.
- **Policy: unknown = `Error`, inapplicable-kind = `Warning`, never silent** (BUG-016/023/025). A key unknown to the whole catalogue (typo), malformed YAML, or an out-of-shape value is an `Error`; a catalogue-known key not applicable to this declaration kind (e.g. `deterministic` on a model) is a `Warning` with the block retained; nothing is silently dropped. Matches the §"Unified frontmatter rule" contract.
- **Surface as a diagnostic; do not fail-fast in discovery** (BUG-023). Parse errors become `Error` diagnostics in `file_diagnostics` (collected, anchored), not a discovery abort. *Rationale*: the **Workspace-loading-parity rule** requires CLI and LSP to discover identically, and the LSP must keep giving partial results on a single bad block.
- **Gating is already in place.** diag-parity **P2** gates the run pipeline (CLI + `execute_project`) on **all** `severity == Error` diagnostics via one shared helper. This sub-plan only makes the frontmatter errors **surface**; the build then refuses automatically. No new gate.
- **No new diagnostic code is minted.** Unknown/malformed model frontmatter → existing `FrontmatterParseError`; `timeseries:` violations (unknown sub-key, out-of-enum, missing required) → existing `MalformedTimeseries`. `FrontmatterDiagnostic` (severity + message) moves to smelt-core so both crates share one type; `smelt-db` anchors it as today.

## Per-phase routine
1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. If red, check **what** is red: if the failure is this phase's own acceptance target (the example/test this phase exists to make green), that is expected — **proceed**. If the red is unrelated breakage, treat it as a block (record + continue, per §"Block conditions"); do not build on a broken baseline.
2. **Spec increment** (only the phases that list one): edit the named spec section first; keep it timeless (no phase vocabulary in the spec body). *(The cross-cutting error-handling invariant is already landed in §"Unified frontmatter rule"; per-phase increments below are none unless listed.)*
3. **Red-green `/smelt:implement`.** Write the phase's failing test(s) first, then the implementation, with the spec as oracle. Implementer pass, then reviewer pass (material findings only).
4. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green; plus the phase's specific gates: `example_diagnostics`, `example_workspaces`, `smelt-runtime`. For `example_builds`: run it **scoped** to the workspace(s) this phase touches — `SMELT_EXAMPLE_BUILDS_ONLY="<ws1>,<ws2>" cargo test -p smelt-cli --test example_builds`. The full sweep runs only in U6 / CI.
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
| U1 | `deny_unknown_fields` on `TimeseriesConfig` (unknown `timeseries:` sub-keys become a serde error) | pending | BUG-025 | | |
| U2 | `FrontmatterCatalogue` + single `parse_frontmatter(text, kind)` in smelt-core (move `FrontmatterDiagnostic` down); unknown→`Error`, inapplicable-kind→`Warning`. Unit-tested in isolation; not yet wired to callers | pending | — (foundation) | | |
| U3 | Route the **model** path through `parse_frontmatter`: `ModelMetadata` deserializes from the validated map; surface errors via `file_diagnostics` (stop swallowing at the discovery + smelt-db sites); a model with a function key keeps `materialization: table` + emits a `Warning` | pending | BUG-016, BUG-023 | | |
| U4 | Route the **function/extern** path through `parse_frontmatter`: `FunctionProperties` from a lenient `RawFunctionProperties` serde derive; **delete** `parse_function_properties` (one parser remains) | pending | — (unification) | | |
| U5 | Example fixtures + gates: function-key-on-model builds as a TABLE (with `Warning`); invalid-granularity and unknown-`timeseries`-key models build-refused with `MalformedTimeseries`; an unknown top-level key build-refused with `FrontmatterParseError` | pending | BUG-016/023/025 (e2e) | | |
| U6 | Close-out: flip BUG-016/023/025 to `fixed` in the ledger with regression-test names; update master sub-plan table + ROADMAP; note the public schema-registration API as deferred; full `example_builds` + all gates green | pending | — | | |

**Status values**: `pending`, `done`, `blocked`. A phase is `done` only when its tests are red-green confirmed and all gates are green. A `blocked` phase has a dated §"Blocked phases" entry and returns to `pending` once a human resolves it.

## Blocked phases

Append-only log of phases the loop recorded as `blocked` and continued past. Each entry: date, phase id, reason/decision, candidate options. *(None yet.)*

## Phase detail

### U1 — `deny_unknown_fields` on `TimeseriesConfig` (BUG-025)
- **Change**: add `#[serde(deny_unknown_fields)]` to `TimeseriesConfig` (`crates/smelt-core/src/config.rs` ~line 353), matching every sibling frontmatter struct (`ModelMetadata`, `IncrementalConfig`, `SchemaEvolutionConfig`, `TestConfig`, `ColumnMetadata`). This governs the **nested** `timeseries:` sub-keys; the top-level catalogue (U2) governs top-level keys.
- **Tests (red-green)**: `crates/smelt-core` unit test — a `timeseries:` block with a typo'd sub-key (`partion_column`) now returns a serde `Err` (previously `Ok`, key silently dropped). Existing fixtures stay green.
- **Note**: makes the error *exist*; U3 routes it to a diagnostic. Orthogonal, lands first as a clean quick win.
- **Commit**: `fix(core): deny unknown keys in TimeseriesConfig frontmatter (closes BUG-025)`

### U2 — `FrontmatterCatalogue` + single `parse_frontmatter` in smelt-core (foundation)
- **Change**: introduce a `FrontmatterCatalogue` (static registry of the built-in key schemas: `key → {value-shape, applicable kinds ∈ {Model, Define, Extern}, owning feature}`) and `parse_frontmatter(text, kind) -> (validated_map, Vec<FrontmatterDiagnostic>)` doing: YAML→`Mapping` (non-mapping top level → `Error`); per-key catalogue lookup — unknown→`Error`, known-but-inapplicable-to-`kind`→`Warning`, applicable→kept. Move `FrontmatterDiagnostic` + `FrontmatterSeverity` from `smelt-planner` into `smelt-core` (planner re-exports for now). **No caller is rewired yet** — pure addition.
- **Tests (red-green)**: `crates/smelt-core` unit tests — unknown key → one `Error`; `deterministic` with `kind=Model` → one `Warning`, key still in validated_map=false; `deterministic` with `kind=Define` → no diagnostic; non-mapping top level → `Error`; empty/null → clean.
- **Commit**: `feat(core): frontmatter catalogue + single parse_frontmatter over per-kind key sets`

### U3 — Route the model path through `parse_frontmatter` (BUG-016, BUG-023)
- **Change**: `ModelMetadata` is deserialized from `parse_frontmatter`'s validated map (a lenient `#[serde(default)]` derive — the catalogue, not `deny_unknown_fields`, now owns unknown-key detection). Stop swallowing at the two sites — `crates/smelt-core/src/discovery.rs` (~264–270 `Err(e)=>…None`) and `crates/smelt-db/src/lib.rs` (~1170–1200 generic-serde `_ => None`) — and emit the catalogue diagnostics into `file_diagnostics` (`Error`/`Warning`), anchored at the declaration: unknown/malformed → `FrontmatterParseError`; `timeseries:` violation → `MalformedTimeseries`. Discovery stays resilient (collect + continue).
- **Tests (red-green)**: smelt-db — `file_diagnostics` on `granularity: fortnight` → one `MalformedTimeseries`; unknown `timeseries:` sub-key → one `MalformedTimeseries`; unknown top-level key → one `FrontmatterParseError`; a model with `materialization: table` + `deterministic: true` → one `Warning` **and** metadata retains `materialization == table` (previously `Err` → dropped → `view`). With diag-parity's gate in place, the corresponding `smelt build` now exits non-zero.
- **Commit**: `fix(db,core): parse model frontmatter via the catalogue; surface errors instead of dropping the block (closes BUG-016, BUG-023)`

### U4 — Route the function/extern path through `parse_frontmatter`; delete the second parser (unification)
- **Change**: `FunctionProperties` is built from a lenient `RawFunctionProperties` serde derive deserialized from `parse_frontmatter`'s validated map (`kind = Define`/`Extern`); **delete** the hand-rolled `parse_function_properties` walk (`crates/smelt-planner/src/logical.rs`). The planner now consumes the shared parser's output — one parser remains.
- **Tests (red-green)**: existing `parse_function_properties` unit tests (`logical.rs` tests) are ported to drive `parse_frontmatter` + the `RawFunctionProperties` projection and stay green (same `FunctionProperties` results + the same diagnostics, now from the unified path); a function carrying a model-only key (e.g. `materialization`) → `Warning`. `cargo test -p smelt-planner` + `-p smelt-db` green.
- **Commit**: `refactor(planner,core): consume the shared frontmatter parser; delete parse_function_properties`

### U5 — Example fixtures + end-to-end gates (BUG-016/023/025)
- **Fixtures** (under `examples/`):
  - `frontmatter_function_key_on_model` — a model with `materialization: table` + `deterministic: true`. **Positive**: builds + executes as a **TABLE** and the analyzer emits the inapplicable-key `Warning` (not a `*_broken_*` fixture).
  - `timeseries_broken_invalid_granularity` — `granularity: fortnight` → build-refused with `MalformedTimeseries`.
  - `timeseries_broken_unknown_key` — unknown `timeseries:` sub-key → build-refused with `MalformedTimeseries`.
  - `frontmatter_broken_unknown_key` — an unknown **top-level** key (typo) → build-refused with `FrontmatterParseError`.
- **Gates**: `crates/smelt-cli/tests/example_diagnostics.rs` (codes/warning surface; positive fixture clean apart from the expected `Warning`) and `crates/smelt-cli/tests/example_builds.rs` (broken fixtures build-refused naming the code; positive fixture builds + executes as a table). Confirms analysis↔build parity end-to-end.
- **Commit**: `test(examples): frontmatter-parity fixtures — function key on model warns+builds, malformed/unknown refused (closes BUG-016/023/025 e2e)`

### U6 — Close-out
- **Tests**: full `example_builds` (var unset) + full suite + all gates green.
- **Docs**: flip BUG-016/023/025 to `fixed` in `docs/bug-hunt/2026-05-30-findings.md` with their regression-test names; set the master plan's §"Spawned sub-plans" row to `done`; update `docs/ROADMAP.md`. Record the **deferred** public/dynamic schema-registration API (for non-built-in rules) as an explicit Known item, linked to `planner_rule_api_design.md`.
- **Commit**: `docs(frontmatter-parity): close out — one parser over the catalogue, ledger + roadmap updated`

## Verification
- Every table row `done` (or `blocked` with a recorded entry).
- `cargo fmt --all -- --check`, `cargo clippy --all-targets` (no warnings), `cargo test`, `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-cli --test example_builds`, `cargo test -p smelt-lsp --test example_workspaces`, `cargo test -p smelt-planner` all green.
- `parse_function_properties` no longer exists (one parser remains); `rg parse_function_properties crates/` returns nothing outside history.
- Each `Closes` bug has a red-green regression test, confirmed red on `git stash` of the fix.
