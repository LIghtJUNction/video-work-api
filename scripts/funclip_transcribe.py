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
    (0x2E80, 0x2E99),
    (0x2E9B, 0x2EF3),
    (0x2F00, 0x2FD5),
    (0x3005, 0x3005),
    (0x3007, 0x3007),
    (0x3021, 0x3029),
    (0x3038, 0x303B),
    (0x3400, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0xF900, 0xFA6D),
    (0xFA70, 0xFAD9),
    (0x16FE2, 0x16FE3),
    (0x16FF0, 0x16FF1),
    (0x20000, 0x2A6DF),
    (0x2A700, 0x2B739),
    (0x2B740, 0x2B81D),
    (0x2B820, 0x2CEA1),
    (0x2CEB0, 0x2EBE0),
    (0x2EBF0, 0x2EE5D),
    (0x2F800, 0x2FA1D),
    (0x30000, 0x3134A),
    (0x31350, 0x323AF),
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


def _validate_sentence_entries(sentences, context: str = "FunASR") -> None:
    """Reject malformed nested sentence entries instead of treating them as silence."""

    if sentences is None:
        return
    if not isinstance(sentences, (list, tuple)):
        raise RuntimeError(f"{context} returned non-list sentence information")
    previous_sentence_end = 0.0
    for index, sentence in enumerate(sentences):
        if not isinstance(sentence, dict):
            raise RuntimeError(f"{context} returned a non-object sentence at index {index}")
        raw_timestamps = sentence.get("timestamp")
        if not isinstance(raw_timestamps, (list, tuple)) or not raw_timestamps:
            raise RuntimeError(
                f"{context} returned a sentence without valid timestamps at index {index}"
            )
        try:
            _validate_timestamp_entries(
                raw_timestamps, f"{context} sentence {index}"
            )
        except RuntimeError as exc:
            raise RuntimeError(
                f"{context} returned malformed timestamps at sentence index {index}"
            ) from exc
        sentence_start = float(raw_timestamps[0][0])
        sentence_end = float(raw_timestamps[-1][1])
        if sentence_start < previous_sentence_end:
            raise RuntimeError(
                f"{context} returned out-of-order sentence at index {index}"
            )
        previous_sentence_end = sentence_end


def _validate_timestamp_entries(timestamps, context: str = "FunASR") -> None:
    """Validate the complete raw timestamp payload before any filtering."""

    if timestamps is None:
        return
    if not isinstance(timestamps, (list, tuple)):
        raise RuntimeError(f"{context} returned non-list timestamps")
    previous_end = 0.0
    for index, item in enumerate(timestamps):
        if not isinstance(item, (list, tuple)) or len(item) != 2:
            raise RuntimeError(f"{context} returned malformed timestamp at index {index}")
        try:
            start = float(item[0])
            end = float(item[1])
        except (TypeError, ValueError) as exc:
            raise RuntimeError(
                f"{context} returned malformed timestamp at index {index}"
            ) from exc
        if not np.isfinite(start) or not np.isfinite(end) or end <= start:
            raise RuntimeError(f"{context} returned malformed timestamp at index {index}")
        if start < previous_end:
            raise RuntimeError(
                f"{context} returned out-of-order timestamp at index {index}"
            )
        previous_end = end


def _shift_sentences(sentences, offset_ms: int) -> list[dict]:
    shifted: list[dict] = []
    _validate_sentence_entries(sentences)
    for sentence in sentences or []:
        timestamps = _shift_timestamps(sentence.get("timestamp"), offset_ms)
        item = dict(sentence)
        item["timestamp"] = timestamps
        shifted.append(item)
    return shifted


def _token_count(text: str) -> int:
    """Match Rust's word parser before publishing any token timestamps."""

    return len(_tokenize(text))


def _is_han(character: str) -> bool:
    codepoint = ord(character)
    # Keep this list byte-for-byte aligned with the Unicode Han property used
    # by Rust's regex crate, including script characters such as U+3007.
    return any(start <= codepoint <= end for start, end in HAN_RANGES)


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


