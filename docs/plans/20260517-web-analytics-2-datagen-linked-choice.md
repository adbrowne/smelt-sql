# Plan: Web Analytics Phase 2 — datagen `linked_choice` + `linked_pools`

**Date**: 2026-05-18
**Spec**: [`docs/specs/datagen.md`](../specs/datagen.md) §Surface (`linked_choice` row + `linked_pools:` YAML block + worked example), §Semantics (`linked_choice` and joint-distribution pools), §Design (`linked_choice` pool-and-reference, weighted-shape vocabulary, isolated pool RNG, pools-cannot-reference-pools), §Constraints & Invariants (items 6–9), §Known Divergences (`linked_choice` entries)
**Spec diff**: committed as `ebe4da8a` (`spec(datagen): linked_choice + linked_pools surface, semantics, design (web-analytics Phase 2)`)
**Tracking branch**: `worktree-web_analytics` (overall plan: [`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md`)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive Phase 2 to completion using `/smelt:implement`, then dispatch the meta-plan §5 expert reviewers (datagen-expert, docs-reviewer), then update the in-repo overall-plan status table and push.

**Before touching any code:**

1. Read this plan in full. Then read `docs/specs/datagen.md` — the `linked_choice` / `linked_pools` entries in Surface, Semantics, Design, Constraints, and Known Divergences are the correctness oracle. Do not re-open settled spec decisions; if a spec rule blocks a green test, run `/smelt:spec datagen` to revise the spec rather than encode the divergence in code.
2. Confirm you are on branch `worktree-web_analytics`. If not, ask before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table below. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent (`model: sonnet`) → reviewer subagent (`model: sonnet`) → iterate → record + commit + push.

**Phase 6 is the expert-reviewer dispatch loop** — after Phases 1–5 commit, dispatch the meta-plan §5 expert reviewers applicable to this phase (`datagen-expert`, `docs-reviewer`), address material findings, and re-dispatch each expert until clean (or stop-the-line per meta-plan §7). Do NOT skip Phase 6. The autonomy loop's `<<PHASE_COMPLETE>>` sentinel may only fire once Phase 6's acceptance gate is met and the overall-plan status row is updated.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec datagen` first to update).
- `cargo test` or `cargo clippy --all-targets` surfaces a pre-existing failure unrelated to the plan.
- Phase 6: an expert flags the same material finding on round 3 (per-expert bound), or two different experts flag the same systemic concern in the same round.

**Conventions every phase:**

- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Subagent model rule: implementer + reviewer + every expert in Phase 6 spawn with `model: "sonnet"`. Do not let them inherit `opus` from the parent autonomy loop.
- Never skip hooks, never `--no-verify`, never force-push the tracking branch.
- Don't widen scope: this plan introduces `linked_choice` + `linked_pools` only. Sample-pipeline code is Phase 3+. No raw-tuple-list mode, no entity-scoped pool draws — those are open questions in the spec's Known Divergences.
- Honor architectural invariants from `CLAUDE.md`: `apply_spec` stays pure of I/O. The new pool-construction code may add a single-shot pre-row-loop pass, mirroring `EntityPool::new`'s shape. No state shared across row generators except via the same arc-shared read-only structures the entity pool already uses.

---

## Context

The web-analytics example (overall plan: [`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md)) needs correlated `(device_id, user_id)` columns on event rows with a realistic co-occurrence distribution: 60% single-owner devices, 25% anonymous-only sessions, 10% shared-device cases, 5% multi-device users. Drawing the two columns independently from foreign-key generators gives every (device, user) pair equal probability, which loses the real distribution that the three identity-stitching algorithms (Phases 5–7) are meant to discriminate between. The user agreed to add a new datagen feature — `linked_choice` + `linked_pools` — rather than push the joint distribution into hand-rolled fixture YAML. The spec increment was committed in `ebe4da8a`. Phase 2 of *this* plan implements the spec.

## Scope

### In scope (spec coverage)

