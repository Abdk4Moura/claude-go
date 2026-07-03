#!/usr/bin/env bash
#
# scripts/release.sh -- cut a new release of claude-go.
#
# Usage:
#   scripts/release.sh v0.1.0
#
# What it does:
#   1. Runs `cargo test` and aborts on failure.
#   2. Updates the `version` line in Cargo.toml.
#   3. Commits the Cargo.toml change.
#   4. Tags the commit with the version (without 'v' prefix in Cargo,
#      but plain semver).
#   5. Pushes the commit and the tag to origin.
#   6. Waits for the GitHub Actions matrix build to finish, then
#      prints the release URL.
#
# Plain semver only. No `-beta` suffixes.

set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: scripts/release.sh vX.Y.Z" >&2
  exit 2
fi

VERSION="$1"

if ! [[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: version must be plain semver with a 'v' prefix (e.g. v0.1.0); got: $VERSION" >&2
  exit 2
fi

# Strip the v prefix for Cargo.toml.
SEMVER="${VERSION#v}"

# Ensure working tree is clean.
if ! git diff --quiet HEAD 2>/dev/null; then
  echo "error: working tree is not clean. Commit or stash first." >&2
  exit 1
fi

# Sanity: confirm Cargo.toml exists and matches.
if [[ ! -f Cargo.toml ]]; then
  echo "error: Cargo.toml not found in $(pwd)" >&2
  exit 1
fi

# Run the test suite.
echo "==> running tests"
cargo test --quiet

# Bump the version in Cargo.toml.
echo "==> bumping Cargo.toml to $SEMVER"
sed -i.bak -E "s|^version = \"[0-9]+\.[0-9]+\.[0-9]+\"$|version = \"$SEMVER\"|" Cargo.toml
rm -f Cargo.toml.bak

# Commit, tag, push.
git add Cargo.toml
git commit -m "release: $VERSION"
git tag -a "$VERSION" -m "Release $VERSION"
git push origin HEAD
git push origin "$VERSION"

echo
echo "Release $VERSION pushed."
echo "GitHub Actions will now build binaries for all 4 platforms."
echo "Watch progress at: $(git remote get-url origin | sed 's|.*github.com[:/]|https://github.com/|;s|\.git$||')/actions"
echo
echo "Once the matrix build finishes, the release URL will be:"
echo "  $(git remote get-url origin | sed 's|.*github.com[:/]|https://github.com/|;s|\.git$||')/releases/tag/$VERSION"
