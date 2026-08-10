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
import os
import subprocess
import sys
import unicodedata
from pathlib import Path

import numpy as np


SAMPLE_RATE = 16_000
# Keep enough context for sentence timestamps while preventing long recordings
# from turning one FunASR request into a duration-sized memory allocation.
CHUNK_SECONDS = 60.0
# Keep one second of acoustic context on both sides of a chunk boundary. The
# merge step below assigns each timestamp to the first chunk that owns it.
CHUNK_OVERLAP_SECONDS = 1.0
# A shorter retry makes Paraformer token/timestamp alignment resilient to a
# pathological long chunk without allowing an unbounded number of retries.
MIN_RETRY_SECONDS = 5.0
HAN_RANGES = (
    (0x3400, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0xF900, 0xFAFF),
    (0x20000, 0x2A6DF),
    (0x2A700, 0x2B73F),
    (0x2B740, 0x2B81F),
    (0x2B820, 0x2CEAF),
    (0x2CEB0, 0x2EBEF),
    (0x2EBF0, 0x2EE5F),
    (0x2F800, 0x2FA1F),
    (0x30000, 0x3134F),
    (0x31350, 0x323AF),
)
HAN_NAME_PREFIXES = (
    "CJK UNIFIED IDEOGRAPH-",
    "CJK COMPATIBILITY IDEOGRAPH-",
)
WORD_JOINERS = frozenset(("_", "-"))
APOSTROPHES = frozenset(("'", "’"))


def _funclip_root() -> Path:
    configured = os.environ.get("VWA_FUNCLIP_ROOT")
    root = (
        Path(configured)
        if configured
        else Path(__file__).resolve().parents[1] / "vendor" / "FunClip"
    )
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

    return len(_tokenize(text))


def _is_han(character: str) -> bool:
    codepoint = ord(character)
    if not any(start <= codepoint <= end for start, end in HAN_RANGES):
        return False
    # The Rust regex property excludes unassigned code points inside the
    # Unicode blocks.  Names provide the same assigned-ideograph check in
    # Python while still covering every current Han extension range.
    return unicodedata.name(character, "").startswith(HAN_NAME_PREFIXES)


def _is_word_character(character: str) -> bool:
    return character.isalnum() or character in WORD_JOINERS


def _tokenize(text: str) -> list[str]:
    """Mirror Rust's Han/letter/number token contract without ``\\p{}`` support."""

    return [token for token, _start, _end in _tokenize_with_spans(text)]


def _tokenize_with_spans(text: str) -> list[tuple[str, int, int]]:
    """Return Rust-compatible tokens together with their source spans."""

    tokens: list[tuple[str, int, int]] = []
    index = 0
    while index < len(text):
        character = text[index]
        if _is_han(character):
            tokens.append((character, index, index + 1))
            index += 1
            continue
        if not _is_word_character(character):
            index += 1
            continue
        start = index
        index += 1
        while index < len(text):
            character = text[index]
            if _is_word_character(character):
                index += 1
                continue
            if (
                character in APOSTROPHES
                and index + 1 < len(text)
                and _is_word_character(text[index + 1])
            ):
                index += 2
                continue
            break
        tokens.append((text[start:index], start, index))
    return tokens


