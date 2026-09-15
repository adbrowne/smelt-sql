# Phase 6 summary — two of three degraded routes proved live; the third blocked on a new gap

## Shipped

- `crates/smelt-cli/tests/trino_incremental_families/degraded_routes.rs` (new submodule, registered
  in `main.rs`), five live-gated tests, all green against the live tier:
  - `per_group_recompute_cell_is_explain_visible_on_trino` — a repair-family cell (`MAX` fold over a
    clocked `mutable_snapshot` source, `crates/smelt-runtime/tests/repair_lowering.rs`'s shape)
    reports `Technique::PerGroupRecompute` with no `state_downgrade` (its `key_scope` is `None`, so
    `required_state_structure` correctly asks for nothing) and its statements carry the affected-key
    scan clamp (`order_date >= … AND order_date < …`), not an unbounded scan.
  - `succession_cell_records_state_downgraded_on_trino` — the succession fixture
    (`smelt_maintenance_testkit::recipe::SuccessionRecipe::new_lead()`, staged live via its own pure
    renderers) carries `state_downgrade { original: SuccessionPatch, missing: "tombstone ledger" }`,
    and `smelt run --target trino` with **no** `--event-time-start/--event-time-end` succeeds — the
    downgraded full-rebuild route never demands a run window.
  - `succession_downgraded_rebuild_matches_ledger_bearing_presented_arm` — the Trino downgraded
    run's presented table is row- and column-identical to a DuckDB run of the SAME fixture through
    the real ledger-bearing `SuccessionPatch` route (compared via `common::fetch_rows` /
    `fetch_trino_rows`, both normalized through `batches_to_sorted_rows`).
  - `sidecar_less_key_addressed_cell_downgrades_on_trino` — `dag_kchain_b`'s `UpstreamMutation`
    cell (reading `dag_kchain_a` as a model edge) carries `state_downgrade { original:
    PerGroupRecompute, missing: "fingerprint sidecar" }` and `technique: DeleteInsert`.
  - `key_addressed_downgrade_matches_full_refresh_on_trino` — that model runs live across two
    windows (an upstream row revision + a new key) and is row-identical to a `--full-refresh`
    oracle. No dispatch-side guard fix was needed — the existing plan-derived `technique:
    DeleteInsert` was already what the runtime dispatched; `tests/state_guard_census.rs` stays green
    unchanged.
- `per_group_recompute_matches_full_refresh_on_trino` (the sixth planned test) is kept in the file as
  an `#[allow(dead_code)]` function rather than a permanently-red `#[test]`, following phase 3's own
  precedent for a discovered-but-unowned gap — see Blocked below.

## Decisions

- Kept the discovered-gap function as dead code with a full doc-comment writeup instead of deleting
  it, so the next phase that fixes the gap has the fixture (`stage_repair_project`,
  `seed_trino_repair_orders`) ready to re-enable rather than rebuilding it.
- The succession DuckDB comparison arm needed an explicit `--event-time-start/--event-time-end`
  window (a test-authoring fix, not a design question): DuckDB's ledger-bearing `SuccessionPatch` is
  a window-forward patch and requires one, unlike Trino's downgraded full-rebuild route.

## For the next planner

**A newly-discovered gap, not anticipated by this phase's plan** (full writeup in outcome.md's
Blocked log, 2026-09-15 "phase 6"): every repair-admitted `PerGroupRecompute` cell is **always**
over a `MutationProfile::MutableSnapshot` source (`derive/new_data.rs`'s own admission gate — repair
narrowing only ever fires for that posture), and `repair::discovery_posture` routes that posture's
affected-key discovery to `RepairDiscovery::SidecarDiff` **unconditionally** — never the plain
clamped scan. So every repair-family cell needs `StateStructure::FingerprintSidecar`, independent of
`key_scope`. `required_state_structure`'s `PerGroupRecompute` arm only asks for the sidecar when
`key_scope` is `Some(...)` (the key-addressed route), so a clamp-bounded repair cell with `key_scope:
None` is never downgraded, and execution hard-refuses live (`Feature not supported by Trino:
group-grain fingerprint-sidecar affected-key discovery for a mutable_snapshot repair source (P9)`).

This is very likely reachable on **BigQuery today, in production** — `realisable_state_structures`
does not list `FingerprintSidecar` for BigQuery either (state_structure.rs's own doc names the sidecar
as BigQuery's own pending work). If so this is a pre-existing correctness/crash gap outside Trino's
scope entirely, and worth escalating regardless of how this outcome proceeds.

The fix shape (not attempted here — it touches `smelt-logical`'s single-owner availability module,
which several other gates depend on, so it deserves its own reviewed phase rather than a rushed
in-place patch): `required_state_structure`'s `PerGroupRecompute` arm must require the sidecar
whenever the cell is repair-admitted (non-empty `scans`, `key_scope: None`), not only when
`key_scope: Some(...)`; and `resolve_availability`'s `PerGroupRecompute` replacement, when the
required structure is unavailable, must additionally clear the cell's `scans` (not just record the
downgrade) — `recompute_equivalent` maps `Corner::ColumnMerge` back to `PerGroupRecompute` itself, so
a bare downgrade record with `scans` left populated still has `has_repair_family_lowering` return
`true` and would still dispatch the same sidecar-needing resolver. The needed shape is exactly what
`has_repair_family_lowering`'s doc comment already describes as the "declined" cell: `state_downgrade:
Some`, `key_scope: None`, `scans: []`, routing to the whole-target rebuild the run shape's own
`key_scope: None ⇒ full-scan recompute` promise already covers.

## Gates

- `cargo test -p smelt-runtime --test state_guard_census --test availability_seam --test execute_parity` — pass (10+4+4).
- `cargo test -p smelt-logical --test maintenance_availability --test walk_coverage` — pass (35+14).
- `cargo test -p smelt-cli --test trino_explain_downgrade --test trino_ci_wiring --test trino_incremental_spec_freshness` — pass (8+5+6).
- Live tier: `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1` — 17/17 pass (all pre-existing families unaffected, all five new tests green).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN.
