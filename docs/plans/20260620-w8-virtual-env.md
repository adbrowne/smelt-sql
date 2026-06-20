# Plan: W8 — Virtual Environments (D-46, D-47)

**Parent (master plan)**: `docs/plans/20260613-spec-impl.md` — the **W8 virtual_env** sub-theme of the spec-remediation backlog. Remediates **D-46** and **D-47** from the 2026-06-13 spec review (Theme "D-surface tail"), both resolved to option **A**. The committed specs (`virtual_environments.md`, `run_state.md`) are the correctness oracle; this wave is **code-only** (no further spec edits except the close-out KD retraction in P5).

**Date**: 2026-06-20
**Spec**: `docs/specs/virtual_environments.md` §"state.mode", §"Semantics / Reuse decision", §"Constraints & Invariants"; `docs/specs/run_state.md` §"Snapshot and environment store" — these are the correctness oracle.
**Spec diff**: none — both decisions were committed during the 2026-06-12/13 review and landed verbatim in the specs. This wave is code-only.
**Tracking branch**: `worktree-spec_review`
**Docs**: code-only. P5 retracts the now-satisfied Known-Divergence notes in `virtual_environments.md` (reuse-condition 3a/3b split, posture lattice + candidate-table precedence) and may touch `docs-site/` only if the reviewer flags a user-facing gap.

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then the spec sections above — they are the correctness oracle; do not re-open the settled decisions (D-46 and D-47 both resolved to option **A**). Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked` rows) using the per-phase routine below (pre-flight → red-green `/smelt:implement` with implementer + reviewer, spec as oracle → verification gates → set the row to `done` + date → commit + push with the phase's commit message). If that was the last `pending` phase, also flip this sub-plan's Status to `done (<today>)` in the master registry (`docs/plans/20260613-spec-impl.md`) and commit together. Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>` (record + continue, see §"Block conditions"), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>` (sub-plan exhausted), or `<<ALL_DONE>>`. A block is recorded and the loop continues — there is no hard-stop.

## Design decisions (resolved — do not re-litigate)

| Dec | One-line contract (the spec is authoritative) |
|-----|-----------------------------------------------|
| **D-46** | Reuse condition 3 is **split** into 3a (deterministic OR `assert_deterministic` ⇒ rebuild-identity preserved; `assert_deterministic` is trusted and logged) and 3b (`accept_current` on a known non-deterministic model ⇒ output-preserving reuse without rebuild-identity; logged explicitly). Each path has its own logged-trust note. The rebuild-identity invariant applies only to 3a. |
| **D-47** | The posture lattice `environments ⊇ intervals ⊇ stateless` is defined explicitly; a model narrowed to `stateless` opts out of reuse entirely. The candidate table is located via the `(environment, model) → physical table` index in `run_state.md` §"Snapshot and environment store". When multiple entries are fingerprint-equal, precedence is: target environment `E` first, then base/production, then other environments lexicographically. |

These are settled; the spec text is the oracle. Do **not** re-litigate option A vs B for either.

## Where the code lives (orientation, not a contract)

The virtual-environments orchestration layer is **entirely unbuilt**. Existing substrate:

- `crates/smelt-fingerprint/` — `output_fingerprint()`, `FingerprintResult` (with `.deterministic` and `.non_determinism` fields), `NonDeterminism` — standalone, not yet wired to any consumer.
- `crates/smelt-state/` — `RunManifest`, `ModelRunRecord`, `IntervalStore`, `FileStore` — already consumed by `smelt-runtime` and `smelt-cli` for interval/manifest tracking. The snapshot/environment store (`(environment, model) → table` map) exists only in the spec; nothing in `smelt-state` implements it yet.
- `crates/smelt-core/src/config.rs` — `Config` struct for `smelt.yml`. No `state.mode` field or `StateMode` enum exists today.
- `crates/smelt-core/src/frontmatter.rs` — `ModelMetadata`. No `reuse.accept_current`, `reuse.assert_deterministic`, or `forward_only` fields exist today.
- No `ReusedDecision` type, no reuse-condition evaluator, no candidate-table lookup, no environment-suffixed addressing, no `smelt plan/apply/promote` commands exist in any crate.

Because the orchestration layer is unbuilt, all phases in this wave add new data types and pure logic only — they do not wire execution paths. The wave delivers a spec-conformant data model and reuse-condition evaluator, leaving the full orchestration integration (environment-suffixed addressing, `smelt plan --environment`, `smelt promote`) to future plans. Phases do not reach into `smelt-runtime` execution paths, keeping the diff bounded.

## Per-phase routine

1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. If red on this phase's own acceptance target (the test it exists to make green), proceed. If red on **unrelated** breakage, treat as a block (record + continue, §"Block conditions").
2. **Red-green `/smelt:implement`.** Write the phase's failing test(s) first, then the implementation, spec as oracle. Implementer pass, then reviewer pass (material findings only).
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green; `cargo test -p smelt-cli --test example_diagnostics` green; `cargo test -p smelt-lsp --test example_workspaces` green; phase-specific tests green.
4. **Record + commit.** Set the table row to `done` + date; commit + push tests + impl + table together with the phase's commit message. Emit `<<PHASE_COMPLETE>>` (or `<<SUBPLAN_ADVANCED>>`/`<<MASTER_EXHAUSTED>>` on the last phase).

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue, no hard-stop)

Set the row to `blocked` with a one-line reason; append a dated entry to §"Blocked phases" (phase id, reason, candidate options); restore the tree to a clean committed state; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:

- The spec is genuinely ambiguous for a case the phase hits (record the question for a human; do not guess).
- A pre-flight failure on unrelated breakage that this phase did not introduce.
- A phase requires touching `smelt-runtime` execution paths (beyond new data types) — scope violation; record and continue to the next phase.

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| P1 | `StateMode` enum + `state:` block in `Config` / `smelt.yml` | pending | D-47 (lattice) | feat(core): add `StateMode` posture lattice to project config (D-47) | |
| P2 | `reuse` frontmatter block: `accept_current`, `assert_deterministic`, `forward_only` | pending | D-46 (hatches) | feat(core): add reuse frontmatter hatches to `ModelMetadata` (D-46) | |
| P3 | Snapshot/environment store types in `smelt-state` | pending | D-47 (candidate index) | feat(state): add snapshot/environment store types and candidate-precedence lookup (D-47) | |
| P4 | Reuse-condition evaluator: conditions 1, 2, 3a, 3b, 4-stub | pending | D-46 (3a/3b split), D-47 | feat(fingerprint): reuse-condition evaluator with 3a/3b split and logged-trust notes (D-46/47) | |
| P5 | Close-out: KD retraction + master registry + ROADMAP | pending | D-46, D-47 close-out | docs(spec-impl): close out W8 virtual_env (D-46, D-47) | |

**Status values**: `pending`, `done`, `blocked`. A phase is `done` only when its tests are red-green confirmed and all gates pass.

---

### Phase P1: `StateMode` enum + `state:` block in `Config`

**Goal.** Add the `StateMode` enum (`Stateless`, `Intervals`, `Environments`) to `smelt-core` and parse a `state:` block in `smelt.yml`. A project declares its posture; the default is `Stateless`. The posture lattice `environments ⊇ intervals ⊇ stateless` is encoded as a `PartialOrd` (or an explicit `can_narrow_to` method) so the per-model narrowing guard can call it. (`virtual_environments.md` §"state.mode"; D-47.)

**Critical files.** `crates/smelt-core/src/config.rs` (add `StateMode`, parse `state.mode`); add `crates/smelt-core/tests/config_state_mode.rs` (new test file or extend existing config tests).

**Test ideas (write first).**
- Parse a `smelt.yml` with `state: {mode: environments}` → `Config.state_mode == StateMode::Environments`.
- Parse with no `state:` block → `Config.state_mode == StateMode::Stateless` (default).
- `StateMode::Stateless` cannot widen to `Intervals` (narrowing direction encoded correctly).
- `StateMode::Environments` can narrow to `Intervals` or `Stateless`.
- Unknown `state.mode` value → deserialization error (fail-loud discipline; no silent `Unknown`).

**Commit.** `feat(core): add StateMode posture lattice to project config (D-47)`

---

### Phase P2: `reuse` frontmatter block

**Goal.** Add `reuse: {accept_current: bool, assert_deterministic: bool}` and `forward_only: bool` to `ModelMetadata` (parsed from SQL frontmatter). Add a `model_state_mode: Option<StateMode>` field so a model can narrow the project posture. Validate that a model does not widen (reject with a diagnostic if `model_state_mode` is higher than the project's). (`virtual_environments.md` §"Author override hatches"; §"state.mode" narrowing rule; D-46.)

**Critical files.** `crates/smelt-core/src/frontmatter.rs`; `crates/smelt-db/src/lib.rs` (add a diagnostic for posture widening — keep `MetadataError` exhaustiveness); `crates/smelt-core/src/metadata.rs` (possibly a new `MetadataError` variant for widening).

**Test ideas (write first).**
- Frontmatter `reuse: {accept_current: true}` → `ModelMetadata.reuse.accept_current == true`.
- Frontmatter `reuse: {assert_deterministic: true}` → `ModelMetadata.reuse.assert_deterministic == true`.
- Frontmatter `forward_only: true` → `ModelMetadata.forward_only == true`.
- Model declares `state: {mode: environments}` in a `stateless` project → diagnostic (widening rejected).
- Model declares `state: {mode: stateless}` in an `environments` project → allowed (narrowing).
- Unknown key under `reuse:` → unknown-frontmatter diagnostic (existing `UnknownFrontmatterKey` path; see D-31).

**Commit.** `feat(core): add reuse frontmatter hatches to ModelMetadata (D-46)`

---

### Phase P3: Snapshot/environment store types in `smelt-state`

**Goal.** Add the `SnapshotStore` struct (the `(environment, model) → physical table` map with per-entry `source_sql` and cached `fingerprint_hex`), a `SnapshotEntry`, and the `FileStore` read/write methods for it (under `.smelt/snapshots.json` or similar, following the fixed-layout rule from `run_state.md`). Implement the **candidate-table precedence rule**: given a target environment `E` and a current fingerprint, `SnapshotStore::find_candidate(model, fingerprint, target_env)` returns the best matching `SnapshotEntry` — target env first, then base/production, then lexicographic by env name. No runtime wiring; pure data layer. (`run_state.md` §"Snapshot and environment store"; `virtual_environments.md` §"Reuse decision" candidate-precedence rule; D-47.)

**Critical files.** `crates/smelt-state/src/lib.rs` or a new `crates/smelt-state/src/snapshot_store.rs`; `crates/smelt-state/src/file_store.rs` (new read/write methods). Follow `RunManifest`'s pattern: new fields are `Option`al or `#[serde(default)]`.

