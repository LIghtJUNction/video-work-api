#!/usr/bin/env python3
"""CosyVoice3 zero-shot and cross-lingual inference helper for the Rust engine."""

from __future__ import annotations

import argparse
import importlib.metadata
import sys
import threading
import types
from pathlib import Path

PROMPT_PREFIX = "You are a helpful assistant.<|endofprompt|>"
_LOCK = threading.Lock()
_MODEL = None


def ensure_pkg_resources_compat() -> None:
    """Supply only Lightning's removed pkg_resources APIs when it is absent.

    setuptools 83 intentionally removed pkg_resources.  Keeping this shim local
    avoids reviving that legacy package or weakening the pinned runtime.
    """
    try:
        import pkg_resources  # noqa: F401
    except ModuleNotFoundError as exc:
        if exc.name != "pkg_resources":
            raise
        module = types.ModuleType("pkg_resources")
        module.declare_namespace = lambda _package_name: None

        def iter_entry_points(group: str, name: str | None = None):
            return iter(importlib.metadata.entry_points().select(group=group, name=name))

        def get_distribution(distribution_name: str):
            return importlib.metadata.distribution(distribution_name)

        module.iter_entry_points = iter_entry_points
        module.get_distribution = get_distribution
        sys.modules["pkg_resources"] = module


def load_model(cosyvoice_root: Path, model_dir: Path):
    global _MODEL
    if _MODEL is not None:
        return _MODEL
    if not cosyvoice_root.is_dir() or not model_dir.is_dir():
        raise SystemExit("CosyVoice source or model is not installed")
    import onnxruntime
    import torch

    if not getattr(onnxruntime, "_vwa_cpu_sessions", False):
        original = onnxruntime.InferenceSession

        def cpu_session(*args, **kwargs):
            kwargs["providers"] = ["CPUExecutionProvider"]
            return original(*args, **kwargs)

        onnxruntime.InferenceSession = cpu_session
        onnxruntime._vwa_cpu_sessions = True
    for path in (cosyvoice_root, cosyvoice_root / "third_party" / "Matcha-TTS"):
        text = str(path)
        if text not in sys.path:
            sys.path.insert(0, text)
    ensure_pkg_resources_compat()
    from cosyvoice.cli.cosyvoice import AutoModel

    _MODEL = AutoModel(
        model_dir=str(model_dir),
        load_trt=False,
        load_vllm=False,
        fp16=torch.cuda.is_available(),
    )
    return _MODEL


def inference_chunks(model, args):
    """Run the selected CosyVoice API without mixing in a foreign transcript."""
    if args.generation_mode == "zero_shot":
        results = model.inference_zero_shot(
            args.target_text,
            PROMPT_PREFIX + args.prompt_text,
            str(args.prompt_wav),
            stream=False,
            speed=args.speed,
            text_frontend=True,
        )
    else:
        results = model.inference_cross_lingual(
            PROMPT_PREFIX + args.target_text,
            str(args.prompt_wav),
            stream=False,
            speed=args.speed,
        )
    chunks = []
    for item in results:
        speech = item.get("tts_speech")
        if speech is not None:
            chunks.append(speech.detach().cpu())
    return chunks


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cosyvoice-root", type=Path, required=True)
    parser.add_argument("--model-dir", type=Path, required=True)
    parser.add_argument("--target-text", required=True)
    parser.add_argument("--prompt-text", required=True)
    parser.add_argument("--prompt-wav", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--speed", type=float, default=1.0)
    parser.add_argument(
        "--generation-mode",
        choices=("zero_shot", "cross_lingual"),
        default="zero_shot",
    )
    args = parser.parse_args()

    with _LOCK:
        model = load_model(args.cosyvoice_root, args.model_dir)
        import soundfile
        import torch
        from cosyvoice.utils.common import set_all_random_seed

        set_all_random_seed(42)
        chunks = inference_chunks(model, args)
        if not chunks:
            raise SystemExit("CosyVoice produced no audio")
        samples = torch.cat(chunks, dim=1).squeeze(0).numpy()
        soundfile.write(
            str(args.output),
            samples,
            int(model.sample_rate),
            subtype="PCM_16",
            format="WAV",
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
