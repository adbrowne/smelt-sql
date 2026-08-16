# Phase 8 summary — persisted per-source watermark

**Shipped:**
- `SourceLanding::watermark: Option<String>` on `smelt-state::landed_deltas`, plus
  `LandedDeltaStore::watermark`/`advance_watermark` (monotone, string-compare on the source's
  own ISO axis) — `crates/smelt-state/src/landed_deltas.rs`.
- New pure `smelt-runtime::watermark::watermark_advances` — advances a source only when every
  model in its consumer set (from `propagation::build_forward_graph`) completed this run.
- `execute.rs`'s single success path now derives `consumers_by_source` from
  `build_forward_graph(&all_models, source_infos)` (never a second ref scan; a graph-build
  failure `tracing::warn!`s and skips — stall, not a speculative advance) and advances every
  eligible source's watermark inside the existing `state_io_lock` critical section, through
  `FileStore` — `state.mode: stateless` writes nothing, no extra branch.
- `propagation::pair_source_deltas_with_watermarks` — the two-`--landed`-spelling rule (bare
  positional / `<address>=<start>..<end>` qualified, mixing refused) plus unpaired-source
  resolution from the watermark to `[watermark, now)`, refusing by name when neither a paired
  `--landed` nor a watermark exists. `pair_source_deltas` (unchanged behaviour) now delegates
  to it with `store: None`.
- `smelt-cli::run.rs::run_since_upstream` loads the landed-delta store via `FileStore` and
  calls the watermark-aware pairing with `now = Utc::now()`.
- Spec: `incremental_models.md` §Surface (`--since-upstream` bullet + "Run flags") states the
  pairing-spelling rule and the named-refusal wording; both Known Divergences bullets (~L1948,
  ~L2030) narrowed to the residue that remains (automatic snapshot diffing only).
  `docs-site/docs/reference/cli.md` and `docs-site/docs/reference/state.md` updated to match;
  `main.rs`'s `--source`/`--landed` doc comments updated.

**Decisions:**
- `consumers_by_source` only includes edges whose upstream is a declared **source** (not a
  model-to-model edge) — the watermark is per-source surface, and a model's own forward
  propagation is a separate concern already covered by its own written window / observed-delta
  record.
- The bare-spelling equal-count rule stays exactly as before *except* when `landed` is entirely
  empty (no `--landed` flags at all) and a store is supplied — that's the one case every source
  is unambiguously unpaired, so it resolves from the watermark. A partial bare list with more
  `--source`s than `--landed`s is still a hard count-mismatch error (bare pairing has no address
  to attribute an unpaired source to); only the qualified spelling supports true partial pairing.
- The e2e fixture (`since_upstream.rs::stage_workspace`) defaults to `state.mode: stateless`
  (no watermark persisted); the two new watermark tests override `smelt.yml` with
  `state.mode: environments` locally rather than changing the shared fixture other tests use.

**For the next planner:**
- Phase 8's own Known-Divergences edit left one adjacent clause untouched: the same bullet's
  "the graph layer's keyed channel now carries resolved key *values* ... but only when a caller
  feeds them in as a seed" reads stale after phase 7's live keyed-seed resolution — out of this
  phase's scope (it's about seeds, not the watermark), flagging for whoever next touches that
  bullet.
- Not attempted: per-`(source, consumer)` watermark granularity (explicitly rejected in
  `incremental_models.md` §Design, unstalls selective runs at correctness cost) and automatic
  snapshot diffing for a source with no native delta feed (§Future Extensions) — both remain
  correctly out of scope.
- Row 9 (`smelt explain` signature headline) can now also mention the watermark as a resolved
  input where relevant, though the plan didn't require it here.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy zero-warnings, full `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-state --test landed_deltas --quiet` — PASS.
- `cargo test -p smelt-runtime --test since_upstream_propagation --quiet` — PASS (28 tests).
- `cargo test -p smelt-cli --test since_upstream --features duckdb --quiet` — PASS (13 tests).
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --quiet` — PASS.
- `cargo test -p smelt-lsp --test example_workspaces --quiet` — PASS (34 tests).
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md docs/specs/run_state.md` — no matches.