- `datagen.md` §Surface — `linked_choice` generator row in the "Composite / structured" table; `linked_pools:` block in the dataset-config YAML reference; worked four-shape example.
- `datagen.md` §Semantics — pool construction (`weight`, `emit`, `sticky`, truncate-to-pool_size); row-time same-tuple-per-row sampling; isolated pool RNG stream; partition + entity interaction; type uniformity across shapes.
- `datagen.md` §Design — five rationale paragraphs (pool-and-reference, weighted-shape vocabulary, isolated pool seed, no nested pools, four-shape table).
- `datagen.md` §Constraints & Invariants — items 6–9 (absolute `pool_size`, shape field-name/type agreement, `linked_choice` forbidden inside shapes, reference resolution).
- `datagen.md` §Known Divergences — five new entries (no entity-scoped pool draws, no raw tuple list, no `emit:` upper-bound check, cross-shape type check is build-time, `--scale-factor` does not scale pools).
- Rust implementation:
  - `crates/smelt-datagen/src/config.rs`: new `LinkedPoolConfig { name, pool_size, seed, shapes: Vec<ShapeConfig> }`, `ShapeConfig { weight, emit, sticky, fields: IndexMap<String, GeneratorSpec> }`, optional `linked_pools: Vec<LinkedPoolConfig>` on `DatasetConfig`, new `GeneratorSpec::LinkedChoice { pool: String, field: String }` variant. Field-type uniformity across shapes enforced at parse time.
  - `crates/smelt-datagen/src/generic.rs`: new `LinkedPool { rows: Vec<IndexMap<String, GenericValue>> }` (or `Vec<Vec<GenericValue>>` with a parallel `field_index: HashMap<String, usize>` — implementer's choice). `LinkedPool::new(seed, &LinkedPoolConfig, &FkCounts) -> LinkedPool` that runs the shape-draw loop and truncates to `pool_size`. `apply_spec` arm for `LinkedChoice` that reads the per-row tuple from a passed-in `linked_pool_rows` map. Pool sampling moved into `generate_row`-equivalent (sample one tuple per pool per row, then look up each `linked_choice` column's field).
  - `crates/smelt-datagen/src/generic_parquet.rs`: pre-row-loop pool construction (mirrors the existing `make_entity_pool` call site), arc-shared across days in the partitioned path. New row-level sampler that draws one tuple per pool per row.
  - `crates/smelt-datagen/src/main.rs`: `--list-generators` gains a `linked_choice` entry; config validation rejects `linked_choice` references to undeclared pools / fields, and rejects `LinkedChoice` inside `shapes[].fields`.
- Unit tests:
  - Config parsing: `LinkedPoolConfig` round-trips through `serde_yaml`; `deny_unknown_fields`; non-empty `shapes:`; `weight: > 0`; `emit: ≥ 1`; `sticky:` is subset of `fields`; cross-shape field-name uniformity; cross-shape field-type uniformity (best-effort, see §Constraints invariant 7); rejection of `LinkedChoice` inside `shapes[].fields`; rejection of `linked_choice` column referencing undeclared pool / field.
  - Pool construction: deterministic for fixed seed; shape weight distribution roughly matches input (statistical test over a large pool); `emit` produces N entries per draw; `sticky:` fields share their value across the emitted entries; truncation makes final pool size exactly `pool_size`.
  - Row-time sampling: same row → same tuple across multiple `linked_choice` columns referencing the same pool; different pools sample independently within a row; pool sampling deterministic across runs.
  - Joint distribution end-to-end: the worked four-shape example pool, sampled into ~10K rows, statistically matches the meta-plan's 60/25/10/5 ratios for the four cases (single-owner, anonymous, shared device, multi-device user). Tolerance: ±2 percentage points.
  - Parquet round-trip: a dataset with two `linked_choice` columns writes to Parquet, reads back, and on every row the device_id and user_id values trace to the same pool entry.
  - Partition interaction: pool is shared across partitions; partition row counts sum to total; per-partition sampling is independent.
  - CLI: `smelt-datagen --list-generators` includes a `linked_choice` entry with the `pool` and `field` parameters.
- User docs: `docs-site/docs/guide/datagen.md` `linked_choice` sub-section with the four-shape example.

### Explicitly deferred

- Entity-scoped pool draws (a `linked_choice` under `entity.columns` that sticks the pool entry per entity) — open question in spec's Known Divergences.
- Raw tuple-list pool mode (`tuples: [(d, u), ...]`) — open question in spec's Known Divergences.
- Parse-time enforcement of every shape's field generator output type — v1 enforces at Arrow build time only (spec invariant 7).
- `--scale-factor` scaling of pools — spec invariant 6 explicitly excludes this.
- Real-pipeline integration (web-analytics events table actually using `linked_choice` against generated `users` and `devices` dimensions). That lands in Phase 3+ of the overall plan.

---

## Progress tracking

