#!/usr/bin/env python3
"""Fixed media worker for queued Video Work API analysis and cover jobs."""

from __future__ import annotations

import hashlib
import ast
import json
import math
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

FFMPEG = "/usr/bin/ffmpeg"
FFPROBE = "/usr/bin/ffprobe"
FONT = "/usr/share/fonts/TTF/DejaVuSans.ttf"


def run(argv: list[str], *, capture: bool = False) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        argv,
        check=True,
        text=True,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
    )


def probe(path: Path) -> dict:
    completed = run(
        [
            FFPROBE,
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_type",
            "-of",
            "json",
            str(path),
        ],
        capture=True,
    )
    value = json.loads(completed.stdout)
    duration = float(value["format"]["duration"])
    if not duration > 0:
        raise ValueError("media duration must be positive")
    return {
        "duration_seconds": duration,
        "has_audio": any(item.get("codec_type") == "audio" for item in value["streams"]),
        "has_video": any(item.get("codec_type") == "video" for item in value["streams"]),
    }


def timestamp(seconds: float) -> str:
    millis = round(max(0.0, seconds) * 1000)
    return (
        f"{millis // 3_600_000:02}:"
        f"{(millis // 60_000) % 60:02}:"
        f"{(millis // 1000) % 60:02}.{millis % 1000:03}"
    )


def hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_new(path: Path, text: str) -> None:
    with path.open("x", encoding="utf-8") as handle:
        handle.write(text)
        handle.flush()


def filter_path(path: Path) -> str:
    return (
        str(path)
        .replace("\\", "\\\\")
        .replace(":", "\\:")
        .replace("'", "\\'")
        .replace(",", "\\,")
        .replace("[", "\\[")
        .replace("]", "\\]")
    )


def fit_text(value: str, max_lines: int, maximum: int, minimum: int) -> tuple[str, int]:
    characters = list(value.strip())
    font_size = maximum
    while font_size > minimum:
        capacity = max(1, int(908 / (font_size * 0.55)))
        if len(characters) <= capacity * max_lines:
            break
        font_size -= 1
    capacity = max(1, int(908 / (font_size * 0.55)))
    lines = [
        "".join(characters[index : index + capacity])
        for index in range(0, len(characters), capacity)
    ]
    if len(lines) > max_lines:
        raise ValueError("cover text cannot fit the declared safe area")
    return "\n".join(lines), font_size


def current_text(segments: list[dict], seconds: float) -> str:
    for segment in segments:
        if segment["start_seconds"] <= seconds < segment["end_seconds"]:
            return segment["text"]
    return ""


def sample(request: dict, source: Path, output: Path, media: dict) -> dict:
    if not media["has_video"]:
        raise ValueError("analysis frame source has no video stream")
    count = request["max_frames"]
    width, height = request["resolution"]
    times = [media["duration_seconds"] * index / count for index in range(count)]
    frames = []
    for index, seconds in enumerate(times):
        name = f"analysis-frame-{index:03}.jpg"
        destination = output / name
        stamp = timestamp(seconds)
        asr = current_text(request["asr_segments"], seconds)
        metadata = stamp if not asr else f"{stamp}  {asr}"
        text_path = output / f".metadata-{index:03}.txt"
        write_new(text_path, metadata.replace("\r", " ").replace("\n", " "))
        vf = (
            f"scale={width}:{height}:force_original_aspect_ratio=decrease,"
            f"pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:black,setsar=1,"
            f"pad=iw:ih+52:0:0:white,"
            f"drawtext=fontfile={FONT}:textfile='{filter_path(text_path)}':"
            "expansion=none:fontcolor=black:fontsize=18:x=12:y=h-37"
        )
        run(
            [
                FFMPEG,
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                str(source),
                "-ss",
                f"{seconds:.6f}",
                "-frames:v",
                "1",
                "-vf",
                vf,
                "-q:v",
                "2",
                "-n",
                str(destination),
            ]
        )
        frames.append(
            {
                "index": index,
                "timestamp_seconds": seconds,
                "timestamp": stamp,
                "asr_text": asr,
                "metadata_text": metadata,
                "file": name,
                "sha256": hash_file(destination),
            }
        )
    return {
        "kind": "analysis_frames",
        "duration_seconds": media["duration_seconds"],
        "frames": frames,
        "capability": {"extraction": "available", "vlm_analysis": "not_provided"},
    }


