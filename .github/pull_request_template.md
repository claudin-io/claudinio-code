<!-- Keep it short. What changed, and why. -->

## What

## Why

Closes #

## Checks

- [ ] `pnpm test` and `pnpm exec tsc --noEmit` pass
- [ ] `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass (from `src-tauri/`)
- [ ] New behaviour has a test; bug fixes have a regression test
- [ ] User-facing strings go through `src/lib/locales/`
- [ ] Touches permissions, the `validate_path` guard, auth/signing or the release
      workflow — **called out below if so**

## Notes for the reviewer

<!-- Screenshots or a clip for UI changes. Rationale for non-obvious decisions. -->
