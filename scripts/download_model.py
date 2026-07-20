#!/usr/bin/env python3
"""Download the pinned, inference-only model snapshot."""

from __future__ import annotations

import argparse
from pathlib import Path


MODEL_ID = "FunAudioLLM/Fun-CosyVoice3-0.5B-2512"
REVISION = "29e01c4e8d000f4bcd70751be16fa94bf3d85a18"
ALLOW_PATTERNS = [
    "CosyVoice-BlankEN/**", "asset/**", "campplus.onnx", "config.json", "configuration.json",
    "cosyvoice3.yaml", "flow.pt", "hift.pt", "llm.pt", "speech_tokenizer_v3.onnx",
]


def main() -> int:
    arguments = argparse.ArgumentParser()
    arguments.add_argument("--output", required=True, type=Path)
    args = arguments.parse_args()
    from huggingface_hub import snapshot_download
    snapshot_download(repo_id=MODEL_ID, revision=REVISION,
                      local_dir=args.output, allow_patterns=ALLOW_PATTERNS)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