def _deduplicate_chunk_tokens(
    previous_text: str,
    candidate_text: str,
    timestamps: list[list[float]],
    previous_end_ms: float,
    previous_tokens: list[str] | None = None,
) -> tuple[str, list[list[float]]]:
    """Remove only lexical overlap and clip genuine next words at the boundary."""

    candidate_spans = _tokenize_with_spans(candidate_text)
    if len(candidate_spans) != len(timestamps):
        raise RuntimeError(
            "FunClip token/timestamp cardinality mismatch while deduplicating chunks: "
            f"{len(candidate_spans)} tokens, {len(timestamps)} timestamps"
        )
    if previous_tokens is None:
        previous_tokens = _tokenize(previous_text)
    candidate_tokens = [token for token, _start, _end in candidate_spans]
    lexical_overlap = _longest_sentence_token_overlap(previous_tokens, candidate_tokens)
    temporal_overlap = 0
    for timestamp in timestamps:
        start_ms = float(timestamp[0])
        if start_ms >= previous_end_ms:
            break
        temporal_overlap += 1
    # Matching words after the previous logical end are a distinct repetition,
    # not acoustic overlap.  Restrict lexical de-duplication to the candidate
    # prefix that actually overlaps the prior word timeline.
    overlap_count = min(lexical_overlap, temporal_overlap)
    selected_indices: list[int] = []
    selected_timestamps: list[list[float]] = []
    cursor_ms = previous_end_ms
    for index in range(overlap_count, len(timestamps)):
        start_ms, end_ms = timestamps[index]
        start_ms = float(start_ms)
        end_ms = float(end_ms)
        token_duration_ms = max(end_ms - start_ms, 1.0)
        if start_ms < cursor_ms:
            # A distinct word can straddle—or, under small model jitter, fall
            # entirely before—the previous word's end.  Shift its interval
            # forward instead of dropping the recognized token.
            start_ms = cursor_ms
            if end_ms <= cursor_ms:
                end_ms = cursor_ms + token_duration_ms
        else:
            start_ms = max(start_ms, cursor_ms)
        if end_ms <= start_ms:
            end_ms = start_ms + token_duration_ms
        selected_indices.append(index)
        selected_timestamps.append([start_ms, end_ms])
        cursor_ms = end_ms
    if not selected_indices:
        return "", []
    selected_text = _sentence_suffix_text(
        candidate_text, overlap_count, selected_indices
    )
    if isinstance(selected_text, list):
        selected_text = _tokens_to_text(selected_text)
    return selected_text.lstrip(), selected_timestamps


def _append_transcript_part(parts: list[str], text: str) -> None:
    """Join chunk text without inserting spaces between adjacent Han tokens."""

    if not text:
        return
    if not parts:
        parts.append(text)
        return
    previous = parts[-1]
    previous_tokens = _tokenize(previous)
    next_tokens = _tokenize(text)
    if (
        previous_tokens
        and next_tokens
        and len(previous_tokens[-1]) == 1
        and len(next_tokens[0]) == 1
        and _is_han(previous_tokens[-1])
        and _is_han(next_tokens[0])
    ):
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


def _sentence_suffix_text(
    text, overlap_count: int, selected_indices: list[int] | None = None
):
    """Return candidate text after ``overlap_count`` lexical tokens.

    String sentence text carries punctuation and spacing which are not part of
    the FunClip token list.  Start a suffix at the end of the last overlapped
    token so that punctuation between the overlap and the new text remains in
    the published sentence.
    """

    if isinstance(text, list):
        if selected_indices is None:
            return [str(token) for token in text[overlap_count:]]
        return [str(text[index]) for index in selected_indices]
    if not isinstance(text, str):
        return ""
    spans = _tokenize_with_spans(text)
    if selected_indices is not None:
        if not selected_indices:
            return ""
        if any(
            index < 0 or index >= len(spans)
            for index in selected_indices
        ):
            return ""
        selected = set(selected_indices)
        first_index = selected_indices[0]
        last_index = selected_indices[-1]
        contiguous = selected_indices == list(range(first_index, last_index + 1))
        if first_index != overlap_count or not contiguous:
            # A skipped token means source punctuation can be attached to the
            # omitted word.  Render only the selected lexical tokens so the
            # sentence text cannot reintroduce an un-timestamped word.
            return _tokens_to_text([spans[index][0] for index in selected_indices])
        if overlap_count:
            if overlap_count > len(spans):
                return ""
            first_start = spans[overlap_count - 1][2]
        else:
            first_start = spans[first_index][1]

        # Stop before the first unselected token after the appended suffix, but
        # keep punctuation/spacing that belongs to the final selected token.
        last_end = len(text)
        for index in range(last_index + 1, len(spans)):
            if index not in selected:
                last_end = spans[index][1]
                break

        # Remove skipped lexical tokens from the retained source slice instead
        # of taking one contiguous span that could re-introduce their text.
        fragments: list[str] = []
        cursor = first_start
        for index, (_token, start, end) in enumerate(spans):
            if end <= first_start:
                continue
            if start >= last_end:
                break
            if index not in selected:
                fragments.append(text[cursor:start])
                cursor = end
        fragments.append(text[cursor:last_end])
        return "".join(fragments)
    if overlap_count <= 0:
        return text
    if overlap_count > len(spans):
        return ""
    return text[spans[overlap_count - 1][2] :]


