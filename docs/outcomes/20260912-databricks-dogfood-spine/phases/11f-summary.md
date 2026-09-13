# Phase 11f summary — a serverless-installable wheel, offline

**Shipped:**
- `scripts/dbx-wheel-build.sh`: single owner of the bundle's smelt wheel build. `build` (default)
  and `verify <wheel>` modes; `MANYLINUX_FLOOR=manylinux_2_28` declared once; `SMELT_WHEEL_BUILDER`
  selects `zig` (default, self-bootstraps `ziglang`/`cargo-zigbuild` via `uv venv` if missing) or
  `docker` (a `quay.io/pypa/manylinux_2_28_x86_64` container, pinned Rust + DuckDB installed inside).
  `build` always ends by verifying every wheel it wrote to `dist/` and deleting any that fail.
- `examples/github_activity/databricks.yml`'s `smelt_wheel` artifact now calls `bash
  scripts/dbx-wheel-build.sh` instead of a bare `maturin build`.
- `crates/smelt-cli/tests/databricks_bundle.rs`: 5 new tests (3 drive `verify` on filename
  fixtures, 2 are structural — the artifact calls the script and the floor literal appears only
  in the script). `bundle_smelt_comes_from_the_locally_built_wheel` updated to check for the
  script instead of the now-moved `maturin build` string.
- `docs-site/docs/guide/targets.md` §Databricks: one paragraph on why the wheel goes through the
  script rather than a bare `maturin build`.

**Decisions:** logged in `outcome.md` "## Decision log" (2026-09-13 entries) — `--zig` worked on
the first try (no docker fallback needed), and `ensure_zig` bootstraps via `uv`, not
`python3 -m venv`, because this host's system Python has no working `ensurepip`.

**For the next planner:**
- The docker builder path is implemented and shellchecked but never actually run end-to-end in
  this phase — worth a live/offline check if the zig path ever stops working on a future host.
- The real proof (task 7): `bash scripts/dbx-wheel-build.sh` from a clean `target/` produced
  `dist/smelt_sql-0.3.2-cp312-cp312-manylinux_2_28_x86_64.whl` in ~19s wall via the self-bootstrapped
  zig path. `objdump -T` on the extracted `smelt` binary: max versioned symbol `GLIBC_2.28`
  (others: 2.18, 2.25) — exactly at, never above, the declared floor. The stale
  `manylinux_2_39` wheel was deleted from `dist/`.
- Row 11g (resume 11e's live legs under this wheel) is next — it needs a reachable Databricks
  workspace and will emit `<<PHASE_BLOCKED>>` if `scripts/dbx-dogfood-env.sh` cannot reach it.

**Gates:**
- `cargo test -p smelt-cli --test databricks_bundle --quiet` — 19 passed.
- `bash .claude/scripts/shellcheck-gate.sh` — PASS (80 scripts, zero findings).
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full test suite,
  example_diagnostics).
- `bash .claude/scripts/large-file-check.sh` — OK, no ratchet change.
- `bash scripts/dbx-wheel-build.sh` (real build, no workspace) — produced a compliant wheel.