| Phase | Status | Commit | Date |
|-------|--------|--------|------|
| 1 | done | `ebe4da8a` | 2026-05-18 |
| 2 | done | `def93feb` | 2026-05-18 |
| 3 | done | `dfa9e587` | 2026-05-18 |
| 4 | done | `1ef5a4bc` | 2026-05-18 |
| 5 | done | *(this commit)* | 2026-05-18 |
| 6 | pending | | |

---

### Phase 1: Commit spec increment + user docs

**Goal.** Land the `datagen.md` and `docs-site/docs/guide/datagen.md` edits that introduce `linked_choice` + `linked_pools` to the spec and user guide. No code in `crates/` changes in this phase — the spec is the oracle, and committing it first means Phase 2's red TDD tests are written against a frozen spec target.

**Status.** **Done** — committed as `ebe4da8a` at the start of the Phase 2 session before this plan was written. The Phase 2 session writes this plan file as part of Phase 2 (config variant) below if not already on disk; alternatively, the plan file can be committed in a separate `chore(docs)` commit before Phase 2 starts.

**Critical files (allowed in this phase).**

- `docs/specs/datagen.md` (committed)
- `docs-site/docs/guide/datagen.md` (committed)
- `docs/plans/20260517-web-analytics-2-datagen-linked-choice.md` (this file — committed at the start of Phase 2 work, before any code changes)

**Review checklist** (material findings only):

- [x] `linked_choice` row appears in §Surface generator-types tables.
- [x] §Semantics covers pool construction, sticky/emit, RNG isolation, row-time same-tuple-per-row, partition/entity interaction, type uniformity.
- [x] §Design covers pool-and-reference, weighted-shape vocabulary, isolated pool seed, no-nested-pools, four-shape mapping table.
- [x] §Constraints invariants 6–9 added.
- [x] §Known Divergences gains the five new entries.
- [x] User-docs sub-section under "Composite / structured" exists with worked four-shape example.
- [x] No phase-vocabulary callouts in spec body sections (timeless-oracle rule from `CLAUDE.md`).

**Commit.** `spec(datagen): linked_choice + linked_pools surface, semantics, design (web-analytics Phase 2)` — done as `ebe4da8a`.

---

### Phase 2: Config types + serde parsing

**Goal.** Introduce the YAML-deserializable types — `LinkedPoolConfig`, `ShapeConfig`, the optional `linked_pools:` on `DatasetConfig`, and the `GeneratorSpec::LinkedChoice { pool, field }` variant. Validate the cross-shape and reference-resolution invariants. No row-generation behaviour yet.

**Pre-conditions.** Phase 1 committed.

**TDD tests to write first** (in `crates/smelt-datagen/src/config.rs`):

- `linked_pool_parses_minimal` — a YAML with one pool, one shape, two `fields:` (e.g. `device_id` and `user_id`, each a `foreign_key`) parses into `DatasetConfig.linked_pools = Some([LinkedPoolConfig { ... }])`.
- `linked_pool_preserves_shape_order` — shapes declared in YAML order `[A, B, C]` parse with iteration order `[A, B, C]` (matters for deterministic shape selection under fixed seed).
- `linked_pool_preserves_field_order` — within a shape, `fields:` iteration order matches YAML declaration order (matters for deterministic field RNG order).
- `linked_pool_rejects_empty_shapes` — `shapes: []` is a parse error with a clear message.
- `shape_rejects_zero_weight` — `weight: 0` is a parse error.
- `shape_rejects_negative_weight` — `weight: -0.1` is a parse error.
- `shape_rejects_zero_emit` — `emit: 0` is a parse error.
- `shape_default_emit_is_one` — `emit:` omitted parses as `emit: 1`.
- `shape_default_sticky_is_empty` — `sticky:` omitted parses as `sticky: []`.
- `shape_rejects_sticky_field_not_in_fields` — `sticky: [missing]` with `fields: { device_id: ..., user_id: ... }` is a parse error.
- `linked_pool_rejects_disagreeing_field_names_across_shapes` — two shapes with different `fields:` keys is a parse error.
- `linked_pool_rejects_linked_choice_inside_shape_fields` — a shape `fields:` containing a `type: linked_choice` is a parse error (spec invariant 8).
- `linked_pool_rejects_unknown_top_level_keys` — `pool_size: 10, shapes: [...], extra: 1` fails parse via `deny_unknown_fields`.
- `linked_pool_optional_seed_default_is_none` — `seed:` omitted leaves `seed: None` (the run-time path then derives one).
- `linked_choice_variant_parses` — a column whose generator is `{ type: linked_choice, pool: device_user, field: device_id }` parses into `GeneratorSpec::LinkedChoice { pool: "device_user", field: "device_id" }`.
- `linked_choice_arrow_type_resolved_lazily` — `arrow_type()` on a `LinkedChoice` variant **panics or returns a placeholder** (decide at impl time — placeholder Utf8 is acceptable for v1 because schema construction passes the pool definition; see Phase 4). Document the choice with a comment. If a placeholder is chosen, add a test that the eventual schema build path overrides it with the referenced field's actual type.
- `linked_choice_is_not_nullable_unless_field_is_optional` — `is_nullable()` on a `LinkedChoice` returns `false` for v1 — the nullable lookup is delegated to schema construction, which has access to the pool definition (see Phase 4).
- `dataset_config_rejects_linked_choice_pool_not_declared` — a column with `linked_choice` referencing a `pool:` name that doesn't appear in `linked_pools:` is a parse error. (May be enforced via a post-parse `validate()` step rather than serde, since serde doesn't see siblings.)
- `dataset_config_rejects_linked_choice_field_not_in_pool` — a column with `linked_choice { pool: device_user, field: missing }` where `missing` isn't in the named pool's shape `fields:` is a parse error (same post-parse pass).

