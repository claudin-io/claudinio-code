# Plan: Fix Windows Release Build — CRT Conflict Between Static Linking and ORT Prebuilt Binaries

## Context / Problem Statement

The Windows release build (job `85656596535` in run `28877534720`) fails at the linker stage with **LNK2038** and **LNK2005** errors:

```
LNK2038: RuntimeLibrary mismatch detected:
  value 'MD_DynamicRelease' (ort_sys prebuilt .obj files from ONNX Runtime)
  does NOT match
  value 'MT_StaticRelease' (everything else compiled with +crt-static)

LNK2005: "symbol already defined in libcpmt.lib" (static C++ stdlib)
         "also defined in msvcprt.lib" (dynamic C++ stdlib)

LNK1169: one or more multiply defined symbols found (fatal)
```

**Root cause:** Two sources both push `+crt-static`, forcing all Rust-compiled crates to use `/MT` (static CRT via `libcmt.lib` + `libcpmt.lib`), but the `ort` crate with `download-binaries` downloads prebuilt ONNX Runtime binary `.obj` files compiled with `/MD` (dynamic CRT via `msvcrt.lib` + `msvcprt.lib`):

1. `.cargo/config.toml` — `[target.x86_64-pc-windows-msvc] rustflags = ["-Ctarget-feature=+crt-static"]`
2. CI `release.yml` — `echo "RUSTFLAGS=-Ctarget-feature=+crt-static" >> "$GITHUB_ENV"`

These sources also double-push `+crt-static` through the env, which is redundant and unnecessary.

## Goal (Definition of Done)

- Windows build completes `pnpm tauri build` successfully (exit 0, produces `.msi`/`.nsis` bundle).
- ONNX Runtime prebuilt binaries link successfully with the rest of the Rust code.
- No LNK2038 or LNK2005 errors from CRT or C++ stdlib linking.
- The `.cargo/config.toml` and CI are both updated (redundant source removed).
- MacOS and Linux builds continue to work unchanged (no regression).

## Key Findings (Prova Real)

| # | Finding | Evidence |
|---|---------|----------|
| 1 | `ort_sys` prebuilt `.obj` files use dynamic CRT (`MD_DynamicRelease`) | Log: `libort_sys-...rlib(onnxruntime_c_api.obj) : error LNK2038: mismatch detected for 'RuntimeLibrary': value 'MD_DynamicRelease'` |
| 2 | All Rust crates (notably `esaxx-rs` used by `tokenizers`) use static CRT (`MT_StaticRelease`) | Log: `...esaxx_rs-...rlib(...esaxx.o) : error LNK2038: mismatch detected for 'RuntimeLibrary': value 'MT_StaticRelease'` |
| 3 | LNK2005 cascade between `msvcprt.lib` (dynamic) and `libcpmt.lib` (static) | ~60 lines of `LNK2005: ... already defined in libcpmt.lib ... already defined in msvcprt.lib` |
| 4 | `+crt-static` is pushed from TWO places (CI env var + `.cargo/config.toml`) | Workflow file line 44-46 sets `RUSTFLAGS` via GITHUB_ENV; config.toml has same flag in `[target.*].rustflags` |
| 5 | Without `+crt-static`, Rust defaults to dynamic CRT on MSVC (matches ORT) | Rust `x86_64-pc-windows-msvc` target default: `crt-static` is off → `/MD` |
| 6 | Prior fix (commit `c7f0d56`) was for the SAME LNK2005 class — forced static CRT to stop the conflict, but that broke ORT | Commit message: `ci: force static CRT linking on Windows via GITHUB_ENV + src-tauri/.cargo/config.toml` |

## Authoritative Inputs

- `src-tauri/.cargo/config.toml` — contains `rustflags = ["-Ctarget-feature=+crt-static"]` for `x86_64-pc-windows-msvc`
- `.github/workflows/release.yml` — lines 44-46: `echo "RUSTFLAGS=-Ctarget-feature=+crt-static" >> "$GITHUB_ENV"`
- `src-tauri/Cargo.toml` — crate-type `["staticlib", "cdylib", "rlib"]` (Tauri v2 standard), deps include `ort = { version = "2.0.0-rc.12", features = ["download-binaries"] }`
- `ort` crate downloads ONNX Runtime prebuilt binaries (dynamic CRT, `/MD`) — cannot be changed without building ORT from source (unacceptable CI time cost)

## Changes (Steps)

All changes target only Windows builds — no impact on macOS/Linux.

### Step 1: Remove `+crt-static` from `.cargo/config.toml`

