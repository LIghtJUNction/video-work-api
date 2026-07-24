#!/usr/bin/env python3
"""Download the pinned MADLAD-400-3B-MT snapshot via the Hugging Face `hf` CLI.

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

# Official Google checkpoint (Apache-2.0). Also mirrored as jbochi/madlad400-3b-mt.
MODEL_ID = "google/madlad400-3b-mt"
INCLUDE_PATTERNS = [
    "config.json",
    "generation_config.json",
    "model.safetensors",
    "special_tokens_map.json",
    "spiece.model",
    "tokenizer.json",
    "tokenizer_config.json",
    "added_tokens.json",
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
    completed = subprocess.run(cmd, check=False)
    if completed.returncode != 0:
        return completed.returncode

    required = [
        "config.json",
        "model.safetensors",
        "spiece.model",
        "tokenizer_config.json",
    ]
    missing = [name for name in required if not (output / name).is_file()]
    if missing:
        print(
            "Download finished but required files are missing:",
            ", ".join(missing),
            file=sys.stderr,
        )
        return 1
    print(f"MADLAD model ready at {output}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