def _select_chunk_tokens(
    raw_text: str,
    timestamps: list[list[float]],
    lower_bound_ms: float,
    upper_bound_ms: float,
) -> tuple[str, list[list[float]]]:
    """Select the first-owner words for one logical chunk.

    Recognition windows overlap so FunASR can see speech at a boundary.  Only
    words whose starts belong to the logical chunk are published; this keeps
    the overlap out of the final wire payload while retaining the surrounding
    acoustic context during recognition.
    """

    token_spans = _tokenize_with_spans(raw_text)
    if len(token_spans) != len(timestamps):
        raise RuntimeError(
            "FunClip token/timestamp cardinality mismatch while merging chunks: "
            f"{len(token_spans)} tokens, {len(timestamps)} timestamps"
        )
    selected: list[tuple[int, list[float]]] = []
    for index, (_token, _start, _end) in enumerate(token_spans):
        timestamp = timestamps[index]
        if not isinstance(timestamp, (list, tuple)) or len(timestamp) != 2:
            continue
        try:
            start_ms = float(timestamp[0])
            end_ms = float(timestamp[1])
        except (TypeError, ValueError):
            continue
        if not np.isfinite(start_ms) or not np.isfinite(end_ms) or end_ms <= start_ms:
            continue
        if start_ms < lower_bound_ms:
            continue
        if start_ms >= upper_bound_ms:
            break
        selected.append((index, [start_ms, end_ms]))
    if not selected:
        return "", []
    first_index = selected[0][0]
    last_index = selected[-1][0]
    first_start = token_spans[first_index][1]
    last_end = token_spans[last_index][2]
    return raw_text[first_start:last_end], [item[1] for item in selected]


def _append_transcript_part(parts: list[str], text: str) -> None:
    """Join chunk text without inserting spaces between adjacent Han tokens."""

    if not text:
        return
    if not parts:
        parts.append(text)
        return
    previous = parts[-1]
    if previous and _is_han(previous[-1]) and _is_han(text[0]):
        parts.append(text)
    else:
        parts.append(" " + text)


def _sentence_bounds(sentence: dict) -> tuple[float, float]:
    timestamps = sentence.get("timestamp") or []
    if not timestamps:
        raise RuntimeError("FunClip produced a sentence without timestamps")
    return float(timestamps[0][0]), float(timestamps[-1][1])


def _sentence_tokens(sentence: dict) -> list[str]:
    text = sentence.get("text")
    if isinstance(text, list):
        return [str(token) for token in text]
    if isinstance(text, str):
        return _tokenize(text)
    return []


def _reconcile_overlapping_sentence(previous: dict, candidate: dict) -> dict:
    """Keep the complete sentence emitted by an overlapping context window.

    A long utterance can be truncated at the right edge of one window and be
    returned in full by the next window.  Prefer that later complete result
    when it has the same (or earlier) start.  If model segmentation shifts the
    start forward, append only the candidate suffix so the earlier prefix is
    retained instead of being dropped.
    """

    previous_start, previous_end = _sentence_bounds(previous)
    candidate_start, candidate_end = _sentence_bounds(candidate)
    if candidate_end <= previous_end:
        return previous
    if candidate_start <= previous_start + 250.0:
        return candidate

    previous_timestamps = [list(item) for item in previous["timestamp"]]
    candidate_timestamps = candidate["timestamp"]
    previous_tokens = _sentence_tokens(previous)
    candidate_tokens = _sentence_tokens(candidate)
    if len(previous_tokens) != len(previous_timestamps) or len(candidate_tokens) != len(
        candidate_timestamps
    ):
        return candidate

    merged_tokens = list(previous_tokens)
    merged_timestamps = previous_timestamps
    merged_end = previous_end
    for token, timestamp in zip(candidate_tokens, candidate_timestamps):
        start, end = float(timestamp[0]), float(timestamp[1])
        if end <= merged_end:
            continue
        if (
            merged_tokens
            and token == merged_tokens[-1]
            and start < merged_end
        ):
            merged_timestamps[-1][1] = max(merged_timestamps[-1][1], end)
            merged_end = merged_timestamps[-1][1]
            continue
        merged_tokens.append(token)
        merged_timestamps.append([max(start, merged_end), end])
        merged_end = end
    merged = dict(previous)
    merged["text"] = merged_tokens
    merged["timestamp"] = merged_timestamps
    return merged


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


