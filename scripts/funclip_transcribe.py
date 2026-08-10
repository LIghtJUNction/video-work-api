#!/usr/bin/env python3
"""Bounded-memory FunClip stage-1 transcription for long recordings.

The upstream ``videoclipper.py`` loads a recording with ``librosa.load`` in a
single call.  That is convenient for short clips but makes memory usage grow
with the complete duration.  This helper keeps the same FunClip/FunASR
recognizer loaded once and feeds it fixed-size PCM chunks decoded by ffmpeg.
The output files intentionally use the same names and formats as FunClip
stage-1 so the Rust service keeps one parser and one response contract.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

import numpy as np


SAMPLE_RATE = 16_000
# Keep enough context for sentence timestamps while preventing long recordings
# from turning one FunASR request into a duration-sized memory allocation.
CHUNK_SECONDS = 60.0
# A shorter retry makes Paraformer token/timestamp alignment resilient to a
# pathological long chunk without allowing an unbounded number of retries.
MIN_RETRY_SECONDS = 5.0
TOKEN_RE = re.compile(r"[\u4e00-\u9fff]|[\w-]+(?:['’][\w-]+)*", re.UNICODE)


def _funclip_root() -> Path:
    root = Path(__file__).resolve().parents[1] / "vendor" / "FunClip"
    if not root.is_dir():
        raise RuntimeError(f"FunClip root is unavailable: {root}")
    # FunClip's vendored module imports ``utils`` as a top-level package when
    # run as a script, so retain both script-style and package-style paths.
    for import_root in (root, root / "funclip"):
        if str(import_root) not in sys.path:
            sys.path.insert(0, str(import_root))
    return root


def _duration_seconds(path: Path) -> float:
    completed = subprocess.run(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
            str(path),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip()[-2048:]
        raise RuntimeError(f"ffprobe could not read recording{(': ' + detail) if detail else ''}")
    try:
        duration = float(completed.stdout.strip())
    except ValueError as exc:
        raise RuntimeError("ffprobe returned no recording duration") from exc
    if not np.isfinite(duration) or duration <= 0:
        raise RuntimeError("recording duration is invalid")
    return duration


def _decode_chunk(path: Path, start: float, duration: float) -> np.ndarray:
    completed = subprocess.run(
        [
            "ffmpeg",
            "-nostdin",
            "-v",
            "error",
            "-ss",
            f"{start:.6f}",
            "-i",
            str(path),
            "-t",
            f"{duration:.6f}",
            "-vn",
            "-sn",
            "-dn",
            "-ac",
            "1",
            "-ar",
            str(SAMPLE_RATE),
            "-f",
            "f32le",
            "pipe:1",
        ],
        check=False,
        capture_output=True,
    )
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", errors="replace").strip()[-2048:]
        raise RuntimeError(f"ffmpeg could not decode recording{(': ' + detail) if detail else ''}")
    samples = np.frombuffer(completed.stdout, dtype=np.float32)
    if samples.size == 0:
        return samples
    return samples.copy()


def _shift_timestamps(timestamps, offset_ms: int) -> list[list[float]]:
    shifted: list[list[float]] = []
    for item in timestamps or []:
        if not isinstance(item, (list, tuple)) or len(item) != 2:
            continue
        start, end = item
        try:
            start = float(start) + offset_ms
            end = float(end) + offset_ms
        except (TypeError, ValueError):
            continue
        if np.isfinite(start) and np.isfinite(end) and end > start:
            shifted.append([start, end])
    return shifted


def _shift_sentences(sentences, offset_ms: int) -> list[dict]:
    shifted: list[dict] = []
    for sentence in sentences or []:
        if not isinstance(sentence, dict):
            continue
        timestamps = _shift_timestamps(sentence.get("timestamp"), offset_ms)
        if not timestamps:
            continue
        item = dict(sentence)
        item["timestamp"] = timestamps
        shifted.append(item)
    return shifted


def _token_count(text: str) -> int:
    """Match Rust's word parser before publishing any token timestamps."""

    return sum(1 for _ in TOKEN_RE.finditer(text))


def _validate_timeline(sentences: list[dict], timestamps: list[list[float]], duration: float) -> None:
    """Reject output that cannot be trusted as a chronological recording."""

    limit_ms = duration * 1000.0 + 250.0
    previous_end = 0.0
    for sentence in sentences:
        sentence_timestamps = sentence.get("timestamp") or []
        if not sentence_timestamps:
            raise RuntimeError("FunClip produced a sentence without timestamps")
        start = float(sentence_timestamps[0][0])
        end = float(sentence_timestamps[-1][1])
        if start < previous_end or end <= start or end > limit_ms:
            raise RuntimeError("FunClip produced a non-monotonic or out-of-range SRT timeline")
        previous_end = end

    previous_end = 0.0
    for start, end in timestamps:
        if start < previous_end or end <= start or end > limit_ms:
            raise RuntimeError("FunClip produced invalid word timestamps")
        previous_end = end


