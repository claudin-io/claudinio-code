# MTP (Multi-Token Prediction) speculative decoding — MLX first, then llama.cpp

## Context

Neither engine emits a single speculative-decoding flag today. `grep -i "mtp|speculative|draft"` over `src/`, `src-tauri/` and `docs/` returns nothing, and `StartSpec` (`src-tauri/src/llama/supervisor.rs:194`) has no field for a drafter. Both engines, however, already carry the capability upstream — we simply never turn it on.

The user tests on macOS, so MLX ships first.

## Verified ground truth (checked live, not assumed)

**MLX — `mlx-swift-lm` @ `bd4b743` already ships MTP.** The sidecar depends on it and ignores it:

| File (`Libraries/MLXLMCommon/`) | Role |
|---|---|
| `MTPSpeculativeTokenIterator.swift` | `TokenIteratorProtocol`; drafts `blockSize - 1` tokens/round sharing the target's K/V |
| `MTPDrafterModel.swift` | drafter protocol + `MTPDrafterContext`/`Container`; the `mtp*` state keys |
| `MTPDrafterModelFactory.swift` | loads a drafter checkpoint; `MTPDrafterRegistry` |
| `SpeculativeDecoding.swift` | telemetry (`acceptanceRate`, rounds, accepted/proposed) |

**MTP is Gemma 4-only right now.** Only `MLXVLM/Models/Gemma4.swift` emits `mtpLastHiddenStatesKey` / `mtpSharedKVStatesKey`, and `MTPDrafterTypeRegistry` knows exactly one type, `gemma4_assistant`, registered from `MLXVLM/Gemma4AssistantRegistration.swift`. None of the models in `mlx_tiers.rs` qualify — so enabling MTP means adding Gemma 4 pairs to the catalog, otherwise there is nothing to switch it on for.

Repos confirmed live on the HF API (sizes are full-tree):

| Target (`model_type=gemma4`) | Size | Drafter (`model_type=gemma4_assistant`) | Size |
|---|---|---|---|
| `mlx-community/gemma-4-31b-it-4bit` | 18.44 GB | `mlx-community/gemma-4-31B-it-assistant-bf16` | 0.97 GB |
| `mlx-community/gemma-4-26b-a4b-it-4bit` | 15.37 GB | `mlx-community/gemma-4-26B-A4B-it-assistant-bf16` | 0.87 GB |

Both fit the test machine (M2 Max, 64 GB). The drafter is ~1 GB — cheap enough that pairing it with its target is not a meaningful download tax.

**llama.cpp — native, one flag.** `llama-server --help` (b10107 locally; we pin b10502, newer) exposes a whole *speculative params* section:

```
--spec-type none,draft-simple,draft-eagle3,draft-mtp,draft-dflash,ngram-*
--spec-draft-model, -md FNAME
--spec-draft-n-max N        (default: 3)
--spec-draft-ngl / --spec-draft-hf / --spec-draft-p-min ...
```

`draft-mtp` is a first-class mode. Nothing needs building — only argv and provisioning.

## Constraint that shapes the MLX design

`MTPSpeculativeTokenIterator.init` runs its own prefill inside `init` (`prepare(input:windowSize:)`), single-shot. Our cached path deliberately does *not*: `ModelHost.generate` chunks the prefill 4096 tokens at a time (`ModelHost.swift:293`) because a 30k-token prompt as one command buffer gets killed by the macOS watchdog as `Impacting Interactivity`, taking the process down mid-answer. It also pins a mid-prefill cache snapshot for `PrefixCache`.

Neither may be given up. The iterator's `mainCache:` parameter is the seam: keep our chunked prefill for every window but the last, then hand `MTPSpeculativeTokenIterator` the **already-warmed cache plus the final slice only**. That is byte-for-byte what `prefill(_:)` does today with `TokenIterator`, so pinning and chunking both survive.

Two guards on the same path:
- `canTrimPromptCache(cache)` must hold — the iterator throws otherwise. Fall back to `TokenIterator`.
- The iterator goes sticky-passthrough if the target stops emitting drafter state (e.g. KV quantization). Our `cacheable` gate already requires `kvBits == nil`, so this should not fire; report it in stats rather than hide it.

## Phases

### Phase 1 — sidecar (`mlx-sidecar`, separate repo) — **done, unreleased**

