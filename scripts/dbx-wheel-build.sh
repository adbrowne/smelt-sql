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
# disk for `bundle deploy` to upload.
#
#     bash scripts/dbx-wheel-build.sh                 # build (default): writes dist/, verifies its own output
#     bash scripts/dbx-wheel-build.sh verify <wheel>   # check one wheel's platform tag against the floor, no build
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
  echo "usage: $(basename "$0") [build] | verify <wheel>" >&2
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
    if ! uv venv --python 3.12 --allow-existing "${venv_dir}" >&2; then
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
  ensure_zig
  ensure_cargo_zigbuild
  (
    cd "${REPO_ROOT}"
    maturin build --release --zig --compatibility "${MANYLINUX_FLOOR}" --out "${DIST_DIR}"
  )
}

build_with_docker() {
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
      export PATH=/opt/python/cp312-cp312/bin:\$HOME/.cargo/bin:\$PATH
      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain '${rust_channel}'
      pip install --quiet maturin
      mkdir -p /tmp/duckdb-lib
      curl -sL 'https://github.com/duckdb/duckdb/releases/download/v${DUCKDB_VERSION}/libduckdb-linux-amd64.zip' -o /tmp/duckdb.zip
      unzip -oq /tmp/duckdb.zip libduckdb.so -d /tmp/duckdb-lib
      export DUCKDB_LIB_DIR=/tmp/duckdb-lib
      export LD_LIBRARY_PATH=\"\${DUCKDB_LIB_DIR}:\${LD_LIBRARY_PATH:-}\"
      rm -rf target/release/build/libduckdb-sys-* target/release/deps/libduckdb_sys-* target/release/deps/libduckdb-*.rlib target/release/deps/smelt-* target/release/smelt target/maturin
      maturin build --release --compatibility '${MANYLINUX_FLOOR}' --out dist
    "
}

do_build() {
  echo "building smelt wheel via SMELT_WHEEL_BUILDER=${BUILDER} (floor: ${MANYLINUX_FLOOR})" >&2

  mkdir -p "${DIST_DIR}"
  local before
  before="$(find "${DIST_DIR}" -maxdepth 1 -name '*.whl' 2>/dev/null | sort)"

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
      build_with_zig
      ;;
    docker)
      build_with_docker
      ;;
    *)
      echo "ERROR: unsupported SMELT_WHEEL_BUILDER '${BUILDER}' — only zig and docker are wired" >&2
      exit 1
      ;;
  esac

  local after
  after="$(find "${DIST_DIR}" -maxdepth 1 -name '*.whl' 2>/dev/null | sort)"
  local new_wheels
  new_wheels="$(comm -13 <(echo "${before}") <(echo "${after}") | sed '/^$/d')"

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

main() {
  local cmd="${1:-build}"
  case "${cmd}" in
    build)
      do_build
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