**Implementation.**

- Add `LinkedPoolConfig { name: String, pool_size: usize, seed: Option<u64>, shapes: Vec<ShapeConfig> }` to `config.rs`.
- Add `ShapeConfig { weight: f64, #[serde(default = "default_shape_emit")] emit: usize, #[serde(default)] sticky: Vec<String>, fields: IndexMap<String, GeneratorSpec> }`.
- Add `linked_pools: Option<Vec<LinkedPoolConfig>>` to `DatasetConfig`.
- Add `LinkedChoice { pool: String, field: String }` to `GeneratorSpec`. Extend `arrow_type()` (placeholder `Utf8`, see test above) and `is_nullable()` (placeholder `false`).
- Add a post-parse `validate()` step (either a free function called from `main.rs`'s `run_config` before the row-write loop, or a method on `DatasetConfig`) that performs the cross-shape and reference-resolution checks. Reusing the existing FK-validation pattern in `main.rs::run_config` is the cleanest fit — extend that pass with the new checks.
- All new structs derive `#[serde(deny_unknown_fields)]`.

**Critical files.**

- `crates/smelt-datagen/src/config.rs`
- `crates/smelt-datagen/src/main.rs` — `run_config` validation pass.

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] `LinkedPoolConfig`, `ShapeConfig`, `linked_pools:` on `DatasetConfig`, `GeneratorSpec::LinkedChoice` all present with `deny_unknown_fields`.
- [ ] Cross-shape field-name agreement enforced at parse time with a clear error.
- [ ] `LinkedChoice` rejection inside `shapes[].fields` works recursively (a `LinkedChoice` nested inside an `Optional` inside a `JsonObject` inside a shape `fields` is still rejected).
- [ ] Reference-resolution validator integrates with the existing FK-validation pass (no second walk of the config tree).
- [ ] `arrow_type()` and `is_nullable()` placeholder semantics for `LinkedChoice` are documented inline with a `// see schema construction in generic_parquet.rs` pointer.

**Commit.** `feat(datagen): linked_pools + LinkedChoice config types + serde tests (web-analytics Phase 2)`

---

### Phase 3: Pool construction (`LinkedPool::new`)

**Goal.** Implement the shape-draw → pool-build algorithm. Each shape draw samples one shape by weight, draws sticky fields once, then emits `emit` entries with sticky values shared and non-sticky values redrawn. Terminate when the pool reaches `pool_size`; truncate any overshoot.

**Pre-conditions.** Phase 2 committed.

**TDD tests to write first** (in `crates/smelt-datagen/src/generic.rs::tests` or a new `linked_pool_tests.rs`):