def safe_trim_analysis(request: dict, source: Path, output: Path, media: dict) -> dict:
    if not media["has_audio"]:
        raise ValueError("safe trim analysis requires an audio stream")
    completed = run(
        [
            FFMPEG,
            "-nostdin",
            "-hide_banner",
            "-i",
            str(source),
            "-af",
            "silencedetect=noise=-35dB:d=0.08",
            "-f",
            "null",
            "-",
        ],
        capture=True,
    )
    starts = [float(item) for item in re.findall(r"silence_start: ([0-9.]+)", completed.stderr)]
    ends = [float(item) for item in re.findall(r"silence_end: ([0-9.]+)", completed.stderr)]
    if len(starts) > len(ends):
        ends.append(media["duration_seconds"])
    silent = [
        {"start": start, "end": min(end, media["duration_seconds"]), "breath": False}
        for start, end in zip(starts, ends)
        if end > start
    ]
    voiced = []
    cursor = 0.0
    for interval in silent:
        if interval["start"] > cursor:
            voiced.append({"start": cursor, "end": interval["start"], "breath": False})
        cursor = interval["end"]
    if cursor < media["duration_seconds"]:
        voiced.append(
            {"start": cursor, "end": media["duration_seconds"], "breath": False}
        )
    words = request.get("words") or []
    word_backend = "supplied_genuine_timestamps"
    if not words:
        funclip_root = os.environ.get("VWA_FUNCLIP_ROOT", "")
        if funclip_root:
            words = extract_funclip_words(source, output, Path(funclip_root))
            word_backend = "funclip_funasr_genuine_token_timestamps"
        else:
            word_backend = "unavailable_funclip_not_configured"
    return {
        "kind": "safe_trims",
        "duration_seconds": media["duration_seconds"],
        "silent_intervals": silent,
        "voiced_intervals": voiced,
        "words": words,
        "capability": {
            "vad_alignment": "ffmpeg_silencedetect",
            "word_level_asr_backend": word_backend,
            "segment_level_asr_backend": "funclip",
            "timestamp_interpolation": "not_performed",
        },
    }


def parse_funclip_words(raw_text: str, timestamp_literal: str) -> list[dict]:
    timestamps = ast.literal_eval(timestamp_literal)
    tokens = re.findall(r"[\u4e00-\u9fff]|[\w-]+", raw_text, flags=re.UNICODE)
    if not isinstance(timestamps, list) or len(tokens) != len(timestamps):
        raise ValueError("FunClip token/timestamp cardinality mismatch")
    result = []
    previous_end = 0.0
    for token, interval in zip(tokens, timestamps):
        if not isinstance(interval, (list, tuple)) or len(interval) != 2:
            raise ValueError("invalid FunClip token timestamp interval")
        start, end = float(interval[0]) / 1000.0, float(interval[1]) / 1000.0
        if (
            not start >= previous_end
            or not end > start
            or not math.isfinite(start)
            or not math.isfinite(end)
        ):
            raise ValueError("FunClip token timestamps are not finite and monotonic")
        result.append({"word": token, "start": start, "end": end})
        previous_end = end
    return result


def extract_funclip_words(source: Path, output: Path, root: Path) -> list[dict]:
    script = root / "funclip" / "videoclipper.py"
    if not script.is_file():
        raise ValueError("configured FunClip stage-1 helper is unavailable")
    python = os.environ.get("VWA_PYTHON", sys.executable)
    with tempfile.TemporaryDirectory(prefix=".funclip-", dir=output) as directory:
        state = Path(directory)
        run(
            [
                python,
                str(script),
                "--stage",
                "1",
                "--file",
                str(source),
                "--output_dir",
                str(state),
            ]
        )
        return parse_funclip_words(
            (state / "recog_res_raw").read_text(encoding="utf-8"),
            (state / "timestamp").read_text(encoding="utf-8"),
        )


