#!/usr/bin/env bash
# dbx-wheel-build.sh — the single owner of the Databricks bundle's smelt
# wheel build (docs/outcomes/20260912-databricks-dogfood-spine/phases/
# 11f-plan.md, criterion 11). A bare `maturin build` links against whatever
# glibc the build host ships (measured on this dev box: manylinux_2_39),
# which Databricks serverless compute's installer refuses. This script
# builds against a declared manylinux_2_28 floor (Free Edition serverless
# `client: "2"` is Ubuntu-22.04-class, glibc 2.35; the vendored
# `libduckdb.so` needs at most GLIBC_2.25/GLIBCXX_3.4.22, so only the
# locally-linked `smelt` binary forces the floor up) and refuses to leave a
# wheel tagged above that floor, or an unrepaired plain `linux_*` wheel, on
# disk for `bundle deploy` to upload. Every `maturin build` invocation here
# passes `--features databricks` — `smelt-cli`'s default feature set has no
# Databricks backend at all (`crates/smelt-cli/Cargo.toml`), a gap every
# earlier live phase's `cargo build -p smelt-cli --features databricks`
# masked because none of them ran the *wheel* path; the deployed bundle
# task failed with "Databricks backend not available" until this script
# started passing the flag (measured phase 11m).
#
#     bash scripts/dbx-wheel-build.sh                        # build all (default): x86_64 + aarch64, writes dist/, verifies output
#     bash scripts/dbx-wheel-build.sh build [x86_64|aarch64|all]
#     bash scripts/dbx-wheel-build.sh verify <wheel>          # check one wheel's platform tag against the floor, no build
#
# Databricks serverless compute can land a job task on either aarch64 or
# x86_64, and there is no pinning mechanism on the platform side (docs.
# databricks.com/compute/serverless/dependencies, confirmed 11g) — so the
# bundle needs a compliant wheel for both architectures, not just the build
# host's own.
#
# SMELT_WHEEL_BUILDER selects the build path:
#   zig     (default) — `maturin build --zig --compatibility manylinux_2_28`.
#             `--zig` needs the `zig` binary and `cargo-zigbuild`; if either
#             is missing this script bootstraps them into a dedicated venv
#             (SMELT_ZIG_VENV, default target/smelt-zig-venv) / `~/.cargo/bin`
#             rather than letting maturin fail deep with a bare "zig not
#             found".
#   docker  — builds inside `quay.io/pypa/manylinux_2_28_x86_64`, installing
#             the repo's pinned Rust toolchain (rust-toolchain.toml) and the
#             pinned DuckDB release (mise's setup-duckdb version) inside the
#             container. Documented fallback for a host where `--zig` cannot
#             produce a compliant wheel; do not lower the floor to work
#             around that instead — switch builders and record why.
#
# Both paths end at the same `verify` check below, so neither can leave a
# non-compliant wheel behind unnoticed.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
MANYLINUX_FLOOR="manylinux_2_28"
FLOOR_MAJOR=2
FLOOR_MINOR=28
DUCKDB_VERSION="1.5.4"
BUILDER="${SMELT_WHEEL_BUILDER:-zig}"
DIST_DIR="${REPO_ROOT}/dist"

usage() {
  echo "usage: $(basename "$0") [build [x86_64|aarch64|all]] | verify <wheel>" >&2
  exit 1
}

# Parses a wheel filename's platform tag into "<glibc-major> <glibc-minor>"
# on stdout, or fails (no stdout) if the tag is not a manylinux tag at all —
# a plain `linux_*` wheel, which means auditwheel repair never ran and the
# vendored libduckdb is almost certainly missing.
glibc_version_for_tag() {
  local platform_tag="$1"
  case "${platform_tag}" in
    manylinux1_*)
      echo "2 5"
      ;;
    manylinux2010_*)
      echo "2 12"
      ;;
    manylinux2014_*)
      echo "2 17"
      ;;
    manylinux_*)
      local rest="${platform_tag#manylinux_}"
      local major="${rest%%_*}"
      local minor_and_arch="${rest#*_}"
      local minor="${minor_and_arch%%_*}"
      case "${major}${minor}" in
        *[!0-9]*|"")
          return 1
          ;;
      esac
      echo "${major} ${minor}"
      ;;
    *)
      return 1
      ;;
  esac
}

