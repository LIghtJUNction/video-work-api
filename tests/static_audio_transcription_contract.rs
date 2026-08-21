const INDEX: &str = include_str!("../static/index.html");
const SCRIPT: &str = include_str!("../static/app.js");
const DOCS: &str = include_str!("../static/docs.html");
const ROUTES: &str = include_str!("../src/http/routes.rs");
const README: &str = include_str!("../README.md");
const README_ZH: &str = include_str!("../README.zh-CN.md");
const OPERATIONS: &str = include_str!("../.agents/skills/video-work-api/references/operations.md");

#[test]
fn recording_transcription_batch_keeps_per_file_text_srt_and_segments_visible() {
    for id in [
        "audioTranscriptionForm",
        "audioTranscriptionFile",
        "audioTranscriptionCount",
        "audioTranscriptionButton",
        "audioTranscriptionBatch",
        "audioTranscriptionProgressText",
        "audioTranscriptionProgress",
        "audioTranscriptionJobs",
        "copyAllAudioTranscriptions",
        "downloadAllAudioTranscriptions",
        "retryAudioTranscriptionFailures",
    ] {
        assert!(INDEX.contains(&format!("id=\"{id}\"")), "missing #{id}");
    }
    assert!(INDEX.contains("accept=\".wav,.mp3,.aac,.m4a,.flac\""));
    assert!(INDEX.contains("name=\"audio\" type=\"file\" multiple"));
    assert!(SCRIPT.contains("/api/audio/transcriptions/upload"));
    assert!(SCRIPT.contains("body.append(\"audio\", job.file)"));
    assert!(SCRIPT.contains("AUDIO_TRANSCRIPTION_EXTENSIONS"));
    assert!(SCRIPT.contains("files.length > MAX_BATCH_ITEMS"));
    assert!(!SCRIPT.contains("MAX_AUDIO_TRANSCRIPTION_BYTES"));
    assert!(!SCRIPT.contains("audioTranscriptionFileTooLarge"));
    assert!(SCRIPT.contains("audioTranscriptionEpoch"));
    assert!(SCRIPT.contains("if (epoch !== audioTranscriptionEpoch) return;"));
    assert!(SCRIPT.contains("resetAudioTranscriptionBatch"));
    assert!(SCRIPT.contains("revokeAudioTranscriptionUrls"));
    assert!(SCRIPT.contains("await runSequential("));
    assert!(SCRIPT.contains("() => epoch !== audioTranscriptionEpoch,"));
    assert!(SCRIPT.contains("audioTranscriptionJobs.filter((job) => job.status === \"failed\")"));
    assert!(!SCRIPT.contains("audioTranscriptionResult"));
    assert!(SCRIPT.contains("transcriptionSegments"));
    assert!(SCRIPT.contains("subtitleDownloadName(job.label)"));
    assert!(SCRIPT.contains("formatTranscriptionResults(audioTranscriptionJobs)"));
    assert!(SCRIPT.contains("recording-transcriptions.txt"));
    assert!(SCRIPT.contains("copyTextToClipboard(text)"));
    assert!(DOCS.contains("/api/audio/transcriptions"));
    assert!(DOCS.contains("/api/audio/transcriptions/upload"));
    assert!(DOCS.contains("每次请求一个 multipart"));
    assert!(DOCS.contains("transcribe_audio"));

    for obsolete in [
        "≤50 MB",
        "≤50 MiB",
        "exceeds 50 MB",
        "超过 50 MB",
        "50 MiB each",
        "每个 50 MiB",
    ] {
        assert!(!INDEX.contains(obsolete), "obsolete UI limit: {obsolete}");
        assert!(
            !SCRIPT.contains(obsolete),
            "obsolete client limit: {obsolete}"
        );
        assert!(!DOCS.contains(obsolete), "obsolete docs limit: {obsolete}");
        assert!(
            !README.contains(obsolete),
            "obsolete README limit: {obsolete}"
        );
        assert!(
            !README_ZH.contains(obsolete),
            "obsolete Chinese README limit: {obsolete}"
        );
        assert!(
            !OPERATIONS.contains(obsolete),
            "obsolete operations limit: {obsolete}"
        );
    }
}

#[test]
fn transcription_upload_disables_only_the_old_route_body_limit_and_keeps_streaming() {
    assert!(ROUTES.contains("post(audio_transcription_upload).layer(DefaultBodyLimit::disable())"));
    let handler_start = ROUTES.find("async fn audio_transcription_upload").unwrap();
    let handler_end = ROUTES[handler_start..]
        .find("async fn video_subtitles(")
        .map(|offset| handler_start + offset)
        .unwrap();
    let handler = &ROUTES[handler_start..handler_end];

    assert!(handler.contains(".chunk()"));
    assert!(!handler.contains("MAX_UPLOAD_BYTES"));
    assert!(!handler.contains("field.bytes()"));
    assert!(!handler.contains("Audio exceeds 50 MiB"));
}

#[test]
fn audio_transcription_epoch_blocks_stale_responses_before_render_or_unlock() {
    let run = SCRIPT
        .find("async function runAudioTranscriptionJobs(jobs)")
        .unwrap();
    let sequential = SCRIPT[run..].find("await runSequential(").unwrap();
    let guard = SCRIPT[run..]
        .find("if (epoch !== audioTranscriptionEpoch) return;")
        .unwrap();
    let unlock = SCRIPT[run..]
        .find("setAudioTranscriptionLocked(false);")
        .unwrap();

    assert!(
        sequential < guard,
        "check the epoch after the awaited batch"
    );
    assert!(guard < unlock, "never unlock a newer audio batch");
    assert!(SCRIPT.contains("if (epoch === audioTranscriptionEpoch) {"));
}

#[test]
fn audio_transcription_blob_urls_are_revoked_on_batch_clear_and_pagehide() {
    let reset = SCRIPT
        .find("function resetAudioTranscriptionBatch")
        .unwrap();
    let revoke_on_clear = SCRIPT[reset..]
        .find("revokeAudioTranscriptionUrls();")
        .unwrap();
    let clear_jobs = SCRIPT[reset..]
        .find("audioTranscriptionJobs = [];")
        .unwrap();
    assert!(revoke_on_clear < clear_jobs);

    let pagehide = SCRIPT.find("window.addEventListener(\"pagehide\"").unwrap();
    let revoke_on_pagehide = SCRIPT[pagehide..]
        .find("revokeAudioTranscriptionUrls();")
        .unwrap();
    let persisted_return = SCRIPT[pagehide..]
        .find("if (event.persisted) return;")
        .unwrap();
    assert!(revoke_on_pagehide < persisted_return);
}