**Test ideas (write first).**
- `find_candidate` with one entry whose fingerprint matches → returns it.
- `find_candidate` with entries for envs `["dev", "prod", "staging"]`, all fingerprint-equal → returns `prod` entry when target is `dev` (base env precedence).
- `find_candidate` with entries for `["alpha", "beta"]` (no base/prod env), target is `gamma` → returns `alpha` (lexicographic tiebreak).
- `find_candidate` when target env `E` already has a matching entry → returns it first (already-correct, no repoint).
- `FileStore::save_snapshot_store` / `load_snapshot_store` roundtrip.
- `SnapshotEntry` with no stored fingerprint (fingerprint is ephemeral per `run_state.md` §Design "Persist the SQL, treat the fingerprint as ephemeral") serializes cleanly; reuse evaluation recomputes from `source_sql`.

**Commit.** `feat(state): add snapshot/environment store types and candidate-precedence lookup (D-47)`

---

### Phase P4: Reuse-condition evaluator (conditions 1, 2, 3a, 3b; condition 4 = stub)

**Goal.** Add a pure `evaluate_reuse(params: ReuseParams) -> ReuseDecision` function in `smelt-fingerprint` (or a new `crates/smelt-fingerprint/src/reuse.rs` module). It checks all four conditions in order and returns a typed `ReuseDecision` enum: `Reuse(ReusePath)` or `Rebuild(Vec<ReuseConditionFailed>)`. The four conditions map directly to the spec:

- **Condition 1**: model is under `state.mode: environments` (not `stateless`). A model narrowed to `stateless` → `Rebuild([NotEnvironmentMode])`.
- **Condition 2**: `fingerprint(M_current) == fingerprint(T.source)`. The caller passes the pre-computed current fingerprint and the candidate `SnapshotEntry`; the evaluator recomputes `fingerprint(T.source)` from `T.source_sql` (using `output_fingerprint_from_sql`) to check equality. No mismatch of compiler version can silently produce a false positive.
- **Condition 3a**: model is deterministic (`FingerprintResult.deterministic == true`) OR `model_metadata.reuse.assert_deterministic == true`. When 3a is satisfied: `ReusePath::RebuildIdentical`. When `assert_deterministic` was the deciding factor, log a trust note in the returned decision.
- **Condition 3b**: model is non-deterministic AND `model_metadata.reuse.accept_current == true`. When 3b is satisfied: `ReusePath::OutputPreserving`. The logged-trust note is always set when 3b fires.
- **Condition 4**: no schema migration required. Stub: always passes (returns `true`); the full check is deferred to `schema_evolution.md` work. Log as stub in the decision.

The function is pure (no I/O); it takes all inputs as parameters so it is trivially unit-testable and can be called from any context. (`virtual_environments.md` §"Semantics / Reuse decision"; §"Constraints & Invariants"; D-46, D-47.)