# Checks one wheel file's platform tag against the declared floor. Prints an
# OK/REFUSED line and returns 0/1.
verify_wheel() {
  local wheel="$1"
  local base
  base="$(basename "${wheel}")"
  local stem="${base%.whl}"
  local platform_tag="${stem##*-}"

  local version
  if ! version="$(glibc_version_for_tag "${platform_tag}")"; then
    echo "REFUSED: ${base}: platform tag '${platform_tag}' is not a manylinux tag — auditwheel repair did not run, so the vendored libduckdb is almost certainly missing" >&2
    return 1
  fi

  local major minor
  read -r major minor <<<"${version}"

  if ((major > FLOOR_MAJOR || (major == FLOOR_MAJOR && minor > FLOOR_MINOR))); then
    echo "REFUSED: ${base}: platform tag '${platform_tag}' (glibc ${major}.${minor}) is newer than the declared floor ${MANYLINUX_FLOOR} (glibc ${FLOOR_MAJOR}.${FLOOR_MINOR}) — Databricks serverless compute's installer will refuse it" >&2
    return 1
  fi

  echo "OK: ${base}: platform tag '${platform_tag}' (glibc ${major}.${minor}) is within the ${MANYLINUX_FLOOR} floor (glibc ${FLOOR_MAJOR}.${FLOOR_MINOR})"
  return 0
}

# Makes sure a `zig` binary is reachable on PATH, bootstrapping the `ziglang`
# PyPI package into a dedicated venv (via `uv`, the same tool
# scripts/dbx-dogfood-venv.sh and scripts/bigquery-venv.sh use, since the
# system `python3` on this class of host has no working `venv`/`ensurepip`
# — measured: `python3 -m venv` fails outright without the distro's
# `python3-venv` package) if a system `zig` is not already present.
# `ziglang` ships its binary at `<package>/zig`, not on PATH, so a symlink
# into the venv's `bin/` is what actually makes `zig` resolvable.
#
# This venv's `python3` is also the interpreter maturin resolves first on
# PATH, which matters beyond zig: `bindings = "bin"` still tags the wheel
# with that interpreter's exact `cpXYZ-cpXYZ` ABI (maturin always builds one
# wheel per interpreter, version-specific tag, for bin bindings — the
# abi3-* escape hatch is pyo3-extension-only). Pinned to 3.11 because
# Databricks serverless environment version 2 (the `client: "2"` this
# bundle's job tasks declare) ships Python 3.11.10 — a cp312 wheel measured
# phase 11g: `pip` refuses it outright ("not a supported wheel on this
# platform"), independent of the manylinux glibc floor above.
ensure_zig() {
  if command -v zig >/dev/null 2>&1; then
    return 0
  fi

  if ! command -v uv >/dev/null 2>&1; then
    echo "ERROR: zig not found on PATH and 'uv' not found to bootstrap it — install zig yourself (https://ziglang.org/download/) and put it on PATH, install uv (curl -LsSf https://astral.sh/uv/install.sh | sh), or set SMELT_WHEEL_BUILDER=docker" >&2
    exit 1
  fi

  local venv_dir="${SMELT_ZIG_VENV:-${REPO_ROOT}/target/smelt-zig-venv}"
  local zig_link="${venv_dir}/bin/zig"

  if [[ ! -x "${zig_link}" ]]; then
    echo "zig not found on PATH — bootstrapping ziglang into ${venv_dir} (one-time)" >&2
    if ! uv venv --python 3.11 --allow-existing "${venv_dir}" >&2; then
      echo "ERROR: 'uv venv' failed to create ${venv_dir} — install zig yourself and put it on PATH, or set SMELT_WHEEL_BUILDER=docker" >&2
      exit 1
    fi
    if ! uv pip install --quiet --python "${venv_dir}/bin/python" ziglang >&2; then
      echo "ERROR: 'uv pip install ziglang' failed in ${venv_dir} — install zig yourself and put it on PATH, or set SMELT_WHEEL_BUILDER=docker" >&2
      exit 1
    fi
    local ziglang_bin
    if ! ziglang_bin="$("${venv_dir}/bin/python" -c 'import os, ziglang; print(os.path.join(os.path.dirname(ziglang.__file__), "zig"))')"; then
      echo "ERROR: could not locate the zig binary inside the installed ziglang package" >&2
      exit 1
    fi
    ln -sf "${ziglang_bin}" "${zig_link}"
  fi

  export PATH="${venv_dir}/bin:${PATH}"
}

