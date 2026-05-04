# Plan: smelt-state and CLI bug fixes

**Date**: 2026-05-05
**Tracking branch**: main (small fixes, commit directly)
**Spec**: `docs/specs/schema_evolution.md` (stale-cleanup spec added 2026-05-05), `docs/specs/cli.md` (no-op-output spec added 2026-05-05)

**Spec diff**: Phases 1–2 implement against specs that were added as part of the skill-audit work on 2026-05-05. Phase 3 is a packaging fix with no spec surface.

**Source**: Skill audit (`/home/andrew/.claude/plans/can-you-look-at-distributed-sutherland.md`) — bugs surfaced by the smelt-loop integration test harness and confirmed against specs.

## Context

Three implementation bugs were confirmed against current specs during a skill-item audit (2026-05-05):

1. **Phantom nullability in `smelt diff`** — after a clean build of an unchanged model, `smelt diff` emits spurious `ChangeNullability` entries because the nullable flag is not stored and re-read consistently. `schema_evolution.md` specifies exact conditions under which ChangeNullability should fire; the implementation fires outside those conditions.

2. **Stale schema cache not cleaned up** — when a `.sql` model file is deleted, its `.smelt/schemas/<name>.json` entry persists. Every subsequent `smelt diff` reports that model as `REMOVED` indefinitely. The spec (§"Stale schema cleanup") now says a successful build must remove orphan entries. `FileStore` has no `delete_schema()` method and no cleanup hook is called from the build lifecycle.

3. **No-op rebuild is completely silent** — when `--select` matches no models, or when nothing needs rebuilding, smelt emits nothing to stderr. Users can't distinguish a no-op from a hung process. The spec (cli.md §"No-op rebuild output") now requires a one-line stderr message. Current implementation logs via `info!()` only, invisible without `RUST_LOG`.

A fourth item — broken sdist on PyPI — is a packaging concern tracked in `docs/research/20260417-0.3-regression-triage.md` and is included here for completeness (Phase 4).

## Phase order

```
1. Phantom nullability fix  (smelt-state — self-contained, no API changes)
2. Stale schema cleanup     (FileStore API extension + build/run lifecycle hook)
3. No-op rebuild stderr     (smelt-cli output paths)
4. Broken sdist             (packaging — separate from Rust code)
5. Skill update             (remove stale workaround notes after Phases 2–3 ship)
```

## Progress tracking

| Phase | Topic | Status | Date | Commit |
|-------|-------|--------|------|--------|
| 1 | Phantom nullability fix | done (regression test added; bug not reproduced — likely fixed by B8/B9) | 2026-05-05 | 253ed4a |
| 2 | Stale schema cache cleanup | done | 2026-05-05 | 253ed4a |
| 3 | No-op rebuild stderr output | done | 2026-05-05 | 253ed4a |
| 4 | Broken sdist on PyPI | done | 2026-05-05 | (next commit) |
| 5 | Skill update | done | 2026-05-05 | (next commit) |

---

## Phase 1 — Phantom nullability fix

**Goal.** A round-trip build of an unchanged model (build → build → diff) must produce zero `ChangeNullability` entries.

**Spec anchor.** `schema_evolution.md` §"Change classification": `ChangeNullability` fires on `NOT NULL → NULL` (safe) and `NULL → NOT NULL` (blocked). It must not fire when nullability has not changed between the saved schema and the newly inferred schema.

### Root cause investigation

`crates/smelt-state/src/schema_tracking.rs` emits `ChangeNullability` when:
```rust
if deployed_col.nullable != col.nullable {
    changes.push(SchemaChange::ChangeNullability { ... });
}
```

The likely cause is that `save_schema()` stores `nullable: true` for all columns (or all non-aggregated columns), while type inference at diff-time produces `nullable: false` for NOT NULL columns — or vice versa. This means a column that has not changed will flip its nullable flag between save and read.

### Tests (write red first)

In `crates/smelt-state/tests/schema_roundtrip.rs` (new file or add to existing integration tests):

