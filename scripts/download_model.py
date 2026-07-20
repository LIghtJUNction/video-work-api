#!/usr/bin/env python3
"""Download the pinned CosyVoice3 snapshot via the Hugging Face `hf` CLI.

Uses the standard Hub cache (~/.cache/huggingface/hub by default), so repeated
runs and other tools sharing that cache stay warm.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

MODEL_ID = "FunAudioLLM/Fun-CosyVoice3-0.5B-2512"
REVISION = "29e01c4e8d000f4bcd70751be16fa94bf3d85a18"
INCLUDE_PATTERNS = [
    "CosyVoice-BlankEN/**",
    "asset/**",
    "campplus.onnx",
    "config.json",
    "configuration.json",
    "cosyvoice3.yaml",
    "flow.pt",
    "hift.pt",
    "llm.pt",
    "speech_tokenizer_v3.onnx",
]


def find_hf() -> str:
    for name in ("hf", "huggingface-cli"):
        path = shutil.which(name)
        if path:
            return path
    raise SystemExit(
        "Hugging Face CLI not found. Install it (e.g. pacman -S python-huggingface-hub) "
        "so `hf` is on PATH, then retry."
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--cache-dir",
        type=Path,
        default=None,
        help="Optional HF cache dir (default: HF_HUB_CACHE / ~/.cache/huggingface/hub)",
    )
    args = parser.parse_args()

    output: Path = args.output.expanduser().resolve()
    output.mkdir(parents=True, exist_ok=True)

    hf = find_hf()
    cmd: list[str] = [
        hf,
        "download",
        MODEL_ID,
        "--repo-type",
        "model",
        "--revision",
        REVISION,
        "--local-dir",
        str(output),
    ]
    for pattern in INCLUDE_PATTERNS:
        cmd.extend(["--include", pattern])

    if args.cache_dir is not None:
        cmd.extend(["--cache-dir", str(args.cache_dir.expanduser())])
    elif cache := os.environ.get("HF_HUB_CACHE") or os.environ.get("HUGGINGFACE_HUB_CACHE"):
        cmd.extend(["--cache-dir", cache])

    print("Running:", " ".join(cmd), file=sys.stderr)
    # Inherit stdout/stderr so progress bars and cache hits are visible.
    completed = subprocess.run(cmd, check=False)
    if completed.returncode != 0:
        print(f"hf download failed with exit {completed.returncode}", file=sys.stderr)
        return completed.returncode

    # Sanity: pinned config must exist after a successful download / cache materialize.
    config = output / "config.json"
    if not config.is_file():
        print(f"Expected {config} after download; missing", file=sys.stderr)
        return 1

    print(f"Model ready at {output}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