- `test_linked_pool_size_is_exact` — a pool with `pool_size: 1000` and shapes with various `emit:` values produces exactly 1000 entries.
- `test_linked_pool_deterministic` — two builds with the same seed and config produce byte-identical pool contents.
- `test_linked_pool_seed_isolation` — changing the dataset's `num_rows` does not change the pool contents (pool RNG stream is independent of row stream).
- `test_linked_pool_emit_one_basic` — a single-shape pool with `emit: 1` produces N tuples whose fields are drawn independently per entry.
- `test_linked_pool_emit_two_sticky` — `emit: 2, sticky: [device_id], fields: { device_id: fk(devices), user_id: fk(users) }` produces pairs of entries with identical `device_id` and (almost surely) different `user_id` (with `devices: 50K, users: 50K`, collision probability is `1/50K` per pair, so over a 1000-entry pool we expect ~10 collisions; the test asserts that the device_id within each emitted pair is identical for every pair — collision tolerance is on the cross-pair non-sticky field only).
- `test_linked_pool_emit_two_sticky_user` — same as above but `sticky: [user_id]` — user_id is repeated within each emitted pair.
- `test_linked_pool_shape_weight_distribution` — a pool with three shapes weighted `[0.6, 0.3, 0.1]`, `pool_size: 100_000` (each shape `emit: 1`), groups entries by which shape produced them (track via a synthetic generator that records its origin — e.g. a constant per shape), expects shape ratios within ±1 percentage point of the input weights.
- `test_linked_pool_normalises_weights` — weights `[2.0, 1.0]` (sum 3.0) produce the same distribution as `[0.667, 0.333]` (sum 1.0) modulo RNG variance.
- `test_linked_pool_truncates_overshoot` — `pool_size: 100` with a single shape with `emit: 30` produces exactly 100 entries (the fourth shape draw would overshoot to 120; the implementation truncates).
- `test_linked_pool_handles_pool_size_one` — degenerate `pool_size: 1` works.
- `test_linked_pool_foreign_key_resolves` — a shape field that is `foreign_key { dataset: devices }` with `FkCounts: { devices: 100 }` produces values in `[1, 100]`.

**Implementation.**

- Add `LinkedPool` struct: `{ rows: Vec<Vec<GenericValue>>, field_index: IndexMap<String, usize> }`. `rows[i]` is the i-th pool entry as a tuple of `GenericValue`s in the order the shape's `fields:` were declared. `field_index` maps field name → position (built once from the first shape, since all shapes agree on field names per spec invariant 7).
- Add `LinkedPool::new(seed: u64, cfg: &LinkedPoolConfig, fk_counts: &FkCounts) -> LinkedPool`. Algorithm:
  1. Seed a `ChaCha8Rng`.
  2. Build a `WeightedIndex` over `cfg.shapes` from their `weight`s.
  3. Loop: sample a shape index, draw the sticky fields once (call `apply_spec` for each sticky field name in `fields:` declaration order), then for each of `emit:` emissions, redraw the non-sticky fields and assemble a tuple (sticky values reused, non-sticky values fresh). Append `emit` tuples to `rows`.
  4. Stop when `rows.len() >= pool_size`; truncate to `pool_size`.
