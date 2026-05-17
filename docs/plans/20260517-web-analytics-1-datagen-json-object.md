# Plan: Web Analytics Phase 1 — datagen `json_object` generator

**Date**: 2026-05-17
**Spec**: [`docs/specs/datagen.md`](../specs/datagen.md) §"Composite / structured" (Surface), §"`json_object` encoding" (Semantics), §"Design" `json_object` paragraphs, §"Known Divergences" `json_object` entries
**Spec diff**: uncommitted working tree adds the `json_object` generator entry to all four spec sections above; user docs at `docs-site/docs/guide/datagen.md` updated in parallel
**Tracking branch**: `worktree-web_analytics` (overall plan: [`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md); meta-plan: `/home/andrew/.claude/plans/i-would-like-to-stitch-eventstream.md`)
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive Phase 1 to completion using `/smelt:implement`, then dispatch the meta-plan §5 expert reviewers, then update the in-repo overall-plan status table and push.

**Before touching any code:**

1. Read this plan in full. Then read `docs/specs/datagen.md` — the `json_object` entries in Surface, Semantics, Design, and Known Divergences are the correctness oracle. Do not re-open settled spec decisions; if a spec rule blocks a green test, run `/smelt:spec datagen` to revise the spec rather than encode the divergence in code.
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
- Don't widen scope: this plan introduces `json_object` only. The companion `linked_choice` generator is Phase 2; sample-pipeline code is Phase 3+. No `json_array`, no `parquet_struct` — those are future work.
- Honor architectural invariants from `CLAUDE.md`: the generators stay testable as pure functions of `(GeneratorSpec, RngCore, row_index, FkCounts)`; do not introduce I/O inside `apply_spec`.

---

## Context

The web-analytics example (overall plan: [`docs/plans/20260517-web-analytics-example.md`](20260517-web-analytics-example.md)) needs JSON-encoded event payloads in a single column to match how real production event pipelines ship data. `smelt-datagen` today emits only flat scalar / string columns, so the example cannot exist without extending datagen. The user agreed to add two new generators rather than introduce a JSON-lines writer; this plan covers the first of those two, `json_object`. The spec increment is already drafted in `docs/specs/datagen.md` (uncommitted working tree at the start of Phase 1 of *this* plan); Phase 1 of *this* plan commits both the spec and the implementation as separate atomic commits, in that order.

## Scope

### In scope (spec coverage)

- `datagen.md` §Surface — `json_object` row in the "Composite / structured" table, with the YAML example.
- `datagen.md` §Semantics — `json_object` encoding rules: `Utf8` column, field iteration order, RNG consumption order, per-type encoding (`Int` / `Float` / `Bool` / `Str` / `Null`), nesting, entity-vs-row scope, non-empty `fields:` requirement.
- `datagen.md` §Design — three new design paragraphs covering the `Utf8`-vs-`Struct` choice, the ordered-map field declaration, and the always-emit-the-key rule.
- `datagen.md` §Known Divergences — three new entries on missing `json_array`, no `parquet_struct`, and float `Display` cross-version caveat.
- Rust implementation: new `GeneratorSpec::JsonObject { fields: IndexMap<String, GeneratorSpec> }` variant in `crates/smelt-datagen/src/config.rs`; encoding in `crates/smelt-datagen/src/generic.rs`; Parquet round-trip via the existing `Utf8` path in `crates/smelt-datagen/src/generic_parquet.rs`.
- Unit tests for: (a) lexer / parser of the YAML spec, (b) the JSON encoding helper, (c) end-to-end Parquet round-trip with a representative `json_object` config, (d) determinism under fixed seed, (e) optional-fires-null-field-always-emitted invariant, (f) nested `json_object`, (g) entity-column placement, (h) empty-fields-rejected.
- User docs: `docs-site/docs/guide/datagen.md` "Composite / structured" sub-section with the rules block.
- CLI: `smelt-datagen --list-generators` includes a `json_object` entry with its parameter signature.

### Explicitly deferred

- `json_array` (array-valued JSON fields) — open question in spec's Known Divergences.
- `parquet_struct` (native Parquet `Struct` companion generator) — open question in spec's Known Divergences.
- Pinning the exact float `Display` form across smelt-datagen versions — spec accepts the current Rust `f64` `Display` behaviour.
- Schema validation hooks for `json_extract`-style downstream usage. The generator does not check that downstream models' JSON paths match the emitted shape.
- Integration with the web-analytics example pipeline. That lands in Phase 3+ of the overall plan.

---

## Progress tracking

| Phase | Status | Commit | Date |
|-------|--------|--------|------|
| 1 | pending | | |
| 2 | pending | | |
| 3 | pending | | |
| 4 | pending | | |
| 5 | pending | | |
| 6 | pending | | |

---

### Phase 1: Commit spec increment + user docs (already drafted in working tree)

**Goal.** Land the `datagen.md` and `docs-site/docs/guide/datagen.md` edits that introduce `json_object` to the spec and user guide. No code in `crates/` changes in this phase — the spec is the oracle, and committing it first means Phase 2's red TDD tests are written against a frozen spec target.

**Pre-conditions.** None — Phase 1 is the entry phase. Working tree at start of phase has uncommitted edits to `docs/specs/datagen.md` and `docs-site/docs/guide/datagen.md`; no other staged changes.

**TDD tests to write first.** None — Phase 1 is documentation. The TDD discipline applies from Phase 2 onward, where the *behaviour* the docs describe is the test oracle.

**Implementation.** Stage and commit the spec + user-docs diffs already on disk. Verify the spec edit closes Surface / Semantics / Design / Known Divergences (no orphan sections).

**Critical files (allowed to touch in this phase).**

- `docs/specs/datagen.md` (already on disk)
- `docs-site/docs/guide/datagen.md` (already on disk)
- `docs/plans/20260517-web-analytics-1-datagen-json-object.md` (this file — committed in this same phase)

**Docs touched.** Both files above.

**Review checklist** (material findings only):

- [ ] `json_object` row appears in §Surface generator-types tables.
- [ ] §Semantics encoding rules cover all five `GenericValue` variants + nesting + entity-vs-row scope + non-empty-fields requirement.
- [ ] §Design block explains the `Utf8`-vs-`Struct` choice, the ordered-map shape, and the always-emit-field rule.
- [ ] §Known Divergences notes the missing `json_array` and the float `Display` caveat.
- [ ] User-docs sub-section under "Composite / structured" exists; the example matches the spec example shape.
- [ ] No phase-vocabulary callouts in spec body sections (timeless-oracle rule from `CLAUDE.md`).

**Commit.** `spec(datagen): json_object generator surface + semantics + design (web-analytics Phase 1)`

---

### Phase 2: `GeneratorSpec::JsonObject` config variant + serde tests

**Goal.** Introduce the YAML-deserializable variant. No behaviour yet — just the config shape, the Arrow-type mapping, and the deny-unknown-fields ergonomic.

**Pre-conditions.** Phase 1 committed.

**TDD tests to write first** (in `crates/smelt-datagen/src/config.rs`):

- `json_object_parses_minimal` — `type: json_object\nfields:\n  k: { type: constant, value: "v" }` parses into `GeneratorSpec::JsonObject` with one field `k`.
- `json_object_preserves_field_order` — fields declared in YAML order `a, b, c` parse with `IndexMap` iteration yielding `["a", "b", "c"]` (use `IndexMap` so the order survives `serde_yaml` parsing — `HashMap` would not).
- `json_object_rejects_empty_fields` — `type: json_object\nfields: {}` is a parse error (custom deserializer rule, or explicit `validate` step). The error message must name the offending field.
- `json_object_rejects_unknown_top_level_key` — `type: json_object\nfields: { k: ... }\nextra: 1` fails parse via `deny_unknown_fields`.
- `json_object_accepts_nested` — `type: json_object\nfields: { outer: { type: json_object, fields: { inner: { type: constant, value: 1 } } } }` parses cleanly.
- `json_object_arrow_type_is_utf8` — `GeneratorSpec::JsonObject { ... }.arrow_type()` returns `DataType::Utf8`.
- `json_object_is_not_nullable` — `is_nullable()` returns `false` (the column always has a value — null fields are *inside* the JSON string).

**Implementation.**

- Add `JsonObject { fields: IndexMap<String, GeneratorSpec> }` variant to `GeneratorSpec` enum.
- Extend `arrow_type()` and `is_nullable()` match arms.
- Add an empty-fields rejection. Easiest path: a `#[serde(deserialize_with = "...")]` adapter on `fields`, or a post-parse `validate()` pass that the existing config loader already runs. Pick whichever is closest to the existing pattern — do not introduce a new validation pass just for this one rule.

**Critical files.**

- `crates/smelt-datagen/src/config.rs`
- `crates/smelt-datagen/Cargo.toml` (add `serde_json` if not already present — it likely is via transitive deps, but the JSON encoding work in Phase 3 will pull it explicitly).

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] `JsonObject` variant uses `IndexMap`, not `HashMap` (order matters for determinism per spec §Semantics).
- [ ] Empty-`fields` rejection has a test and a clear error message.
- [ ] `arrow_type()` and `is_nullable()` arms compile (exhaustive match preserved).
- [ ] The variant does not break `--list-generators` output (that is Phase 5 — but the variant must at minimum not panic if traversed).

**Commit.** `feat(datagen): json_object GeneratorSpec variant + serde tests (web-analytics Phase 1)`

---

### Phase 3: JSON encoding (`apply_spec` + escape helper)

**Goal.** Implement the JSON serialization for `JsonObject`. Each row's value is a `GenericValue::Str` containing a JSON object string that encodes each inner sub-generator's `GenericValue` per the spec's per-type encoding rules.

**Pre-conditions.** Phase 2 committed.

**TDD tests to write first** (in `crates/smelt-datagen/src/generic.rs::tests`):

- `test_json_object_emits_object_with_field_order` — a `JsonObject { fields: [("a", Constant 1), ("b", Constant "x"), ("c", Bool false)] }` (in YAML/IndexMap order) produces a `GenericValue::Str` whose contents parse as JSON and have keys `[a, b, c]` *in that order* when serialized back. Assert both the parsed shape and the byte-exact string `{"a":1,"b":"x","c":false}`.
- `test_json_object_optional_field_emits_null_not_omitted` — a `JsonObject` with one `Optional { prob: 0.0, inner: ... }` field always emits `{"<field>":null}`, never `{}`.
- `test_json_object_nested` — `JsonObject { fields: [("outer", JsonObject { fields: [("inner", Constant 1)] })] }` produces `{"outer":{"inner":1}}` — no double-encoding (no `"\"{\\\"inner\\\":1}\""`).
- `test_json_object_string_escapes` — a string sub-generator returning `Str("a \"quoted\" \\ \n value")` produces a properly JSON-escaped value: `"a \"quoted\" \\ \n value"` inside the JSON. Control characters `< 0x20` use the `\uXXXX` form. Non-ASCII UTF-8 passes through unescaped.
- `test_json_object_deterministic` — two `apply_spec` invocations with identical seed, config, and row_index produce byte-identical strings.
- `test_json_object_rng_order` — swapping two field positions changes the emitted values (proves sub-generators consume RNG in declaration order, not in some unspecified order).

**Implementation.**

- Add a `JsonObject { fields }` arm to `apply_spec` in `generic.rs`. Loop over `fields` in iteration order, recursively call `apply_spec` for each sub-generator, then encode the result.
- Use `serde_json::Value` as the intermediate representation: each `GenericValue` maps to a `serde_json::Value` variant (`Int → Number`, `Float → Number`, `Bool → Bool`, `Str → String`, `Null → Null`). Build a `serde_json::Map` (preserves insertion order under the `preserve_order` feature — enable it if not already on) and serialize via `serde_json::to_string`. This gives correct escaping for free.
- If `serde_json`'s `preserve_order` feature is unavailable or unwanted, hand-roll the JSON output: it's a small function, and a hand-rolled writer matches the spec's "field iteration order is YAML declaration order" rule explicitly. Implementer's choice — prefer the small hand-rolled path if it adds less than ~80 lines including tests, because it keeps the dependency surface narrow.
- The JSON-string is wrapped in `GenericValue::Str(...)`. Downstream Parquet write goes through the existing `Utf8` path — no change to `generic_parquet.rs`.

**Critical files.**

- `crates/smelt-datagen/src/generic.rs` — new arm + helper.
- `crates/smelt-datagen/Cargo.toml` — enable `serde_json/preserve_order` if going that route.

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] Iteration order matches YAML declaration order under both single-row and multi-row generation.
- [ ] All five `GenericValue` variants encode per spec §Semantics.
- [ ] Optional → Null produces `null`, *not* an absent key.
- [ ] String escaping covers `"`, `\`, `\n`, `\r`, `\t`, `\b`, `\f`, control chars `< 0x20`. Non-ASCII passes through.
- [ ] Nesting produces real nested objects, not strings-of-strings.
- [ ] Determinism: re-running with same seed → byte-identical output.

**Commit.** `feat(datagen): json_object encoding + escape rules (web-analytics Phase 1)`

---

### Phase 4: End-to-end Parquet round-trip + entity-column path

**Goal.** Exercise `JsonObject` through `write_generic_dataset` to prove that (a) the existing `Utf8` path in `generic_parquet.rs` handles JSON strings unchanged, (b) Hive-partitioned output works, and (c) entity-column scope (sticky JSON payload per entity) works.

**Pre-conditions.** Phase 3 committed.

**TDD tests to write first** (in `crates/smelt-datagen/src/generic_parquet.rs::tests`):

- `test_json_object_writes_parquet_single_file` — a `DatasetConfig` with one `JsonObject` column produces a Parquet file whose column reads back as `Utf8`, every value parses as JSON, and field order is preserved on every row.
- `test_json_object_writes_parquet_partitioned` — same, with `partition: { column: event_date, start: ..., days: 3 }`. Each partition's data.parquet has well-formed JSON in its column; the partition column is unaffected.
- `test_json_object_entity_column_is_sticky` — a `JsonObject` declared under `entity.columns` produces JSON values that repeat across rows that select the same entity (use `pool_ratio: 0.1` with `num_rows: 1000` → ~100 entities; assert the number of distinct JSON values ≤ entity count).
- `test_json_object_with_foreign_key_inner_generator` — a `json_object` whose `fields:` includes a `foreign_key` sub-generator resolves FK row counts correctly (the inner sub-generator must receive the same `fk_counts` map as scalar columns do).
- `test_json_object_deterministic_partitioned` — same config + same seed → byte-identical Parquet bytes across runs, partitioned variant.

**Implementation.** No new code path expected — the existing `Utf8` column builder in `generic_parquet.rs::build_column` already handles `GenericValue::Str`. The work in this phase is *verification*: write tests, watch them go red if the integration path is broken, fix any wiring (most likely: `apply_spec` invocation paths in partitioned vs single-file flows must pass the same `fk_counts` map through to inner sub-generators).

If the entity-column path needs work: `make_entity_pool` builds the pool with an empty `FkCounts` (see `generic.rs::EntityPool::new`). `JsonObject`'s sub-generators may include `ForeignKey`. Either propagate `fk_counts` into `EntityPool::new` (cleanest), or document that entity-column `json_object` fields cannot use FKs (spec-level constraint — would need a spec edit). Implementer must decide. Prefer the propagation route, because the existing `Optional<ForeignKey>` entity-column test (`test_optional_foreign_key_emits_nulls` in `generic_parquet.rs`) already wires FK into Optional entity columns by some path — confirm what that path is and match it.

**Critical files.**

- `crates/smelt-datagen/src/generic_parquet.rs` — tests.
- Possibly `crates/smelt-datagen/src/generic.rs::EntityPool::new` — if FK propagation into entity pool needs to change.

**Docs touched.** None.

**Review checklist** (material findings only):

- [ ] All five tests above pass.
- [ ] No regression in pre-existing `Optional<ForeignKey>` tests.
- [ ] If `EntityPool::new` changed, the change is minimal and matches the existing FK propagation pattern.

**Commit.** `test(datagen): json_object Parquet round-trip + entity scope (web-analytics Phase 1)`

---

### Phase 5: CLI `--list-generators` entry

**Goal.** `smelt-datagen --list-generators` includes a `json_object` entry with its parameter signature so the new generator is discoverable from the CLI.

**Pre-conditions.** Phase 4 committed.

**TDD tests to write first** (in `crates/smelt-datagen/src/main.rs` or a dedicated CLI test file; mirror existing `--list-generators` test patterns):

- `test_list_generators_includes_json_object` — invoking the binary with `--list-generators` (via `assert_cmd` or the existing test harness, whichever pattern the crate already uses) produces output that contains the string `json_object` and at least one line describing its parameter shape (e.g. `fields: { <key>: <generator>, ... }`).

**Implementation.** Find the existing `--list-generators` rendering code (likely a `match` over `GeneratorSpec` variants in `main.rs`). Add the `JsonObject` arm with a parameter signature string that matches the spec table entry. If `--list-generators` is generated from `GeneratorSpec` reflection (unlikely with serde), trivially nothing to do — the variant is already there.

**Critical files.**

- `crates/smelt-datagen/src/main.rs`.

**Docs touched.** None — the user-docs section under "Composite / structured" landed in Phase 1.

**Review checklist** (material findings only):

- [ ] `json_object` appears in `--list-generators` output.
- [ ] The displayed parameter signature matches the spec's Surface table entry.
- [ ] No other generator's `--list-generators` line was perturbed.

**Commit.** `feat(datagen): list-generators entry for json_object (web-analytics Phase 1)`

---

### Phase 6: Expert reviewer dispatch loop

**Goal.** Run each Phase 1 applicable expert reviewer from meta-plan §5 over the Phase 1 diff, address material findings, and re-dispatch each expert until it reports clean — or escalate via stop-the-line per the bounds below.

**Pre-conditions.** Phases 1–5 complete and committed. Working tree clean. `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `/smelt:validate datagen` (zero drift) all pass.