**Critical files.** `crates/smelt-fingerprint/src/reuse.rs` (new); `crates/smelt-fingerprint/src/lib.rs` (re-export); `crates/smelt-fingerprint/tests/reuse_conditions.rs` (new test file).

**Test ideas (write first).**
- Condition 1 fails (mode = `stateless`) → `Rebuild([NotEnvironmentMode])` regardless of other inputs.
- Condition 1 fails (model narrowed to `stateless` in an `environments` project) → same.
- Condition 2 fails (fingerprint mismatch) → `Rebuild([FingerprintMismatch])`.
- Condition 3: deterministic model, no override → `Reuse(RebuildIdentical)`, no logged-trust note.
- Condition 3a: non-deterministic model + `assert_deterministic: true` → `Reuse(RebuildIdentical)` with logged-trust note `AssertDeterministicTrusted`.
- Condition 3b: non-deterministic model + `accept_current: true` → `Reuse(OutputPreserving)` with logged-trust note `AcceptCurrentApplied`.
- Condition 3: non-deterministic model, no override → `Rebuild([NeitherReuseHatchSet])`.
- Both 3a and 3b set simultaneously → prefer 3a (rebuild-identity is the stronger contract). (The spec doesn't explicitly order them when both set; prefer 3a as the safer claim — document this as a spec-silent tie-break in a comment.)
- Condition 4 stub: always passes; `ReuseDecision` carries `schema_migration_checked: false` flag.
- Full happy path (all conditions pass, deterministic model) → `Reuse(RebuildIdentical)`, clean decision struct.

**Commit.** `feat(fingerprint): reuse-condition evaluator with 3a/3b split and logged-trust notes (D-46/47)`

---

### Phase P5: Close-out (KD retraction, master registry, ROADMAP)

**Goal.** Retract the now-satisfied Known-Divergence entries in `virtual_environments.md` for D-46 (the "reuse condition 3 was merged" gap, now fixed by the 3a/3b split) and D-47 (the "posture lattice undefined, candidate lookup unspecified" gap, now fixed by `StateMode` + `SnapshotStore::find_candidate`). Update the master registry and ROADMAP. **Do not retract the "orchestration layer is unbuilt" KD** — that remains true; the runtime wiring (environment-suffixed addressing, `smelt plan/apply/promote` commands) is not landed by this wave and is still tracked in the KD.

**Critical files.** `docs/specs/virtual_environments.md` (KD section only — retract D-46/D-47 gap notes, keep orchestration-layer and cross-model-lineage notes); `docs/plans/20260613-spec-impl.md` (flip W8 virtual_env row to `done`); `docs/ROADMAP.md` (add completion line).

**Review checklist.** Retracted KD notes are genuinely satisfied by the landed code; orchestration-layer KD is not retracted; registry and ROADMAP updated; no spec-body edits outside the KD section.

**Commit.** `docs(spec-impl): close out W8 virtual_env (D-46, D-47)`

---

## Deferred during implementation

(Append-only.)

- **Full orchestration layer** (environment-suffixed addressing, `smelt plan --environment`, `smelt apply --environment`, `smelt promote`, runtime wiring of the reuse-condition evaluator into the build pipeline) — the entire execution-time surface specified in `virtual_environments.md` §"Surface" is out of scope for this wave. This wave delivers data types and pure logic only. Future sub-plan under W8 or a dedicated wave.
- **Condition 4 (schema migration)** — the stub in P4 always passes. Full implementation depends on `schema_evolution.md` work (not yet scaffolded).
- **Promotion (`smelt promote`)** — deferred with the orchestration layer.
- **`smelt plan --environment` categorization** (breaking/non-breaking/unchanged) — deferred; requires cross-model column lineage, itself a Known Divergence.

## Blocked phases

Append-only log. None yet.

## Verification

- `cargo test` green; `cargo test -p smelt-core` (config + frontmatter), `cargo test -p smelt-state` (snapshot store roundtrip), `cargo test -p smelt-fingerprint` (reuse-condition evaluator) all green.
- `cargo test -p smelt-cli --test example_diagnostics` green; `cargo test -p smelt-lsp --test example_workspaces` green.
- `/smelt:validate virtual_environments` reports no behavioural drift on the reuse-condition 3a/3b split, posture lattice, and candidate-table precedence surfaces.
- The four fail-loud CI gates (unwrap ratchet, `println!`, unknown-census, `MetadataError` exhaustiveness) still pass — any new `MetadataError` variant for posture widening must be listed in `map_metadata_error_to_diagnostic`.
