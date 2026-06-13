# Plan: W1 — Universal discovery & `paths:`-strip addressing (D-resolve)

**Parent (master plan)**: `docs/plans/20260613-spec-impl.md` — the first wave of the spec-remediation implementation backlog. Remediates the **D-resolve** cluster of the 2026-06-13 spec review: **D-01/D-05** (no dedicated scan roots — discover every non-excluded subdirectory, kind by content/extension; `paths:` is a strip-list, not a scan gate), **D-02** (`DuplicateEmittedName` — fail loud on a non-injective `_`-join clobber), **D-06** (one address-based uniqueness rule), **D-04** (`schema` optional, default `main`). The autonomy loop works this sub-plan phase by phase and rolls up to the master only when it is exhausted.

**Date**: 2026-06-13
**Spec**: `docs/specs/architecture.md` §"Resolution" (universal scan, `paths:`-strip rule, single address-uniqueness rule) + §"Default materialization name mapping" (the `DuplicateEmittedName` paragraph) + §"Workspace loading parity rule (CLI ↔ LSP)" (project-wide eager discovery, no hardcoded `functions/` gate); `docs/specs/smelt_yml.md` §"Top-level keys" (`paths:` strip-list, Semantics 5/8) + §"Target object" (`schema` optional, default `main`); `docs/specs/seeds.md` §"What a seed is" (project-wide discovery); `docs/specs/diagnostics.md` Models/core table (`DuplicateEmittedName` row).
**Spec diff**: `e862ebec..HEAD` — **already landed** (the 2026-06-13 review committed all four decisions to the specs above). No further spec edits in this plan except the P5 close-out retraction of any now-satisfied Known-Divergence note. This plan is code-catching-up-to-spec.
**Tracking branch**: `worktree-spec_review`
**Docs**: code-only. The user-visible specs already landed; no `docs-site/` page changes (the per-key reference already reflects the strip-list framing per the review). The close-out updates the master registry + `docs/ROADMAP.md` only.

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then read the spec sections above — they are the correctness oracle; do not re-open the settled decisions. Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked` rows) using the per-phase routine below (pre-flight → red-green `/smelt:implement` with implementer + reviewer, spec as oracle → verification gates → set the row to `done` + date → commit + push with the phase's commit message). If that was the last `pending` phase, also flip this sub-plan's Status to `done (<today>)` in the master registry and commit together. Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>` (record + continue, see §"Block conditions"), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>` (sub-plan exhausted), or `<<ALL_DONE>>`. A block is recorded and the loop continues — there is no hard-stop.

## Goal

Make smelt's discovery and addressing match the universal-addressing model the review settled on:

- **D-01/D-05 (major, behavioural).** Today discovery is **gated** by `config.paths` (`crates/smelt-core/src/discovery.rs:158-205` walks only the listed dirs; functions are hardcoded to `<root>/functions/` at `discovery.rs:125-143`; seeds/sources scan `config.paths` via `crates/smelt-db/src/queries/project.rs:151-176`). The spec now mandates **project-wide** discovery — walk *every* non-excluded subdirectory under the project root, determine kind from file format/content (the existing `classify()` at `resolver.rs:115-164`), never from location — and repurposes `paths:` as a pure **address strip-list**. A model at the project root addresses as `smelt.<stem>`; a function in `random/x.sql` is callable as `smelt.random.helper`; a `sources/raw/events.yml` (not in `paths:`) addresses as `smelt.sources.raw.events`.
- **D-04 (minor).** `targets.<name>.schema` is currently a required `String` (`config.rs:120-139`). The spec makes it **optional, default `main`**.
- **D-02 (major, correctness).** The default DB name is `<schema>.<address joined by _>` (`crates/smelt-runtime/src/compile.rs:949,954`), a deliberately readable but **non-injective** map: `smelt.staging.orders` and `smelt.staging_orders` both emit `main.staging_orders`. `project_address_collisions` (which checks *address* uniqueness) cannot catch this. Add a sibling structural check — **emitted-name uniqueness** over `(active-target schema, joined table name)` across all persisted entities — emitting `DuplicateEmittedName` (Error) rather than silently clobbering (fail-loud: a clobber is "wrong data, exit 0").
- **D-06 (cleanup).** The single normative uniqueness rule is address-based (`project_address_collisions`); confirm no residual file-stem-uniqueness check remains.

## Design decisions (resolved — do not re-litigate; from `docs/research/20260613-spec-remediation-decisions.md` D-01/02/04/05/06 + the spec)

- **Discovery is unconditional; `paths:` only strips addresses.** Walk every non-excluded subdirectory under the project root. **Excluded** = hidden directories (`.`-prefixed, e.g. `.git`, `.smelt`) and the conventional build output `target/` (the fixed skip-list; an `exclude:` key is an explicit Known-Divergence/Open-Question, **not** in scope here). Kind comes from `classify()` (extension + content), never from directory.
- **Address = path relative to the project root, minus any matching `paths:` prefix.** Under the default `paths: ["models"]`, stripping `models/` reproduces today's addresses exactly — so existing example workspaces keep identical addresses (the safety rail for the P1 pure-function change). A root-level file → bare leaf name. Multiple `paths:` prefixes are each stripped independently into one shared namespace (already the rule for collisions).
- **No dedicated `functions/` gate.** Function discovery is folded into the universal walk: any `.sql` declaring `smelt.define` is a function wherever it lives; its address is its location (a kind-named `functions/` dir survives in the address only because it isn't stripped, not via special-casing). Drop the hardcoded `project_root.join("functions")` walk.
- **`DuplicateEmittedName` is structural and shares the identity authority's home.** It depends only on each persisted entity's address + the active target's schema — never on row contents — so it lives beside `project_address_collisions` in `smelt-db/src/queries/project.rs` and is a new `DiagnosticCode` variant (like `DuplicateAddress`), **not** a `MetadataError` (so the `map_metadata_error_to_diagnostic` exhaustiveness gate is untouched). Error severity, anchored at the second entity, project-scoped, evaluated **per active target**. Excludes non-persisted entities (`smelt.define` functions, ephemeral models, sources are *included* — they need a target name). Use the active/default target's resolved schema (with D-04's `main` default).
- **Reuse, don't rebuild.** `resolve_address_map` (`resolver.rs:236-303`) and `project_address_collisions` (`project.rs:276-330`) stay; W1 changes the *discovery* that feeds them (universal walk) and the *address derivation* (`compute_address_segments`, `discovery.rs:207-230`), and adds the emitted-name projection alongside the address map.
- **Salsa purity + workspace-loading parity hold.** The universal-walk logic stays pure in `smelt-core` (the eager `load_workspace` and the lazy `project_seeds`/`project_sources` queries both consume the same project-wide universe so CLI↔LSP parity is by construction); the Salsa queries stay thin. Do not split discovery into a second pass.

## Per-phase routine
1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. If red on this phase's own acceptance target (the test it exists to make green), proceed. If red on **unrelated** breakage, treat as a block (record + continue, §"Block conditions").
2. **Red-green `/smelt:implement`.** Write the phase's failing test(s) first, then the implementation, spec as oracle. Implementer pass, then reviewer pass (material findings only).
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green; plus the **dual gate** `cargo test -p smelt-cli --test example_diagnostics` **and** `cargo test -p smelt-lsp --test example_workspaces` (the LSP gate is what catches CLI/Salsa discovery divergence — the whole point of this cluster). For `example_builds`, run **scoped**: `SMELT_EXAMPLE_BUILDS_ONLY="<ws…>" cargo test -p smelt-cli --test example_builds`.
4. **Record + commit.** Set the table row to `done` + date; commit + push tests + impl + table together with the phase's commit message. Emit `<<PHASE_COMPLETE>>` (or `<<ALL_DONE>>`/roll-up on the last phase).

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue, no hard-stop)
Set the row to `blocked` with a one-line reason; append a dated entry to §"Blocked phases" (phase id, reason, candidate options); restore the tree to a clean committed state; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:
- A design decision **not** answered by this plan or the spec — e.g. the **multi-target** emitted-name check (which target's schema when targets differ) needs a product call; the `DuplicateEmittedName` anchor range needs a UX call; or the universal walk surfaces a non-obvious classification ambiguity.
- Pre-flight red on **unrelated** breakage.
- The tree can't be returned to green after the phase (e.g. an example workspace has a genuine, spec-correct new collision that needs a fixture redesign larger than this phase).

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| P1 | Addressing: `paths:` as strip-list; address = rel-to-root minus prefix; root file → bare name | done | D-01 (addr), D-05 (addr) | feat(core): address = project-relative path with paths-prefix stripped (D-01) | 2026-06-13 |
| P2 | Universal discovery: walk every non-excluded subdir, kind by content; functions anywhere; seeds/sources project-wide | done | D-01 (disc), D-05 (disc) | feat(core): project-wide discovery by file kind, no scan-root gate (D-01, D-05) | 2026-06-14 |
| P3 | `schema` optional, default `main` | done | D-04 | feat(core): default target schema to `main` when omitted (D-04) | 2026-06-14 |
| P4 | `DuplicateEmittedName` emitted-name collision diagnostic | done | D-02 | feat(db): enforce emitted (schema,table) uniqueness via DuplicateEmittedName (D-02) | 2026-06-14 |
| P5 | Close-out: single address-rule audit (D-06) + KD retraction + registry/ROADMAP | done | D-06 | docs(spec-impl): close out W1 — universal addressing landed; registry + roadmap | 2026-06-14 |

**Status values**: `pending`, `done`, `blocked`. A phase is `done` only when its tests are red-green confirmed and all gates are green. A `blocked` phase has a dated §"Blocked phases" entry and returns to `pending` once a human resolves it.

---

### Phase P1: Addressing — `paths:` as strip-list

**Goal.** Make address derivation compute the entity address as its path relative to the project root with any matching `paths:` prefix stripped; a root-level file becomes its bare leaf name. Pure-function change, behaviour-preserving under the default `paths: ["models"]`.

**Pre-conditions.** None (first phase).

**TDD tests to write first** (write failing first):
- `crates/smelt-core/src/discovery.rs::tests::address_root_level_file_is_bare_name` — a `.sql` directly in the project root → `address_segments == ["<stem>"]`.
- `crates/smelt-core/src/discovery.rs::tests::address_strips_only_configured_prefix` — with `paths: ["models"]`, `sources/raw/events.yml`-style path keeps `["sources","raw","events"]` (prefix not stripped) while `models/marts/x.sql` → `["marts","x"]`.
- `crates/smelt-core/src/discovery.rs::tests::address_multi_prefix_independent_strip` — with `paths: ["models","fixtures"]`, both `models/users.sql` and `fixtures/users.sql` → `["users"]` (so they collide — exercised in P4/collision tests, here just assert the segments).
- Confirm the existing `canonical_path_at_scan_root` / `canonical_path_*` tests (`discovery.rs:565-685`) still pass unchanged (default-layout invariance).

**Implementation shape.** Rework `ModelDiscovery::compute_address_segments` (`discovery.rs:207-230`) so the stripping key is the set of `config.paths` prefixes applied to the **project-root-relative** path, rather than the single matched `scan_root`. Add a small pure helper `strip_paths_prefix(rel_path, &config.paths) -> Vec<String>`. Apply the same derivation to seed/source `address_segments` (wherever `discover_seed_infos*` / `discover_source_infos` compute them) so all kinds share one rule. Keep the discovery universe unchanged this phase (still paths-gated) — only the address math changes.

**Critical files (allowed to touch).**
- `crates/smelt-core/src/discovery.rs` — `compute_address_segments`, new strip helper, unit tests.
- `crates/smelt-core/src/{seeds,sources}.rs` (or wherever seed/source `address_segments` are computed) — adopt the shared strip rule.

**Review checklist** (material only):
- [ ] TDD tests exist and assert the spec's address rule (root→bare, strip-only, multi-prefix).
- [ ] Default-layout addresses unchanged (existing `canonical_path_*` tests green).
- [ ] One address-derivation rule shared by models + seeds + sources (no per-kind divergence).
- [ ] No discovery-universe change leaked into this phase.

**Commit.** `feat(core): address = project-relative path with paths-prefix stripped (D-01)`

---

### Phase P2: Universal discovery by file kind

**Goal.** Replace the `config.paths`-gated, per-kind walks with one project-wide walk over every non-excluded subdirectory, classifying each file by format/content; functions are discoverable anywhere (drop the hardcoded `functions/` gate); seeds and sources are discovered project-wide.

**Pre-conditions.** P1 (addresses already computed by the strip rule, so files found outside `models/` get correct addresses).

**TDD tests to write first:**
- `crates/smelt-core/src/discovery.rs::tests::discovers_model_outside_models_dir` — a bare-SELECT `.sql` under `billing/staging/` (with default `paths`) is discovered and addressed `smelt.billing.staging.<stem>`.
- `crates/smelt-core/src/discovery.rs::tests::discovers_function_anywhere` — a `smelt.define helper` in `random/x.sql` is discovered as a function callable at `smelt.random.helper` (no `functions/` dir).
- `crates/smelt-core/tests/...::seed_and_source_discovered_project_wide` — a `.csv` (seed) and a standalone `.yml` (source) under an arbitrary domain dir are discovered without being listed in `paths:`.
- `crates/smelt-core/src/discovery.rs::tests::excludes_hidden_and_target_dirs` — files under `.smelt/`, `.git/`, and `target/` are **not** discovered.
- A new example fixture `examples/architecture_domain_layout/` (models/sources/seeds co-located under one domain subtree) builds clean: dual gate green.

**Implementation shape.** Introduce one project-wide directory walk in `smelt-core` (extend/replace `ModelDiscovery::discover_models` `discovery.rs:158-205` and fold in `discover_function_file_paths` `discovery.rs:125-143`) that yields all candidate files, applying the fixed exclusion skip-list (hidden dirs + `target/`), then `classify()` (`resolver.rs:115-164`) per file. Route the eager `load_workspace` (`workspace.rs:78`) and the lazy `project_seeds`/`project_sources` (`smelt-db/src/queries/project.rs:151-176`) through the **same** project-wide universe (replace their `config.paths` scan-gate use with the universal walk; `config.paths` now feeds only addressing). Preserve workspace-loading parity (one `load_workspace`) and Salsa purity (pure walk in `smelt-core`, thin query wrapper).

**Critical files (allowed to touch).**
- `crates/smelt-core/src/discovery.rs` — universal walk + exclusion list; subsume function discovery.
- `crates/smelt-core/src/workspace.rs` — `load_workspace` consumes the universal walk; drop the hardcoded `functions/` path.
- `crates/smelt-core/src/{seeds,sources}.rs` — discovery no longer gated by `paths:`.
- `crates/smelt-db/src/queries/project.rs` — `project_seeds`/`project_sources`/`project_paths` consume the universal universe; `paths:` retained only for address stripping.
- `examples/architecture_domain_layout/**` — new fixture.

**Review checklist:**
- [ ] Discovery walks every non-excluded subdir; kind from `classify()`, never from location.
- [ ] Hidden dirs + `target/` excluded; functions found without a `functions/` gate.
- [ ] Eager (`load_workspace`) and lazy (`project_seeds`/`project_sources`) share one universe — CLI↔LSP parity holds (dual gate green).
- [ ] Salsa purity preserved (pure walk in `smelt-core`).
- [ ] Existing example workspaces still green; any genuinely new spec-correct collision is surfaced (block if a fixture redesign is needed).

**Commit.** `feat(core): project-wide discovery by file kind, no scan-root gate (D-01, D-05)`

---

### Phase P3: `schema` optional, default `main`

**Goal.** Make `targets.<name>.schema` optional, defaulting to `main` when omitted.

**Pre-conditions.** None hard; sequence before P4 (emitted-name uses the resolved schema).

**TDD tests to write first:**
- `crates/smelt-core/src/config.rs::tests::target_schema_defaults_to_main` — a target YAML omitting `schema` parses with `schema == "main"`.
- `crates/smelt-core/src/config.rs::tests::explicit_schema_is_honored` — an explicit `schema: analytics` is preserved.
- An example/runtime test asserting a model under a schema-less target materialises at `main.<name>` (reuse the emitted-name path in `compile.rs`).

**Implementation shape.** Change `Target.schema` (`config.rs:120-139`) to default via serde (`#[serde(default = "default_schema")]` returning `"main"`), keeping the field a `String` so downstream readers (`compile.rs:949,954`) are unchanged. Audit any code that treated a missing schema as an error.

**Critical files (allowed to touch).**
- `crates/smelt-core/src/config.rs` — `Target.schema` default + tests.

**Review checklist:**
- [ ] Omitted `schema` → `main`; explicit value preserved.
- [ ] No downstream reader regressed (emitted-name still `<schema>.<segs_>`).

**Commit.** `feat(core): default target schema to `main` when omitted (D-04)`

---

### Phase P4: `DuplicateEmittedName` collision diagnostic

**Goal.** Fail loud when two persisted entities resolve to the same `(active-target schema, joined `_` table name)`, even though their addresses differ — preventing a silent table clobber.

**Pre-conditions.** P1–P3 (addresses + schema default in place).

**TDD tests to write first:**
- `crates/smelt-core/src/resolver.rs::tests::emitted_name_collision_detected` — addresses `["staging","orders"]` and `["staging_orders"]` under schema `main` both emit `main.staging_orders` → one collision; distinct emitted names → none; functions/ephemeral excluded; sources included.
- `crates/smelt-cli/tests/emitted_name_collision.rs::smelt_build_refuses_emitted_name_collision` — an example workspace with the two colliding models → `smelt build`/`explain --json` exits non-zero with exactly one `DuplicateEmittedName` Error.
- `crates/smelt-cli/tests/...::clean_workspace_has_no_emitted_name_collision` — known-good examples emit none.
- A `smelt-lsp` `example_workspaces` assertion that the fixture surfaces exactly one `DuplicateEmittedName` (CLI↔LSP parity).
- Fixture `examples/architecture_broken_emitted_name_collision/` (two models whose addresses differ but `_`-join to one name).

**Implementation shape.** Add `DiagnosticCode::DuplicateEmittedName` (`crates/smelt-db/src/diagnostics_types.rs` beside `DuplicateAddress:690-696`). Add a pure projection in `smelt-core` (next to `resolve_address_map`, `resolver.rs:236-303`) that maps each **persisted** entity to `(schema, segs.join("_"))` and reports collisions (`EmittedNameCollision { name, first, second }`); persisted = models materialising to table/view/materialized_view, target-schema seeds, sources — excludes `smelt.define`, ephemeral. Add a `project_emitted_name_collisions` Salsa query (`smelt-db/src/queries/project.rs` beside `project_address_collisions:276-330`) reading the **address-only + schema** projection (never row contents), resolving the schema from the active/default target (D-04 default). Route into the build gate + LSP exactly like `project_address_collisions`. Classify the new code wherever the diagnostics catalogue/coverage gate requires (the diagnostics.md row already exists). **No** `MetadataError` variant — this is structural like `DuplicateAddress`.

**Critical files (allowed to touch).**
- `crates/smelt-db/src/diagnostics_types.rs` — new `DiagnosticCode` variant.
- `crates/smelt-core/src/resolver.rs` — emitted-name projection + collision struct.
- `crates/smelt-db/src/queries/project.rs` — `project_emitted_name_collisions` query + diagnostic wiring.
- `crates/smelt-cli/tests/emitted_name_collision.rs`, `examples/architecture_broken_emitted_name_collision/**`, and the `smelt-lsp` example-workspaces assertion.

**Review checklist:**
- [ ] Fires on `(schema, joined name)` collision across distinct addresses; not on distinct emitted names.
- [ ] Persisted-only (functions/ephemeral excluded; sources included); evaluated per active target.
- [ ] Structural — depends only on address + target schema, never row contents (a CSV content edit does not change the diagnostic).
- [ ] CLI↔LSP parity (both gates assert the one Error); fail-loud (non-zero exit, no silent clobber).
- [ ] If multi-target schemas make "active target" ambiguous in the Salsa context → **block** for a product call rather than guess.

**Commit.** `feat(db): enforce emitted (schema,table) uniqueness via DuplicateEmittedName (D-02)`

---

### Phase P5: Close-out — single address-rule audit + retraction

**Goal.** Confirm address-collision is the **single** uniqueness rule (no residual file-stem-uniqueness check), retract any now-satisfied Known-Divergence note, and roll up the wave.

**Pre-conditions.** P1–P4 done.

**TDD tests to write first:**
- A test (or audit assertion) that a same-stem file pair registering **different** addresses (e.g. `data/users.csv` seed + a function-only `data/users.sql` declaring `smelt.define helper`) is **allowed** (no stem rule), while same-address pairs still error (`DuplicateAddress`). If a stem-uniqueness check still exists in code, this test is red until it's removed.

**Implementation shape.** `rg` for any file-stem-uniqueness enforcement (the dead `walk_paths` stem resolver was superseded by the address authority — confirm it's gone or has no production callers). Remove any residual stem rule. If `architecture.md`/`smelt_yml.md` carries a Known-Divergence note that this wave now satisfies (e.g. a "paths is a scan gate" or "address-uniqueness not yet enforced across kinds" remnant), retract it (timeless edit). Update the master registry row to `done (2026-06-13)` and add a `docs/ROADMAP.md` line.

**Critical files (allowed to touch).**
- `crates/smelt-core/src/resolver.rs` (stem-rule removal if any), the relevant spec file (KD retraction only), `docs/plans/20260613-spec-impl.md` (registry row), `docs/ROADMAP.md`.

**Review checklist:**
- [ ] Address-based collision is the only uniqueness rule; no stem rule remains.
- [ ] Any retracted Known-Divergence is genuinely satisfied; spec edit is timeless (no phase vocabulary).
- [ ] Master registry row flipped to `done`; ROADMAP updated.
- [ ] Full dual gate + scoped `example_builds` green.

**Commit.** `docs(spec-impl): close out W1 — universal addressing landed; registry + roadmap`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- Configurable discovery exclusions (`exclude:` key) — Open Question in `smelt_yml.md`; the fixed hidden-dir + `target/` skip-list is what W1 implements.
- Per-entity emitted-name **overrides** (the manual escape hatch the spec mentions "once per-entity name overrides land") — not in W1.

## Blocked phases

Append-only log of phases the loop recorded as `blocked` and continued past. Each entry: date, phase id, reason/decision, candidate options. None yet.

## Verification

- `cargo test -p smelt-core` (discovery/addressing units), `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-lsp --test example_workspaces` all green.
- Scoped `SMELT_EXAMPLE_BUILDS_ONLY="architecture_domain_layout architecture_broken_emitted_name_collision" cargo test -p smelt-cli --test example_builds`.
- Manual smoke: a workspace with a root-level `foo.sql` resolves to `smelt.foo`; a domain-grouped layout (`billing/staging/x.sql`, `billing/raw/y.yml`, `billing/seeds/z.csv`) resolves to `smelt.billing.*`; a deliberate `_`-join clash raises `DuplicateEmittedName`.
- `/smelt:validate architecture` and `/smelt:validate smelt_yml` report no behavioural drift on the Resolution / `paths:` / `schema` surfaces.