- Per-pool seed resolution: `cfg.seed.unwrap_or(dataset_seed.wrapping_add(linked_pool_index as u64 + 1))` (chosen offset is `linked_pool_index + 1` so the first pool's seed is `dataset_seed.wrapping_add(1)`, which collides with the existing entity-pool offset — therefore use `+ 100 + linked_pool_index as u64` or a similar stable offset that does not collide with the entity-pool stream. Pick whichever offset matches the spec wording exactly; if the spec says `dataset_seed.wrapping_add(linked_pool_index + 1)`, update the spec via a one-line clarifying edit and `/smelt:spec datagen` — *do not* silently diverge).

**Critical files.**

- `crates/smelt-datagen/src/generic.rs` — new `LinkedPool` struct + builder.

**Docs touched.** Possibly `docs/specs/datagen.md` if the seed offset needs clarification (run `/smelt:spec datagen` if so).

**Review checklist** (material findings only):

- [ ] `LinkedPool::new` is a pure function of `(seed, cfg, fk_counts)`. No I/O, no global state.
- [ ] Shape selection uses `WeightedIndex` (matches `weighted_choice` semantics — see `generators.rs`).
- [ ] `sticky:` fields are drawn exactly once per shape draw, **before** the emit loop. Field iteration order matches YAML order.
- [ ] Pool truncation lands at exactly `pool_size` regardless of `emit` values.
- [ ] Pool RNG stream is **isolated** from the row RNG stream — changing `num_rows` does not perturb pool contents (test `test_linked_pool_seed_isolation`).
- [ ] Per-pool seed offset matches the spec exactly. If the implementer revises it, the spec is updated in lockstep via `/smelt:spec datagen` (not a silent divergence).

**Commit.** `feat(datagen): LinkedPool construction (weighted shapes + sticky + emit) (web-analytics Phase 2)`

---

### Phase 4: Row-time `linked_choice` + Parquet round-trip + schema resolution

**Goal.** Wire the row-generation path to:
1. Build all `linked_pools` for a dataset before the row loop (mirroring `make_entity_pool`).
2. For each row, sample one pool index per pool (single draw per pool per row, not per `linked_choice` column).
3. For each `linked_choice` column, look up the pool's `(field)` value at the sampled index.
4. Compute the Parquet schema using the referenced pool field's actual `arrow_type()`, not the `LinkedChoice` variant's placeholder.

**Pre-conditions.** Phase 3 committed.

**TDD tests to write first** (in `crates/smelt-datagen/src/generic_parquet.rs::tests`):

- `test_linked_choice_writes_parquet_single_file` — a dataset with two `linked_choice` columns referencing the same pool writes a Parquet file whose two columns, row-by-row, jointly trace to one pool entry. I.e. for every row, there exists exactly one pool entry whose `(device_id, user_id)` equals `(row.device_id, row.user_id)`. Tested by reading the pool back deterministically (rebuild with the same seed) and looking up each row.
- `test_linked_choice_writes_parquet_partitioned` — same, with date partitioning. Pool is shared across partitions; per-partition row counts sum to total.
- `test_linked_choice_schema_resolves_field_type` — a pool whose `device_id` field is `foreign_key { dataset: ... }` (`Int32`) and `user_id` is `optional<foreign_key>` (`Int32`, nullable) produces a Parquet schema with `device_id: Int32 NOT NULL` and `user_id: Int32 NULL`. The `LinkedChoice` variant's placeholder `arrow_type()` is overridden in schema construction.
- `test_linked_choice_same_row_same_tuple` — for every row, every `linked_choice` column referencing pool P sees the same pool entry index. (Verify by writing two columns where the field values are non-overlapping ranges, e.g. `device_id` is `foreign_key` `[1, 100]` and `user_id` is `uniform_int` `[1000, 2000]`; check that no row pairs a `device_id` with a `user_id` from a different pool entry.)
- `test_linked_choice_different_pools_independent` — two pools `pool_A` and `pool_B` sample independently in the same row (different row indices in each).
- `test_linked_choice_joint_distribution_matches_meta_plan` — the four-shape pool from the spec's worked example (60/25/10/5), sampled into 10K rows, statistically matches the input ratios. Tolerance: ±2 percentage points per category. Categories: (a) single-owner — `device_id` appears once in the pool and `user_id` is non-null; (b) anonymous — `user_id` is null; (c) shared device — `device_id` appears in ≥2 pool entries with different `user_id`s; (d) multi-device user — `user_id` appears in ≥2 pool entries with different `device_id`s. Aggregate from the pool snapshot rather than the event rows (cleaner; row sampling is uniform).
- `test_linked_choice_deterministic_across_runs` — same config + same seed → byte-identical Parquet bytes.
- `test_linked_choice_with_entity_columns` — an entity-pool dataset that also has `linked_pools` works correctly: every event row has an entity row's columns *and* a freshly-drawn pool tuple, and the two are independent. Verify that pool entries do not stick to entities.

**Implementation.**

- In `generic_parquet.rs::write_generic_dataset` (or `write_partitioned` / `write_single`):
  1. Before the row loop, iterate `config.linked_pools` (if `Some`) and build each pool: `let pool = LinkedPool::new(per_pool_seed, pool_cfg, fk_counts);`. Collect into `Arc<HashMap<String, Arc<LinkedPool>>>` keyed by pool name.
  2. Pass the map through to `generate_row` (extend its signature, or — cleaner — wrap the existing parameter list into a `RowContext` struct).
  3. `generate_row` samples one tuple index per pool per row, *before* iterating columns. The index map (`HashMap<&str, usize>`) is row-scoped.
  4. The `LinkedChoice` arm in `apply_spec` becomes: look up the row's tuple index for the named pool, look up the field's position via the pool's `field_index`, return `pool.rows[tuple_idx][field_idx].clone()`.
- In `build_schema` (`generic_parquet.rs:51`):
  - For each `LinkedChoice { pool, field }` column, walk `config.linked_pools` to find the named pool, then walk its first shape's `fields:` to find the named field, then use **that** generator's `arrow_type()` and `is_nullable()`. This is the "schema overrides placeholder" pattern.
  - If the lookup fails (pool not found / field not found), surface a clear error from `write_generic_dataset` (the post-parse validate from Phase 2 should already have rejected this configuration, but defensive code here is acceptable and small).

**Schema-construction caveat.** Because `build_schema` is a pure function over `&DatasetConfig`, the pool lookup is purely structural — no row data needed. This keeps the schema construction phase synchronous with config parsing.

**Critical files.**

- `crates/smelt-datagen/src/generic.rs` — extend `apply_spec`, extend `generate_row` signature or wrap into a context struct.
- `crates/smelt-datagen/src/generic_parquet.rs` — extend `build_schema`, `write_partitioned`, `write_single`, `write_rows_to_file` to thread the pool map through.

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] One pool sample per (row, pool) — the same `linked_choice` column resolves to the same tuple as its sibling.
- [ ] Pool map is built once before row-write (entity-pool precedent).
- [ ] Partitioned path shares pools across days (Arc + clone).
- [ ] Schema construction overrides the `LinkedChoice` placeholder with the referenced field's type and nullability.
- [ ] No regression in existing entity-pool tests or `Optional<ForeignKey>` tests.
- [ ] The joint-distribution test (`test_linked_choice_joint_distribution_matches_meta_plan`) is statistically meaningful — pool size and tolerance chosen so the test is robust under repeated CI runs.