1. `ModelCompatibility`: `mtpPairing(target:drafter:)` — target `model_type` must be `gemma4`, drafter `gemma4_assistant`. A mismatched pair is refused at load with a message naming both, never silently degraded to non-speculative.
2. `main.swift`: `--draft-model <dir>` and `--draft-block-size N` (default 4, `>= 2`). Usage + README.
3. `ModelHost.load`: when a drafter dir is given, `await Gemma4AssistantRegistration.register()` then load through `MTPDrafterModelFactory.shared`.
4. `ModelHost.generate` cached path: loop over `any TokenIteratorProtocol`; build the MTP iterator from the warmed cache + final slice when a drafter is loaded and the cache is trimmable.
5. `generateUncached`: upstream's `generate(input:cache:parameters:context:mtpDrafter:blockSize:)`.
6. `/stats`: surface `acceptanceRate` and any `passthroughReason`. Without an acceptance rate, "MTP is on" and "MTP is helping" are indistinguishable, and tuning `blockSize` is guesswork.
7. Tests for 1; `bench` on a real Gemma 4 pair for the tok/s delta.

Note: the sidecar checkout has uncommitted work (cancel-in-flight-generation in `Server.swift`). Do not clobber it.

### Phase 2 — app, MLX side — **done except the tier entries (11)**

8. `StartSpec` gains `draft_model_path: Option<PathBuf>` + `draft_block_size: u32`; `build_mlx_args` emits the flags; supervisor tests assert both presence and absence.
9. Pairing table (target repo → drafter repo) next to `mlx_tiers.rs`; only the two verified pairs.
10. Provisioning downloads the drafter into its own directory, verified against the Hub's sha256 like every other weight.
11. Gemma 4 entries in the 32GB tier, flagged MTP-capable.
12. Settings toggle + indicator, off by default until the acceptance rate is measured.

### Phase 3 — app, llama.cpp side — **argv done (13); provisioning (14) not started**

13. `--spec-type draft-mtp`, `--spec-draft-model`, `--spec-draft-n-max` from the same `StartSpec` fields.
14. `hf.rs` learns a second GGUF per model — today `group_quants` resolves exactly one file, which is the real work here.

## Status at 2026-08-23

Phase 1 and 2 are implemented and green (`swift test` 37, `cargo test --lib` 698,
`vitest src/components` 491). Two things remain before a user can switch the
toggle on and have it do anything:

- **The sidecar is unreleased.** `mlx_pins.rs` still points at v0.1.5, which has
  no `--draft-model`. Cutting v0.1.6 and re-running `python3 scripts/pin_mlx.py`
  is what connects the two halves. Until then the app can be pointed at a local
  build by overwriting
  `~/Library/Application Support/claudinio-code/llama/mlx-v0.1.5/claudinio-mlx`.
- **No Gemma 4 in `mlx_tiers.rs`.** Nothing in the suggestion list has a drafter,
  so the toggle has nothing to act on until a pair is installed by repo name.

Phase 3 argv is written and tested but unreachable: no llama.cpp model can
resolve a drafter path until `hf.rs` learns to install a second GGUF.

## Measured on the target machine (M2 Max, 64 GB)

### Method, and two results that had to be thrown away

The first two comparisons were wrong in the same way: a cold case measured
against a warm one. Weights come off disk on first load, and Gemma 4 26B A4B is
MoE — its experts page in lazily *during generation*, not at load — so whichever
case ran second won.

- llama.cpp, first attempt: two servers resident at 16.5 GB each, contending.
  Read as "no gain".
- llama.cpp, second attempt: baseline first at 10.66 / 11.19 / 11.48 tok/s —
  monotonically rising, still warming — against a settled MTP run. Read as
  **+20%**. With a discarded warmup generation the baseline is **13.99 / 13.89**,
  so that +20% was mostly the warm-up curve.
- MLX Gemma 4, first attempt: baseline cold at 36.5, MTP warm at 50.0. Read as
  **+37%**. Interleaved, both warm, it reverses.

Anything below is interleaved, with a warmup generation discarded.

### MLX, external drafter (our sidecar) — MTP loses

`gemma-4-26b-a4b-it-4bit` + `gemma-4-26B-A4B-it-assistant-bf16`, `bench`, 3 rounds:

| | no MTP | MTP (block 4) |
|---|---|---|
| tok/s | 72.13 / 72.03 / 71.85 | 62.54 / 66.45 / 64.64 |

**~10% slower**, at 75–85% acceptance. Acceptance was never the problem: the
drafter's own forward pass plus the batched verify costs more than it saves on a
model with only 4B active parameters, which is already fast per token.

### llama.cpp, external drafter — a wash