# Makes sure `cargo zigbuild` (maturin's `--zig` flag shells out to it) is
# on PATH, installing it into `~/.cargo/bin` via `cargo install` if missing.
ensure_cargo_zigbuild() {
  if command -v cargo-zigbuild >/dev/null 2>&1; then
    return 0
  fi
  echo "cargo-zigbuild not found on PATH — installing via 'cargo install cargo-zigbuild' (one-time)" >&2
  if ! cargo install --quiet cargo-zigbuild; then
    echo "ERROR: 'cargo install cargo-zigbuild' failed — install it yourself, or set SMELT_WHEEL_BUILDER=docker" >&2
    exit 1
  fi
}

resolve_duckdb_lib_dir() {
  for d in /usr/local/lib "${HOME}/.local/lib/duckdb"; do
    if [[ -e "${d}/libduckdb.so" ]]; then
      echo "${d}"
      return 0
    fi
  done
  return 1
}

# Makes sure the `aarch64-unknown-linux-gnu` Rust target is installed,
# idempotently — matches `ensure_zig`'s shape (check, bootstrap if missing,
# clear error over a deep maturin failure).
ensure_aarch64_target() {
  if ! command -v rustup >/dev/null 2>&1; then
    echo "ERROR: rustup not found on PATH — cannot add the aarch64-unknown-linux-gnu target" >&2
    exit 1
  fi
  if rustup target list --installed | grep -qx "aarch64-unknown-linux-gnu"; then
    return 0
  fi
  echo "aarch64-unknown-linux-gnu target not installed — adding it (one-time)" >&2
  if ! rustup target add aarch64-unknown-linux-gnu >&2; then
    echo "ERROR: 'rustup target add aarch64-unknown-linux-gnu' failed" >&2
    exit 1
  fi
}

# Makes sure an aarch64 libduckdb.so is cached on disk, downloading it the
# same way build_with_docker already pulls DuckDB for its container, and
# prints the cached directory on stdout. `duckdb-sys` links against
# libduckdb at compile time, so cross-building for aarch64 needs an aarch64
# libduckdb.so, not the host's own x86_64 one.
ensure_aarch64_duckdb_lib() {
  local cache_dir="${REPO_ROOT}/target/duckdb-lib/aarch64"
  if [[ -e "${cache_dir}/libduckdb.so" ]]; then
    echo "${cache_dir}"
    return 0
  fi
  mkdir -p "${cache_dir}"
  echo "aarch64 libduckdb.so not cached — downloading v${DUCKDB_VERSION} (one-time)" >&2
  local zip_path
  zip_path="$(mktemp)"
  # DuckDB's own release asset naming is "arm64", not "aarch64" (measured
  # against the v1.5.4 release manifest) — Rust's target triple spells it
  # aarch64, DuckDB's asset filenames spell it arm64, and this script bridges
  # the two.
  if ! curl -sL --fail "https://github.com/duckdb/duckdb/releases/download/v${DUCKDB_VERSION}/libduckdb-linux-arm64.zip" -o "${zip_path}"; then
    rm -f "${zip_path}"
    echo "ERROR: failed to download aarch64 libduckdb.so v${DUCKDB_VERSION}" >&2
    exit 1
  fi
  if ! unzip -oq "${zip_path}" libduckdb.so -d "${cache_dir}"; then
    rm -f "${zip_path}"
    echo "ERROR: failed to unzip aarch64 libduckdb.so into ${cache_dir}" >&2
    exit 1
  fi
  rm -f "${zip_path}"
  echo "${cache_dir}"
}

