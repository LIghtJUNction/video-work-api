use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::json;
use tempfile::tempdir;

fn tool_exists(path: &str) -> bool {
    Path::new(path).is_file()
}

fn make_clip(path: &Path, color: &str, frequency: &str) {
    let status = Command::new("/usr/bin/ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:size=160x90:rate=10"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency={frequency}:sample_rate=48000"),
            "-t",
            "0.8",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-y",
        ])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn renderer_outputs_master_and_portrait_variant_with_faststart() {
    if !tool_exists("/usr/bin/ffmpeg")
        || !tool_exists("/usr/bin/ffprobe")
        || !tool_exists("/usr/bin/python3")
    {
        eprintln!("skipping because FFmpeg or Python is unavailable");
        return;
    }
    let root = tempdir().unwrap();
    let assets = root.path().join("assets");
    let output = root.path().join("output");
    fs::create_dir(&assets).unwrap();
    make_clip(&assets.join("one.mp4"), "red", "440");
    make_clip(&assets.join("two.mp4"), "blue", "660");
    fs::write(
        assets.join("captions.ass"),
        "[Script Info]\nScriptType: v4.00+\nPlayResX: 1080\nPlayResY: 1920\n\
         [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, \
         OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, \
         Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, \
         MarginV, Encoding\nStyle: Default,DejaVu Sans,64,&H00FFFFFF,&H000000FF,\
         &H00000000,&H80000000,0,0,0,0,100,100,0,0,1,3,0,2,80,80,140,1\n\
         [Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, \
         Effect, Text\nDialogue: 0,0:00:00.00,0:00:01.30,Default,,0,0,0,,Hello\n",
    )
    .unwrap();
    let clip = |source: &str, start: f64| {
        json!({
            "source": source,
            "source_in": 0.0,
            "source_out": 0.8,
            "timeline_in": start,
            "timeline_out": start + 0.8
        })
    };
    let plan = json!({
        "schema_version": 1,
        "project": {
            "id": "render-test",
            "revision": 7,
            "document_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "canvas": {"width": 160, "height": 90, "frame_rate": 10.0},
        "sources": {
            "one": {
                "path": "one.mp4", "sha256": "one", "size": 1,
                "duration": 0.8, "width": 160, "height": 90, "has_audio": true
            },
            "two": {
                "path": "two.mp4", "sha256": "two", "size": 1,
                "duration": 0.8, "width": 160, "height": 90, "has_audio": true
            }
        },
        "timeline": {
            "main_tracks": [
                {"name": null, "clips": [clip("one", 0.0), clip("two", 0.8)]},
                {"name": "top", "clips": [clip("two", 0.0), clip("one", 0.8)]}
            ],
            "overlay_tracks": [],
            "markers": [{"name": "beat", "timeline_time": 0.8}],
            "transitions": [{"kind": "cross_dissolve", "timeline_time": 0.8, "duration": 0.2}],
            "variants": [
                {
                    "language": "EN", "aspect": "9:16", "subtitles": "captions.ass",
                    "watermark": null, "cta": null
                },
                {
                    "language": "EN", "aspect": "1:1", "subtitles": "captions.ass",
                    "watermark": null, "cta": "Review"
                }
            ],
            "opening_hook": {"min_seconds": 2.8, "max_seconds": 3.2}
        },
        "cuts": [{"timeline_time": 0.8}],
        "hold_sources": [],
        "asset_identities": [
            {"path": "one.mp4", "sha256": "one", "size": 1, "duration": 0.8,
             "width": 160, "height": 90, "has_audio": true},
            {"path": "two.mp4", "sha256": "two", "size": 1, "duration": 0.8,
             "width": 160, "height": 90, "has_audio": true},
            {"path": "captions.ass", "sha256": "subs", "size": 1, "has_audio": false}
        ]
    });
    let canonical = root.path().join("canonical-edl.json");
    fs::write(&canonical, serde_json::to_vec(&plan).unwrap()).unwrap();
    let status = Command::new("/usr/bin/python3")
        .arg(format!(
            "{}/scripts/video_project_render.py",
            env!("CARGO_MANIFEST_DIR")
        ))
        .args([
            "--job-id",
            "render-test-job",
            "--project-id",
            "render-test",
            "--revision",
            "7",
            "--document-sha256",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--replay-bundle-sha256",
        ])
        .arg(video_work_api::provenance::sha256_file(&canonical).unwrap())
        .arg("--replay-output")
        .arg(root.path().join("replay-output"))
        .arg(&canonical)
        .arg(&assets)
        .arg(output.join("render-test-job"))
        .status()
        .unwrap();
    assert!(status.success());

    for path in [
        output.join("render-test-job/master.mp4"),
        output.join("render-test-job/v001-en-9x16.mp4"),
        output.join("render-test-job/v002-en-1x1.mp4"),
    ] {
        assert!(path.metadata().unwrap().len() > 0);
        let probe = Command::new("/usr/bin/ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=nw=1:nk=1",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(probe.status.success());
        let duration = String::from_utf8(probe.stdout)
            .unwrap()
            .trim()
            .parse::<f64>()
            .unwrap();
        assert!(
            (duration - 1.6).abs() <= 0.06,
            "declared 1.6 second timeline changed to {duration}"
        );
        let bytes = fs::read(path).unwrap();
        let moov = bytes.windows(4).position(|item| item == b"moov").unwrap();
        let mdat = bytes.windows(4).position(|item| item == b"mdat").unwrap();
        assert!(moov < mdat, "output must use MP4 faststart");
    }
    let report: serde_json::Value = serde_json::from_slice(
        &fs::read(output.join("render-test-job/render-report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["project"]["revision"], 7);
    assert_eq!(report["outputs"].as_array().unwrap().len(), 3);
    assert_eq!(report["replay"]["executed"], true);
    assert_eq!(
        report["replay"]["primary_sha256"],
        report["replay"]["replay_sha256"]
    );
}
