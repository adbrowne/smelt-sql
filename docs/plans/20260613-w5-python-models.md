# Plan: W5 — Python models ↔ meta-language reconciliation (D-python)

**Parent (master plan)**: `docs/plans/20260613-spec-impl.md` — the fifth wave of the spec-remediation implementation backlog. Remediates the **D-python** cluster of the 2026-06-13 spec review: **D-22** (Python output uses plain `---`/`---` single-model frontmatter, no `name:` in the delimiter), **D-23** (circularity = non-convergence, not "queries a tag it carries"), **D-25** (location = full workspace-relative `path`, `directory` derived from it), **D-26** (canonical address = directory-prefix + function name, identical to SQL), **D-27** (`PythonModelNameMismatch` is a hard Error that blocks the build, keeping the model's other frontmatter keys). Depends on **W1** (the address authority + `compute_address_segments` that D-26 reuses); aligns with **W4** (`path` vocabulary shared with `ModelRef`). The autonomy loop works this sub-plan phase by phase.

**Date**: 2026-06-13
**Spec**: `docs/specs/python_models.md` §"Model name derivation" (D-22 plain delimiter, D-26 path-derived address, D-27 mismatch), §"Surface" `ModelInfo` table (D-25 `path`/`directory`), §"Circular dependency detection" (D-23 non-convergence); `docs/specs/architecture.md` §"Resolution" (the path-derived address rule W5 reuses from W1); `docs/specs/diagnostics.md` (`PythonModelNameMismatch`, `DuplicateAddress`).
**Spec diff**: `e862ebec..HEAD` — **already landed**. Code-catching-up-to-spec; no spec edits except the P5 close-out retraction of any now-satisfied Known-Divergence note. **Note:** the same diff also lands the **D-24 combined-interleaved-loop** text — *implementing* that interleaving is **W5b**, not this wave (see the scope fence below).
**Tracking branch**: `worktree-spec_review`
**Docs**: code-only for the Rust/SDK changes. (The `python_models.md` surface already landed; `docs-site/` Python-model guide updates for the new `path` attribute may ride in P3 if the reviewer flags a user-facing gap — otherwise close-out.)

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then the spec sections above — they are the correctness oracle; do not re-open the settled decisions. Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked`) per the per-phase routine below. If that was the last `pending` phase, flip this sub-plan's Status to `done (<today>)` in the master registry and commit together. Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>` (record + continue), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>`, or `<<ALL_DONE>>`.

**Scope fence — W5 does NOT build the combined SQL↔Python loop.** The reviewed `python_models.md` describes a single combined, fully-interleaved fixed-point with SQL generators (D-24). **Implementing that interleaving is W5b** (isolated, runs after W5). W5 keeps the existing **Python-only rounds loop** (`crates/smelt-cli/src/python.rs:208-358`) and only: redefines its *circularity* rule (D-23), fixes Python *identity/addressing/location* (D-25/26), and fixes the *frontmatter delimiter + name-mismatch* handling (D-22/27). The non-convergence semantics W5 lands carry over unchanged when W5b makes the loop combined.

## Goal

Bring Python models into line with the reviewed `python_models.md` (code lives in `crates/smelt-cli/src/python.rs`, `crates/smelt-core/src/{python_models,metadata}.rs`, and the SDK `python/smelt/`):

- **D-23 (critical).** Circular detection currently fires on **self-reference by tag/directory** (`validate_fixed_point`, `python.rs:384-426`) — which forbids the very self-referential-generation the feature exists for. Redefine circularity as **non-convergence**: the model set never stabilises across the bounded rounds. The non-convergence error already exists (`python.rs:360-364`); remove the over-strict self-tag/self-dir rule and rely on it.
- **D-26 (major).** A Python model's `address_segments` is the bare function name (`python.rs:345` = `vec![output.name.clone()]`). The spec makes it **path-derived, identical to SQL** — directory prefix (the `.py` file's workspace-relative path minus any `paths:` prefix) + function name. Reuse W1's `compute_address_segments` logic. Closes the standing "Phase 5: compute address_segments from model path" TODOs (`python.rs:619,631,651,663,1327`). This is what lets the CLI-side `resolve_address_map` catch Python↔SQL collisions correctly.
- **D-25 (major).** `find_models` surfaces only `ModelInfo.directory` (final component, `python/smelt/core.py:4-8`; Rust `build_project_context` `python.rs:147-158`). Add the full workspace-relative **`path`** attribute (same vocabulary as `ModelRef.path`), and **derive `directory` from `path`** (final component) so the two never disagree.
- **D-22 (critical).** Python output may use the multi-model `--- name: X ---` delimiter, which clashes with the Layer-1 SQL section delimiter. The spec requires **plain `---`/`---` single-model frontmatter** — identity comes from the function name, the `--- name: <model> ---` section delimiter never appears in Python output; a `name:` may appear only as a frontmatter **body key** (subject to D-27).
- **D-27 (major).** A `name:` body key ≠ function name currently **drops all metadata** (`python.rs:250-260,269-303` return `None`). The spec makes it a hard **Error that blocks the build**, but at analysis time the model is **retained with its other keys** (materialization/tags/owner) and only the bad `name:` is flagged.

## Design decisions (resolved — do not re-litigate; from `docs/research/20260613-spec-remediation-decisions.md` Theme 4)

- **D-22 = A.** Python output uses plain `---`/`---` single-model frontmatter (no `name:` in the delimiter); identity from the function name. Sidesteps the Layer-1 multi-model-delimiter collision entirely.
- **D-23 = A.** Circularity is non-convergence/oscillation (output never stabilises across the bounded rounds), **not** "queries a tag it carries." A monotonically-growing-then-stable set converges and is the *supported* self-referential pattern.
- **D-25 = A.** `find_models` location is the full workspace-relative `path` (matching `ModelRef.path`); `directory` is defined as the final component of `path`.
- **D-26 = A.** Python address = directory-prefix + function name, identical to SQL models (`py/archive.py` → `smelt.archive.users`), so uniqueness/collision is keyed on the full canonical address.
- **D-27 = A.** Hard Error that blocks the build (no "frontmatter dropped, defaults apply" recovery — that clause is dead under fail-loud). At analysis time keep the model's other keys; flag only the bad `name:`.
- **Scope: no D-24 interleaving here** — W5 keeps the Python-only rounds loop; W5b builds the combined loop. D-23's non-convergence rule is loop-shape-agnostic and carries over.

## Per-phase routine
1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. Red on this phase's own target → proceed; unrelated red → block.
2. **Red-green `/smelt:implement`.** Failing test(s) first, then implementation, spec as oracle. Implementer then reviewer. Python SDK changes get a matching Python-side test where one exists.
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green (incl. `crates/smelt-cli/src/python.rs::tests`); the dual gate `cargo test -p smelt-cli --test example_diagnostics` + `cargo test -p smelt-lsp --test example_workspaces`; scoped `example_builds` for the Python-model fixtures (`SMELT_EXAMPLE_BUILDS_ONLY="test_workspace …"`). Python models need the interpreter — if a phase's test requires the `python` feature / a live interpreter and it's unavailable in the loop's environment, mark the test `#[ignore]` with a note and rely on the subprocess-path unit coverage, or **block** if the behavior can't be verified at all.
4. **Record + commit.** Row `done` + date; commit + push tests + impl + table with the phase's commit message. Emit `<<PHASE_COMPLETE>>` (or roll-up on the last phase).

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue)
Set the row `blocked` + one-line reason; append a dated §"Blocked phases" entry; restore a clean committed tree; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:
- A design decision not answered by this plan or the spec — e.g. D-26's path derivation for a Python model whose `.py` file location doesn't map cleanly onto W1's `compute_address_segments`; or whether `find_models(directory=…)` callers in existing fixtures break under the `directory`-derived-from-`path` change (decide vs the spec).
- A phase's behavior genuinely can't be verified without a live Python interpreter the loop lacks.
- Pre-flight red on unrelated breakage; tree can't return to green.
- **Do not** start building the combined SQL↔Python loop (that's W5b) — if a test seems to require it, the phase boundary is wrong; block.

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| P1 | Circularity = non-convergence (drop self-tag/self-dir rule) | done | D-23 | fix(cli): Python circularity is non-convergence, not self-tag/self-dir (D-23) | 2026-06-15 |
| P2 | Python canonical address = directory-prefix + function name | done | D-26 | feat(cli): path-derive Python model address_segments like SQL models (D-26) | 2026-06-15 |
| P3 | `find_models` location = full workspace-relative `path`; `directory` derived | done | D-25 | feat(python): expose full path on ModelInfo; derive directory from it (D-25) | 2026-06-15 |
| P4 | Python frontmatter: plain single-model delimiter; name-mismatch blocks build, keeps other keys | done | D-22, D-27 | fix(cli): Python single-model frontmatter; PythonModelNameMismatch blocks build, retains other keys (D-22, D-27) | 2026-06-15 |
| P5 | Close-out: registry + ROADMAP | pending | — | docs(spec-impl): close out W5 — Python-model reconciliation landed; registry + roadmap | |

**Status values**: `pending`, `done`, `blocked`.

---

### Phase P1: Circularity = non-convergence

**Goal.** A self-referential generator pattern (a generator tags `staging`, another queries `tag=staging`) that converges is **legal**; only a model set that never stabilises across the bounded rounds is a circular-meta-dependency error.

**Pre-conditions.** None.

**TDD tests to write first:**
- `crates/smelt-cli/src/python.rs::tests::self_referential_convergent_generation_is_legal` — a generator that emits models carrying a tag a sibling queries, which **converges** within the bound, produces **no** circular error (flip the existing `test_circular_meta_dependency` / `test_validate_fixed_point_detects_circular_tag` at `python.rs:918,1304` to assert the convergent case is accepted).
- `...::non_convergent_set_errors` — a set whose output keeps changing through round 5 → non-convergence error (`python.rs:360-364`).

**Implementation shape.** Remove `validate_fixed_point`'s self-tag/self-dir rule (`python.rs:384-426`) and its call site (`python.rs:353`); rely solely on the existing exhausted-rounds non-convergence error (`python.rs:360-364`). Keep the Python-only rounds loop shape (combined loop is W5b).

**Critical files.** `crates/smelt-cli/src/python.rs`.

**Review checklist:** convergent self-referential generation accepted; non-convergence still errors; no combined-loop work; the two stale tests flipped; dual gate green.

**Commit.** `fix(cli): Python circularity is non-convergence, not self-tag/self-dir (D-23)`

---

### Phase P2: Python canonical address = directory-prefix + function name

**Goal.** A Python model's `address_segments` is path-derived exactly like SQL models — the `.py` file's directory address (workspace-relative path minus any `paths:` prefix) + the function name.

**Pre-conditions.** **W1** (the `compute_address_segments` / strip logic this reuses).

**TDD tests to write first:**
- `crates/smelt-cli/src/python.rs::tests::python_address_is_path_derived` — `@model def users` in `py/archive.py` → `address_segments == ["archive", "users"]` (address `smelt.archive.users`), not `["users"]`; a root-level `.py` → `["users"]`.
- `...::python_sql_address_collision_is_duplicate_address` — a Python `@model def x` whose path-derived address equals a SQL model's address → `DuplicateAddress` (the CLI-side `resolve_address_map` now sees the populated segments).

**Implementation shape.** Replace `address_segments: vec![output.name.clone()]` (`python.rs:345`) with the path-derived segments: compute the `.py` file's directory address via the same logic W1 lands in `smelt_core` (`compute_address_segments` / the `paths:`-strip helper), then append the function name as the leaf. Remove the "Phase 5" TODOs (`python.rs:619,631,651,663,1327`). Ensure the CLI-side combined-model `resolve_address_map` consumes the populated segments.

**Critical files.** `crates/smelt-cli/src/python.rs`; reuse (don't duplicate) the `smelt_core` address helper from W1.

**Review checklist:** Python address path-derived, matching SQL; Python↔SQL and Python↔Python collisions surface `DuplicateAddress`; reuses W1's helper (no duplicate strip logic); TODOs removed; gates green.

**Commit.** `feat(cli): path-derive Python model address_segments like SQL models (D-26)`

---

### Phase P3: `find_models` location = full `path`

**Goal.** `find_models` results expose the full workspace-relative `path` (matching `ModelRef.path`), with `directory` defined as its final component.

**Pre-conditions.** P2 (the path computation).

**TDD tests to write first:**
- `crates/smelt-cli/src/python.rs::tests::project_context_exposes_full_path` — `build_project_context` (`python.rs:147-158`) populates `path` = full workspace-relative `/`-normalised path; `directory` = final component of that path.
- A Python-SDK-level test (or a Rust round-trip) that `find_models(directory=…)` still matches on the final component, now defined as `path`'s last segment.

**Implementation shape.** Add a `path` attribute to `ModelInfo` (`python/smelt/core.py:4-8`) and the Rust `ProjectModelInfo` (`python.rs:147-158`); set `path` to the full workspace-relative path; define `directory` as the final component of `path` (derive, don't store independently). Keep `find_models(directory=…)` filtering on that derived `directory`.

**Critical files.** `python/smelt/core.py`, `crates/smelt-cli/src/python.rs`; possibly `docs-site/docs/...` Python-model guide if the reviewer flags the new `path` attribute as user-facing.

**Review checklist:** `path` full + `/`-normalised, matching `ModelRef.path` vocabulary; `directory` derived from `path` (never disagrees); `find_models(directory=…)` unchanged in behavior; gates green.

**Commit.** `feat(python): expose full path on ModelInfo; derive directory from it (D-25)`

---

### Phase P4: Python frontmatter — plain delimiter + name-mismatch handling

**Goal.** Python output is parsed as plain `---`/`---` single-model frontmatter (the `--- name: X ---` multi-model delimiter never applies to Python); a `name:` body key ≠ function name is a hard `PythonModelNameMismatch` Error that blocks the build, while the model keeps its other frontmatter keys.

**Pre-conditions.** None hard.

**TDD tests to write first:**
- `crates/smelt-cli/src/python.rs::tests::python_name_mismatch_blocks_and_retains_other_keys` — Python output `---\nname: other\nmaterialization: table\ntags: [x]\n---\nSELECT …` (function `combined`) → `PythonModelNameMismatch` Error **and** the model retains `materialization: table` / `tags: [x]` (only `name:` flagged). Update the stale `test_python_model_frontmatter_name_mismatch_emits_diagnostic` (`python.rs:1426-1527`) which currently asserts metadata is dropped.
- `...::python_plain_frontmatter_single_model` — plain `---` frontmatter (no name key) parses as single-model with identity = function name.
- `...::python_multimodel_delimiter_not_a_section` — `--- name: X ---` in Python output does **not** create a multi-model section (it's not the Python surface); it's handled as single-model / flagged per the name rule.

**Implementation shape.** Route Python output through **single-model** frontmatter extraction (`smelt_core::metadata::extract_file_metadata`, `metadata.rs:416-448`) — Python output is never multi-model. In the mismatch branch (`python.rs:236-328`), stop returning `None` (which drops everything): keep the parsed metadata minus the `name:` key, and emit `PythonModelNameMismatch` (Error). Ensure `PythonModelNameMismatch` is a catalogued `DiagnosticCode` (add the variant + diagnostics.md row if it's currently only a string-prefixed `ParseError`).

**Critical files.** `crates/smelt-cli/src/python.rs`, `crates/smelt-core/src/metadata.rs` (single-model extraction for the Python path), `crates/smelt-db/src/diagnostics_types.rs` (if the code variant is missing), `docs/specs/diagnostics.md` (catalogue row only if the variant is added).

**Review checklist:** Python output is single-model frontmatter; `--- name: X ---` never creates a Python section; mismatch is a blocking Error retaining other keys (only `name:` flagged); `PythonModelNameMismatch` catalogued; the stale test flipped; catalogue + dual gates green.

**Commit.** `fix(cli): Python single-model frontmatter; PythonModelNameMismatch blocks build, retains other keys (D-22, D-27)`

---

### Phase P5: Close-out

**Goal.** Retract now-satisfied Known-Divergence notes, roll up.

**Pre-conditions.** P1–P4 done.

**TDD tests to write first:** none new — runs the gates.

**Implementation shape.** Retract any python_models.md Known-Divergence note this wave satisfies (e.g. the BUG-038 name-override-ignored note, the bare-function-name-address note, the `directory`-final-component note now that `path` exists) — timeless edits. **Keep** any note that still describes a W5b gap (the combined-loop interleaving). Flip the master registry W5 row to `done (2026-06-13)`; add a `docs/ROADMAP.md` line.

**Critical files.** `docs/specs/python_models.md` (KD retraction only), `docs/plans/20260613-spec-impl.md`, `docs/ROADMAP.md`.

**Review checklist:** retractions genuinely satisfied + timeless; combined-loop (W5b) gap note retained; registry row `done`; ROADMAP updated.

**Commit.** `docs(spec-impl): close out W5 — Python-model reconciliation landed; registry + roadmap`

---

## Deferred during implementation

(Append-only.)

- The **combined fully-interleaved SQL↔Python fixed-point loop** (D-24) is **W5b** — W5 keeps the Python-only rounds loop and only redefines its circularity rule; the non-convergence semantics carry over to the combined loop.

## Blocked phases

Append-only log. None yet.

## Verification

- `cargo test -p smelt-cli` (incl. `python.rs::tests`), `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-lsp --test example_workspaces` green; scoped `example_builds` for `test_workspace` (Python models) green.
- Manual smoke: a `@model def users` in `py/archive.py` addresses as `smelt.archive.users` and collides loudly with a SQL `archive.users`; a convergent self-referential generator pair builds clean; `find_models()` results carry a full `path`; a Python `name:` ≠ function name blocks the build but keeps `materialization`/`tags`.
- `/smelt:validate python_models` reports no behavioural drift on these surfaces (the combined-loop interleaving remains a documented W5b gap).
