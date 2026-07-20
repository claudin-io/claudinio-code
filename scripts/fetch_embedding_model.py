#!/usr/bin/env python3
"""Download the bundled embedding model into src-tauri/models/ with pinned
hashes, so `tauri build` ships semantic search 100% offline.

Run once before building:   python3 scripts/fetch_embedding_model.py
Idempotent: files whose sha256 already matches are skipped. CI runs this
right before `tauri build` (see .github/workflows/release.yml). Stdlib only —
no pip installs on any runner.
"""

import hashlib
import os
import sys
import time
import urllib.error
import urllib.request

REPO = "Xenova/all-MiniLM-L6-v2"
# Pinned commit of the HF repo — never `main`, so builds stay reproducible
# even if upstream re-encodes files. Update REVISION and FILES together.
REVISION = "751bff37182d3f1213fa05d7196b954e230abad9"
DEST = os.path.normpath(
    os.path.join(os.path.dirname(__file__), "..", "src-tauri", "models", "all-MiniLM-L6-v2")
)

# remote path in the repo -> (local filename, sha256, size in bytes)
FILES = {
    "onnx/model_quantized.onnx": (
        "model_quantized.onnx",
        "afdb6f1a0e45b715d0bb9b11772f032c399babd23bfc31fed1c170afc848bdb1",
        22_972_370,
    ),
    "tokenizer.json": (
        "tokenizer.json",
        "da0e79933b9ed51798a3ae27893d3c5fa4a201126cef75586296df9b4d2c62a0",
        711_661,
    ),
    "config.json": (
        "config.json",
        "7135149f7cffa1a573466c6e4d8423ed73b62fd2332c575bf738a0d033f70df7",
        650,
    ),
}

ATTEMPTS = 3
BACKOFF_SECONDS = [2, 4]
CHUNK = 1 << 20


def sha256_of(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(CHUNK), b""):
            h.update(chunk)
    return h.hexdigest()


def download(url: str, dest: str, expected_sha: str, expected_size: int) -> None:
    """Stream to dest+'.part' with an incremental hash, verify, atomic-rename."""
    part = dest + ".part"
    h = hashlib.sha256()
    size = 0
    with urllib.request.urlopen(url, timeout=120) as resp, open(part, "wb") as out:
        while True:
            chunk = resp.read(CHUNK)
            if not chunk:
                break
            h.update(chunk)
            size += len(chunk)
            out.write(chunk)
    if size != expected_size or h.hexdigest() != expected_sha:
        os.remove(part)
        raise ValueError(
            f"verification failed for {url}: got {size} bytes sha256 {h.hexdigest()}, "
            f"expected {expected_size} bytes sha256 {expected_sha}. If HuggingFace "
            f"re-encoded the file, update REVISION and the pinned hashes together."
        )
    os.replace(part, dest)


def main() -> int:
    os.makedirs(DEST, exist_ok=True)
    for remote, (local, sha, expected_size) in FILES.items():
        dest = os.path.join(DEST, local)
        if os.path.exists(dest) and sha256_of(dest) == sha:
            print(f"[fetch-model] {local}: up to date")
            continue
        url = f"https://huggingface.co/{REPO}/resolve/{REVISION}/{remote}"
        last_error: Exception | None = None
        for attempt in range(ATTEMPTS):
            try:
                print(f"[fetch-model] downloading {local} ({expected_size} bytes) ...")
                download(url, dest, sha, expected_size)
                print(f"[fetch-model] {local}: verified")
                last_error = None
                break
            except (urllib.error.URLError, OSError, ValueError) as e:
                last_error = e
                if attempt < ATTEMPTS - 1:
                    delay = BACKOFF_SECONDS[min(attempt, len(BACKOFF_SECONDS) - 1)]
                    print(f"[fetch-model] attempt {attempt + 1} failed ({e}); retrying in {delay}s")
                    time.sleep(delay)
        if last_error is not None:
            print(f"[fetch-model] FAILED to fetch {local}: {last_error}", file=sys.stderr)
            return 1
    print(f"[fetch-model] model ready at {DEST}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
