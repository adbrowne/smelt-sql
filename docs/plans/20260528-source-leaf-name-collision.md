# Plan: `smelt.sources.*` paths shadowed by colliding model leaf names

**Date**: 2026-05-28
**Spec**: [`docs/specs/sources.md`](../specs/sources.md)
**Spec diff**: §Constraints & Invariants gains a clause specifying that a `smelt.sources.<path>` reference resolves under the sources namespace regardless of model-name collisions on the leaf segment; §Known Divergences notes the now-closed gap.
**Tracking PR / branch**: branch `worktree-unknown_types` (continuation; same branch as PR #124's struct-field validation work).
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/sources.md` §Semantics #2 (the schema-as-contract clause) and §Constraints & Invariants — the correctness oracle.
2. Confirm you are on branch `worktree-unknown_types`. If not, ask before continuing.
3. Find the next `pending` phase in the Progress table. If all are `done`, run Verification and stop.

**Per-phase loop (`/smelt:implement`):** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**
- The reviewer surfaces the same material finding across two implementer passes.
- A pre-existing failure unrelated to this plan surfaces.
- The fix shape grows beyond `process_table_ref_pure` (e.g., touches the `lookup_column` precedence) — flag for confirmation before widening.

**Conventions every phase:**
- Red-green TDD; the diagnostic / typing test drives the *real* `model_function_type` (or `typed_model_schema`) query, not a sub-helper.
- Real-fixture coverage: `examples/meta_columns/` is the existing reproducer. `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` are the standing gates.
- Atomic per-phase commit using the `Commit.` line verbatim; push after each.
- Honor `CLAUDE.md` invariants: `smelt-db` pure-function rule.
- **Timeless-oracle rule.** Phase vocabulary lives in this plan only. Spec / `docs-site` edits describe the feature as if it has always existed.

---

## Context

`examples/meta_columns/` has both:
- `models/orders.sql` (a model whose name is `orders`)
- `models/sources/raw/orders.yml` (a per-entity source whose final path segment is also `orders`)

The source loader correctly parses the YAML — its declared column types (`id: INTEGER`, `customer_name: VARCHAR`, `amount: DOUBLE`, `discount: DOUBLE`) reach `discover_source_infos` and the `TypeContext.source_columns` map. Despite that, `target/debug/smelt type --project-dir examples/meta_columns` reports every column of the `orders` model as `UNKNOWN`.

Root cause is a precedence bug in `crates/smelt-db/src/queries/schema.rs:805-833`, inside `process_table_ref_pure`. When a `smelt.sources.raw.orders` reference is processed:

```rust
if let Some((entity_name, cols)) = refs
    .seed_columns(&seed_key)
    .or_else(|| {
        refs.resolved_columns(&model_name)             // model_name == "orders"
            .map(|c| (model_name.clone(), c))
    })
{
    for (col_name, typed_col) in &cols {
        ctx.add_model_column(&entity_name, col_name, typed_col.clone());
    }
    let bind_to = table_ref.alias().unwrap_or_else(|| entity_name.clone());
    ctx.add_alias(&bind_to, &entity_name);
} else if segments.first().map(|s| s.as_str()) == Some("sources") {
    // Dead code in the collision case — the .or_else above already matched.
    let bind_to = table_ref.alias().unwrap_or_else(|| model_name.clone());
    ctx.add_alias(&bind_to, &model_name);
}
```

`refs.resolved_columns("orders")` matches `models/orders.sql` by leaf name and returns *its own* columns — which at this point are themselves still `Unknown` (the only way they'd be concrete is if the source they read from typed correctly, which it can't, because the source loader writes into `source_columns` and `model_columns` for `orders` has been polluted with `Unknown` here). Those `Unknown` entries land in `model_columns["orders"]`. Then `lookup_column_inner` (`crates/smelt-db/src/type_inference/type_context.rs:408-440`) checks `model_columns` *before* `source_columns`, so the `Unknown` model entry wins over the correctly-typed source entry.

This violates `sources.md` §Semantics #2 ("Schema is the contract: when a model references a source column, the smelt type-checker uses the YAML's declared type"). The `else if segments.first() == Some("sources")` branch shows the original author intended sources to take a different path; the `.or_else` chain inadvertently made it unreachable.

## Scope

### In scope (spec coverage)
- `sources.md` §Constraints & Invariants gains: "A `smelt.sources.<path>` reference resolves under the sources namespace. A model whose leaf name collides with a source's leaf segment does not shadow the source schema for that reference; the path prefix is dispositive."
- `crates/smelt-db/src/queries/schema.rs:805-833`: re-order branches so `sources.*`-prefixed paths skip the model-leaf fallback.

### Explicitly out of scope
- The broader question of canonical addressing for cross-namespace references (covered by separate planning).
- The `compile.rs:677` adjacent observation about `apply_type_casts` and per-entity sources — non-blocking, separate.
- Changes to `lookup_column_inner` precedence between `model_columns` and `source_columns`. The fix is to stop polluting `model_columns`, not to change lookup order.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 1ed38a1e | 2026-05-28 |

---

### Phase 1: `sources.*` paths skip the model-leaf fallback

**Goal.** A `smelt.sources.<path>` reference resolves under the sources namespace regardless of whether the leaf segment matches a model name in the same project. Source-declared column types reach the typed schema.

**Pre-conditions.** None.

**TDD tests to write first.**

1. `crates/smelt-db/src/tests.rs` (or a new `crates/smelt-db/tests/source_leaf_collision.rs` if a `TestDb` helper for per-entity sources doesn't already exist — extend it if needed): ingest a project where `models/orders.sql` reads `FROM smelt.sources.raw.orders` and `models/sources/raw/orders.yml` declares `id: INTEGER`, `amount: DOUBLE`. Drive `model_function_type` (the real query) for `orders.sql`. Assert the output schema has `id: Integer`, `amount: Double` — NOT `Unknown`.
2. Regression case in the same test file: when there is *no* collision (e.g. model `use_source.sql` reads `smelt.sources.raw.orders`), the existing typed path still works. Mirrors `crates/smelt-cli/tests/source_guard_and_name_override.rs` semantics.
3. `crates/smelt-cli/tests/example_diagnostics.rs` (broken-workspace-style assertion or extension of the existing `meta_columns` coverage): assert that running through the typed-schema path against `examples/meta_columns/` yields concrete column types for `orders` and `orders_safe`. A complementary `cargo test -p smelt-lsp --test example_workspaces` run must remain green.
4. Regression: `example_diagnostics` (currently 75) and `example_workspaces` (21) stay green — no existing example newly flags or newly loses concrete types.

**Implementation shape.** In `crates/smelt-db/src/queries/schema.rs:805-833`, restructure `process_table_ref_pure`'s `smelt_path_ref` branch so that when the first segment is `sources` the model-leaf fallback is not consulted. Two equivalent shapes — pick whichever is cleaner:

- (a) Hoist the `else if segments.first() == Some("sources")` branch above the `seed/model` branch; route `sources.*` paths through the source-only path. Source columns are installed by `add_source_info_to_type_context` later; this branch just registers the alias.
- (b) Wrap the `.or_else(refs.resolved_columns(&model_name))` in a guard that excludes `sources`-prefixed paths.

Keep the analysis pure (no Salsa inside the helper); the Salsa query stays a thin wrapper. Do not change `lookup_column_inner` — the fix is to stop polluting `model_columns`, not to reorder lookup.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/queries/schema.rs` — the branch reorder.
- `crates/smelt-db/src/tests.rs` and/or `crates/smelt-db/tests/source_leaf_collision.rs` — unit tests; extend the test harness if needed to register a per-entity `SourceInfo` directly.
- `crates/smelt-cli/tests/example_diagnostics.rs` — typed-schema assertion for `meta_columns` (only if today's diagnostic-presence assertion isn't enough).

**Docs touched (timeless phrasing — no plan/phase vocabulary in body).**
- `docs/specs/sources.md` — add a Constraints & Invariants clause about leaf collisions; remove any §Known Divergences entry that names this gap (none expected, but check).
- `docs-site/docs/reference/sources.md` (or equivalent user-doc) — one sentence: "A `smelt.sources.<path>` reference always resolves under the sources namespace; a model whose name happens to collide with a leaf segment does not shadow the source."

**Review checklist (material findings only):**
- [ ] TDD tests exist, drive the real typed-schema query, and assert concrete types on the colliding-leaf case.
- [ ] `sources.md` §Semantics #2 satisfied; the source's declared types reach the typed schema.
- [ ] `smelt-db` pure-function rule preserved.
- [ ] `example_diagnostics` + `example_workspaces` stay green; `examples/meta_columns/` now reports concrete types via `smelt type`.
- [ ] No scope creep into `lookup_column_inner` precedence or canonical-addressing redesign.
- [ ] Spec + user-doc edits are timeless (no `Phase X`).

**Commit.** `fix(types): smelt.sources.* resolves under sources namespace even when leaf collides with a model name`

---

## Deferred during implementation

(Append-only.)

## Verification

- `target/debug/smelt type --project-dir examples/meta_columns` shows no `UNKNOWN` for `orders` or `orders_safe`.
- `cargo test -p smelt-cli --test example_diagnostics` — green.
- `cargo test -p smelt-lsp --test example_workspaces` — green.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` — green.
- `/smelt:validate sources` — no drift on the §Constraints & Invariants additions.