# Makes sure an aarch64 CPython 3.11 shared library (plus its sysconfigdata)
# is cached on disk, and prints the cached directory on stdout — the
# PYO3_CROSS_LIB_DIR pyo3-build-config's cross-compile path needs.
#
# `smelt-cli`'s pyo3 dependency uses `abi3-py39` (`Cargo.toml`): when
# maturin cannot execute a foreign-arch interpreter to introspect it (there
# is no aarch64 Python on this x86_64 host), pyo3-build-config's
# `default_cross_compile` derives the link-time library name from the abi3
# *floor* version alone — `python3.9` — regardless of which real interpreter
# is targeted (`pyo3-build-config-0.28.3/src/impl_.rs::default_lib_name_for_target`).
# The linker resolves `-lpython3.9` by filename in `PYO3_CROSS_LIB_DIR`, but
# what ends up in the built binary's `DT_NEEDED` is the *target file's own*
# ELF SONAME, not the symlink name used to find it — so a `libpython3.9.so`
# symlink pointing at the real cpython-3.11 shared object makes the linker
# happy while the produced binary correctly declares a dependency on
# `libpython3.11.so.1.0` (verified with `readelf -d`, phase 11m). This is
# safe under abi3: a newer CPython's shared library is a superset of the
# 3.9 stable-ABI symbol table by construction. The resulting binary is not
# bundled with its own libpython (manylinux's repair step excludes
# `libpython*` from external-library bundling on purpose — the wheel
# consumer's own Python provides it) — same trust boundary as any wheel
# generally.
ensure_aarch64_python_lib() {
  local cache_dir="${REPO_ROOT}/target/python-lib/aarch64"
  local abi3_floor_name="libpython3.9.so"
  if [[ -e "${cache_dir}/${abi3_floor_name}" ]]; then
    echo "${cache_dir}"
    return 0
  fi
  mkdir -p "${cache_dir}/lib/python3.11"
  echo "aarch64 CPython 3.11 shared library not cached — downloading (one-time)" >&2
  local release_asset
  release_asset="$(curl -sL --fail "https://api.github.com/repos/astral-sh/python-build-standalone/releases/latest" \
    | python3 -c "
import json, sys
data = json.load(sys.stdin)
for asset in data.get('assets', []):
    name = asset['name']
    if 'cpython-3.11' in name and 'aarch64-unknown-linux-gnu' in name and name.endswith('install_only_stripped.tar.gz'):
        print(asset['browser_download_url'])
        break
")"
  if [[ -z "${release_asset}" ]]; then
    echo "ERROR: could not find an aarch64-unknown-linux-gnu cpython-3.11 asset in python-build-standalone's latest release" >&2
    exit 1
  fi
  local tar_path
  tar_path="$(mktemp)"
  if ! curl -sL --fail "${release_asset}" -o "${tar_path}"; then
    rm -f "${tar_path}"
    echo "ERROR: failed to download aarch64 cpython-3.11 from ${release_asset}" >&2
    exit 1
  fi
  if ! tar xzf "${tar_path}" -C "${cache_dir}" --strip-components=2 \
    python/lib/libpython3.11.so python/lib/libpython3.11.so.1.0; then
    rm -f "${tar_path}"
    echo "ERROR: failed to extract libpython3.11.so from ${tar_path}" >&2
    exit 1
  fi
  if ! tar xzf "${tar_path}" -C "${cache_dir}/lib/python3.11" --strip-components=3 \
    python/lib/python3.11/_sysconfigdata__linux_aarch64-linux-gnu.py; then
    rm -f "${tar_path}"
    echo "ERROR: failed to extract _sysconfigdata from ${tar_path}" >&2
    exit 1
  fi
  rm -f "${tar_path}"
  ln -sf libpython3.11.so.1.0 "${cache_dir}/${abi3_floor_name}"
  echo "${cache_dir}"
}

