#!/usr/bin/env python3
"""Zero-shot CosyVoice3 inference helper invoked by the Rust engine."""

from __future__ import annotations

import argparse
import sys
import threading
from pathlib import Path

PROMPT_PREFIX = "You are a helpful assistant.<|endofprompt|>"
_LOCK = threading.Lock()
_MODEL = None


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
    sys.modules["wetext"] = None
    for path in (cosyvoice_root, cosyvoice_root / "third_party" / "Matcha-TTS"):
        text = str(path)
        if text not in sys.path:
            sys.path.insert(0, text)
    from cosyvoice.cli.cosyvoice import AutoModel

    _MODEL = AutoModel(
        model_dir=str(model_dir),
        load_trt=False,
        load_vllm=False,
        fp16=torch.cuda.is_available(),
    )
    return _MODEL


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cosyvoice-root", type=Path, required=True)
    parser.add_argument("--model-dir", type=Path, required=True)
    parser.add_argument("--target-text", required=True)
    parser.add_argument("--prompt-text", required=True)
    parser.add_argument("--prompt-wav", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--speed", type=float, default=1.0)
    args = parser.parse_args()

    with _LOCK:
        model = load_model(args.cosyvoice_root, args.model_dir)
        import soundfile
        import torch
        from cosyvoice.utils.common import set_all_random_seed

        set_all_random_seed(42)
        chunks = []
        for item in model.inference_zero_shot(
            args.target_text,
            PROMPT_PREFIX + args.prompt_text,
            str(args.prompt_wav),
            stream=False,
            speed=args.speed,
            text_frontend=True,
        ):
            speech = item.get("tts_speech")
            if speech is not None:
                chunks.append(speech.detach().cpu())
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