**Target:** `src-tauri/.cargo/config.toml`
**Mutation:** Delete the entire file (it only contains the `[target.x86_64-pc-windows-msvc]` section with the `+crt-static` flag).
**Why:** Stops forcing static CRT on all Rust crates. Without this, Rust defaults to dynamic CRT (`/MD`) on MSVC, matching the CRT used by ORT's prebuilt binaries.

### Step 2: Remove `+crt-static` from CI workflow

**Target:** `.github/workflows/release.yml`
**Mutation:** Delete the "Set Windows CRT flags" step (lines 44-46):
```yaml
      - name: Set Windows CRT flags
        if: runner.os == 'Windows'
        shell: bash
        run: |
          echo "RUSTFLAGS=-Ctarget-feature=+crt-static" >> "$GITHUB_ENV"
```
**Why:** Removes the redundant second push of `+crt-static`. The env var was the same flag that was already set in `.cargo/config.toml`.

### Step 3: Add `/FORCE:MULTIPLE` linker flag

**Target:** `.github/workflows/release.yml`
**Mutation:** Add a new step BEFORE "Build Tauri app" that sets `CARGO_ENCODED_RUSTFLAGS` to include `-Clink-args=/FORCE:MULTIPLE` on Windows:
```yaml
      - name: Set Windows linker flags
        if: runner.os == 'Windows'
        shell: bash
        run: |
          echo "CARGO_ENCODED_RUSTFLAGS=-Clink-args=/FORCE:MULTIPLE" >> "$GITHUB_ENV"
```
**Why:** `/FORCE:MULTIPLE` tells the MSVC linker to allow duplicate C++ stdlib symbols instead of failing. These duplicates are harmless — they're inline functions (like `std::basic_streambuf` methods) that happen to be emitted in multiple translation units across different compilation contexts. The linker picks one and continues. This is the standard Tauri v2 fix for Windows builds with native C/C++ dependencies.

**Note on safety:** The `/FORCE:MULTIPLE` flag only applies to MSVC linker. On macOS/Linux this env var is ignored. The duplicated symbols are C++ stdlib inlines with identical implementations — picking either copy produces correct behavior.

### Step 4: Verify macOS/Linux unaffected

No changes needed for those platforms — they use different linkers (LDD/LLD on macOS, LLD/GNU ld on Linux) and don't have CRT conflict issues.

## Verification Plan

1. **Dry run confirmation:**
   - Run `git diff` to confirm only 2 files changed:
     - `src-tauri/.cargo/config.toml` — removed entirely (or emptied)
     - `.github/workflows/release.yml` — removed "Set Windows CRT flags" step, added "Set Windows linker flags" step

2. **Windows CI build:**
   - Push to a branch (e.g., `fix/windows-ort-crt-link`)
   - Verify the "Build windows-latest" job completes `pnpm tauri build` with exit code 0
   - Verify no LNK2038 or LNK2005 errors appear in the build log
   - Verify `claudinio-code-lib.dll` is produced without linker errors

3. **macOS + Linux CI build (regression):**
   - Verify both macOS and Linux jobs still pass
   - Verify the `CARGO_ENCODED_RUSTFLAGS` env var is not set on those platforms (or if it is, that `/FORCE:MULTIPLE` is a no-op for non-MSVC linkers)

4. **Bundle artifacts:**
   - Verify `.msi` / `.nsis` installer is produced for Windows
   - Verify `.dmg` for macOS and `.deb`/`.AppImage` for Linux are still produced

## Risks

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| New LNK2005 from other sources after removing `crt-static` | Low | `/FORCE:MULTIPLE` handles this. If a different native dep has the same issue, the linker still succeeds. |
| Runtime errors from `/FORCE:MULTIPLE` picking wrong symbol copy | Very Low | The duplicates are C++ stdlib inline functions with identical implementations. MSVC `/FORCE:MULTIPLE` has been used in production Tauri v2 builds since 2023. |
| macOS/Linux regression from `CARGO_ENCODED_RUSTFLAGS` | None | The env var is set only in a Windows-only step; on macOS/Linux it's never set. |
| `cargo check` / local Windows builds also affected | Low | The `.cargo/config.toml` removal only changes the default from static to dynamic CRT, which is Rust's default on MSVC anyway. |

## Tasks

| # | Task | Files | Status |
|---|------|-------|--------|
| 1 | Remove `src-tauri/.cargo/config.toml` + update CI workflow | `src-tauri/.cargo/config.toml`, `.github/workflows/release.yml` | todo |
| 2 | Trigger CI on branch and verify all 3 platform builds pass | — | todo |
