#!/usr/bin/env python3
"""MADLAD-400-3B machine translation helper invoked by the Rust service.

Reads a JSON request from stdin and writes a JSON response to stdout:

  Input:  {"target_lang": "en", "texts": ["你好", "世界"]}
  Output: {"translations": ["Hello", "World"]}

Target language uses ISO 639 codes (en, ru, zh, …). MADLAD is instructed with
the standard ``<2xx>`` target token.
"""

from __future__ import annotations

import argparse
import json
import sys
import threading
from pathlib import Path

_LOCK = threading.Lock()
_MODEL = None
_TOKENIZER = None
_DEVICE = None


def load_model(model_dir: Path):
    global _MODEL, _TOKENIZER, _DEVICE
    if _MODEL is not None:
        return _MODEL, _TOKENIZER, _DEVICE
    if not model_dir.is_dir():
        raise SystemExit(f"MADLAD model directory missing: {model_dir}")

    import torch
    from transformers import T5ForConditionalGeneration, T5Tokenizer

    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    tokenizer = T5Tokenizer.from_pretrained(str(model_dir))
    model = T5ForConditionalGeneration.from_pretrained(str(model_dir))
    model = model.to(device)
    model.eval()
    _MODEL = model
    _TOKENIZER = tokenizer
    _DEVICE = device
    return _MODEL, _TOKENIZER, _DEVICE


def normalize_lang(code: str) -> str:
    value = (code or "").strip().lower().replace("_", "-")
    if not value:
        raise SystemExit("target_lang is required")
    # Accept BCP-47 primary tags; MADLAD uses bare ISO codes.
    primary = value.split("-", 1)[0]
    if not (2 <= len(primary) <= 3 and primary.isalpha()):
        raise SystemExit(f"invalid target_lang: {code!r}")
    return primary


def translate_batch(model, tokenizer, device, target_lang: str, texts: list[str]) -> list[str]:
    import torch

    if not texts:
        return []
    prompts = [f"<2{target_lang}> {text}" for text in texts]
    # Keep generation bounded; long SRT lines still fit typical T5 limits.
    encoded = tokenizer(
        prompts,
        return_tensors="pt",
        padding=True,
        truncation=True,
        max_length=512,
    )
    encoded = {key: value.to(device) for key, value in encoded.items()}
    with torch.no_grad():
        outputs = model.generate(
            **encoded,
            max_new_tokens=512,
            num_beams=4,
            early_stopping=True,
        )
    return [
        tokenizer.decode(seq, skip_special_tokens=True).strip() for seq in outputs
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model-dir", type=Path, required=True)
    parser.add_argument(
        "--request-file",
        type=Path,
        default=None,
        help="Optional path to JSON request; default reads stdin",
    )
    args = parser.parse_args()

    if args.request_file is not None:
        raw = args.request_file.read_text(encoding="utf-8")
    else:
        raw = sys.stdin.read()
    try:
        request = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise SystemExit(f"invalid JSON request: {exc}") from exc

    target_lang = normalize_lang(str(request.get("target_lang", "")))
    texts = request.get("texts")
    if not isinstance(texts, list):
        raise SystemExit("request.texts must be a JSON array of strings")
    clean: list[str] = []
    for index, item in enumerate(texts):
        if not isinstance(item, str):
            raise SystemExit(f"texts[{index}] must be a string")
        clean.append(item)

    with _LOCK:
        model, tokenizer, device = load_model(args.model_dir.expanduser().resolve())
        # Translate one-by-one for memory safety on long SRT batches; still one
        # process-level model load. Small batches stay batched.
        translations: list[str] = []
        batch_size = 8
        for start in range(0, len(clean), batch_size):
            chunk = clean[start : start + batch_size]
            translations.extend(
                translate_batch(model, tokenizer, device, target_lang, chunk)
            )

    json.dump({"translations": translations}, sys.stdout, ensure_ascii=False)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
