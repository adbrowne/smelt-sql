# Plan: Production readiness W5 — release engineering

**Date**: 2026-07-19
**Spec**: [`docs/specs/cli.md`](../specs/cli.md) (Phase 1 `smelt ui` surface only); research basis [`docs/research/20260719-production-release-review.md`](../research/20260719-production-release-review.md) (blocker #9 + secondary items)
**Spec diff**: none yet — Phase 1 carries its own spec edit; Phases 2–6 are repo metadata / CI with no feature-spec surface
**Tracking PR / branch**: `worktree-production` (production-readiness master; see [`docs/plans/20260719-production-readiness.md`](20260719-production-readiness.md))
**Docs**: code+docs   <!-- Phases 2–6 are largely repo-metadata; each still names its docs-site touch or states "none" -->

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. For Phase 1 also read `docs/specs/cli.md` (the `smelt ui` command surface) — it is the correctness oracle for that phase. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-production`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**Repo-metadata phases (2–6).** These produce CI workflows, repo policy files, and packaging artifacts rather than library code. Red-green TDD does not apply where nothing is unit-testable; each such phase instead lists **verification commands** that must pass (and, where a GitHub-side effect can only be observed on a tag push, an explicit human-gated step recorded under "Deferred during implementation"). Never invent a runtime test harness just to satisfy the TDD convention — say which check stands in for it.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- A step needs credentials or an external resource only Andrew controls (crates.io ownership, a `homebrew-smelt` tap repo, ghcr.io settings). Record it human-gated and move on.
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call: fmt + clippy + tests + example_diagnostics) — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/cli.md` and `docs-site/docs/...` describe the feature as if it has always existed.

---

## Context

The production-release review found the release plumbing genuinely strong (tag-triggered PyPI wheels on four targets, VS Marketplace + Open VSX, nightly TestPyPI, a version-check job) but the surrounding hygiene missing: no `CHANGELOG.md`/`SECURITY.md`/`RELEASING.md`, a docs claim of macOS-Intel standalone binaries that `release.yml` does not build, no Docker image or Homebrew path, a permissive-CORS UI server that will bind non-loopback without a warning, and an inconsistent crates-publishing posture (16 of 21 crates carry `publish = false`; `smelt-parser`, `smelt-types`, `smelt-logical`, `smelt-dialect`, `smelt-fingerprint` do not). This plan closes those gaps for the v0.5 cut.

## Scope

### In scope
- `smelt ui` network hardening: origin-restricted CORS, explicit opt-in + warning for non-loopback bind (`cli.md` §`smelt ui`).
- `CHANGELOG.md` (keep-a-changelog, seeded for 0.5.0 from history since `v0.3.2`) + `RELEASING.md` checklist matching `release.yml`.
- `SECURITY.md`; reconcile the macOS-Intel standalone-binary claim in `docs-site/docs/getting-started/installation.md` with `release.yml`.
- `Dockerfile` + tag-triggered ghcr.io publish job with a smoke test.
- Homebrew formula + tap instructions (external repo step human-gated).
- Crates-publishing decision brief + consistent `publish` flags.

### Explicitly deferred
- UI authentication (login, tokens). The v0.5 stance is localhost-only-by-default, documented; auth is post-0.5.
- Actually publishing any crate to crates.io (human-gated; this plan only makes the workspace posture consistent and writes the brief).
- Windows/ARM Docker images — linux/amd64 (+arm64 if free) only.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | (this commit) | 2026-07-20 |
| 2     | done     | (this commit) | 2026-07-20 |
| 3     | done     | (this commit) | 2026-07-20 |
| 4     | done     | (this commit) | 2026-07-20 |
| 5     | done     | (this commit) | 2026-07-20 |
| 6     | pending  |        |      |

## Phase detail

### Phase 1: UI server network hardening

**Goal.** `smelt ui` is safe by default: CORS restricted to the served origin (today `CorsLayer::permissive()` at `crates/smelt-ui/src/server.rs:90`), and a non-loopback `--host` requires an explicit `--allow-remote` flag plus a startup warning. Default bind is already `127.0.0.1` (`crates/smelt-cli/src/main.rs` `UiArgs`); keep it.

**Pre-conditions.** None.

**TDD tests to write first.**
- `crates/smelt-ui/tests/server_hardening.rs::cors_rejects_cross_origin` — a request with `Origin: http://evil.example` against a running test server gets no `Access-Control-Allow-Origin: *` (and no echo of the foreign origin); a same-origin request succeeds.
- `crates/smelt-ui/tests/server_hardening.rs::non_loopback_bind_requires_opt_in` — `start_server` (or its config validation) with host `0.0.0.0` and `allow_remote = false` returns an error naming the flag; with `allow_remote = true` it proceeds.
- `crates/smelt-cli` arg-parse test — `smelt ui --host 0.0.0.0` without `--allow-remote` fails with the guidance message; with the flag it parses.

**Implementation shape.** Add `allow_remote: bool` to `UiArgs` and thread it into `smelt_ui::start_server`. In `server.rs`, replace `CorsLayer::permissive()` with a layer allowing only the bound origin(s) (`http://{host}:{port}`, plus `http://localhost:{port}` when bound to loopback). Validate host: loopback ⇒ ok; otherwise require `allow_remote` and emit a `tracing::warn!` (user-facing startup line via existing stdout path is fine — `smelt-ui` binary stdout is excluded from the println gate, but prefer the existing reporter if one exists).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-ui/src/server.rs` — CORS layer, bind validation, warning
- `crates/smelt-ui/src/lib.rs` — signature threading
- `crates/smelt-cli/src/main.rs`, `crates/smelt-cli/src/commands/ui.rs` — `--allow-remote` flag
- `crates/smelt-ui/tests/server_hardening.rs` — new

**Docs touched.**
- `docs/specs/cli.md` — `smelt ui` Surface: `--host`, `--allow-remote`, the loopback default and remote-bind rule
- `docs-site/docs/guide/web-ui.md` + `docs-site/docs/reference/cli.md` — same surface, plus a short "network exposure" note stating the no-auth/localhost-only stance

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] No permissive CORS remains; loopback default unchanged
- [ ] Fail-loud: refusing non-loopback without the flag is an error, not a silent rebind
- [ ] User docs + spec updated, timeless

**Commit.** `fix(ui): origin-restricted CORS + explicit --allow-remote gate for non-loopback bind`

### Phase 2: CHANGELOG.md + RELEASING.md

**Goal.** A curated `CHANGELOG.md` (keep-a-changelog format, `Unreleased` → `0.5.0` section seeded from `git log v0.3.2..HEAD` highlights, backfilled one-liners for 0.1–0.3.2 from tags) and a `RELEASING.md` checklist that matches what `release.yml` actually does (version-check job expects `Cargo.toml` + `editors/vscode/package.json` to match the tag; wheel matrix; VSIX publish).

**Pre-conditions.** None.

**Verification commands (stand in for TDD — repo-metadata phase).**
- `grep -q '## \[0.5.0\]' CHANGELOG.md` and every section header parses as keep-a-changelog (`Added/Changed/Fixed/...`).
- Each step in `RELEASING.md` cross-checked against `.github/workflows/release.yml` job names — reviewer verifies no invented step.

**Implementation shape.** Curate, don't dump: group the v0.3.2→HEAD history by theme (maintenance-plan programme, composed axes, parser conformance, meta-language, tooling), ~30–50 bullet lines total, linking specs where they are the better reference. `RELEASING.md`: bump versions → update CHANGELOG → tag `vX.Y.Z` → what CI does → manual follow-ups (VSIX marketplace propagation, announcement).

**Critical files.** `CHANGELOG.md` (new), `RELEASING.md` (new).

**Docs touched.** None (repo metadata; docs-site untouched).

**Review checklist:**
- [ ] Every 0.5.0 bullet corresponds to something actually on `main` (spot-check against `git log`)
- [ ] `RELEASING.md` steps match `release.yml` jobs exactly
- [ ] No phase/plan vocabulary in either file

**Commit.** `docs(release): CHANGELOG seeded for 0.5.0 + RELEASING checklist`

### Phase 3: SECURITY.md + macOS-Intel claim reconciliation

**Goal.** A `SECURITY.md` (supported-version table pointing at the latest minor, private report channel `brownie@brownie.com.au`, response expectation, an explicit no-telemetry statement). Reconcile `docs-site/docs/getting-started/installation.md` §"Standalone binaries", which claims `macOS x86_64 (Intel)` while neither binary matrix in `release.yml` builds it.

**Pre-conditions.** None.

**Verification commands.**
- `test -f SECURITY.md`
- After reconciliation, `rg -i 'x86_64 \(Intel\)' docs-site/docs/getting-started/installation.md` matches only if `release.yml` gained a `x86_64-apple-darwin` entry in **both** the wheel and binary matrices.

**Implementation shape.** Audit first: an Intel build is a cross-compile from the `macos-latest` arm runner (`rustup target add x86_64-apple-darwin` + `--target`); it is a small matrix addition **iff** the bundled-DuckDB C++ build cross-compiles cleanly — do not burn more than one CI experiment on it. Default outcome: drop the Intel line from the docs (pip sdist fallback already covers Intel Macs, say so) and record the option under Deferred. Stretch outcome: add the target to both matrices in `release.yml` (+ `dev-release.yml` for nightly coverage) and keep the docs line.

**Critical files.** `SECURITY.md` (new), `docs-site/docs/getting-started/installation.md`, optionally `.github/workflows/release.yml` + `.github/workflows/dev-release.yml`.

**Docs touched.** `installation.md` as above (timeless — state what is supported, not what changed).

**Review checklist:**
- [ ] Docs claim and `release.yml` matrices agree after the phase, whichever direction was taken
- [ ] `SECURITY.md` names a real contact and the no-telemetry stance
- [ ] If the workflow was touched, `dev-release.yml` and `release.yml` stayed in sync

**Commit.** `docs(release): SECURITY.md + reconcile macOS-Intel binary claim with release matrix`

### Phase 4: Docker image + ghcr.io publish

**Goal.** A multi-stage `Dockerfile` (builder: rust + `cargo build --release -p smelt-cli` with system DuckDB via `DUCKDB_LIB_DIR`; runtime: slim Debian with `libduckdb.so` copied in and `LD_LIBRARY_PATH` set) and a `docker` job in `release.yml` publishing `ghcr.io/adbrowne/smelt` on `v*` tags, with a CI smoke test.

**Pre-conditions.** Phase 2 (RELEASING.md gains the Docker step).

**Verification commands (CI-locale; also runnable locally).**
- `docker build -t smelt-local .` succeeds.
- `docker run --rm smelt-local --version` prints the workspace version.
- `docker run --rm -v $PWD/examples/bounded_domain_declared:/w -w /w smelt-local build` exits 0 — wire exactly this as the workflow's smoke-test step so it is enforced per release; also add it to a PR-triggered `docker-build` job gated on `Dockerfile`/workflow path changes so image breakage is caught pre-tag. (Substituted for `examples/test_workspace`: that workspace is on the `KNOWN_UNBUILDABLE` allow-list in `crates/smelt-cli/tests/e2e/example_builds.rs` — its `raw_events` model reads an unseeded external source, so `smelt build` fails standalone regardless of the Docker image's correctness. `bounded_domain_declared` is a clean, self-contained workspace with no external sources.)

**Implementation shape.** Pin the DuckDB version to the one in `Cargo.toml` (v1.5.4 per CLAUDE.md setup); fetch `libduckdb-linux-amd64.zip` in the builder stage. Publish `linux/amd64`; add `linux/arm64` via buildx only if it needs no extra plumbing. Tag `latest` + the version. Update `RELEASING.md` and add an "Install via Docker" subsection to `installation.md`.

**Critical files.** `Dockerfile` (new), `.dockerignore` (new), `.github/workflows/release.yml` (docker job), `.github/workflows/test.yml` or `release.yml` (path-gated PR build), `RELEASING.md`, `docs-site/docs/getting-started/installation.md`.

**Docs touched.** `installation.md` Docker subsection (timeless).

**Review checklist:**
- [x] Smoke test (`--version` + `bounded_domain_declared` build) runs in CI, not just documented
- [x] Image does not embed the source tree or build cache (multi-stage verified)
- [x] ghcr push happens only on tags; PR job builds without pushing

**Commit.** `feat(release): Dockerfile + ghcr.io publish with example-workspace smoke test`

### Phase 5: Homebrew formula

**Goal.** A Homebrew formula for the standalone binary (`Formula/smelt.rb` in a new in-repo `packaging/homebrew/` directory), templated against GitHub release artifact URLs + sha256s, and a `RELEASING.md` step for updating it. Creating and pushing the external `adbrowne/homebrew-smelt` tap repo is **human-gated**.

**Pre-conditions.** Phase 2.

**Verification commands.**
- `brew ruby -c packaging/homebrew/Formula/smelt.rb` (syntax check; if `brew` is unavailable in the environment, `ruby -c` on the formula file) — record which ran.
- Formula URLs match the actual `release.yml` binary artifact naming (reviewer cross-checks the upload step's file names).

**Implementation shape.** Formula with `on_macos`/`on_linux` + arch blocks pointing at the four (or five, post-Phase-3) binary tarballs; a small `packaging/homebrew/README.md` documenting the tap-repo bootstrap (`brew tap adbrowne/smelt && brew install smelt`) and the per-release sha256 update (scriptable: `scripts/update-homebrew-formula.sh` taking a version, fetching artifacts, rewriting sha256s). A decision brief section in that README: in-repo tap dir vs dedicated tap repo (recommend dedicated repo, generated from this file), left for Andrew.

**Critical files.** `packaging/homebrew/Formula/smelt.rb` (new), `packaging/homebrew/README.md` (new), `scripts/update-homebrew-formula.sh` (new), `RELEASING.md`, `docs-site/docs/getting-started/installation.md` (Homebrew subsection, marked as requiring the tap once live).

**Docs touched.** `installation.md` (timeless; if the tap is not yet live, the subsection ships in the same commit as the human-gated note in this plan, and the docs text must not promise a channel that does not exist — gate its inclusion on Andrew's go-ahead, otherwise record under Deferred).

**Review checklist:**
- [ ] Artifact names/URLs in the formula match `release.yml` outputs exactly
- [ ] sha256 update path is scripted, not hand-edited
- [ ] External-repo step recorded human-gated, not silently assumed done

**Commit.** `feat(release): Homebrew formula + tap bootstrap under packaging/homebrew`

### Phase 6: Crates-publishing posture

**Goal.** A one-page decision brief (`docs/research/20260719-crates-publishing.md`) enumerating the 21 workspace crates and recommending a posture, plus workspace `Cargo.toml` changes making the posture explicit so a stray `cargo publish` cannot half-publish.

**Pre-conditions.** None.

**Verification commands.**
- `for c in crates/*/Cargo.toml; do rg -q '^publish' $c || echo "MISSING: $c"; done` prints nothing — every crate has an explicit `publish` value.
- `cargo metadata --format-version 1 | jq '[.packages[] | select(.publish == null)] | length'` equals the count the brief says is publishable.

**Implementation shape.** Current state: 16 crates carry `publish = false`; `smelt-parser`, `smelt-types`, `smelt-logical`, `smelt-dialect`, `smelt-fingerprint` do not (and the brief must check whether any are actually on crates.io — the review said 3/21 published; verify with `cargo search`/crates.io API and record findings). Recommended default: `publish = false` on **all** crates for v0.5 (distribution is wheels/binaries/Docker; the parser stack can be opened deliberately post-0.5 with real semver commitments) — but write the brief first and follow its conclusion. Publishing anything is human-gated.

**Critical files.** `docs/research/20260719-crates-publishing.md` (new), `crates/{smelt-parser,smelt-types,smelt-logical,smelt-dialect,smelt-fingerprint}/Cargo.toml`.

**Docs touched.** None.

**Review checklist:**
- [ ] Brief reflects verified crates.io state, not the review's unverified "3/21"
- [ ] Every crate has an explicit publish posture after the phase
- [ ] No crate was actually published

**Commit.** `chore(release): explicit publish posture for all crates + publishing decision brief`

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **Phase 5 (2026-07-20)**: creating and pushing the external `adbrowne/homebrew-smelt` tap repo is human-gated — Andrew owns that repo. Until it exists, `installation.md` does not gain a Homebrew subsection (would promise a channel that isn't live yet); `packaging/homebrew/README.md` documents the bootstrap once the tap repo is created. Formula sha256 placeholders are filled by `scripts/update-homebrew-formula.sh` after the first tagged release with standalone artifacts to fetch.

## Verification

How to confirm the plan is satisfied at the end:
- `bash .claude/scripts/verify-phase.sh`
- Phase-1 hardening tests: `cargo test -p smelt-ui --test server_hardening`
- `docker build . && docker run --rm smelt-local --version` (Phase 4 smoke, locally)
- `test -f CHANGELOG.md -a -f SECURITY.md -a -f RELEASING.md`
- Docs/matrix agreement: installation.md platform list matches `release.yml` matrices
- All six phases `done` in Progress tracking; human-gated items (tap repo, any crates.io action) listed under Deferred, not silently dropped
