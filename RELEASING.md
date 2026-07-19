# Releasing smelt

This is the checklist for cutting a stable release. It describes exactly
what `.github/workflows/release.yml` does on a `v*` tag push — read that
file if a step here and the workflow ever disagree, the workflow wins.

## 1. Prepare the release

1. Update `CHANGELOG.md`: move `[Unreleased]` entries into a new
   `## [X.Y.Z] - YYYY-MM-DD` section (keep an empty `[Unreleased]` above it).
2. Bump the version in **both** places the `version-check` job compares
   against the tag:
   - `Cargo.toml` (workspace root `[workspace.package] version`)
   - `editors/vscode/package.json`
3. Run the standard verification gate: `bash .claude/scripts/verify-phase.sh`.
4. Commit: `chore: bump version to X.Y.Z` (include the CHANGELOG edit).
5. Push to `main` and confirm CI is green before tagging.

## 2. Tag and push

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

Use a `-rc`/`-beta`/`-alpha` suffix (e.g. `v0.5.0-rc1`) for a pre-release —
the workflow routes pre-releases to TestPyPI instead of PyPI and skips the
VS Marketplace / crates.io / GitHub-prerelease-flag steps accordingly (see
below).

## 3. What CI does automatically

Pushing the tag triggers `.github/workflows/release.yml`:

1. **Version Check** — fails the whole run if `Cargo.toml` or
   `editors/vscode/package.json` doesn't match the tag.
2. **Wheel / \*** — builds Python wheels via `maturin` for
   linux-x86_64, linux-aarch64, macos-aarch64, windows-x86_64
   (bundled DuckDB).
3. **Standalone / \*** — builds `smelt` + `smelt-lsp` + `smelt-datagen`
   binaries for the same four targets and packages them as
   `.tar.gz` (Unix) / `.zip` (Windows) with `LICENSE` + `README.md`.
4. **GitHub Release** — collects every wheel/standalone artifact,
   generates `SHA256SUMS.txt`, writes release notes from
   `git log <prev-tag>..HEAD --no-merges`, and creates the GitHub release
   (marked pre-release if the tag has a `-rc`/`-beta`/`-alpha` suffix).
5. **Publish to PyPI** — stable tags only; uploads all wheels
   (`skip-existing: true`).
6. **Publish to TestPyPI** — pre-release tags only, same wheels.
7. **Publish VSCode Extension** — stable tags only; publishes to the VS
   Marketplace and (best-effort, `continue-on-error`) Open VSX.
8. **Publish to crates.io** — stable tags only; publishes `smelt-types`,
   `smelt-parser`, `smelt-dialect` in dependency order with a 30s pause
   between each for the crates.io index to catch up. Only the crates
   without `publish = false` are included here — confirm the publish
   posture is deliberate (not an oversight) before adding another crate.

Nightly/dev builds are handled separately by `.github/workflows/dev-release.yml`
and are not part of this checklist.

## 4. Manual follow-ups (not automated)

- **VS Marketplace / Open VSX propagation** — the extension listing can take
  a few minutes to show the new version; confirm both marketplaces show
  `vX.Y.Z` before announcing.
- **Docker image** — if `Dockerfile` + the ghcr.io publish job are present,
  confirm the tagged image built and the smoke test passed; `docker pull`
  it once to sanity-check.
- **Homebrew tap** — bump the formula in the `homebrew-smelt` tap repo (a
  separate repo Andrew owns) to point at the new tag's tarball + updated
  SHA256; this is not driven by `release.yml`.
- **Announcement** — post the GitHub release notes wherever the project
  announces releases.

## 5. If something fails mid-release

- `version-check` failing blocks every downstream job — fix the mismatched
  version file, delete the tag (`git push --delete origin vX.Y.Z` after
  confirming with Andrew), and re-tag.
- Wheel/standalone build failures are per-target (`fail-fast: false`); a
  single-target failure does not need a full re-tag — re-run the failed
  matrix job from the Actions UI once the fix is on `main`, then re-tag if
  the fix required a code change.
- PyPI/crates.io publishes use `skip-existing: true` / are idempotent per
  crate, so re-running the workflow after a partial publish is safe.