**Commit.** `feat(datagen): linked_choice row sampling + Parquet round-trip + schema resolution (web-analytics Phase 2)`

---

### Phase 5: CLI `--list-generators` entry

**Goal.** `smelt-datagen --list-generators` includes a `linked_choice` entry with its parameter signature so the new generator is discoverable from the CLI.

**Pre-conditions.** Phase 4 committed.

**TDD tests to write first** (mirror the Phase 1 `--list-generators` test pattern from `web-analytics-1`):

- `test_list_generators_includes_linked_choice` — invoking the binary with `--list-generators` produces output that contains `linked_choice` and lines describing `pool:` and `field:` parameters.

**Implementation.** Find `GENERATOR_HELP` in `main.rs` (line 59); append a `linked_choice` block that mirrors the spec's Surface table entry. Optionally also a `linked_pools` block describing the dataset-level YAML.

**Critical files.**

- `crates/smelt-datagen/src/main.rs`.

**Docs touched.** None — the user-docs section under "Composite / structured" landed in Phase 1.

**Review checklist** (material findings only):

- [ ] `linked_choice` appears in `--list-generators` output.
- [ ] The displayed parameter signature matches the spec's Surface table entry.
- [ ] If `linked_pools` is mentioned, the description is honest about it being a dataset-level section, not a generator.
- [ ] No other generator's `--list-generators` line was perturbed.

**Commit.** `feat(datagen): list-generators entry for linked_choice (web-analytics Phase 2)`

---

### Phase 6: Expert reviewer dispatch loop

**Goal.** Run each Phase 2 applicable expert reviewer from meta-plan §5 over the Phase 2 diff, address material findings, and re-dispatch each expert until it reports clean — or escalate via stop-the-line per the bounds below.