def cover(request: dict, source: Path, output: Path, media: dict) -> dict:
    if not media["has_video"]:
        raise ValueError("cover source has no video stream")
    spec = request["spec"]
    moment = spec["frame_timestamp"]
    if moment >= media["duration_seconds"]:
        raise ValueError("cover frame_timestamp must be strictly before media EOF")
    stem = request["stem"]
    original = output / f"{stem}-cover-original.png"
    final = output / f"{stem}.jpg"
    title_file = output / ".title.txt"
    subtitle_file = output / ".subtitle.txt"
    max_title_lines = 2 if spec["layout_profile"] == "banner-card" else 3
    fitted_title, title_size = fit_text(
        spec["title"].replace("\r", " ").replace("\n", " "),
        max_title_lines,
        72,
        30,
    )
    fitted_subtitle, subtitle_size = fit_text(
        spec["subtitle"].replace("\r", " ").replace("\n", " "),
        2,
        40,
        14,
    )
    write_new(title_file, fitted_title)
    write_new(subtitle_file, fitted_subtitle)
    run(
        [
            FFMPEG,
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            str(source),
            "-ss",
            f"{moment:.6f}",
            "-frames:v",
            "1",
            "-vf",
            "scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920",
            "-n",
            str(original),
        ]
    )
    profiles = {
        "smoke-glass": ("0:1120:1080:570:black@0.58", 1180, "white", "#eeeeee"),
        "banner-card": ("70:1220:940:430:white@0.82", 1280, "black", "#222222"),
        "diagonal": ("0:1040:1080:590:#111111@0.70", 1110, "white", "#f4d35e"),
        "editorial-black-gold": ("60:1060:960:610:black@0.86", 1140, "#e6c66a", "white"),
    }
    box, title_y, title_color, subtitle_color = profiles[spec["layout_profile"]]
    filters = [
        f"drawbox={box}:t=fill",
        (
            f"drawtext=fontfile={FONT}:textfile='{filter_path(title_file)}':"
            f"expansion=none:fontcolor={title_color}:fontsize={title_size}:"
            f"line_spacing=12:x=86:y={title_y}:box=0"
        ),
    ]
    if spec["subtitle"]:
        filters.append(
            f"drawtext=fontfile={FONT}:textfile='{filter_path(subtitle_file)}':"
            f"expansion=none:fontcolor={subtitle_color}:fontsize={subtitle_size}:"
            f"line_spacing=8:x=86:y={title_y + 230}"
        )
    run(
        [
            FFMPEG,
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            str(original),
            "-frames:v",
            "1",
            "-vf",
            ",".join(filters),
            "-q:v",
            "2",
            "-n",
            str(final),
        ]
    )
    return {
        "kind": "cover",
        "duration_seconds": media["duration_seconds"],
        "frame_timestamp": moment,
        "layout_profile": spec["layout_profile"],
        "original_png": original.name,
        "final_jpg": final.name,
        "original_sha256": hash_file(original),
        "final_sha256": hash_file(final),
    }


def main() -> int:
    if len(sys.argv) != 4:
        raise SystemExit("usage: video_media_job.py REQUEST INPUT OUTPUT")
    request_path, source, output = map(Path, sys.argv[1:])
    request = json.loads(request_path.read_text(encoding="utf-8"))
    output.mkdir(mode=0o700)
    media = probe(source)
    kind = request["kind"]
    if kind == "analysis_frames":
        report = sample(request, source, output, media)
    elif kind == "safe_trims":
        report = safe_trim_analysis(request, source, output, media)
    elif kind == "cover":
        report = cover(request, source, output, media)
    else:
        raise ValueError("unsupported media job kind")
    write_new(output / "media-report.json", json.dumps(report, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
