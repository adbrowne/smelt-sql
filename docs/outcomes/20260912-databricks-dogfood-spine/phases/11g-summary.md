# Phase 11g summary (attempt 1) — cp311 fix landed, blocked on aarch64/x86_64 variability

**Status: blocked.** Tasks 1-4 done. Task 4 (manual smoke run) surfaced a second, orthogonal
wheel-platform defect beyond 11f's manylinux-floor scope. Tasks 5-13 not reached.

## Shipped

- `scripts/dbx-wheel-build.sh` — two fixes:
  1. `do_build`'s "did a wheel get written" check switched from a before/after filename-set diff
     to a `mktemp` marker's mtime (`find -newer`): `maturin` writes the same filename on every
     rebuild, so the set diff falsely reported "no new wheel" on a second build.
  2. The zig-bootstrap venv (and the Docker builder's `/opt/python/...`) pinned to Python 3.11,
     matching Databricks serverless environment version 2's Python 3.11.10 exactly — `bindings =
     "bin"` always tags the wheel with whatever interpreter it finds, and nothing constrained
     that before. Produces a `cp311-cp311` wheel now.
- `crates/smelt-cli/tests/github_activity_dbx_scheduled.rs` — repointed to `phases/11g-runs.json`
  / `phases/11g-equivalence.json` (still skip-on-missing: no evidence landed this session).
- `docs/outcomes/20260912-databricks-dogfood-spine/outcome.md` — new `## Blocked` entry; phase
  table row 11g flipped to `blocked`; two new rows (11h offline dual-arch build, 11i live resume)
  added.

## Decisions

- The cp311 fix is real and necessary but not sufficient — kept and committed regardless, since
  reverting it would just reintroduce a second, silent defect.
- Did not attempt the `aarch64` wheel build in this session: it needs new infrastructure (a
  second cross-compilation target, a second vendored `libduckdb.so`, a `dbx-wheel-build.sh`
  rewrite to produce/verify two wheels, a `platform_machine`-marker rewrite of the bundle's
  dependency list) — a design decision, not a patch, matching the shape 11e's own wheel blocker
  had before 11f scoped it.
- Left the workspace deployed at the committed daily cadence with the (still incomplete) cp311
  wheel rather than reverting the deploy — it is a strict improvement over 11e's cp312 state,
  even though scheduled runs will keep failing on `aarch64` landings until 11h lands.

## For the next planner

- Land 11h (offline, no workspace) before resuming live legs: dual-arch wheel build +
  `platform_machine`-scoped dependency entries. Confirmed root cause via `WebFetch` against
  Databricks' own docs (`docs/compute/serverless/dependencies`): "A notebook or job can run on
  either `aarch64` or `x86_64`, and the architecture can change between runs" — no pinning
  mechanism exists.
- Workspace state unchanged from 11e: 12 fixture days loaded (`2026-08-05`..`2026-08-16`),
  `github_events` at 40,911 rows. 11i should account for this, not assume 9f's 11-day state.
- `dbx-verify.sh` was green throughout this session; the credential and reachability path needed
  no attention.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-cli --test github_activity_dbx_scheduled --test databricks_bundle --test dbx_dogfood_loader` — all passed (19 + 22 + 5)
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` — 18 passed (stayed green)
- `bash scripts/dbx-bundle.sh validate` — Validation OK