`Qwen3.8-27B-UD-Q4_K_M` + the `MTP/mtp-*.gguf` the repo ships. Warm baseline
13.99 / 13.89 tok/s against ~13.3 for `--spec-draft-n-max 3`. `n-max 6` fell to
~8.6 despite drafting more, and `n-max 2` was no better: the verify pass is the
cost, not the drafting.

### MTPLX, native MTP head — 1.92x

[MTPLX](https://github.com/youssofal/MTPLX) (Apache-2.0, Python, `mtplx 2.9.1`)
uses the MTP head the checkpoint already carries. No second model. Its server
takes `generation_mode` per request, which finally allows an A/B **inside one
warm process** — no reload, no page-cache difference, nothing left to confound.

`Youssofal/Qwen3.8-27B-MTPLX-Optimized-Speed`, `temperature 0.6, top_p 0.95`,
alternating AR/MTP for 3 rounds:

| Round | AR | MTP |
|---|---|---|
| 1 | 12.06 | 26.27 |
| 2 | 13.98 | 26.35 |
| 3 | 13.08 | 22.55 |
| mean | **13.04** | **25.06** |

**1.92x.** Cross-check: that AR mean lands on llama.cpp's warm baseline for the
same model (13.99 / 13.89), which is what a trustworthy baseline should do.

`/metrics` at depth 3: `drafted_by_depth [87, 87, 87]`, `accepted_by_depth
[76, 67, 61]` — 87 / 77 / 70% — plus 27 residual-correction tokens.

### What the three rows say

The external-drafter approach does not pay on this machine, in either engine.
The native head does, by roughly 2x. The difference is the drafter's own forward
pass: MTPLX has no second model to run.

## Consequence for the plan

Phases 1–3 are implemented, tested and correct, and the toggle is exactly the
right shape — the measurements above are the argument for having shipped it off
by default. But on this hardware the feature as built earns nothing.

The open question is whether MLX support should move to MTPLX as a third
engine. It fits the supervisor's existing shape: OpenAI surface, `/health`,
`/metrics`, `--api-key` with `Authorization: Bearer`, `--port`, `--host`. It
contradicts the sidecar's founding constraint — "No Python. A single binary, so
the app never has to provision an interpreter on a user's machine" — and that
constraint was written before there was a 2x on the table.

## Phase 4 — MTPLX as a third engine (prototype, 2026-08-24)

`mtplx tune` on this M2 Max, `Youssofal/Qwen3.8-27B-MTPLX-Optimized-Speed`,
depths 1–3 (the head has three levels; 4+ is rejected):

| | tok/s | vs AR | acceptance by level |
|---|---|---|---|
| AR | 14.7 | 1.00x | — |
| D1 | 22.6 | 1.54x | 99.5% |
| D2 | 23.7 | 1.61x | 96.9 / 91.4% |
| **D3** | **33.0** | **2.24x** | 94.3 / 87.7 / 81.0% |

Depth 3 wins, and `draft_block_size: 4` already maps to it (`--depth` counts
levels, the block counts the bonus too).

Implemented: `Engine::Mtplx` (format `mlx`, so catalog and downloader are
shared), `Engine::for_model` — `for_format` can no longer decide alone now that
two engines read one format — `catalog::is_mtplx_model` keying off
`mtplx_mtp_contract` in the checkpoint's config, `build_mtplx_args`,
`read_mtplx_stats` against `/metrics`, and `LocalPrefs::mtplx_path`.

Verified against the real binary, not just unit tests: the argv the supervisor
generates starts `mtplx 2.9.1` and `/health` reports back `generation_mode: mtp`,
`depth: 3`, `api_key_required: true`. All four fields `read_mtplx_stats` reads
(`decode_tok_s`, `prefill_tok_s`, `completion_tokens`, `prompt_tokens`) exist on
`latest` and carry sane values.

### A bug this turned up

`drafter_path` never had its llama.cpp branch. An earlier `str.replace` missed
because `cargo fmt` had already split the function signature across lines, and
`replace` fails silently where `assert` would not have. Every existing test
covered `build_args`, which takes the resolved path as an *input* — so the suite
stayed green while the app could not resolve a GGUF drafter at all. Fixed, and
`drafter_path` now has tests of its own.

### Not done, and deliberately

`mtplx_path` is pointed at, not provisioned. Every other engine here is a pinned
archive verified against a sha256 before it runs; MTPLX is a Python package and
`pip install` resolves an unpinned dependency tree over the network. Wiring that
into `provision.rs` is a decision about what this app is willing to execute, and
it should be made on its own rather than arriving behind a 2.24x.