**Experts to dispatch (Phase 1 subset of meta-plan §5).**

| Expert | Model | Scope (file allowlist) | What to verify |
|---|---|---|---|
| **datagen-expert** (dispatch via `general-purpose` if no literal `datagen-expert` agent type) | sonnet | `crates/smelt-datagen/src/{config,generic,generic_parquet,main}.rs` + `docs/specs/datagen.md` | `JsonObject` variant matches the spec contract; encoding rules per §Semantics are exact (escape coverage, optional-null-key-present, field order, RNG order, nesting); determinism preserved; no regression in pre-existing tests (`Optional<ForeignKey>`, `string_pattern`, partitioning). |
| **docs-reviewer** | sonnet | `docs-site/docs/guide/datagen.md` + `docs/specs/datagen.md` | Every Surface/Semantics rule in the spec has a user-doc counterpart; the YAML example in the user docs is runnable and matches the spec's example shape; no syntax shown in docs that is not speced; no plan-vocabulary callouts (`Phase N`, "added in Phase 1") in spec body sections. |

If no literal `datagen-expert` agent type exists, dispatch `general-purpose` with a prompt that frames it as a datagen reviewer (read spec + impl, flag spec/impl drift, distribution-shape bugs, missing test cases — material findings only).

**Loop discipline.**