clean_stale_build_artifacts() {
  rm -rf \
    "${REPO_ROOT}/target/release/build/libduckdb-sys-"* \
    "${REPO_ROOT}/target/release/deps/libduckdb_sys-"* \
    "${REPO_ROOT}/target/release/deps/libduckdb-"*.rlib \
    "${REPO_ROOT}/target/release/deps/smelt-"* \
    "${REPO_ROOT}/target/release/smelt" \
    "${REPO_ROOT}/target/maturin"
}

build_with_zig() {
  local arch="$1"
  ensure_zig
  ensure_cargo_zigbuild

  if [[ "${arch}" == "aarch64" ]]; then
    ensure_aarch64_target
    local aarch64_duckdb_lib
    aarch64_duckdb_lib="$(ensure_aarch64_duckdb_lib)"
    local aarch64_python_lib
    aarch64_python_lib="$(ensure_aarch64_python_lib)"
    # maturin cross-compiling a foreign target cannot execute a
    # target-architecture Python to discover it (there isn't one on this
    # host), so the interpreter must be named explicitly. The host's own
    # cp311 venv (already the interpreter maturin resolves for the x86_64
    # build) supplies the same version/ABI tag; zig alone cross-compiles the
    # Rust binary, so the interpreter that stamps the wheel's tag never needs
    # to run under the target architecture.
    local venv_dir="${SMELT_ZIG_VENV:-${REPO_ROOT}/target/smelt-zig-venv}"
    (
      cd "${REPO_ROOT}"
      export DUCKDB_LIB_DIR="${aarch64_duckdb_lib}"
      export LD_LIBRARY_PATH="${DUCKDB_LIB_DIR}:${LD_LIBRARY_PATH:-}"
      export PYO3_CROSS_LIB_DIR="${aarch64_python_lib}"
      maturin build --release --zig --target aarch64-unknown-linux-gnu --compatibility "${MANYLINUX_FLOOR}" --interpreter "${venv_dir}/bin/python3.11" --out "${DIST_DIR}" --features databricks
    )
  else
    (
      cd "${REPO_ROOT}"
      maturin build --release --zig --compatibility "${MANYLINUX_FLOOR}" --out "${DIST_DIR}" --features databricks
    )
  fi
}

build_with_docker() {
  local arch="$1"
  if [[ "${arch}" == "aarch64" ]]; then
    echo "ERROR: SMELT_WHEEL_BUILDER=docker cannot build aarch64 — cross-arch emulation inside quay.io/pypa/manylinux_2_28_aarch64 needs binfmt/QEMU this box is not known to have. Set SMELT_WHEEL_BUILDER=zig for aarch64, or build on a native aarch64 host." >&2
    exit 1
  fi

  if ! command -v docker >/dev/null 2>&1; then
    echo "ERROR: docker not found on PATH — install docker to use SMELT_WHEEL_BUILDER=docker, or set SMELT_WHEEL_BUILDER=zig" >&2
    exit 1
  fi

  local rust_channel
  rust_channel="$(sed -n 's/^channel = "\(.*\)"/\1/p' "${REPO_ROOT}/rust-toolchain.toml")"
  if [[ -z "${rust_channel}" ]]; then
    echo "ERROR: could not read the pinned Rust channel from rust-toolchain.toml" >&2
    exit 1
  fi

  echo "building via docker (quay.io/pypa/manylinux_2_28_x86_64), rust ${rust_channel}, duckdb ${DUCKDB_VERSION}" >&2

  mkdir -p "${DIST_DIR}"

  docker run --rm \
    -v "${REPO_ROOT}:/io" \
    -w /io \
    "quay.io/pypa/manylinux_2_28_x86_64" \
    bash -c "
      set -euo pipefail
      export PATH=/opt/python/cp311-cp311/bin:\$HOME/.cargo/bin:\$PATH
      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain '${rust_channel}'
      pip install --quiet maturin
      mkdir -p /tmp/duckdb-lib
      curl -sL 'https://github.com/duckdb/duckdb/releases/download/v${DUCKDB_VERSION}/libduckdb-linux-amd64.zip' -o /tmp/duckdb.zip
      unzip -oq /tmp/duckdb.zip libduckdb.so -d /tmp/duckdb-lib
      export DUCKDB_LIB_DIR=/tmp/duckdb-lib
      export LD_LIBRARY_PATH=\"\${DUCKDB_LIB_DIR}:\${LD_LIBRARY_PATH:-}\"
      rm -rf target/release/build/libduckdb-sys-* target/release/deps/libduckdb_sys-* target/release/deps/libduckdb-*.rlib target/release/deps/smelt-* target/release/smelt target/maturin
      maturin build --release --compatibility '${MANYLINUX_FLOOR}' --out dist --features databricks
    "
}

