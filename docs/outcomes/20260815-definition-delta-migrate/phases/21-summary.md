# Phase 21 summary — Keyed dirt cascades and is consumed

**Shipped:**
- `smelt_logical::maintenance::propagate::propagate` (`crates/smelt-logical/src/maintenance/propagate.rs`): the topological node walk now visits a node's outbound edges when it carries EITHER interval `dirty` OR keyed `keyed_dirty` — fixing the one-hop dead end where a node dirtied only through the keyed channel (both endpoints of its own inbound edge keyed-grain) never had its own outbound edges classified. Added `push_keyed_dirt` dedup helper and a `keyed_dirty` empty-entry prune mirroring the existing `dirty` prune.
- `smelt_runtime::propagation::plan_since_upstream_with_observed_deltas` (`crates/smelt-runtime/src/propagation.rs`): consumes `prop.keyed_dirty` — a keyed-only-dirty node in the caller's `order` (not an origin, not already covered by interval dirt) gets a whole-table `PropagatedRun` (`start`/`end` both `None`), deduplicated against an interval-dirt run for the same model. The dirty-set report gains a `<-(keyed) ` per-edge line and a `RUN <model>: keyed (keys: …)` line, distinct from the interval forms.
- Tests: 3 new pure-math tests in `crates/smelt-logical/tests/maintenance_propagation_adjoint.rs` (cascade past one hop, widen-to-whole for a clocked reader, diamond dedup); 2 new real-workspace tests in `crates/smelt-runtime/tests/since_upstream_propagation.rs`; 1 new CLI e2e test in `crates/smelt-cli/tests/since_upstream.rs`.
- Spec: `docs/specs/incremental_models.md` §"The graph layer" → "Keyed dirt-sets and the narrowed refusal" gained the cascade paragraph (widen-never-narrow for the keyed channel, whole-table scheduling, report naming).

**Decisions:**
- 2026-09-03: kept the CLI e2e fixture to a 2-node keyed→keyed chain (`keyed_a` origin → `keyed_b` reader) rather than the originally-sketched 3-node keyed→keyed→clocked-reader chain, because admitting a clocked downstream's key-addressed model-edge cell (`admit_key_addressed_recompute`) needs the downstream's own grain provably resolved either via a declared `unique_key` (which flips `derive_grain` to `Key`/`KeyPerPartition`, conflicting with an asserted `grain: partition`) or via a GROUP BY the walk can resolve through a JOIN-based `JoinContext` (which a plain single-source `FROM` never populates) — a real gap, not a phase-21 scope item; the widen-to-whole-for-a-clocked-reader claim is still pinned at the pure-math layer (`keyed_only_node_widens_to_whole_table_for_a_clocked_reader`) and the real-workspace layer (`bare_keyed_model_with_readers_is_scheduled`, which does chain through a clocked `reader` successfully because reader's grain proof there resolves through `keyed_b`'s ANY_VALUE/GROUP BY shape without needing a JOIN — see below).
- `bare_keyed_model_with_readers_is_scheduled`'s clocked `reader` needed `unique_key: [user_id]` declared on `reader` itself failed (`GrainAssertionMismatch`, since a declared identity + declared clock derives `KeyPerPartition`/`Key`, never `Partition`) — the working fixture instead relies on `reader`'s own passthrough SQL reading `user_id` directly from `keyed_b`, admitted via `admit_key_addressed_recompute`'s SQL-structural grain resolution (no declared `unique_key` on `reader`).

**For the next planner:**
- The CLI/runtime gap above (a `grain: partition` downstream's key-addressed model-edge admission when reached only through a plain `FROM`, no `JOIN`) is worth a dedicated look if a real workspace ever needs a bare-keyed-to-clocked-reader edge through a non-JOIN read — today it structurally can't admit one outside the narrow SQL shapes `bare_keyed_model_with_readers_is_scheduled`'s fixture happens to satisfy.
- Phase 22 (time-unrolled self-edges) and phase 23 (`--select` scoping) are next per the outcome table; neither depends on this phase's changes beyond the general graph-layer machinery.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-logical --test maintenance_propagation_adjoint` — 24 passed
- `cargo test -p smelt-runtime --test since_upstream_propagation --test execute_parity` — 26 passed
- `cargo test -p smelt-cli --features duckdb --test since_upstream` — 13 passed
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance` — 74 passed