def _append_sentence_text(
    previous_text,
    candidate_text,
    overlap_count: int,
    selected_indices: list[int] | None = None,
):
    """Append a reconciled candidate suffix without changing text format."""

    if isinstance(previous_text, list):
        suffix = _sentence_suffix_text(candidate_text, overlap_count, selected_indices)
        if isinstance(suffix, list):
            return [str(token) for token in previous_text] + suffix
        return [str(token) for token in previous_text] + _tokenize(suffix)
    if not isinstance(previous_text, str):
        return previous_text

    suffix = _sentence_suffix_text(candidate_text, overlap_count, selected_indices)
    if isinstance(suffix, list):
        suffix = _tokens_to_text(suffix)
    if not suffix:
        return previous_text

    base = previous_text.rstrip()
    suffix = suffix.lstrip() if suffix[0].isspace() else suffix
    if not suffix:
        return previous_text
    if suffix[0] in ",.!?;:，。！？；：、…)]}»”'’" and base.endswith(suffix[0]):
        suffix = suffix[1:].lstrip()
        if not suffix:
            return previous_text
    if suffix[0] in ",.!?;:，。！？；：、…)]}»”'’" or suffix[0].isspace():
        return base + suffix
    if base and _is_han(base[-1]) and _is_han(suffix[0]):
        return base + suffix
    return f"{base} {suffix}"


def _tokens_to_text(tokens: list[str]) -> str:
    """Render lexical tokens using FunClip's Han/word spacing convention."""

    rendered = ""
    previous_token = ""
    for token in tokens:
        if not token:
            continue
        adjacent_han = (
            len(previous_token) == 1
            and len(token) == 1
            and _is_han(previous_token)
            and _is_han(token)
        )
        if rendered and not adjacent_han:
            rendered += " "
        rendered += token
        previous_token = token
    return rendered


def _longest_sentence_token_overlap(
    previous_tokens: list[str], candidate_tokens: list[str]
) -> int:
    """Find the largest previous-suffix/candidate-prefix token overlap."""

    maximum = min(len(previous_tokens), len(candidate_tokens))
    for count in range(maximum, 0, -1):
        if previous_tokens[-count:] == candidate_tokens[:count]:
            return count
    return 0


def _reconcile_overlapping_sentence(previous: dict, candidate: dict) -> dict:
    """Keep the complete sentence emitted by an overlapping context window.

    A long utterance can be truncated at the right edge of one window and be
    returned in full by the next window.  Prefer that later complete result
    when it has the same (or earlier) start.  If model segmentation shifts the
    start forward, append only the candidate suffix so the earlier prefix is
    retained instead of being dropped.
    """

    _previous_start, previous_end = _sentence_bounds(previous)
    _candidate_start, candidate_end = _sentence_bounds(candidate)
    previous_tokens = _sentence_tokens(previous)
    candidate_tokens = _sentence_tokens(candidate)
    if candidate_end <= previous_end:
        return previous
    candidate_has_previous_prefix = (
        len(candidate_tokens) >= len(previous_tokens)
        and candidate_tokens[: len(previous_tokens)] == previous_tokens
    )
    previous_timestamps = [list(item) for item in previous["timestamp"]]
    candidate_timestamps = candidate["timestamp"]
    if len(previous_tokens) != len(previous_timestamps) or len(candidate_tokens) != len(
        candidate_timestamps
    ):
        # SenseVoice does not promise one timestamp per lexical token.  Keep
        # the trusted prior timing and append only the candidate's time tail;
        # the lexical suffix is merged by sentence text, not paired with
        # individual word timestamps.
        overlap_count = _longest_sentence_token_overlap(
            previous_tokens, candidate_tokens
        )
        merged_end = previous_end
        candidate_tail: list[list[float]] = []
        for timestamp in candidate_timestamps:
            start, end = float(timestamp[0]), float(timestamp[1])
            if end <= merged_end:
                continue
            start = max(start, merged_end)
            if end <= start:
                continue
            candidate_tail.append([start, end])
            merged_end = end
        if not candidate_tail:
            candidate_tail = [[previous_end, candidate_end]]
        merged = dict(previous)
        merged["text"] = _append_sentence_text(
            previous.get("text"), candidate.get("text"), overlap_count
        )
        merged["timestamp"] = previous_timestamps + candidate_tail
        return merged
    if candidate_has_previous_prefix and _candidate_start >= _previous_start:
        return candidate

    overlap_count = _longest_sentence_token_overlap(previous_tokens, candidate_tokens)
    merged_timestamps = previous_timestamps
    merged_end = previous_end
    previous_overlap_start = len(previous_tokens) - overlap_count
    appended_indices: list[int] = []
    for index, timestamp in enumerate(candidate_timestamps):
        start, end = float(timestamp[0]), float(timestamp[1])
        if index < overlap_count:
            previous_index = previous_overlap_start + index
            previous_token_start, previous_token_end = merged_timestamps[previous_index]
            if start > previous_token_start or end > previous_token_end:
                preceding_end = (
                    merged_timestamps[previous_index - 1][1]
                    if previous_index
                    else previous_token_start
                )
                merged_timestamps[previous_index] = [
                    max(start, preceding_end),
                    max(end, max(start, preceding_end)),
                ]
            merged_end = max(merged_end, merged_timestamps[previous_index][1])
            continue
        if end <= merged_end:
            continue
        merged_timestamps.append([max(start, merged_end), end])
        merged_end = end
        appended_indices.append(index)
    merged = dict(previous)
    merged["text"] = _append_sentence_text(
        previous.get("text"),
        candidate.get("text"),
        overlap_count,
        appended_indices,
    )
    merged["timestamp"] = merged_timestamps
    return merged


