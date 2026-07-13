---
name: deploy
description: Deploy claudinio-code app (Tauri desktop app) via GitHub Actions. Executes the full release pipeline: bump version in all required config files, run pre-deploy validation (tests + build), commit, tag, push, and monitor the CI release workflow. On failure, diagnose and fix errors and re-submit the tag. Use when the user says "deploy", "release", "make a release", "bump version", "tag v{version}", "submit tag", or "publish a new version" or "criar release".
version: 1.0.0
license: MIT
metadata:
  author: claudinio
  project: claudinio-code
---

# Deploy Skill — claudinio-code

Release pipeline for the [claudinio-code](https://github.com/claudin-io/claudinio-code) Tauri v2 desktop app.

## Pre-deploy Checklist

Progress:
- [ ] Step 1: Bump version in all config files
- [ ] Step 2: Sync Cargo.lock
- [ ] Step 3: Run tests
- [ ] Step 4: Build and verify
- [ ] Step 5: Commit the version bump
- [ ] Step 6: Create and push tag
- [ ] Step 7: Monitor GitHub Actions workflow
- [ ] Step 8: Verify release was created on GitHub

---

## Step-by-step

### 1. Check current state

```bash
git status                          # must be clean, on main
git log --oneline -5                # last commits
git tag -l | sort -V | tail -5      # existing tags
```

### 2. Bump version in all config files

Three files MUST be updated to the new version (replace X.Y.Z):

| File | Field to change |
|---|---|
| package.json | "version": "X.Y.Z" |
| src-tauri/Cargo.toml | version = "X.Y.Z" |
| src-tauri/tauri.conf.json | "version": "X.Y.Z" |

**Gotcha:** src-tauri/Cargo.lock contains inline version strings for the package. Run cargo check (or pnpm tauri build) AFTER bumping the .toml so Cargo auto-updates the lockfile. Without this step, CI's pnpm install --frozen-lockfile will fail because the lockfile is stale.

The APP_VERSION constant used in the UI is injected at build time via Vite's define from package.json, so it auto-updates. No frontend code changes needed.

### 3. Validation (MUST pass before tagging)

```bash
pnpm test                           # 625+ tests must pass
pnpm build                          # Build succeeds, Vite build <15s
```

### 4. Commit and tag

```bash
git add package.json src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tauri.conf.json
git commit -m "chore: bump version to X.Y.Z"
git tag -a vX.Y.Z -m "vX.Y.Z"
```

### 5. Push

```bash
git push origin main
git push origin vX.Y.Z
```

The second push triggers the GitHub Actions workflow .github/workflows/release.yml.

### 6. Monitor CI

```bash
gh run list --workflow=release.yml --limit=3
gh run view <run-id> --json status,conclusion
```

Expected CI duration: ~35 minutes (3 platforms: Windows, macOS ARM, Linux x64).

---

## Release Pipeline Architecture

The CI workflow runs in two phases:

1. Build (parallel): Builds Tauri app on 3 runners (windows-latest, macos-latest, ubuntu-24.04) and uploads artifacts.
2. Create Release (sequential, after all builds): Downloads artifacts, collects installer files (.msi, .dmg, .deb, .AppImage), generates release notes from git log, and publishes a GitHub Release.

## Common failures and fixes

### Failure: pnpm install --frozen-lockfile fails

Root cause: Cargo.lock was not regenerated after bumping Cargo.toml.

Fix: Run cargo check in src-tauri/ (or run pnpm tauri build locally), which will update Cargo.lock. Commit the updated lockfile.

### Failure: TAURI_SIGNING_PRIVATE_KEY secret missing

Root cause: The signing key is not in GitHub Secrets.

Fix: Add the private key and its password to GitHub repo secrets (Settings &rarr; Secrets and variables &rarr; Actions).

### Failure: Rust compilation error

Root cause: A code change that does not compile on one of the target platforms.

Fix: Run cargo check --target <target> locally for the failing target.

### Failure: Tag format mismatch

Root cause: The workflow triggers on v[0-9]+.[0-9]+.[0-9]+ or [0-9]+.[0-9]+.[0-9]+. Pre-release tags like v0.1.6-rc1 do NOT trigger.

Fix: Use the exact format: vX.Y.Z.

### Failure: Release created but no release notes or wrong content

Root cause: The git log in release notes generation uses git tag --sort=-v:refname for sorting. If the previous tag is not found, it shows the entire history.

Fix: Verify tags are pushed and sorted correctly.

### Failure: Version mismatch between files

Root cause: Forgetting to update one of the three version files.

Fix: Run grep -n "version" package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json to find stale references.

### Failure: Workflow queued but never starts

Root cause: GitHub runner quota exhausted or workflow file is malformed.

Fix: Check workflow validity with gh workflow view.

---

## After successful release

1. Verify the GitHub Release page at https://github.com/claudin-io/claudinio-code/releases
2. Confirm release has:
   - Correct version tag
   - Release notes with git log
   - Download artifacts for all 3 platforms (Windows .msi, macOS .dmg, Linux .deb/.AppImage)

---

## Force-redeploy (re-submitting a tag)

If the release fails mid-way, DO NOT re-use the same tag -- GitHub Releases will not overwrite.

Option A: Delete and recreate the tag (if release was not created yet):
```bash
git tag -d vX.Y.Z
git push origin :refs/tags/vX.Y.Z
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin vX.Y.Z
```

Option B: Bump to the next patch version and release that instead.