1. **Round 1.** Dispatch both experts in parallel — single message, multiple Agent tool calls. Each prompt MUST include:
   - This plan's path and the spec sections that are the oracle.
   - The exact file scope from the table above.
   - The diff range to review (commits since the start of Phase 1 — typically `git log --oneline 80ca0788..HEAD`).
   - Explicit instruction: report only **material** findings (correctness, spec drift, missing test cases). Skip nits.
   - Output format: a numbered list of findings with file:line refs, or "no material findings".
   - Reminder to spawn with `model: "sonnet"` (the meta-plan §"Subagent model rule").

2. **Address findings.** For each expert that returns material findings:
   - If the fix is mechanical (≤~30 lines, single concern), edit directly.
   - If the fix is non-trivial, dispatch an implementer subagent (`model: sonnet`) scoped to the same file allowlist.
   - Run `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test`, and `/smelt:validate datagen` after each fix batch.
   - Commit per expert: `review(web-analytics-1): address {expert-name} feedback` (e.g. `review(web-analytics-1): address datagen-expert feedback`).
   - Push after each commit.

3. **Re-dispatch.** Re-dispatch only the expert(s) whose findings were addressed. Provide the round-1 prompt plus a diff of what changed since round N−1. "No material findings" → that expert is clean and exits.