def _contiguous_index_runs(indices: list[int]) -> list[tuple[int, int]]:
    """Return inclusive contiguous runs from an already ordered index list."""

    if not indices:
        return []
    runs: list[tuple[int, int]] = []
    start = previous = indices[0]
    for index in indices[1:]:
        if index != previous + 1:
            runs.append((start, previous))
            start = index
        previous = index
    runs.append((start, previous))
    return runs


def _align_sentences_to_words(
    sentences: list[dict], raw_text: str, timestamps: list[list[float]]
) -> list[dict]:
    """Make SRT sentence text a lossless view of the published word timeline.

    FunASR can segment the same overlapping chunk differently from its raw
    token stream.  Keeping those two lists independent can duplicate one word
    in SRT while omitting another.  Assign every published word to the
    sentence interval it overlaps (or to a synthetic gap segment), then retain
    original punctuation only when its lexical tokens still match exactly.
    """

    if not timestamps:
        return sentences
    raw_tokens = _tokenize(raw_text)
    if len(raw_tokens) != len(timestamps):
        raise RuntimeError(
            "FunClip token/timestamp cardinality mismatch while aligning SRT: "
            f"{len(raw_tokens)} tokens, {len(timestamps)} timestamps"
        )

    intervals: list[tuple[float, float, int]] = []
    for index, sentence in enumerate(sentences):
        start, end = _sentence_bounds(sentence)
        if end > start:
            intervals.append((start, end, index))

    assignments: list[list[int]] = [[] for _ in sentences]
    unassigned: list[int] = []
    interval_cursor = 0
    active_intervals: list[tuple[float, float, int]] = []
    for word_index, timestamp in enumerate(timestamps):
        start, end = float(timestamp[0]), float(timestamp[1])
        while interval_cursor < len(intervals) and intervals[interval_cursor][0] < end:
            active_intervals.append(intervals[interval_cursor])
            interval_cursor += 1
        active_intervals = [
            item for item in active_intervals if item[1] > start
        ]
        best: tuple[float, float, int] | None = None
        for sentence_start, sentence_end, sentence_index in active_intervals:
            overlap = min(end, sentence_end) - max(start, sentence_start)
            if overlap > 0:
                candidate = (overlap, sentence_start, sentence_index)
                if best is None or (
                    candidate[0], -candidate[1], -candidate[2]
                ) > (best[0], -best[1], -best[2]):
                    best = candidate
        if best is None:
            unassigned.append(word_index)
            continue
        _overlap, _sentence_start, sentence_index = best
        assignments[sentence_index].append(word_index)

    events: list[tuple[int, dict]] = []

    def append_event(
        first_index: int,
        last_index: int,
        source_sentence: dict | None,
    ) -> None:
        indices = list(range(first_index, last_index + 1))
        tokens = raw_tokens[first_index : last_index + 1]
        item = dict(source_sentence) if source_sentence is not None else {}
        if source_sentence is not None and _sentence_tokens(source_sentence) == tokens:
            source_text = source_sentence.get("text")
            # FunClip's Text2SRT renderer concatenates a list item containing a
            # mixed-script token (`oppo`, `还`) without a separator.  Render
            # list-backed sentences through the same token-aware joiner used
            # for synthetic segments so SRT text cannot merge two word tokens.
            item["text"] = (
                _tokens_to_text(tokens) if isinstance(source_text, list) else source_text
            )
        else:
            item["text"] = _tokens_to_text(tokens)
        item["timestamp"] = [list(timestamps[index]) for index in indices]
        events.append((first_index, item))

    for sentence_index, assigned in enumerate(assignments):
        if not assigned:
            continue
        for first_index, last_index in _contiguous_index_runs(assigned):
            source = (
                sentences[sentence_index]
                if first_index == assigned[0] and last_index == assigned[-1]
                else None
            )
            append_event(first_index, last_index, source)

    for first_index, last_index in _contiguous_index_runs(unassigned):
        append_event(first_index, last_index, None)

    events.sort(key=lambda event: event[0])
    return [sentence for _first_index, sentence in events]


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
        _validate_sentence_entries(state.get("sentences"))
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
    _validate_sentence_entries(result[0].get("sentence_info"))
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
    _validate_timestamp_entries(state.get("timestamp"))
    timestamps = _shift_timestamps(state.get("timestamp"), 0)
    if model_name == "paraformer" and _token_count(str(raw_text)) != len(timestamps):
        if duration <= MIN_RETRY_SECONDS or samples.size < SAMPLE_RATE * MIN_RETRY_SECONDS:
            raise RuntimeError(
                "FunClip token/timestamp cardinality mismatch in recording chunk "
                f"{start:.3f}-{start + duration:.3f}s: "
                f"{_token_count(str(raw_text))} tokens, {len(timestamps)} timestamps"
            )
        midpoint = samples.size // 2
        overlap_samples = min(
            midpoint, int(round(SAMPLE_RATE * CHUNK_OVERLAP_SECONDS))
        )
        right_start_index = midpoint - overlap_samples
        left_end_index = min(samples.size, midpoint + overlap_samples)
        left_samples = samples[:left_end_index]
        right_samples = samples[right_start_index:]
        left_duration = left_samples.size / SAMPLE_RATE
        right_start = start + right_start_index / SAMPLE_RATE
        right_duration = right_samples.size / SAMPLE_RATE
        print(
            f"FunClip token/timestamp mismatch in {start:.3f}-{start + duration:.3f}s; "
            f"retrying {start:.3f}-{right_start:.3f}s and {right_start:.3f}-"
            f"{right_start + right_duration:.3f}s with overlap",
            file=sys.stderr,
        )
        return _recognize_chunk(
            clipper, left_samples, start, left_duration, model_name
        ) + _recognize_chunk(
            clipper, right_samples, right_start, right_duration, model_name
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
    all_raw_tokens: list[str] = []
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
            raise RuntimeError(
                "ffmpeg returned no audio samples for recording chunk "
                f"{decode_start:.3f}-{decode_end:.3f}s"
            )
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
                    logical_start_ms,
                    logical_end_ms,
                )
                selected_text, selected_timestamps = _deduplicate_chunk_tokens(
                    "",
                    selected_text,
                    selected_timestamps,
                    previous_word_end_ms,
                    previous_tokens=all_raw_tokens,
                )
                if selected_timestamps:
                    selected_tokens = _tokenize(selected_text)
                    if len(selected_tokens) != len(selected_timestamps):
                        raise RuntimeError(
                            "FunClip token/timestamp cardinality mismatch after chunk de-duplication: "
                            f"{len(selected_tokens)} tokens, {len(selected_timestamps)} timestamps"
                        )
                    _append_transcript_part(all_raw_text, selected_text)
                    all_raw_tokens.extend(selected_tokens)
                    all_timestamps.extend(selected_timestamps)
                    previous_word_end_ms = selected_timestamps[-1][1]
        chunk_start += chunk_duration

    if not all_sentences:
        raise RuntimeError("FunClip produced no timestamped segments")
    if model_name == "paraformer":
        all_sentences = _align_sentences_to_words(
            all_sentences, "".join(all_raw_text), all_timestamps
        )
        if not all_sentences:
            raise RuntimeError("FunClip produced no word-aligned SRT segments")
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