**Pre-conditions.** Phases 1–5 complete and committed. Working tree clean. `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `/smelt:validate datagen` (zero drift) all pass.

**Experts to dispatch (Phase 2 subset of meta-plan §5).**

| Expert | Model | Scope (file allowlist) | What to verify |
|---|---|---|---|
| **datagen-expert** (dispatch via `general-purpose` if no literal `datagen-expert` agent type) | sonnet | `crates/smelt-datagen/src/{config,generic,generic_parquet,main}.rs` + `docs/specs/datagen.md` | `LinkedPoolConfig` / `ShapeConfig` / `LinkedChoice` match the spec contract; pool construction algorithm (weighted draws, sticky, emit, truncate) is correct; row-time sampling is one-per-(row, pool); RNG stream isolation holds (changing `num_rows` doesn't perturb pool); joint-distribution test is statistically sound; no regression in pre-existing tests (`json_object`, entity-pool, `Optional<ForeignKey>`, partitioning, `string_pattern`). |
| **docs-reviewer** | sonnet | `docs-site/docs/guide/datagen.md` + `docs/specs/datagen.md` | Every Surface/Semantics rule in the spec has a user-doc counterpart; the YAML example in the user docs is runnable and matches the spec's example shape; no syntax shown in docs that is not specced; no plan-vocabulary callouts (`Phase N`, "added in Phase 2") in spec body sections. |

If no literal `datagen-expert` agent type exists, dispatch `general-purpose` with a prompt that frames it as a datagen reviewer (read spec + impl, flag spec/impl drift, distribution-shape bugs, missing test cases — material findings only).

**Loop discipline.**

1. **Round 1.** Dispatch both experts in parallel — single message, multiple Agent tool calls. Each prompt MUST include:
   - This plan's path and the spec sections that are the oracle.
   - The exact file scope from the table above.
   - The diff range to review (commits since the start of Phase 2 — typically `git log --oneline 16fb4f03..HEAD`).
   - Explicit instruction: report only **material** findings (correctness, spec drift, missing test cases). Skip nits.
   - Output format: a numbered list of findings with file:line refs, or "no material findings".
   - Reminder to spawn with `model: "sonnet"` (meta-plan §"Subagent model rule").

2. **Address findings.** For each expert that returns material findings:
   - If the fix is mechanical (≤~30 lines, single concern), edit directly.
   - If the fix is non-trivial, dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test`, and `/smelt:validate datagen` after each fix batch.
   - Commit per expert: `review(web-analytics-2): address {expert-name} feedback` (e.g. `review(web-analytics-2): address datagen-expert feedback`).
   - Push after each commit.

3. **Re-dispatch.** Re-dispatch only the expert(s) whose findings were addressed. Provide the round-1 prompt plus a diff of what changed since round N−1. "No material findings" → that expert is clean and exits.

4. **Repeat** until both experts are clean.

5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason on the line above) and stop the autonomy loop if any of the following fires:
   - Same expert flags a material finding on round 3 (per-expert bound).
   - Both experts flag the same systemic concern in the same round (per meta-plan §7).
   - An expert's findings would force a spec change. Run `/smelt:spec datagen` first; if non-trivial, pause for the user.
   - A fix surfaces a pre-existing failure unrelated to Phase 2.

**Critical files (allowed to touch in this phase).** Anything within an expert's scope per the table above, plus `docs/plans/20260517-web-analytics-2-datagen-linked-choice.md` (to record round counts and the final clean status) and `docs/plans/20260517-web-analytics-example.md` (to flip the overall-plan status row).

**Review checklist** (material findings only — applied to the expert-dispatch *process*, not to a code diff):

- [ ] Both experts dispatched at least once.
- [ ] Every material finding either fixed or escalated; none silently dropped.
- [ ] Round count per expert recorded in "Deferred during implementation" below.
- [ ] No expert ran more than 3 rounds; if any did, `<<PAUSE_FOR_HUMAN>>` was emitted.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `/smelt:validate datagen` (zero drift) all green at end of phase.

**Acceptance gate.** Append a one-line summary to "Deferred during implementation" of the form:

> Phase 6 expert review: datagen-expert clean (R{n}), docs-reviewer clean (R{n}). No stop-the-line fired.

After acceptance gate: flip the overall-plan status row for Phase 2 in `docs/plans/20260517-web-analytics-example.md` to `done` with today's date and the latest commit SHA. Commit and push that change. Then emit `<<PHASE_COMPLETE>>` as the autonomy loop's sentinel.

**Commit(s).** Per round, per expert with findings: `review(web-analytics-2): address {expert-name} feedback`. The status-table flip lands as: `chore(web-analytics-2): mark Phase 2 done in overall plan`.

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

---

## Verification

How to confirm the spec is satisfied at the end of Phase 6:

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets` passes with zero warnings.
- `cargo test` passes — `crates/smelt-datagen/` tests include the new `linked_choice` / `LinkedPool` cases from Phases 2, 3, 4, 5.
- `/smelt:validate datagen` reports zero drift.
- The four-shape example from `docs/specs/datagen.md` §Surface — generated against a small `num_rows: 10000` dataset — produces a Parquet file whose `(device_id, user_id)` distribution matches the meta-plan's 60/25/10/5 co-occurrence ratios (±2 pp tolerance) and traces every row's pair back to a single pool entry.
- Phase 6 acceptance gate met: both applicable expert reviewers (`datagen-expert`, `docs-reviewer`) reported "no material findings" on final dispatch. No stop-the-line condition fired.
- The overall-plan status row for Phase 2 in `docs/plans/20260517-web-analytics-example.md` is flipped to `done` with date and commit SHA.
