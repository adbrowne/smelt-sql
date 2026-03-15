#!/usr/bin/env bash
set -euo pipefail

# prepare-release.sh — Prints the checklist for cutting a new smelt release.
# Does NOT auto-edit files. Run it, review the output, then do the steps manually.

if [ $# -ne 1 ]; then
    echo "Usage: $0 <version>"
    echo "Example: $0 0.2.0"
    exit 1
fi

VERSION="$1"
TAG="v${VERSION}"

echo "=== smelt release checklist for ${TAG} ==="
echo ""
echo "1. Update version strings in these files:"
echo "   - Cargo.toml              (workspace.package.version = \"${VERSION}\")"
echo "   - pyproject.toml          (project.version = \"${VERSION}\")"
echo "   - python/pyproject.toml   (project.version = \"${VERSION}\")"
echo "   - editors/vscode/package.json (\"version\": \"${VERSION}\")"
echo ""
echo "2. Update ROADMAP / CHANGELOG if needed"
echo ""
echo "3. Commit and push:"
echo "   git add -p"
echo "   git commit -m 'Bump version to ${VERSION}'"
echo "   git push origin HEAD"
echo ""
echo "4. Create and push the tag:"
echo "   git tag ${TAG}"
echo "   git push origin ${TAG}"
echo ""
echo "5. The release.yml workflow will:"
echo "   - Verify version strings match the tag"
echo "   - Build wheels and standalone binaries for all platforms"
echo "   - Create a GitHub Release with artifacts and checksums"
echo ""
echo "For a release candidate, use e.g.: $0 0.2.0-rc1"