def _recognize_chunk(clipper, samples: np.ndarray, start: float, duration: float, model_name: str):
    """Recognize one chunk, splitting it when Paraformer alignment is bad.

    Some FunASR outputs contain a token count that does not match the returned
    timestamps for a 60-second request.  Retrying the same samples in smaller
    windows preserves the transcript while keeping the strict wire contract.
    """

    try:
        _text, _srt, state = clipper.recog((SAMPLE_RATE, samples))
    except IndexError as exc:
        # FunASR's Paraformer wrapper returns an empty list for a fully silent
        # chunk; FunClip then indexes ``rec_result[0]`` and leaks that as
        # ``IndexError``.  A long recording may legitimately contain such a
        # chunk, so treat only this known empty-result shape as silence.
        if str(exc) != "list index out of range":
            raise
        print(
            f"FunClip returned no recognition for chunk "
            f"{start:.3f}-{start + duration:.3f}s; skipping",
            file=sys.stderr,
        )
        return []

    raw_text = state.get("recog_res_raw") or ""
    timestamps = _shift_timestamps(state.get("timestamp"), 0)
    if model_name == "paraformer" and _token_count(str(raw_text)) != len(timestamps):
        if duration <= MIN_RETRY_SECONDS or samples.size < SAMPLE_RATE * MIN_RETRY_SECONDS:
            raise RuntimeError(
                "FunClip token/timestamp cardinality mismatch in recording chunk "
                f"{start:.3f}-{start + duration:.3f}s: "
                f"{_token_count(str(raw_text))} tokens, {len(timestamps)} timestamps"
            )
        midpoint = samples.size // 2
        left_samples = samples[:midpoint]
        right_samples = samples[midpoint:]
        left_duration = left_samples.size / SAMPLE_RATE
        right_start = start + left_duration
        print(
            f"FunClip token/timestamp mismatch in {start:.3f}-{start + duration:.3f}s; "
            f"retrying {start:.3f}-{right_start:.3f}s and {right_start:.3f}-"
            f"{right_start + right_samples.size / SAMPLE_RATE:.3f}s",
            file=sys.stderr,
        )
        return _recognize_chunk(
            clipper, left_samples, start, left_duration, model_name
        ) + _recognize_chunk(
            clipper, right_samples, right_start, right_samples.size / SAMPLE_RATE, model_name
        )
    return [(start, duration, state)]


def transcribe(path: Path, output_dir: Path, model_name: str) -> None:
    _funclip_root()
    from funclip.videoclipper import VideoClipper, create_asr_model
    from funclip.utils.subtitle_utils import generate_srt

    output_dir.mkdir(parents=True, exist_ok=True)
    duration = _duration_seconds(path)
    funasr_model = create_asr_model(model_name, "zh")
    clipper = VideoClipper(funasr_model, asr_model=model_name)
    clipper.lang = "zh" if model_name == "paraformer" else "multilingual"

    all_sentences: list[dict] = []
    all_raw_text: list[str] = []
    all_timestamps: list[list[float]] = []
    chunk_start = 0.0
    while chunk_start < duration:
        chunk_duration = min(CHUNK_SECONDS, duration - chunk_start)
        samples = _decode_chunk(path, chunk_start, chunk_duration)
        if samples.size == 0:
            break
        recognized = _recognize_chunk(
            clipper, samples, chunk_start, chunk_duration, model_name
        )
        for sub_start, _sub_duration, state in recognized:
            offset_ms = round(sub_start * 1000)
            all_sentences.extend(_shift_sentences(state.get("sentences"), offset_ms))
            raw_text = state.get("recog_res_raw") or ""
            chunk_timestamps = _shift_timestamps(state.get("timestamp"), offset_ms)
            if raw_text:
                all_raw_text.append(str(raw_text))
            all_timestamps.extend(chunk_timestamps)
        chunk_start += chunk_duration

    if not all_sentences:
        raise RuntimeError("FunClip produced no timestamped segments")
    _validate_timeline(
        all_sentences,
        all_timestamps if model_name == "paraformer" else [],
        duration,
    )
    srt = generate_srt(all_sentences)
    if not srt.strip():
        raise RuntimeError("FunClip produced an empty SRT file")
    (output_dir / "total.srt").write_text(srt, encoding="utf-8")
    (output_dir / "recog_res_raw").write_text(" ".join(all_raw_text), encoding="utf-8")
    (output_dir / "timestamp").write_text(
        json.dumps(all_timestamps, separators=(",", ":")), encoding="utf-8"
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--file", type=Path, required=True)
    parser.add_argument("--output_dir", type=Path, required=True)
    parser.add_argument(
        "--model", choices=("paraformer", "sensevoice"), default="paraformer"
    )
    args = parser.parse_args()
    transcribe(args.file, args.output_dir, args.model)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
