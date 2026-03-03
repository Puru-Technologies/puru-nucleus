#!/usr/bin/env bash
set -euo pipefail

# ── Usage ────────────────────────────────────────────────────────
# ./scripts/release.sh <version>
#
# Examples:
#   ./scripts/release.sh 0.2.0          → stable release
#   ./scripts/release.sh 0.3.0-beta.1   → beta release
# ─────────────────────────────────────────────────────────────────

VERSION="${1:-}"

if [ -z "$VERSION" ]; then
  echo "Usage: ./scripts/release.sh <version>"
  echo "  e.g. ./scripts/release.sh 0.2.0"
  exit 1
fi

# Strip leading 'v' if provided
VERSION="${VERSION#v}"

# Validate semver-ish format
if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+'; then
  echo "Error: Version must be semver (e.g. 0.2.0 or 0.2.0-beta.1)"
  exit 1
fi

TAG="v${VERSION}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Check for clean working tree
if [ -n "$(git -C "$ROOT" status --porcelain)" ]; then
  echo "Error: Working tree is not clean. Commit or stash changes first."
  exit 1
fi

# Check we're on main
BRANCH="$(git -C "$ROOT" branch --show-current)"
if [ "$BRANCH" != "main" ]; then
  echo "Warning: You are on branch '$BRANCH', not 'main'."
  read -rp "Continue anyway? [y/N] " confirm
  if [ "$confirm" != "y" ] && [ "$confirm" != "Y" ]; then
    exit 1
  fi
fi

# Check tag doesn't already exist
if git -C "$ROOT" tag -l "$TAG" | grep -q "$TAG"; then
  echo "Error: Tag $TAG already exists."
  exit 1
fi

echo "Bumping version to $VERSION..."

# ── 1. package.json ──────────────────────────────────────────────
cd "$ROOT"
node -e "
  const fs = require('fs');
  const pkg = JSON.parse(fs.readFileSync('package.json', 'utf8'));
  pkg.version = '${VERSION}';
  fs.writeFileSync('package.json', JSON.stringify(pkg, null, 2) + '\n');
"
echo "  Updated package.json"

# ── 2. src-tauri/Cargo.toml ─────────────────────────────────────
sed -i.bak -E "0,/^version = \".*\"/s//version = \"${VERSION}\"/" src-tauri/Cargo.toml
rm -f src-tauri/Cargo.toml.bak
echo "  Updated src-tauri/Cargo.toml"

# ── 3. src-tauri/tauri.conf.json ────────────────────────────────
node -e "
  const fs = require('fs');
  const conf = JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json', 'utf8'));
  conf.version = '${VERSION}';
  fs.writeFileSync('src-tauri/tauri.conf.json', JSON.stringify(conf, null, 2) + '\n');
"
echo "  Updated src-tauri/tauri.conf.json"

# ── 4. Commit, tag, push ────────────────────────────────────────
git -C "$ROOT" add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json
git -C "$ROOT" commit -m "Release ${TAG}"
git -C "$ROOT" tag "$TAG"

echo ""
echo "Version bumped and tagged as $TAG."
read -rp "Push to origin? [Y/n] " push_confirm
if [ "$push_confirm" = "n" ] || [ "$push_confirm" = "N" ]; then
  echo "Skipped push. Run manually:"
  echo "  git push origin main --tags"
  exit 0
fi

git -C "$ROOT" push origin main --tags

echo ""
echo "Pushed $TAG. GitHub Actions will build the release."
echo "Monitor: https://github.com/Puru-Technologies/puru-nucleus/actions"