```rust
// Test: build an unchanged model twice; diff must be empty
// 1. Build model → save schema
// 2. Build same model again → save schema (same content)
// 3. Load both DeployedSchema values; diff_schemas() must return SchemaDiff::empty()
#[test]
fn roundtrip_no_phantom_nullability() {
    let schema = DeployedSchema {
        model: "stg_orders".to_string(),
        version: 1,
        deployed_at: "2026-05-05T00:00:00Z".to_string(),
        model_hash: "abc123".to_string(),
        columns: vec![
            DeployedColumn { name: "order_id".to_string(), data_type: "INTEGER".to_string(), nullable: false },
            DeployedColumn { name: "amount".to_string(), data_type: "DOUBLE".to_string(), nullable: false },
        ],
    };
    let diff = diff_schemas(&schema, &schema);
    assert!(diff.is_empty(), "round-trip diff must be empty, got: {:?}", diff.changes);
}
```

### Fix

1. Audit `save_schema()` in `crates/smelt-state/src/file_store.rs` — what nullable value does it write?
2. Audit where `diff_schemas()` gets its `col.nullable` from — is it from the inferred schema or a freshly constructed `DeployedColumn`?
3. Ensure the nullable value written by `save_schema()` and the nullable value read back by `diff_schemas()` use the same convention (true = nullable, false = NOT NULL, consistently).
4. Add the round-trip test above; iterate until it passes.

**Files:**
- `crates/smelt-state/src/schema_tracking.rs` — diff logic
- `crates/smelt-state/src/file_store.rs` — save/load serialization
- `crates/smelt-cli/src/migration.rs` — where `DeployedColumn` values are constructed from inference

**Commit message:** `fix(schema): eliminate phantom ChangeNullability on clean round-trip builds`

---

## Phase 2 — Stale schema cache cleanup

**Goal.** After a successful `smelt build` or `smelt run`, `.smelt/schemas/` contains entries only for models that exist in the current project. Orphan entries from deleted models are deleted.

**Spec anchor.** `schema_evolution.md` §"Stale schema cleanup":
> After a successful `smelt run` or `smelt build`, smelt scans `.smelt/schemas/` and deletes any `.json` entry whose model name is not in the set of models discovered in the current project.
> The cleanup runs only after a *successful* build — a failed build does not trigger cleanup.

### Tests (write red first)

In `crates/smelt-cli/tests/` or `crates/smelt-state/tests/`:

```rust
// Test: build a project with model A; then "delete" model A (remove from discovered set);
// rebuild → .smelt/schemas/A.json must no longer exist.
#[tokio::test]
async fn stale_schema_cleanup_on_rebuild() {
    // 1. Set up tmp dir with smelt.yml + model A
    // 2. smelt build → .smelt/schemas/A.json exists
    // 3. Remove model A from project
    // 4. smelt build → .smelt/schemas/A.json is gone
    // 5. smelt diff → model A not reported as REMOVED
}
```

### Fix

**Step A — Add `delete_schema()` to `FileStore`** (`crates/smelt-state/src/file_store.rs`):

```rust
pub fn delete_schema(&self, model_name: &str) -> Result<()> {
    let path = self.schemas_dir().join(format!("{model_name}.json"));
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}
```

**Step B — Add cleanup call after successful build** (`crates/smelt-cli/src/commands/run.rs` or `build.rs`):

After all models run successfully:
```rust
let deployed = file_store.list_deployed_model_names();
let current: HashSet<String> = discovered_models.iter().map(|m| m.name.clone()).collect();
for orphan in deployed.iter().filter(|n| !current.contains(*n)) {
    file_store.delete_schema(orphan)?;
}
```

This cleanup runs only when the build/run exits successfully (i.e., it is inside the success path, not a `finally`-equivalent).

**Files:**
- `crates/smelt-state/src/file_store.rs` — add `delete_schema()`
- `crates/smelt-cli/src/commands/run.rs` — add cleanup after success
- `crates/smelt-cli/src/commands/build.rs` — if build has its own success path, add there too

**Commit message:** `fix(state): delete stale .smelt/schemas/ entries after successful build`

---

## Phase 3 — No-op rebuild stderr output

