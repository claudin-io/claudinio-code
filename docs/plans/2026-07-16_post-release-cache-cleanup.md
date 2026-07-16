# Post-Release Cache Cleanup

## Context

The release workflow (`.github/workflows/release.yml`) builds Claudinio Code for 5 platforms on every tag push to `claudin-io/claudinio-code`. Each run creates GitHub Actions cache entries from:
- `actions/setup-node` — pnpm store cache (~500MB+ each)
- `dtolnay/rust-toolchain` — Rust toolchain + cargo registry cache (~1GB+ each)
- `actions/cache` — potential indirect cache usage

These caches persist indefinitely after each release, accumulating storage. Since releases are infrequent (per version tag), cached build artifacts from the previous release are stale by the next one — the Rust dependency set, toolchain, and pnpm store may have changed. Keeping them wastes GitHub Actions cache quota without meaningful build speed benefit.

## Solution Design

**Strategy:** Add one cleanup step at the end of the existing `create-release` job to delete ALL GitHub Actions caches for the `claudin-io/claudinio-code` repo after the release artifacts have been published.

**Why `create-release` and not `build`:**
- The `build` job matrix has 5 parallel runners — each would race to delete caches while others still need them.
- The `create-release` job runs only once after all builds finish, so caches are no longer needed.
- It's the natural "release is done, now clean up" hook.

**Change:**

1. **`.github/workflows/release.yml`** — `create-release` job:
   - Add `actions: write` to the `permissions` block (required to delete caches via the API).
   - Add a step at the very end of the job (after `Create Release`) that runs `gh cache delete --all`.

The step uses the built-in `gh` CLI (pre-installed on GitHub Actions runners) with `GITHUB_TOKEN` which is automatically scoped to the current repository. Since the workflow runs on `claudin-io/claudinio-code` (the source repo), this deletes caches for that same repo — exactly what we want.

## Low-Level Design

### File: `.github/workflows/release.yml`

**Change 1 — Permissions (line ~87, current `permissions` block):**
```yaml
    permissions:
      contents: write
      actions: write    # needed for gh cache delete
```

**Change 2 — New step at end of `create-release` job (after `Create Release`):**
```yaml
      - name: Clear GitHub Actions caches
        shell: bash
        run: gh cache delete --all
        env:
          GH_TOKEN: ${{ github.token }}
```

### Data flow
1. Tag `vX.Y.Z` pushed → triggers `release.yml`
2. `build` job matrix runs (5 parallel platform builds) → populates caches
3. `create-release` job waits for all `build` jobs
4. `Create Release` step publishes artifacts to `claudin-io/claudinio-code-releases`
5. `Clear GitHub Actions caches` step runs `gh cache delete --all` targeting the current repo (`claudin-io/claudinio-code`)
6. All cache entries are deleted via the GitHub Cache API (authenticated via `GITHUB_TOKEN`)

### Integration wiring
- **Permission seam:** `actions: write` permission must be added to the job. Without it, `gh cache delete` returns a 403. Proof: workflow run will fail at step 5.
- **Token seam:** `${{ github.token }}` resolves to the `GITHUB_TOKEN` which is auto-scoped to `claudin-io/claudinio-code`. This is the same token used by the existing `Create Release` step for `contents: write` — the `actions: write` scope is additive.
- **Idempotency:** `gh cache delete --all` succeeds even if there are zero caches (returns exit 0 with a message).

### Risks
- **Cache loss during concurrent builds:** The cleanup only runs after all `build` jobs finish (`needs: build` and `create-release` is serial), so no race condition.
- **In-flight PR builds:** If a PR build was started before the release but finishes after, its cache is also deleted. Acceptable — PR builds will regenerate caches on next run.
- **`gh cache delete --all` missing on old runners:** `gh` is pre-installed on `ubuntu-22.04` (the runner OS for `create-release`). The `gh cache` subcommand was introduced in GitHub CLI 2.20.0+. Ubuntu 22.04 ships with a sufficiently recent version. If missing, the step would fail and the workflow would show an error — but this runner has been used successfully with `gh` for the release creation step.

### Non-goals
- Not adding cache cleanup to the `build` job itself (per-job cleanup would be wasteful and add complexity).
- Not selectively deleting only certain cache keys — full deletion is simpler and the user confirmed it.

## Tasks summary

1. **Add `actions: write` permission to `create-release` job** — enable cache deletion API access.
2. **Add cache cleanup step** — append `gh cache delete --all` step after `Create Release`.


## Implementation Log — 2026-07-16 23:21
**Summary:** Clear GitHub Actions caches after each release
**Changed files:** M	.github/workflows/release.yml
**Commits:** cc15dbe feat: clear GH Actions caches after release to free storage
**Journal:** Implemented post-release cache cleanup for claudin-io/claudinio-code repo. Two changes to .github/workflows/release.yml: (1) added `actions: write` permission to the create-release job, and (2) added a `gh cache delete --all` step at the end of that job. The key insight is to put cleanup in create-release (not build) since it runs once after all parallel builds finish, avoiding race conditions. Using `GH_TOKEN` env var (not `GITHUB_TOKEN`) because `gh` CLI expects `GH_TOKEN`. Merged straight to main per user instruction.

**Task journal:**
- Add `actions: write` permission to create-release job: Perm added: actions: write after contents: write
- Add cache cleanup step to create-release job: Step added after Create Release with gh cache delete --all
- Merge to main and push: Branch merged directly to main per user request; commit cc15dbe pushado para main
