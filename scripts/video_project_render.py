#!/usr/bin/env python3
"""Render one immutable Video Project Editor bundle with FFmpeg.

The service invokes this file through a verified file descriptor.  It is also
usable directly for deterministic integration tests:

    video_project_render.py canonical-edl.json assets-root output-directory
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


ASPECTS = {
    "9:16": (1080, 1920, "portrait"),
    "16:9": (1920, 1080, "landscape"),
    "1:1": (1080, 1080, "square"),
}
EFFECTS = {"grayscale", "vignette"}
TRANSITIONS = {"cross_dissolve"}
SAFE_NAME = re.compile(r"[^a-zA-Z0-9_.-]+")


class RenderError(RuntimeError):
    pass


def variant_key(index: int, variant: dict[str, Any]) -> str:
    language = SAFE_NAME.sub("-", str(variant["language"]).lower()).strip("-_.")
    language = language or "variant"
    aspect = {"9:16": "9x16", "16:9": "16x9", "1:1": "1x1"}[variant["aspect"]]
    return f"v{index:03d}-{language}-{aspect}"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def run(argv: list[str]) -> None:
    completed = subprocess.run(argv, check=False)
    if completed.returncode != 0:
        raise RenderError(
            f"renderer command failed with exit code {completed.returncode}"
        )


def ffmpeg_escape(value: str) -> str:
    return (
        value.replace("\\", "\\\\")
        .replace(":", "\\:")
        .replace("'", "\\'")
        .replace("[", "\\[")
        .replace("]", "\\]")
        .replace(",", "\\,")
    )


def safe_asset(root: Path, value: str) -> Path:
    relative = Path(value)
    if relative.is_absolute() or not relative.parts:
        raise RenderError("asset path must be relative")
    if relative.parts[0] == "assets":
        relative = Path(*relative.parts[1:])
    if not relative.parts or any(part in ("", ".", "..") for part in relative.parts):
        raise RenderError("asset path is not canonical")
    candidate = root.joinpath(relative)
    root_real = root.resolve(strict=True)
    candidate_real = candidate.resolve(strict=True)
    if candidate_real.parent != root_real and root_real not in candidate_real.parents:
        raise RenderError("asset escaped the assets root")
    if candidate.is_symlink() or not candidate_real.is_file():
        raise RenderError("asset must be a regular non-symlink file")
    return candidate_real


def validate_plan(plan: dict[str, Any]) -> None:
    if plan.get("schema_version") != 1:
        raise RenderError("unsupported canonical EDL schema")
    timeline = plan["timeline"]
    tracks = timeline["main_tracks"]
    if not tracks or any(not track["clips"] for track in tracks):
        raise RenderError("every main track must contain clips")
    boundaries = {
        track.get("name"): {
            round(clip["timeline_out"], 6) for clip in track["clips"][:-1]
        }
        for track in tracks
    }
    seen: set[tuple[str | None, float]] = set()
    for transition in timeline.get("transitions", []):
        if transition["kind"] not in TRANSITIONS:
            raise RenderError("unsupported transition")
        boundary = round(transition["timeline_time"], 6)
        track = transition.get("track")
        key = (track, boundary)
        if track not in boundaries or boundary not in boundaries[track] or key in seen:
            raise RenderError("transition must identify one clip boundary")
        seen.add(key)
    for overlay in timeline.get("overlay_tracks", []):
        if overlay["kind"] == "effect" and overlay["name"] not in EFFECTS:
            raise RenderError("unsupported effect")
    for variant in timeline.get("variants", []):
        if variant["aspect"] not in ASPECTS:
            raise RenderError("unsupported variant aspect")


def add_input(
    argv: list[str], path: Path, source_in: float | None = None
) -> int:
    index = sum(1 for item in argv if item == "-i")
    if source_in is not None:
        argv.extend(["-ss", f"{source_in:.6f}"])
    argv.extend(["-i", str(path)])
    return index


def source_path(plan: dict[str, Any], assets_root: Path, alias: str) -> Path:
    source = plan["sources"].get(alias)
    if not isinstance(source, dict):
        raise RenderError("canonical EDL references an unknown source")
    return safe_asset(assets_root, source["path"])


def build_master(
    plan: dict[str, Any], assets_root: Path, temporary: Path, ffmpeg: str
) -> None:
    canvas = plan["canvas"]
    width, height = int(canvas["width"]), int(canvas["height"])
    fps = float(canvas["frame_rate"])
    argv = [ffmpeg, "-hide_banner", "-nostdin", "-loglevel", "error"]
    filters: list[str] = []
    track_videos: list[str] = []
    track_audios: list[str] = []
    track_durations: list[float] = []
    for track_index, track in enumerate(plan["timeline"]["main_tracks"]):
        clips = track["clips"]
        video_labels: list[str] = []
        audio_labels: list[str] = []
        input_indices: list[int] = []
        durations: list[float] = []
        for position, clip in enumerate(clips):
            duration = float(clip["source_out"]) - float(clip["source_in"])
            if duration <= 0:
                raise RenderError("clip duration must be positive")
            durations.append(duration)
            source = plan["sources"][clip["source"]]
            input_index = add_input(
                argv,
                source_path(plan, assets_root, clip["source"]),
                float(clip["source_in"]),
            )
            input_indices.append(input_index)
            video = f"t{track_index}v{position}"
            audio = f"t{track_index}a{position}"
            filters.append(
                f"[{input_index}:v]trim=duration={duration:.6f},setpts=PTS-STARTPTS,"
                f"fps={fps:.6f},scale={width}:{height}:force_original_aspect_ratio=decrease,"
                f"pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:black,format=yuv420p[{video}]"
            )
            video_labels.append(video)
            if source.get("has_audio", False):
                filters.append(
                    f"[{input_index}:a]atrim=duration={duration:.6f},"
                    f"asetpts=PTS-STARTPTS,aresample=48000[{audio}]"
                )
            else:
                filters.append(
                    f"anullsrc=r=48000:cl=stereo,atrim=duration={duration:.6f},"
                    f"asetpts=PTS-STARTPTS[{audio}]"
                )
            audio_labels.append(audio)

        joined_video = f"track{track_index}basev"
        joined_audio = f"track{track_index}basea"
        if len(clips) == 1:
            filters.append(f"[{video_labels[0]}]null[{joined_video}]")
            filters.append(f"[{audio_labels[0]}]anull[{joined_audio}]")
        else:
            concat_inputs = "".join(
                f"[{video}][{audio}]"
                for video, audio in zip(video_labels, audio_labels)
            )
            filters.append(
                f"{concat_inputs}concat=n={len(clips)}:v=1:a=1"
                f"[{joined_video}][{joined_audio}]"
            )

        transitions = [
            item
            for item in plan["timeline"].get("transitions", [])
            if item.get("track") == track.get("name")
        ]
        current_video = joined_video
        current_audio = joined_audio
        for transition_index, transition in enumerate(transitions):
            boundary = float(transition["timeline_time"])
            duration = float(transition["duration"])
            clip_index = next(
                (
                    index
                    for index, clip in enumerate(clips[:-1])
                    if abs(float(clip["timeline_out"]) - boundary) <= 0.000001
                ),
                None,
            )
            if clip_index is None:
                raise RenderError("transition does not identify a clip boundary")
            if duration > durations[clip_index] or duration > durations[clip_index + 1]:
                raise RenderError("cross dissolve exceeds an adjacent clip")
            transition_video = f"track{track_index}transition{transition_index}"
            next_video = f"track{track_index}transitioned{transition_index}"
            next_audio = f"track{track_index}transitioneda{transition_index}"
            outgoing_start = durations[clip_index] - duration
            filters.append(
                f"[{input_indices[clip_index]}:v]trim=start={outgoing_start:.6f}:"
                f"duration={duration:.6f},setpts=PTS-STARTPTS+{boundary:.6f}/TB,"
                f"fps={fps:.6f},scale={width}:{height}:force_original_aspect_ratio=decrease,"
                f"pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:black,format=yuva420p,"
                f"fade=t=out:st=0:d={duration:.6f}:alpha=1[{transition_video}]"
            )
            filters.append(
                f"[{current_video}][{transition_video}]overlay=0:0:"
                f"enable='between(t,{boundary:.6f},{boundary + duration:.6f})'"
                f"[{next_video}]"
            )
            filters.append(
                f"[{current_audio}]afade=t=out:st={max(0.0, boundary - duration):.6f}:"
                f"d={duration:.6f},afade=t=in:st={boundary:.6f}:d={duration:.6f}"
                f"[{next_audio}]"
            )
            current_video, current_audio = next_video, next_audio
        track_videos.append(current_video)
        track_audios.append(current_audio)
        track_durations.append(sum(durations))

    if max(track_durations) - min(track_durations) > 0.000001:
        raise RenderError("main tracks do not have a common duration")
    current_v = track_videos[0]
    for track_index, video in enumerate(track_videos[1:], 1):
        next_video = f"maintrackoverlay{track_index}"
        filters.append(f"[{current_v}][{video}]overlay=0:0[{next_video}]")
        current_v = next_video
    if len(track_audios) == 1:
        current_a = track_audios[0]
    else:
        current_a = "maintrackaudio"
        inputs = "".join(f"[{audio}]" for audio in track_audios)
        filters.append(
            f"{inputs}amix=inputs={len(track_audios)}:duration=longest:normalize=1"
            f"[{current_a}]"
        )

    hold_sources = {
        (round(float(item["timeline_in"]), 6), round(float(item["timeline_out"]), 6)):
        item["source"]
        for item in plan.get("hold_sources", [])
    }
    for position, overlay in enumerate(plan["timeline"].get("overlay_tracks", [])):
        kind = overlay["kind"]
        start = float(overlay["timeline_in"] if kind in ("effect", "hold") else overlay["clip"]["timeline_in"])
        end = float(overlay["timeline_out"] if kind in ("effect", "hold") else overlay["clip"]["timeline_out"])
        next_v = f"overlayv{position}"
        if kind in ("broll", "pip"):
            clip = overlay["clip"]
            input_index = add_input(
                argv,
                source_path(plan, assets_root, clip["source"]),
                float(clip["source_in"]),
            )
            duration = float(clip["source_out"]) - float(clip["source_in"])
            if kind == "broll":
                geometry = (
                    f"scale={width}:{height}:force_original_aspect_ratio=decrease,"
                    f"pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:black"
                )
                placement = "0:0"
            else:
                pip_width = max(2, (width * 30 // 100) // 2 * 2)
                geometry = f"scale={pip_width}:-2"
                margin = max(16, width // 30)
                placement = f"W-w-{margin}:H-h-{margin}"
            filters.append(
                f"[{input_index}:v]trim=duration={duration:.6f},setpts=PTS-STARTPTS+"
                f"{start:.6f}/TB,{geometry},format=yuva420p[assetv{position}]"
            )
            filters.append(
                f"[{current_v}][assetv{position}]overlay={placement}:"
                f"enable='between(t,{start:.6f},{end:.6f})'[{next_v}]"
            )
        elif kind == "hold":
            alias = hold_sources.get((round(start, 6), round(end, 6)))
            if alias is None:
                raise RenderError("hold source mapping is missing")
            input_index = add_input(
                argv, source_path(plan, assets_root, alias), float(overlay["source_time"])
            )
            duration = end - start
            filters.append(
                f"[{input_index}:v]trim=end_frame=1,setpts=PTS-STARTPTS,"
                f"scale={width}:{height}:force_original_aspect_ratio=decrease,"
                f"pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:black,"
                f"tpad=stop_mode=clone:stop_duration={duration:.6f},"
                f"setpts=PTS+{start:.6f}/TB[assetv{position}]"
            )
            filters.append(
                f"[{current_v}][assetv{position}]overlay=0:0:"
                f"enable='between(t,{start:.6f},{end:.6f})'[{next_v}]"
            )
        elif kind == "effect":
            if overlay["name"] == "grayscale":
                effect = f"hue=s=0:enable='between(t,{start:.6f},{end:.6f})'"
            elif overlay["name"] == "vignette":
                effect = f"vignette=enable='between(t,{start:.6f},{end:.6f})'"
            else:
                raise RenderError("unsupported effect")
            filters.append(f"[{current_v}]{effect}[{next_v}]")
        else:
            raise RenderError("unsupported overlay kind")
        current_v = next_v

    argv.extend(
        [
            "-filter_complex",
            ";".join(filters),
            "-map",
            f"[{current_v}]",
            "-map",
            f"[{current_a}]",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "18",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
            "-map_metadata",
            "-1",
            "-y",
            str(temporary),
        ]
    )
    run(argv)


def build_variant(
    plan: dict[str, Any],
    variant: dict[str, Any],
    master: Path,
    assets_root: Path,
    temporary: Path,
    ffmpeg: str,
) -> None:
    width, height, _ = ASPECTS[variant["aspect"]]
    argv = [ffmpeg, "-hide_banner", "-nostdin", "-loglevel", "error", "-i", str(master)]
    filters = [
        f"[0:v]scale={width}:{height}:force_original_aspect_ratio=increase,"
        f"crop={width}:{height}[base]"
    ]
    current = "base"
    if variant.get("subtitles"):
        subtitle = safe_asset(assets_root, variant["subtitles"])
        next_label = "subbed"
        filters.append(
            f"[{current}]subtitles=filename='{ffmpeg_escape(str(subtitle))}'[{next_label}]"
        )
        current = next_label
    if variant.get("watermark"):
        watermark = safe_asset(assets_root, variant["watermark"])
        index = add_input(argv, watermark)
        margin = max(16, width // 40)
        filters.append(f"[{index}:v]scale={max(80, width // 6)}:-2[wm]")
        filters.append(
            f"[{current}][wm]overlay=W-w-{margin}:{margin}[watermarked]"
        )
        current = "watermarked"
    if variant.get("cta"):
        text = ffmpeg_escape(str(variant["cta"]))
        font_size = max(28, width // 24)
        filters.append(
            f"[{current}]drawbox=x=iw*0.1:y=ih*0.82:w=iw*0.8:h=ih*0.1:"
            f"color=black@0.65:t=fill,"
            f"drawtext=text='{text}':fontcolor=white:fontsize={font_size}:"
            f"x=(w-text_w)/2:y=h*0.85[cta]"
        )
        current = "cta"
    argv.extend(
        [
            "-filter_complex",
            ";".join(filters),
            "-map",
            f"[{current}]",
            "-map",
            "0:a?",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "20",
            "-c:a",
            "copy",
            "-movflags",
            "+faststart",
            "-map_metadata",
            "-1",
            "-y",
            str(temporary),
        ]
    )
    run(argv)


def publish(render: Any, destination: Path) -> None:
    if destination.exists():
        raise RenderError(f"refusing to overwrite output {destination.name}")
    destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd, temporary_name = tempfile.mkstemp(
        prefix=f".{destination.name}.", suffix=".tmp.mp4", dir=destination.parent
    )
    os.close(fd)
    temporary = Path(temporary_name)
    try:
        render(temporary)
        if not temporary.is_file() or temporary.stat().st_size == 0:
            raise RenderError("FFmpeg produced an empty output")
        with temporary.open("rb") as handle:
            os.fsync(handle.fileno())
        os.link(temporary, destination)
        os.unlink(temporary)
        directory_fd = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def publish_json(value: dict[str, Any], destination: Path) -> None:
    if destination.exists():
        raise RenderError(f"refusing to overwrite output {destination.name}")
    data = json.dumps(value, sort_keys=True, indent=2).encode("utf-8") + b"\n"
    fd, name = tempfile.mkstemp(prefix=".render-report.", dir=destination.parent)
    temporary = Path(name)
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        os.link(temporary, destination)
        temporary.unlink()
        directory_fd = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def render_outputs(
    plan: dict[str, Any],
    assets_root: Path,
    output_root: Path,
    ffmpeg: str,
) -> list[Path]:
    output_root.mkdir(mode=0o700, parents=True, exist_ok=False)
    master = output_root / "master.mp4"
    publish(
        lambda temporary: build_master(plan, assets_root, temporary, ffmpeg),
        master,
    )
    outputs = [master]
    for index, variant in enumerate(plan["timeline"].get("variants", []), 1):
        destination = output_root / f"{variant_key(index, variant)}.mp4"
        publish(
            lambda temporary, item=variant: build_variant(
                plan, item, master, assets_root, temporary, ffmpeg
            ),
            destination,
        )
        outputs.append(destination)
    return outputs


def canonical_mapping_sha256(mapping: dict[str, str]) -> str:
    data = json.dumps(
        mapping, sort_keys=True, separators=(",", ":"), ensure_ascii=True
    ).encode("ascii")
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--job-id", required=True)
    parser.add_argument("--project-id", required=True)
    parser.add_argument("--revision", required=True, type=int)
    parser.add_argument("--document-sha256", required=True)
    parser.add_argument("--replay-bundle-sha256", required=True)
    parser.add_argument("--replay-output", required=True, type=Path)
    parser.add_argument("canonical_edl", type=Path)
    parser.add_argument("assets_root", type=Path)
    parser.add_argument("output_dir", type=Path)
    parser.add_argument("--ffmpeg", default="/usr/bin/ffmpeg")
    args = parser.parse_args()

    plan = json.loads(args.canonical_edl.read_text(encoding="utf-8"))
    validate_plan(plan)
    assets_root = args.assets_root.resolve(strict=True)
    if not assets_root.is_dir() or args.assets_root.is_symlink():
        raise RenderError("assets root must be a non-symlink directory")
    if args.output_dir.exists() or args.output_dir.is_symlink():
        raise RenderError("output directory must not be a symlink")
    if args.replay_output.exists() or args.replay_output.is_symlink():
        raise RenderError("replay output directory already exists or is a symlink")
    if (
        plan["project"]["id"] != args.project_id
        or int(plan["project"]["revision"]) != args.revision
        or plan["project"]["document_sha256"] != args.document_sha256
        or sha256_file(args.canonical_edl) != args.replay_bundle_sha256
    ):
        raise RenderError("trusted job binding does not match canonical EDL")

    outputs = render_outputs(plan, assets_root, args.output_dir, args.ffmpeg)
    replay_outputs = render_outputs(
        plan, assets_root, args.replay_output, args.ffmpeg
    )
    output_root = args.output_dir.resolve(strict=True)
    replay_root = args.replay_output.resolve(strict=True)
    primary_by_name = {path.name: sha256_file(path) for path in outputs}
    replay_by_name = {path.name: sha256_file(path) for path in replay_outputs}
    if primary_by_name != replay_by_name:
        raise RenderError("deterministic replay output hashes differ")
    project_outputs = {
        f"exports/{args.job_id}/{name}": digest
        for name, digest in primary_by_name.items()
    }
    variant_outputs = {}
    for index, variant in enumerate(plan["timeline"]["variants"]):
        key = variant_key(index + 1, variant)
        relative = f"exports/{args.job_id}/{key}.mp4"
        variant_outputs[key] = {
            "index": index,
            "language": variant["language"],
            "aspect": variant["aspect"],
            "watermark": variant.get("watermark"),
            "cta": variant.get("cta"),
            "output_relative": relative,
            "sha256": project_outputs[relative],
        }

    identities = [
        {
            "sha256": item["sha256"],
            "size": item["size"],
        }
        for item in plan["asset_identities"]
    ]
    report = {
        "schema_version": 2,
        "job_id": args.job_id,
        "project_id": args.project_id,
        "revision": args.revision,
        "document_sha256": args.document_sha256,
        "replay_bundle_sha256": args.replay_bundle_sha256,
        "output_sha256": project_outputs,
        "canonical_output_sha256": canonical_mapping_sha256(project_outputs),
        "variants": variant_outputs,
        "project": {
            "id": args.project_id,
            "revision": args.revision,
            "document_sha256": args.document_sha256,
            "bundle_sha256": args.replay_bundle_sha256,
        },
        "inputs": identities,
        "outputs": [
            {
                "name": path.name,
                "sha256": sha256_file(path),
                "size": path.stat().st_size,
            }
            for path in outputs
        ],
        "replay": {
            "executed": True,
            "deterministic_executor": True,
            "job_id": args.job_id,
            "project_id": args.project_id,
            "revision": args.revision,
            "document_sha256": args.document_sha256,
            "replay_bundle_sha256": args.replay_bundle_sha256,
            "primary_sha256": primary_by_name,
            "replay_sha256": replay_by_name,
            "private_output_count": len(list(replay_root.iterdir())),
        },
    }
    publish_json(report, output_root / "render-report.json")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, KeyError, TypeError, ValueError, RenderError) as error:
        print(f"video project render failed: {error}", file=sys.stderr)
        raise SystemExit(1)
