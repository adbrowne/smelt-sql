# Plan: Bound DuckDB memory by default (stop single-model host OOM)

**Date:** 2026-06-21
**Spec:** `docs/specs/smelt_yml.md` — Surface (`settings` row), Semantics §8, Constraints §6, Design ("DuckDB defaults bound the host, not just the query").
**Spec diff:** Adds smelt-supplied defaults for DuckDB `memory_limit` and `temp_directory` when a target's `settings:` omits them; user-set keys always win; `threads` untouched.
**Docs:** code + spec + user docs.

## Motivation

A single `smelt build` of a real project (`sherlock`, `raw.events` ≈ 1B rows) reached
~50 GB RSS and climbing on a 60 GB host, tipping the box into memory pressure so
`systemd-oomd` reaped an unrelated tmux scope (the autonomy loop). Root cause: smelt
sets no DuckDB `memory_limit`, so DuckDB uses its native default of ~80% of *total host
RAM* and only spills once it reaches that ceiling. Confirmed: adding
`memory_limit: 8GB` + `temp_directory` held the same model at ~10 GB RSS (spilling),
a ~5× reduction. (Handoff: `docs/handoffs/2026-06-21-autonomy-loop-ooms.md`;
finding memory `project_smelt_build_oom_rootcause`.)

This plan is **Part 2** (the framework root-cause fix). **Part 1** (infra hardening:
run heavy commands under memory-capped systemd scopes, oomd-avoid the loop) is tracked
separately and lands after this.

## Phase 1 — Pure policy function + RAM shim, applied in `new_with_settings`

**Files:** `crates/smelt-backend-duckdb/src/lib.rs`.

**Change:**
- Add pure `resolve_duckdb_settings(user: Option<&BTreeMap<String,String>>, total_ram_bytes: Option<u64>, database_path: &Path) -> BTreeMap<String,String>`:
  - Clone user settings.
  - If `memory_limit` absent **and** `total_ram_bytes` is `Some(r)`: insert
    `default_memory_limit(r)` = `max(min(0.5·r, r − 20 GiB), 0.4·r)`, formatted as `"<N>MiB"`.
    Deliberately conservative — DuckDB's limit bounds the buffer pool, not RSS,
    which measured ~5 GB higher on a 1B-row aggregation.
  - If `temp_directory` absent: insert `<database_path.parent()>/.smelt-duckdb-tmp`.
  - Never touch a key the user already set; never touch `threads`.
- Add `detect_total_ram_bytes() -> Option<u64>`: Linux `/proc/meminfo` `MemTotal`,
  macOS `sysctl hw.memsize`, else `None`. Pure-ish, never panics.
- In `new_with_settings`, replace the direct use of `settings` with
  `resolve_duckdb_settings(settings, detect_total_ram_bytes(), &database_path)`;
  apply the resulting map via the existing SET loop. Create the `temp_directory`
  dir (`create_dir_all`) before SET so DuckDB can use it.

**Tests (red-green, `crates/smelt-backend-duckdb`):**
- `default_memory_limit_*`: 60 GiB → 30 GiB (50% cap); 36 GiB → 16 GiB (RAM−20); 24 GiB → 9.6 GiB (40% floor); 128 GiB → 64 GiB.
- `resolve_injects_defaults_when_absent`: no user settings → map has `memory_limit` (from a fixed `total_ram`) and `temp_directory` ending `/.smelt-duckdb-tmp`.
- `resolve_respects_user_memory_limit`: user `memory_limit: "4GB"` preserved; not overridden.
- `resolve_respects_user_temp_directory`: user value preserved.
- `resolve_no_ram_skips_memory_limit`: `total_ram_bytes = None` → no `memory_limit` key, but `temp_directory` still set.
- `resolve_never_sets_threads`: `threads` absent unless user set it.
- Integration (live DuckDB, existing `settings.rs` style): opening a backend with
  empty settings yields a non-default `memory_limit` readable via
  `current_setting('memory_limit')` below host 80%, and a `temp_directory` under the db dir.

**Commit:** `fix(smelt-backend-duckdb): bound DuckDB memory_limit + temp_directory by default`

## Phase 2 — User docs

**Files:** `docs-site/docs/reference/smelt-yml.md` (settings key), and the relevant
concepts/performance page if one exists.

**Change:** Document the defaulting behavior and the override-wins rule; show how to
raise/lower `memory_limit` and relocate `temp_directory`.

**Commit:** `docs(smelt-yml): document default DuckDB memory_limit/temp_directory`

## Verification gates

- `cargo test -p smelt-backend-duckdb 2>&1 | tail -40`
- `cargo test -p smelt-core --test hardening_budget 2>&1 | tail -20` (no new unwrap/expect/println regressions)
- `cargo fmt --all` ; `cargo clippy --all-targets 2>&1 | tail -30`
- Manual repro: capped `smelt build --select mart_segment_daily` in `sherlock` with
  **no** `settings:` now holds well below host-80% (was ~50 GB).

## Progress

| Phase | Status |
|-------|--------|
| 1 — policy fn + RAM shim + wire-in | pending |
| 2 — user docs | pending |