do_build_one() {
  local arch="$1"
  echo "building smelt wheel (${arch}) via SMELT_WHEEL_BUILDER=${BUILDER} (floor: ${MANYLINUX_FLOOR})" >&2

  mkdir -p "${DIST_DIR}"
  # A marker file's mtime, not a before/after filename-set diff: maturin
  # writes the same filename (same version, same tags) on every rebuild, so
  # a set diff sees no "new" file even though the wheel was just rebuilt.
  local marker
  marker="$(mktemp)"

  if [[ "${BUILDER}" != "docker" ]]; then
    local duckdb_lib_dir
    if duckdb_lib_dir="$(resolve_duckdb_lib_dir)"; then
      export DUCKDB_LIB_DIR="${duckdb_lib_dir}"
    fi
    export LD_LIBRARY_PATH="${DUCKDB_LIB_DIR:-}:${LD_LIBRARY_PATH:-}"
    clean_stale_build_artifacts
  fi

  case "${BUILDER}" in
    zig)
      build_with_zig "${arch}"
      ;;
    docker)
      build_with_docker "${arch}"
      ;;
    *)
      echo "ERROR: unsupported SMELT_WHEEL_BUILDER '${BUILDER}' — only zig and docker are wired" >&2
      exit 1
      ;;
  esac

  local new_wheels
  new_wheels="$(find "${DIST_DIR}" -maxdepth 1 -name '*.whl' -newer "${marker}" 2>/dev/null | sort)"
  rm -f "${marker}"

  if [[ -z "${new_wheels}" ]]; then
    echo "ERROR: build reported success but wrote no new wheel to ${DIST_DIR}" >&2
    exit 1
  fi

  local failed=0
  while IFS= read -r wheel; do
    [[ -z "${wheel}" ]] && continue
    if ! verify_wheel "${wheel}"; then
      rm -f "${wheel}"
      failed=1
    fi
  done <<<"${new_wheels}"

  if [[ "${failed}" -ne 0 ]]; then
    echo "ERROR: build produced a non-compliant wheel — removed from ${DIST_DIR} rather than left for 'bundle deploy' to upload" >&2
    exit 1
  fi
}

do_build() {
  local requested="${1:-all}"
  case "${requested}" in
    x86_64)
      do_build_one x86_64
      ;;
    aarch64)
      do_build_one aarch64
      ;;
    all)
      do_build_one x86_64
      do_build_one aarch64
      ;;
    *)
      echo "ERROR: unsupported arch '${requested}' — only x86_64, aarch64 and all are wired" >&2
      exit 1
      ;;
  esac
}

main() {
  local cmd="${1:-build}"
  case "${cmd}" in
    build)
      do_build "${2:-all}"
      ;;
    verify)
      local wheel="${2:-}"
      if [[ -z "${wheel}" ]]; then
        usage
      fi
      verify_wheel "${wheel}"
      ;;
    *)
      usage
      ;;
  esac
}

main "$@"