**Goal.** When `smelt build` or `smelt run` produces no model output (either because `--select` matched nothing, or because no models needed re-running), emit a one-line diagnostic to stderr.

**Spec anchor.** `cli.md` §"No-op rebuild output":
- `--select` matched nothing → `smelt: no models matched the selector(s)` to stderr
- Nothing to rebuild (up-to-date) → `smelt: nothing to rebuild` to stderr

### Tests (write red first)

```rust
// Test: run smelt build with --select matching no model name; capture stderr
// → stderr contains "no models matched"
#[tokio::test]
async fn no_op_select_emits_to_stderr() {
    let output = Command::new("smelt")
        .args(["build", "--select", "nonexistent_model_xyz"])
        .output().await?;
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("no models matched"), "got stderr: {stderr}");
    assert!(output.status.success());
}
```

### Fix

In `crates/smelt-cli/src/commands/run.rs`:

1. After selector filtering, if `selected_models` is empty:
   ```rust
   eprintln!("smelt: no models matched the selector(s)");
   return Ok(());
   ```
2. After building, if `executed_count == 0` (all models were up-to-date):
   ```rust
   eprintln!("smelt: nothing to rebuild");
   ```

Replace or supplement the current `info!()` log call so the message reaches stderr unconditionally, not only when `RUST_LOG=info`.

**Files:**
- `crates/smelt-cli/src/commands/run.rs`
- `crates/smelt-cli/src/commands/build.rs` (if it has its own early-exit path for the seed-phase no-match case)

**Commit message:** `fix(cli): emit no-op rebuild diagnostics to stderr unconditionally`

---

## Phase 4 — Broken sdist on PyPI

**Goal.** `pip install smelt-sql` without `--only-binary=smelt-sql` must not fail with `failed to load manifest for dependency 'smelt-backend-spark'`.

**Context.** `crates/smelt-cli/Cargo.toml` depends on `smelt-backend-spark`. Maturin's sdist packaging only bundles the primary crate's source, not workspace siblings. When pip falls back to the sdist, `cargo metadata` cannot resolve the missing workspace member.

**Options (choose one):**

A. **Remove the sdist from PyPI** — configure maturin to publish wheels only (`--no-sdist` in the publish step). This is the lowest-risk fix; sdists for native extensions provide minimal value since users cannot build them without the Rust toolchain anyway.

B. **Bundle the full workspace** — include the entire cargo workspace in the sdist. Larger artifact; more complex maturin config.

**Recommended**: Option A. Update the release CI to pass `--no-sdist` to `maturin publish`. Delete existing sdist from PyPI.

**Files:**
- `.github/workflows/release.yml` (or equivalent CI file that calls `maturin publish`)

**Reference:** `docs/research/20260417-0.3-regression-triage.md`

**Commit message:** `fix(packaging): publish wheels-only to PyPI; remove broken sdist`

---

## Phase 5 — Skill update

**Goal.** Remove skill workaround notes that are now obsolete after Phases 2–3 ship.

After Phase 2 ships:
- Remove from SKILL.md: `rm .smelt/schemas/<deleted_model>.json manually` bullet in the Stuck-points checklist

After Phase 3 ships:
- Update SKILL.md: Remove the "silence = success — not that nothing ran" note from the build loop section (or replace with "smelt emits `nothing to rebuild` if nothing changed")

**File:** `.claude/skills/smelt-app-builder/SKILL.md`

**Commit message:** `docs(skill): remove stale workaround notes after state/cli fixes`

---

## Verification (end-to-end)

After all phases:

1. `cargo test -p smelt-state` — round-trip nullability test passes
2. `cargo test -p smelt-cli --test stale_cleanup` — orphan schema file is deleted after rebuild
3. Build a project, delete a model, rebuild → `smelt diff` shows no REMOVED entry
4. `smelt build --select nonexistent` → stderr contains "no models matched", exit 0
5. `smelt build` on an up-to-date project → stderr contains "nothing to rebuild"
6. `pip install smelt-sql` in a fresh venv without `--only-binary` → succeeds (or at least does not fail on cargo metadata)
