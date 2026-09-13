# Phase 11h summary — dual-arch wheel, offline

**Shipped:**
- `scripts/dbx-wheel-build.sh`: `ensure_aarch64_target()` (idempotent `rustup target add
  aarch64-unknown-linux-gnu`), `ensure_aarch64_duckdb_lib()` (cached download of DuckDB's
  aarch64 `libduckdb.so`), `build_with_zig`/`build_with_docker`/`do_build` reworked to take an
  arch argument; `build aarch64` under `docker` refuses immediately (no binfmt/QEMU on this
  box); `main()`'s `build` subcommand now takes `[x86_64|aarch64|all]` (default `all`).
- `examples/github_activity/resources/github_activity_job.yml`: `smelt_env.dependencies` is now
  two `platform_machine`-scoped glob entries instead of the single unscoped
  `../../../dist/*.whl`.
- `docs-site/docs/guide/targets.md`: one new paragraph on the dual-arch wheel and why
  (serverless compute's undocumented per-run arch selection).
- Real proof: `bash scripts/dbx-wheel-build.sh` produced both wheels
  (`smelt_sql-0.3.2-cp311-cp311-manylinux_2_28_x86_64.whl`,
  `...manylinux_2_28_aarch64.whl`); `file`/`objdump` confirmed the aarch64 wheel's `smelt` binary
  and vendored `libduckdb.so` are genuine `ELF 64-bit ... ARM aarch64` (not just tag-renamed
  x86_64 binaries). Wall clock: x86_64 ~18s warm / aarch64 ~82s cold (full workspace rebuild for
  the new target).
- 3 new tests in `crates/smelt-cli/tests/databricks_bundle.rs` (22 total, all green).

**Decisions:** (full rationale in outcome.md's Decision log, 2026-09-13 entries)
- Arch-scoped glob disambiguates by the `_x86_64.whl`/`_aarch64.whl` filename suffix, not by
  restating `manylinux_2_28` — the plan's own test-3 wording conflicted with its own test-4
  single-ownership gate; resolved in test-4's favor.
- Fixed the plan's DuckDB URL: `libduckdb-linux-arm64.zip`, not `-aarch64.zip` (measured against
  the real v1.5.4 release manifest); added `curl --fail` so this class of mistake fails loud.
- `maturin build --target aarch64-unknown-linux-gnu` needs `--interpreter <venv>/bin/python3.11`
  explicitly — maturin can't introspect a foreign-arch interpreter, so a local host-arch Python
  of the matching version stamps the wheel tag instead.

**For the next planner:**
- **Environment corruption, not this phase's fault, needs a note:** `target/release/{build,deps}`
  in this worktree had ~700+ root-owned files/dirs from what looks like an earlier
  `SMELT_WHEEL_BUILDER=docker` run that mounted the repo into a container without uid mapping.
  Three directories (`target/maturin`, two `libduckdb-sys-*` build dirs) couldn't be emptied
  without sudo (not available, no password), so they were renamed aside within the same
  filesystem (`target/release/build/ZZZ-orphan-{1,2}`, `target/maturin-root-owned-orphan-*`) to
  unblock the build rather than deleted. **These orphans still exist and need
  `sudo rm -rf` by a human with shell access** — they're gitignored so they don't affect commits,
  but they're ~dozens of MB of unreclaimable-by-me disk. Not scheduled as outcome work since it's
  a one-off environment fix, not a repo change — flagging here so it isn't mistaken for
  intentional state.
- 11i (resume 11g's live legs under this dual-arch wheel) is next and needs the credential from
  phase 4b/4c — same live-gating as before.

**Gates:**
- `cargo test -p smelt-cli --test databricks_bundle --quiet` — 22 passed, 0 failed
- `bash .claude/scripts/shellcheck-gate.sh` — PASS (80 scripts, zero findings)
- `bash scripts/dbx-wheel-build.sh` — both wheels built and verified (task 8, the phase's real
  proof)
- `bash scripts/dbx-bundle.sh validate` — skips cleanly (databricks CLI not installed in this
  environment), consistent with how this gate has always behaved when the CLI is absent
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, example_diagnostics)
- No ratchet lowered.
