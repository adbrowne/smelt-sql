# Phase 11h — A second, aarch64 wheel, offline

## Objective

11g's manual smoke run surfaced a defect orthogonal to 11f's manylinux floor: Databricks
serverless compute's own docs (`docs/compute/serverless/dependencies`, confirmed via `WebFetch`
in 11g) state that a notebook or job task can land on either `aarch64` or `x86_64`, and which one
it gets **can change between runs** — there is no pinning mechanism on the platform side. The
bundle's `smelt_env` currently installs a single `x86_64`-only wheel
(`../../../dist/*.whl` in `resources/github_activity_job.yml`), so any run that lands on an
`aarch64` node fails to install smelt. This phase gives the bundle a wheel for each architecture
and a dependency list that picks the right one at install time, entirely offline — no workspace,
no credential, no live run.

**Resumes nothing** (11g's live legs stay parked, resumed by 11i under this fix).

## Decided here (do not re-open)

- **Cross-compile via zig, not two Docker images.** `cargo-zigbuild` (already the default builder
  since 11f) accepts `--target aarch64-unknown-linux-gnu` and bundles its own sysroot/linker, so
  it can produce a compliant `aarch64` wheel on this `x86_64` dev box with no QEMU and no second
  container. The Docker fallback (`SMELT_WHEEL_BUILDER=docker`) stays `x86_64`-only — cross-arch
  emulation inside `quay.io/pypa/manylinux_2_28_aarch64` needs `binfmt`/QEMU this box is not known
  to have — and the script must say so in its own error if `docker` is asked for `aarch64`, rather
  than silently building the wrong arch.
- **A second `libduckdb.so` at link time, not a runtime `dlopen`.** The `duckdb-sys` crate links
  against `libduckdb` at compile time, so cross-building for `aarch64` needs an `aarch64`
  `libduckdb.so` on disk, not the host's own `x86_64` one. Pull it the same way the Docker path
  already pulls DuckDB — `https://github.com/duckdb/duckdb/releases/download/v${DUCKDB_VERSION}/libduckdb-linux-aarch64.zip`,
  same pinned `DUCKDB_VERSION=1.5.4` — into a cached `target/duckdb-lib/aarch64/`, and point
  `DUCKDB_LIB_DIR` at it only for the `aarch64` build.
- **`rustup target add aarch64-unknown-linux-gnu` is a one-time, idempotent bootstrap** alongside
  the existing zig/`cargo-zigbuild` bootstrap — checked and added if missing, never assumed
  present, matching `ensure_zig`'s own pattern of a clear error over a deep maturin failure.
- **No wheel renaming.** maturin's own output filename already encodes the platform tag
  (`...manylinux_2_28_x86_64.whl` vs. `...manylinux_2_28_aarch64.whl`), which is exactly what a
  `platform_machine`-scoped PEP 508 requirement needs to disambiguate the two files — the fix
  rewrites `smelt_env.dependencies` to two glob+marker entries, not the build's output naming.
- **`main()` gains an arch argument**: `build [x86_64|aarch64|all]` (default `all` — both wheels,
  since that is what `bundle deploy` needs); `verify <wheel>` is unchanged and arch-agnostic (it
  already parses whatever `_<arch>` suffix the tag carries). Existing callers that invoke
  `dbx-wheel-build.sh` with no arguments now get both wheels instead of one — this is the
  intended fix, not a behaviour change to flag as a divergence.

## Spec delta

`docs-site/docs/guide/targets.md` §"Deployment: Databricks Asset Bundle" (the paragraph 11f
landed about `scripts/dbx-wheel-build.sh` and the manylinux floor): extend it with one short
paragraph stating that the script builds one wheel per architecture and that
`smelt_env.dependencies` selects between them with a `platform_machine` marker, because
serverless compute does not let a job pin its own node architecture. No `docs/specs/` change —
this is deployment tooling, not target semantics.

## Tests

Red-green, all offline; no wheel actually needs building for tests 1-3 (filename fixtures only,
mirroring 11f's `verify` tests).

1. `wheel_verify_accepts_an_aarch64_manylinux_tag` — `verify` on
   `smelt_sql-0.3.2-cp311-cp311-manylinux_2_28_aarch64.whl` exits zero (extends 11f test 2's
   fixture list with an aarch64 tag, proving `glibc_version_for_tag` already parses the arch
   suffix correctly with no code change — a regression guard, not new parsing logic).
2. `dbx_wheel_build_rejects_docker_for_aarch64` — `bash scripts/dbx-wheel-build.sh build aarch64`
   with `SMELT_WHEEL_BUILDER=docker` exits non-zero with an error naming both `aarch64` and
   `docker`, rather than silently building for the wrong arch or hanging on an emulated pull.
3. `smelt_env_dependencies_are_arch_scoped` (in `databricks_bundle.rs`, alongside the existing
   `smelt_wheel_*` tests) — `resources/github_activity_job.yml`'s `smelt_env.dependencies` is
   exactly two entries; one contains `platform_machine == "x86_64"` and a glob ending
   `manylinux_2_28_x86_64.whl`, the other contains `platform_machine == "aarch64"` and a glob
   ending `manylinux_2_28_aarch64.whl`; neither is the old bare `../../../dist/*.whl`.
4. `smelt_wheel_floor_is_stated_once` (existing, 11f) must stay green unchanged — the aarch64
   addition must not restate `manylinux_2_28` anywhere outside the script.

Place 1-2 alongside the existing wheel tests in `crates/smelt-cli/tests/databricks_bundle.rs`.

## Tasks

1. Write tests 1-3 (red) — test 1 against the existing script (already parses correctly, so this
   locks in current behaviour before touching the script); tests 2-3 against the not-yet-changed
   script/job file.
2. Add `ensure_aarch64_target()` to `scripts/dbx-wheel-build.sh` (mirrors `ensure_zig`'s shape):
   check `rustup target list --installed`, `rustup target add aarch64-unknown-linux-gnu` if
   absent, clear error if `rustup` itself is missing.
3. Add `ensure_aarch64_duckdb_lib()`: download+unzip
   `libduckdb-linux-aarch64.zip` (same `DUCKDB_VERSION`) into
   `target/duckdb-lib/aarch64/` if not already cached there; echo the cached path on stdout.
4. Rework `build_with_zig` to take an arch argument (`x86_64`|`aarch64`): for `aarch64`, calls the
   two functions above, exports `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH` pointing at the cached aarch64
   lib (overriding `resolve_duckdb_lib_dir`'s host-arch default), and adds
   `--target aarch64-unknown-linux-gnu` to the `maturin build --zig` invocation; for `x86_64`,
   behaves exactly as today (no `--target`, host's own `libduckdb.so`).
5. Rework `build_with_docker` to accept the same arch argument; `x86_64` is unchanged; `aarch64`
   exits non-zero immediately with the "not supported over Docker" error from test 2, before
   attempting any `docker run`.
6. Rework `do_build`'s marker-mtime "did a wheel get written" check and the `verify` sweep to run
   once per requested arch, and `main()`'s `build` subcommand to accept an optional arch argument
   defaulting to `all` (both, sequentially).
7. Rewrite `resources/github_activity_job.yml`'s `smelt_env.dependencies` from the single glob to
   the two glob+marker entries in tests 3; drive test 3 green.
8. **Prove it for real, offline:** run `bash scripts/dbx-wheel-build.sh` (default `all`) and
   confirm `dist/` holds one compliant `x86_64` wheel and one compliant `aarch64` wheel (both
   `manylinux_2_28`, `cp311`); record wall-clock, both filenames, and — since there is no aarch64
   machine here to execute it — `objdump`/`file` evidence that the aarch64 wheel's bundled
   `libduckdb.so` and the `smelt` binary inside it are genuinely `ELF 64-bit ... ARM aarch64`, in
   `phases/11h-summary.md`.
9. `bash scripts/dbx-bundle.sh validate` (loopback-stub, no workspace) stays green with the
   rewritten job resource.
10. Land the docs-site paragraph.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test databricks_bundle --quiet 2>&1 | tail -30`
- `bash .claude/scripts/shellcheck-gate.sh` (script grows; must stay clean at `warning`)
- `bash scripts/dbx-wheel-build.sh` producing both compliant wheels (task 8) — the phase's real
  proof
- `bash scripts/dbx-bundle.sh validate` (task 9)
- No ratchet lowered.

## Commit message

`feat(databricks): cross-build an aarch64 wheel and arch-scope the bundle's dependency list`
