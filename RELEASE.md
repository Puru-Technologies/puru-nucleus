# Release Guide

## Overview

Puru Nucleus uses a fully automated release pipeline. Push a version tag → GitHub Actions builds for all 3 platforms → creates a GitHub Release → publishes installers to GCS.

```
Developer                     GitHub Actions                         GCS
   │                              │                                   │
   │  git tag v0.2.0 && push     │                                   │
   ├─────────────────────────────►│                                   │
   │                              ├── validate version                │
   │                              ├── build Linux (DEB)               │
   │                              ├── build Windows (MSI)             │
   │                              ├── build macOS (DMG)               │
   │                              ├── create GitHub Release           │
   │                              ├── generate latest.json            │
   │                              ├──────────────────────────────────►│
   │                              │   upload artifacts + manifest     │
   │                              │                                   │
   Hospital ◄─────────────────────────────────────────────────────────┤
              checks latest.json, downloads installer                 │
```

## Quick Start

```bash
./scripts/release.sh 0.2.0
```

That's it. The script bumps version in all config files, commits, tags, and pushes.

## Prerequisites

### GitHub Secrets

| Secret | Description | How to set |
|--------|-------------|------------|
| `GCP_SA_KEY` | GCP service account JSON with `Storage Object Admin` on `puru-releases` bucket | `gh secret set GCP_SA_KEY < key.json` |
| `GITHUB_TOKEN` | Automatic — provided by GitHub Actions | Nothing to do |

### Tools Required (developer machine)

- `git`
- `node` (for version bumping in JSON files)
- `gh` CLI (optional, for checking workflow status)

## Creating a Release

### Using the release script (recommended)

```bash
# Stable release
./scripts/release.sh 0.2.0

# Beta release
./scripts/release.sh 0.3.0-beta.1
```

The script will:
1. Validate semver format
2. Check for clean working tree
3. Warn if not on `main` branch
4. Bump version in `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`
5. Commit as "Release v0.2.0"
6. Create git tag `v0.2.0`
7. Ask confirmation, then push to origin

### Manual release (if needed)

```bash
# 1. Edit version in all 3 files to match:
#    - package.json            →  "version": "0.2.0"
#    - src-tauri/Cargo.toml    →  version = "0.2.0"
#    - src-tauri/tauri.conf.json → "version": "0.2.0"

# 2. Commit and tag
git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "Release v0.2.0"
git tag v0.2.0

# 3. Push
git push origin main --tags
```

## Release Channels

| Tag format | Channel | GitHub Release | GCS path |
|------------|---------|----------------|----------|
| `v0.2.0` | `stable` | Full release | `gs://puru-releases/nucleus/stable/` |
| `v0.3.0-beta.1` | `beta` | Pre-release | `gs://puru-releases/nucleus/beta/` |
| `v0.3.0-alpha.1` | `beta` | Pre-release | `gs://puru-releases/nucleus/beta/` |
| `v0.3.0-rc.1` | `beta` | Pre-release | `gs://puru-releases/nucleus/beta/` |

## What Gets Built

| Platform | Runner | Artifact | Name format |
|----------|--------|----------|-------------|
| Linux | ubuntu-22.04 | DEB | `puru-nucleus-{VERSION}-amd64.deb` |
| Windows | windows-latest | MSI | `puru-nucleus-{VERSION}-x64.msi` |
| macOS | macos-latest | DMG (universal) | `puru-nucleus-{VERSION}-universal.dmg` |

The macOS build produces a **universal binary** (works on both Intel and Apple Silicon).

## What Gets Published

### GitHub Release
- All 3 installers attached as assets
- Auto-generated release notes from commit history
- Marked as pre-release for beta channel

### GCS (`gs://puru-releases/nucleus/{channel}/`)
- All 3 installers
- `latest.json` manifest with SHA256 checksums and file sizes
- Manifest has `Cache-Control: no-cache` so hospitals always get the latest version

### Manifest format (`latest.json`)

```json
{
  "version": "0.2.0",
  "release_date": "2026-03-04",
  "release_notes": "Release v0.2.0",
  "min_supported_version": "0.1.0",
  "platforms": {
    "windows": {
      "x64": {
        "msi": { "file": "puru-nucleus-0.2.0-x64.msi", "sha256": "abc...", "size_mb": 45.2 }
      }
    },
    "linux": {
      "amd64": {
        "deb": { "file": "puru-nucleus-0.2.0-amd64.deb", "sha256": "def...", "size_mb": 42.1 }
      }
    },
    "macos": {
      "universal": {
        "dmg": { "file": "puru-nucleus-0.2.0-universal.dmg", "sha256": "ghi...", "size_mb": 48.3 }
      }
    }
  }
}
```

This matches the `NucleusManifest` struct in `src-tauri/src/releases/mod.rs`.

## Monitoring a Release

```bash
# Watch the workflow run
gh run watch

# Or open in browser
gh run list --workflow=release.yml
```

Or visit: https://github.com/Puru-Technologies/puru-nucleus/actions

## Verifying a Release

```bash
# Check GitHub Release
gh release view v0.2.0

# Check GCS artifacts
gsutil ls -l gs://puru-releases/nucleus/stable/

# Check manifest
gsutil cat gs://puru-releases/nucleus/stable/latest.json | python3 -m json.tool

# Test from a running Nucleus instance
# The app checks latest.json on startup and in the Updates tab
```

## Troubleshooting

### Version mismatch error
The `validate-version` job failed — the tag version doesn't match one of the 3 config files. Delete the tag, fix versions, and re-tag:
```bash
git tag -d v0.2.0
git push origin :refs/tags/v0.2.0
# Fix the version, commit, then re-tag
```

### Build fails on one platform
Other platform builds are independent and will still complete. Fix the issue and create a new patch release.

### GCS upload fails
Check that the `GCP_SA_KEY` secret is set and the service account has `Storage Object Admin` on the `puru-releases` bucket:
```bash
gh secret list
```

### macOS DMG not found
The universal build outputs to `src-tauri/target/universal-apple-darwin/release/bundle/dmg/`. If this path changes in a Tauri update, the workflow will need updating.
