#!/usr/bin/env bash
set -euo pipefail

# ── Usage ────────────────────────────────────────────────────────
# ./scripts/release.sh            → interactive bump (patch/minor/major)
# ./scripts/release.sh 0.2.0     → explicit version
# ─────────────────────────────────────────────────────────────────

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Auto-install pre-push hook if missing
if [ ! -f "$ROOT/.git/hooks/pre-push" ]; then
  cp "$ROOT/scripts/pre-push" "$ROOT/.git/hooks/pre-push"
  echo "Installed pre-push hook (version mismatch guard)"
fi

# ── Read current version ─────────────────────────────────────────
CURRENT=$(node -p "require('./package.json').version")
IFS='.' read -r MAJOR MINOR PATCH <<< "${CURRENT%%-*}"

# ── Determine next version ───────────────────────────────────────
if [ -n "${1:-}" ]; then
  VERSION="${1#v}"
else
  NEXT_PATCH="$MAJOR.$MINOR.$((PATCH + 1))"
  NEXT_MINOR="$MAJOR.$((MINOR + 1)).0"
  NEXT_MAJOR="$((MAJOR + 1)).0.0"

  echo "Current version: $CURRENT"
  echo ""
  echo "  1) patch  → $NEXT_PATCH"
  echo "  2) minor  → $NEXT_MINOR"
  echo "  3) major  → $NEXT_MAJOR"
  echo "  4) beta   → ${NEXT_PATCH}-beta.1"
  echo ""
  read -rp "Select bump type [1-4]: " choice

  case "$choice" in
    1) VERSION="$NEXT_PATCH" ;;
    2) VERSION="$NEXT_MINOR" ;;
    3) VERSION="$NEXT_MAJOR" ;;
    4) VERSION="${NEXT_PATCH}-beta.1" ;;
    *) echo "Invalid choice"; exit 1 ;;
  esac
fi

# Validate semver-ish format
if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+'; then
  echo "Error: Version must be semver (e.g. 0.2.0 or 0.2.0-beta.1)"
  exit 1
fi

TAG="v${VERSION}"

# ── Pre-flight checks ───────────────────────────────────────────
if [ -n "$(git status --porcelain)" ]; then
  echo "Error: Working tree is not clean. Commit or stash changes first."
  exit 1
fi

BRANCH="$(git branch --show-current)"
if [ "$BRANCH" != "main" ]; then
  echo "Warning: You are on branch '$BRANCH', not 'main'."
  read -rp "Continue anyway? [y/N] " confirm
  if [ "$confirm" != "y" ] && [ "$confirm" != "Y" ]; then
    exit 1
  fi
fi

if git tag -l "$TAG" | grep -q "$TAG"; then
  echo "Error: Tag $TAG already exists."
  exit 1
fi

echo ""
echo "Releasing: $CURRENT → $VERSION"
echo ""

# ── Bump versions ────────────────────────────────────────────────
node -e "
  const fs = require('fs');
  const pkg = JSON.parse(fs.readFileSync('package.json', 'utf8'));
  pkg.version = '${VERSION}';
  fs.writeFileSync('package.json', JSON.stringify(pkg, null, 2) + '\n');
"
echo "  Updated package.json"

sed -i.bak -E "0,/^version = \".*\"/s//version = \"${VERSION}\"/" src-tauri/Cargo.toml
rm -f src-tauri/Cargo.toml.bak
echo "  Updated src-tauri/Cargo.toml"

node -e "
  const fs = require('fs');
  const conf = JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json', 'utf8'));
  conf.version = '${VERSION}';
  fs.writeFileSync('src-tauri/tauri.conf.json', JSON.stringify(conf, null, 2) + '\n');
"
echo "  Updated src-tauri/tauri.conf.json"

# ── Commit, tag, push ───────────────────────────────────────────
git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "Release ${TAG}"
git tag "$TAG"

echo ""
echo "Version bumped and tagged as $TAG."
read -rp "Push to origin? [Y/n] " push_confirm
if [ "$push_confirm" = "n" ] || [ "$push_confirm" = "N" ]; then
  echo "Skipped push. Run manually:"
  echo "  git push origin main --tags"
  exit 0
fi

git push origin main --tags

echo ""
echo "Pushed $TAG. GitHub Actions will build the release."
echo "Monitor: https://github.com/Puru-Technologies/puru-nucleus/actions"
