#!/usr/bin/env bash
#
# CI hygiene gate for the property-discovery loop (design §8).
#
# The loop may extend smelt internals as THROWAWAY, but only in test-target
# surface — never in a production planning/execution path. Every extension it
# adds must be tagged `// EXPERIMENTAL(property-discovery): disposable` at its
# site. This gate asserts every such tagged site lives under a test target
# (a `tests/` directory or a `#[cfg(test)]` module / file), failing otherwise —
# so a headless agent cannot silently wire a disposable accessor into production
# code that later work then depends on.
#
# Exit 0 = clean (all tagged sites are test-only, or there are none).
# Exit 1 = a tagged site is outside test surface — reject.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
cd "${REPO_ROOT}"

TAG="EXPERIMENTAL(property-discovery)"
violations=0

# Every file containing the tag, restricted to Rust sources under crates/.
while IFS= read -r file; do
  [ -z "${file}" ] && continue
  # Test surface #1: anything under a tests/ directory is a test target.
  if [[ "${file}" == *"/tests/"* ]]; then
    continue
  fi
  # Test surface #2: a production src/ file is allowed ONLY if every tagged line
  # sits inside a #[cfg(test)] module. Approximate that cheaply: require the file
  # to contain a `#[cfg(test)]` and require the tag to appear AFTER the first such
  # marker. A tag before any cfg(test) marker (i.e. in production code) fails.
  first_cfg_test="$(grep -n '#\[cfg(test)\]' "${file}" | head -1 | cut -d: -f1)"
  if [ -z "${first_cfg_test}" ]; then
    echo "VIOLATION: ${TAG} in a production file with no #[cfg(test)] module: ${file}"
    violations=$((violations + 1))
    continue
  fi
  while IFS= read -r tagline; do
    lineno="${tagline%%:*}"
    if [ "${lineno}" -lt "${first_cfg_test}" ]; then
      echo "VIOLATION: ${TAG} at ${file}:${lineno} precedes the first #[cfg(test)] (production code)"
      violations=$((violations + 1))
    fi
  done < <(grep -n -F "${TAG}" "${file}")
done < <(grep -rl -F "${TAG}" crates --include='*.rs' 2>/dev/null || true)

if [ "${violations}" -gt 0 ]; then
  echo "property-experimental-gate: ${violations} violation(s) — EXPERIMENTAL(property-discovery) sites must be test-target-only."
  exit 1
fi
echo "property-experimental-gate: clean."
exit 0
