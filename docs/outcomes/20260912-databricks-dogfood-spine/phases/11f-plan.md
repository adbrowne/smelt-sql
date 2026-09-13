# Phase 11f — A serverless-installable wheel, offline

## Objective

`bundle deploy` currently stamps the `smelt_wheel` artifact with whatever glibc the build host
links against (`manylinux_2_39` here), which Databricks serverless compute's installer refuses —
the single open blocker on criterion 11's `smelt_run` task. This phase gives the wheel one owner,
`scripts/dbx-wheel-build.sh`, that builds against a declared **manylinux_2_28** floor and refuses
to emit a wheel above it. Entirely offline: no workspace, no credential.

## Decided here (do not re-open)

- **Floor: `manylinux_2_28`.** Serverless `client: "2"` is Ubuntu-22.04-class (glibc 2.35); the
  vendored `libduckdb.so` needs at most `GLIBC_2.25`/`GLIBCXX_3.4.22`, so only the locally-linked
  `smelt` binary forces the floor up.
- **Builder: `maturin build --release --zig --compatibility manylinux_2_28` first**, with a
  manylinux Docker container (`quay.io/pypa/manylinux_2_28_x86_64`) as the fallback selected by
  `SMELT_WHEEL_BUILDER=docker`. Both paths end in the same `verify` check, so neither can pass a
  non-compliant wheel. If `--zig` cannot produce a compliant wheel, switch the script's default to
  `docker` and record why in the summary — do not lower the floor.
- The DuckDB resolution, stale-`libduckdb-sys` cleanup and `--out dist` behaviour currently inlined
  in `databricks.yml`'s `build:` move into the script unchanged; the artifact's `build:` becomes a
  single call to it.

## Spec delta

`docs-site/docs/guide/targets.md` §Databricks, the "smelt reaches the job as a wheel…" paragraph
(around line 315): state that the wheel is built by `scripts/dbx-wheel-build.sh` against a
manylinux floor the platform's installer accepts, and that a bare `maturin build` on a modern
build host produces a wheel serverless compute rejects. One short paragraph plus the script in the
command block; no `docs/specs/` change (this is build toolchain, not target semantics).

## Tests

Red-green, all offline. Tests 1-3 drive the script's `verify` mode on filename fixtures (no build).

1. `wheel_verify_rejects_a_too_new_manylinux_tag` — `verify` on
   `smelt_sql-0.3.2-cp312-cp312-manylinux_2_39_x86_64.whl` exits non-zero and names both the tag
   and the declared floor.
2. `wheel_verify_accepts_the_declared_floor_and_below` — `manylinux_2_28` and `manylinux_2_17`
   (and `manylinux2014`) exit zero.
3. `wheel_verify_rejects_an_unrepaired_linux_tag` — a plain `linux_x86_64` wheel exits non-zero
   (auditwheel repair did not run; the vendored `libduckdb` would be missing).
4. `databricks_bundle.rs::smelt_wheel_build_uses_the_manylinux_build_script` — the `smelt_wheel`
   artifact's `build:` invokes `scripts/dbx-wheel-build.sh` and contains no bare `maturin build`,
   so a second spelling of the build cannot drift back in.
5. `databricks_bundle.rs::smelt_wheel_floor_is_stated_once` — the floor literal appears in the
   script only; `databricks.yml` and the job resource do not restate it.

Place 1-3 alongside the existing bundle tests (`crates/smelt-cli/tests/databricks_bundle.rs`) so
one gate covers the artifact and its builder.

## Tasks

1. Write tests 1-5 against the not-yet-existing script (red).
2. Add `scripts/dbx-wheel-build.sh` with two modes: `build` (default) and `verify <wheel>`; a
   single `MANYLINUX_FLOOR=manylinux_2_28` literal; `SMELT_WHEEL_BUILDER=zig|docker`; the DuckDB
   lib resolution and stale-build cleanup lifted verbatim from `databricks.yml`.
3. `build` ends by calling its own `verify` on every wheel it wrote to `dist/`, so a
   non-compliant wheel is never left on disk for `bundle deploy` to upload.
4. Zig path: ensure `ziglang` and `cargo-zigbuild` are present (install into a dedicated venv /
   `~/.cargo/bin` and say so in a clear error rather than failing deep inside maturin), since
   `bundle deploy` runs this command in its own subprocess environment.
5. Docker path: `docker run --rm -v $PWD:/io -w /io quay.io/pypa/manylinux_2_28_x86_64` building
   with the pinned toolchain and a downloaded `libduckdb.so`; documented in the script header.
6. Repoint `databricks.yml`'s `smelt_wheel` `build:` at the script; drive tests 4-5 green.
7. **Prove it for real, offline:** run `bash scripts/dbx-wheel-build.sh` and confirm `dist/` holds
   a `manylinux_2_28`-or-lower wheel; record the tag, the builder that produced it, wall-clock and
   `ldd`/`objdump -T` evidence that the binary's max versioned symbol is within the floor, in
   `phases/11f-summary.md`. Delete the stale `manylinux_2_39` wheel from `dist/`.
8. Land the docs-site paragraph.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test databricks_bundle --quiet 2>&1 | tail -20`
- `bash .claude/scripts/shellcheck-gate.sh` (new script must be clean at `warning`)
- `bash scripts/dbx-wheel-build.sh` producing a compliant wheel (task 7) — the phase's real proof
- No ratchet lowered; re-baseline the large-file check only if this phase's own growth trips it.

## Commit message

`feat(databricks): build the bundle's smelt wheel against a manylinux_2_28 floor`