def _recognize_once(clipper, samples: np.ndarray, model_name: str):
    """Run one FunASR request and distinguish an empty result explicitly."""

    if model_name != "paraformer":
        # SenseVoice has a different generate contract; let its implementation
        # raise if it returns a malformed result rather than swallowing an
        # IndexError from an unrelated code path.
        _text, _srt, state = clipper.recog((SAMPLE_RATE, samples))
        return state

    from funclip.videoclipper import _normalize_recognition_result
    from funclip.utils.trans_utils import convert_pcm_to_float

    data = convert_pcm_to_float(samples)
    result = clipper.funasr_model.generate(
        data,
        return_spk_res=False,
        sentence_timestamp=True,
        return_raw_text=True,
        is_final=True,
        hotword="",
        output_dir=None,
        pred_timestamp=clipper.lang == "en",
        en_post_proc=clipper.lang == "en",
        cache={},
    )
    if not result:
        return None
    if not isinstance(result[0], dict):
        raise RuntimeError("FunASR returned a non-object recognition result")
    _text, raw_text, timestamps, sentences = _normalize_recognition_result(result[0])
    return {
        "audio_input": (SAMPLE_RATE, data),
        "recog_res_raw": raw_text,
        "timestamp": timestamps,
        "sentences": sentences,
    }


def _recognize_chunk(clipper, samples: np.ndarray, start: float, duration: float, model_name: str):
    """Recognize one chunk, splitting it when Paraformer alignment is bad.

    Some FunASR outputs contain a token count that does not match the returned
    timestamps for a 60-second request.  Retrying the same samples in smaller
    windows preserves the transcript while keeping the strict wire contract.
    """

    state = _recognize_once(clipper, samples, model_name)
    if state is None:
        print(
            f"FunASR returned no recognition result for chunk "
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
    previous_sentence_end_ms = 0.0
    previous_word_end_ms = 0.0
    chunk_start = 0.0
    while chunk_start < duration:
        chunk_duration = min(CHUNK_SECONDS, duration - chunk_start)
        logical_start_ms = chunk_start * 1000.0
        logical_end_ms = (chunk_start + chunk_duration) * 1000.0
        decode_start = max(0.0, chunk_start - CHUNK_OVERLAP_SECONDS)
        decode_end = min(duration, chunk_start + chunk_duration + CHUNK_OVERLAP_SECONDS)
        decode_duration = decode_end - decode_start
        samples = _decode_chunk(path, decode_start, decode_duration)
        if samples.size == 0:
            break
        recognized = _recognize_chunk(
            clipper, samples, decode_start, decode_duration, model_name
        )
        for sub_start, _sub_duration, state in recognized:
            offset_ms = round(sub_start * 1000)
            for sentence in _shift_sentences(state.get("sentences"), offset_ms):
                sentence_start_ms, sentence_end_ms = _sentence_bounds(sentence)
                if sentence_start_ms >= logical_end_ms:
                    continue
                if sentence_start_ms < previous_sentence_end_ms:
                    if all_sentences:
                        reconciled = _reconcile_overlapping_sentence(
                            all_sentences[-1], sentence
                        )
                        if reconciled is not all_sentences[-1]:
                            all_sentences[-1] = reconciled
                            previous_sentence_end_ms = _sentence_bounds(reconciled)[1]
                    continue
                all_sentences.append(sentence)
                previous_sentence_end_ms = max(previous_sentence_end_ms, sentence_end_ms)
            raw_text = state.get("recog_res_raw") or ""
            chunk_timestamps = _shift_timestamps(state.get("timestamp"), offset_ms)
            if model_name == "paraformer" and raw_text:
                selected_text, selected_timestamps = _select_chunk_tokens(
                    str(raw_text),
                    chunk_timestamps,
                    max(logical_start_ms, previous_word_end_ms),
                    logical_end_ms,
                )
                if selected_timestamps:
                    _append_transcript_part(all_raw_text, selected_text)
                    all_timestamps.extend(selected_timestamps)
                    previous_word_end_ms = selected_timestamps[-1][1]
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
    (output_dir / "recog_res_raw").write_text("".join(all_raw_text), encoding="utf-8")
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