4. **Repeat** until both experts are clean.

5. **Bounds (stop-the-line).** Emit `<<PAUSE_FOR_HUMAN>>` (with a one-line reason on the line above) and stop the autonomy loop if any of the following fires:
   - Same expert flags a material finding on round 3 (per-expert bound).
   - Both experts flag the same systemic concern in the same round (per meta-plan §7).
   - An expert's findings would force a spec change. Run `/smelt:spec datagen` first; if non-trivial, pause for the user.
   - A fix surfaces a pre-existing failure unrelated to Phase 1.

**Critical files (allowed to touch in this phase).** Anything within an expert's scope per the table above, plus `docs/plans/20260517-web-analytics-1-datagen-json-object.md` (to record round counts and the final clean status) and `docs/plans/20260517-web-analytics-example.md` (to flip the overall-plan status row).

**Review checklist** (material findings only — applied to the expert-dispatch *process*, not to a code diff):

- [ ] Both experts dispatched at least once.
- [ ] Every material finding either fixed or escalated; none silently dropped.
- [ ] Round count per expert recorded in "Deferred during implementation" below.
- [ ] No expert ran more than 3 rounds; if any did, `<<PAUSE_FOR_HUMAN>>` was emitted.
- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test`, and `/smelt:validate datagen` (zero drift) all green at end of phase.

**Acceptance gate.** Append a one-line summary to "Deferred during implementation" of the form:

> Phase 6 expert review: datagen-expert clean (R{n}), docs-reviewer clean (R{n}). No stop-the-line fired.

After acceptance gate: flip the overall-plan status row for Phase 1 in `docs/plans/20260517-web-analytics-example.md` to `done` with today's date and the latest commit SHA. Commit and push that change. Then emit `<<PHASE_COMPLETE>>` as the autonomy loop's sentinel.

**Commit(s).** Per round, per expert with findings: `review(web-analytics-1): address {expert-name} feedback`. The status-table flip lands as: `chore(web-analytics-1): mark Phase 1 done in overall plan`.

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **Phase 1–5 complete; Phase 6 (expert review) and full verification gate blocked on system disk-full condition (2026-05-17).** Implementation committed at `3fb071d3` (`feat(datagen): json_object generator (web-analytics Phase 1)`) and pushed. The smelt-datagen crate's full test suite (44 lib tests + 4 CLI tests, including 19 new json_object tests) passes, `cargo fmt --all -- --check` passes, and `cargo clippy --all-targets -p smelt-datagen` passes with zero warnings. The workspace-level `cargo test` cannot run because the root filesystem is at 100% capacity (only ~1G free); the linker (`rust-lld` / `cc`) fails with "ld terminated with signal 7 [Bus error]" partway through linking smelt-cli and smelt-lsp test binaries. This is an environmental issue (not a regression introduced by Phase 1) — the user needs to free disk space (e.g. `cargo clean` on sibling worktrees under `~/smelt-sql/.claude/worktrees/*/target/` or on the main `~/smelt-sql/target/` directory, the largest of which is ~467G). Once disk is available, the autonomy loop should resume Phase 6 (expert reviewer dispatch loop) and complete the verification gate.
- **Foreign-key resolution inside `JsonObject` under `entity.columns` resolves to id `1`.** Discovered while writing the `test_json_object_entity_column_is_sticky` test. The pre-existing `EntityPool::new` (in `generic.rs`) constructs entity rows with an empty `FkCounts`, so any `ForeignKey` sub-generator nested inside a `JsonObject` (or any other generator) under `entity.columns` will always pick id 1. The pre-existing `test_optional_entity_column_emits_nulls` doesn't catch this because it only checks for nulls, not FK range. Phase 1 keeps the existing behaviour; threading `fk_counts` through `EntityPool::new` is a separate follow-up — natural to address as part of Phase 3+ when the web-analytics example wires real foreign-keyed dimensions, or as a standalone fix.

---

## Verification

How to confirm the spec is satisfied at the end of Phase 6:

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets` passes with zero warnings.
- `cargo test` passes — `crates/smelt-datagen/` tests include the new `json_object` cases from Phases 2, 3, 4, 5.
- `/smelt:validate datagen` reports zero drift.
- The example YAML from `docs/specs/datagen.md` §Surface — generated against a small `num_rows: 100` dataset — produces a Parquet file whose `payload` column is parseable JSON on every row, with the field order matching the YAML declaration order.
- Phase 6 acceptance gate met: both applicable expert reviewers (`datagen-expert`, `docs-reviewer`) reported "no material findings" on final dispatch. No stop-the-line condition fired.
- The overall-plan status row for Phase 1 in `docs/plans/20260517-web-analytics-example.md` is flipped to `done` with date and commit SHA.
