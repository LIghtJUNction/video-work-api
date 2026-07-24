//! MADLAD-400-3B machine translation for plain text and subtitle segments.
//!
//! Weight download/presence is owned by [`crate::model`] (same mechanism as CosyVoice).
//! This module only handles language catalogs and lazy inference via a Python helper.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model::translation_dir_files_present;
use crate::subtitles::{parse_srt, SubtitleSegment};

/// Soft cap for a single plain-text translation request (Unicode scalars).
pub const MAX_TRANSLATE_TEXT_CHARS: usize = 8_000;
/// Maximum number of subtitle / batch lines per request.
pub const MAX_TRANSLATE_SEGMENTS: usize = 200;
/// Aggregate Unicode scalar budget across a batch.
pub const MAX_TRANSLATE_TOTAL_CHARS: usize = 50_000;

/// Curated language list for UI / MCP discovery.
/// Order is intentional: English and Russian first for convenient selection,
/// then other high-frequency targets.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TranslationLanguage {
    pub code: &'static str,
    pub name_en: &'static str,
    pub name_zh: &'static str,
}

pub const TRANSLATION_LANGUAGES: &[TranslationLanguage] = &[
    TranslationLanguage {
        code: "en",
        name_en: "English",
        name_zh: "英语",
    },
    TranslationLanguage {
        code: "ru",
        name_en: "Russian",
        name_zh: "俄语",
    },
    TranslationLanguage {
        code: "zh",
        name_en: "Chinese",
        name_zh: "中文",
    },
    TranslationLanguage {
        code: "ja",
        name_en: "Japanese",
        name_zh: "日语",
    },
    TranslationLanguage {
        code: "ko",
        name_en: "Korean",
        name_zh: "韩语",
    },
    TranslationLanguage {
        code: "es",
        name_en: "Spanish",
        name_zh: "西班牙语",
    },
    TranslationLanguage {
        code: "fr",
        name_en: "French",
        name_zh: "法语",
    },
    TranslationLanguage {
        code: "de",
        name_en: "German",
        name_zh: "德语",
    },
    TranslationLanguage {
        code: "pt",
        name_en: "Portuguese",
        name_zh: "葡萄牙语",
    },
    TranslationLanguage {
        code: "it",
        name_en: "Italian",
        name_zh: "意大利语",
    },
    TranslationLanguage {
        code: "ar",
        name_en: "Arabic",
        name_zh: "阿拉伯语",
    },
    TranslationLanguage {
        code: "hi",
        name_en: "Hindi",
        name_zh: "印地语",
    },
    TranslationLanguage {
        code: "th",
        name_en: "Thai",
        name_zh: "泰语",
    },
    TranslationLanguage {
        code: "vi",
        name_en: "Vietnamese",
        name_zh: "越南语",
    },
    TranslationLanguage {
        code: "id",
        name_en: "Indonesian",
        name_zh: "印尼语",
    },
    TranslationLanguage {
        code: "tr",
        name_en: "Turkish",
        name_zh: "土耳其语",
    },
    TranslationLanguage {
        code: "pl",
        name_en: "Polish",
        name_zh: "波兰语",
    },
    TranslationLanguage {
        code: "uk",
        name_en: "Ukrainian",
        name_zh: "乌克兰语",
    },
    TranslationLanguage {
        code: "nl",
        name_en: "Dutch",
        name_zh: "荷兰语",
    },
    TranslationLanguage {
        code: "sv",
        name_en: "Swedish",
        name_zh: "瑞典语",
    },
    TranslationLanguage {
        code: "cs",
        name_en: "Czech",
        name_zh: "捷克语",
    },
    TranslationLanguage {
        code: "ro",
        name_en: "Romanian",
        name_zh: "罗马尼亚语",
    },
    TranslationLanguage {
        code: "hu",
        name_en: "Hungarian",
        name_zh: "匈牙利语",
    },
    TranslationLanguage {
        code: "fi",
        name_en: "Finnish",
        name_zh: "芬兰语",
    },
    TranslationLanguage {
        code: "el",
        name_en: "Greek",
        name_zh: "希腊语",
    },
    TranslationLanguage {
        code: "he",
        name_en: "Hebrew",
        name_zh: "希伯来语",
    },
    TranslationLanguage {
        code: "fa",
        name_en: "Persian",
        name_zh: "波斯语",
    },
    TranslationLanguage {
        code: "ms",
        name_en: "Malay",
        name_zh: "马来语",
    },
    TranslationLanguage {
        code: "bn",
        name_en: "Bengali",
        name_zh: "孟加拉语",
    },
    TranslationLanguage {
        code: "ta",
        name_en: "Tamil",
        name_zh: "泰米尔语",
    },
    TranslationLanguage {
        code: "ur",
        name_en: "Urdu",
        name_zh: "乌尔都语",
    },
    TranslationLanguage {
        code: "sw",
        name_en: "Swahili",
        name_zh: "斯瓦希里语",
    },
    TranslationLanguage {
        code: "yue",
        name_en: "Cantonese",
        name_zh: "粤语",
    },
];

pub fn languages_payload() -> Value {
    json!({
        "model": "google/madlad400-3b-mt",
        "languages": TRANSLATION_LANGUAGES,
        "note": "English and Russian are listed first for convenience. Any ISO 639-1/2 MADLAD target code is also accepted.",
    })
}

pub fn normalize_target_lang(raw: &str) -> Result<String> {
    let trimmed = raw.trim().to_ascii_lowercase().replace('_', "-");
    if trimmed.is_empty() {
        bail!("target_lang is required");
    }
    let primary = trimmed.split('-').next().unwrap_or("");
    if !(2..=3).contains(&primary.len()) || !primary.chars().all(|c| c.is_ascii_alphabetic()) {
        bail!("target_lang must be an ISO 639 language code (e.g. en, ru, zh)");
    }
    Ok(primary.to_string())
}

pub fn segments_to_srt(segments: &[SubtitleSegment]) -> String {
    let mut out = String::new();
    for (i, segment) in segments.iter().enumerate() {
        let start = segment.start.replace('.', ",");
        let end = segment.end.replace('.', ",");
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            start,
            end,
            segment.text
        ));
    }
    out
}

/// Trait for machine-translation backends.
pub trait TranslationEngine: Send + Sync {
    fn ready(&self) -> bool;
    fn loaded(&self) -> bool;
    fn translate_texts(&self, target_lang: &str, texts: &[String]) -> Result<Vec<String>>;
}

/// MADLAD-400-3B via a small Python helper that loads Transformers T5.
pub struct MadladEngine {
    model_dir: PathBuf,
    helper: PathBuf,
    python: PathBuf,
    timeout: Duration,
    loaded: AtomicBool,
    lock: Mutex<()>,
}

impl MadladEngine {
    pub fn new(model_dir: PathBuf, project_root: &Path, timeout_seconds: u64) -> Self {
        let helper = project_root.join("scripts").join("madlad_translate.py");
        let python = std::env::var_os("VWA_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("python3"));
        Self {
            model_dir,
            helper,
            python,
            timeout: Duration::from_secs(timeout_seconds.max(30)),
            loaded: AtomicBool::new(false),
            lock: Mutex::new(()),
        }
    }
}

impl TranslationEngine for MadladEngine {
    fn ready(&self) -> bool {
        translation_dir_files_present(&self.model_dir) && self.helper.is_file()
    }

    fn loaded(&self) -> bool {
        self.loaded.load(Ordering::Relaxed)
    }

    fn translate_texts(&self, target_lang: &str, texts: &[String]) -> Result<Vec<String>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow::anyhow!("translation engine lock poisoned"))?;
        if !self.ready() {
            bail!("MADLAD translation model is not installed");
        }
        let request = json!({
            "target_lang": target_lang,
            "texts": texts,
        });
        let mut child = Command::new(&self.python)
            .arg(&self.helper)
            .arg("--model-dir")
            .arg(&self.model_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawn madlad_translate.py")?;
        {
            let mut stdin = child
                .stdin
                .take()
                .context("madlad helper stdin unavailable")?;
            stdin
                .write_all(request.to_string().as_bytes())
                .context("write madlad request")?;
        }

        let started = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() > self.timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!(
                        "MADLAD translation timed out after {}s",
                        self.timeout.as_secs()
                    );
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(error) => bail!("wait madlad helper: {error}"),
            }
        }

        let output = child
            .wait_with_output()
            .context("collect madlad helper output")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            if stderr.is_empty() {
                bail!("MADLAD translation failed (exit {})", output.status);
            }
            bail!("MADLAD translation failed: {stderr}");
        }
        let response: MadladResponse = serde_json::from_slice(&output.stdout)
            .context("parse madlad helper JSON response")?;
        if response.translations.len() != texts.len() {
            bail!(
                "MADLAD returned {} translations for {} inputs",
                response.translations.len(),
                texts.len()
            );
        }
        self.loaded.store(true, Ordering::Relaxed);
        Ok(response.translations)
    }
}

#[derive(Debug, Deserialize)]
struct MadladResponse {
    translations: Vec<String>,
}

/// Test / dry-run engine that mirrors input with a language tag prefix.
pub struct FakeTranslationEngine {
    pub ready_flag: bool,
    pub loaded_flag: AtomicBool,
    pub calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl FakeTranslationEngine {
    pub fn new() -> Self {
        Self {
            ready_flag: true,
            loaded_flag: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl Default for FakeTranslationEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TranslationEngine for FakeTranslationEngine {
    fn ready(&self) -> bool {
        self.ready_flag
    }

    fn loaded(&self) -> bool {
        self.loaded_flag.load(Ordering::Relaxed)
    }

    fn translate_texts(&self, target_lang: &str, texts: &[String]) -> Result<Vec<String>> {
        self.calls
            .lock()
            .unwrap()
            .push((target_lang.to_string(), texts.to_vec()));
        self.loaded_flag.store(true, Ordering::Relaxed);
        Ok(texts
            .iter()
            .map(|text| format!("[{target_lang}] {text}"))
            .collect())
    }
}

/// Validate request limits before invoking the engine.
pub fn validate_texts(texts: &[String]) -> Result<()> {
    if texts.is_empty() {
        bail!("Nothing to translate");
    }
    if texts.len() > MAX_TRANSLATE_SEGMENTS {
        bail!(
            "At most {MAX_TRANSLATE_SEGMENTS} texts/segments can be translated at once"
        );
    }
    let mut total = 0usize;
    for (index, text) in texts.iter().enumerate() {
        let count = text.chars().count();
        if count == 0 {
            continue;
        }
        if count > MAX_TRANSLATE_TEXT_CHARS {
            bail!(
                "Item {} exceeds {MAX_TRANSLATE_TEXT_CHARS} characters (current {count})",
                index + 1
            );
        }
        total = total.saturating_add(count);
    }
    if total == 0 {
        bail!("Nothing to translate");
    }
    if total > MAX_TRANSLATE_TOTAL_CHARS {
        bail!("Total translation input exceeds {MAX_TRANSLATE_TOTAL_CHARS} characters");
    }
    Ok(())
}

/// Translate plain text, optional multi-line batch, SRT, or structured segments.
pub fn translate_request(
    engine: &dyn TranslationEngine,
    target_lang: &str,
    text: Option<&str>,
    texts: Option<&[String]>,
    srt: Option<&str>,
    segments: Option<&[SubtitleSegment]>,
) -> Result<Value> {
    let lang = normalize_target_lang(target_lang)?;
    if !engine.ready() {
        bail!("Translation model is not ready; download MADLAD-400-3B first");
    }

    // Prefer structured segments, then SRT, then explicit texts array, then single text.
    if let Some(source_segments) = segments {
        if source_segments.is_empty() {
            bail!("segments must not be empty");
        }
        let lines: Vec<String> = source_segments
            .iter()
            .map(|segment| segment.text.clone())
            .collect();
        validate_texts(&lines)?;
        let translated = engine.translate_texts(&lang, &lines)?;
        let mut out_segments = Vec::with_capacity(source_segments.len());
        for (index, segment) in source_segments.iter().enumerate() {
            out_segments.push(SubtitleSegment {
                index: index + 1,
                start: segment.start.clone(),
                end: segment.end.clone(),
                text: translated
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| segment.text.clone()),
            });
        }
        let out_srt = segments_to_srt(&out_segments);
        return Ok(json!({
            "target_lang": lang,
            "model": "google/madlad400-3b-mt",
            "segments": out_segments,
            "srt": out_srt,
            "translations": translated,
        }));
    }

    if let Some(srt_body) = srt {
        let parsed = parse_srt(srt_body);
        if parsed.is_empty() {
            bail!("srt contained no subtitle segments");
        }
        return translate_request(engine, &lang, None, None, None, Some(&parsed));
    }

    let batch: Vec<String> = if let Some(items) = texts {
        items.to_vec()
    } else if let Some(single) = text {
        let trimmed = single.trim();
        if trimmed.is_empty() {
            bail!("text is required when segments/srt/texts are omitted");
        }
        vec![trimmed.to_string()]
    } else {
        bail!("Provide text, texts, srt, or segments");
    };
    validate_texts(&batch)?;
    let translated = engine.translate_texts(&lang, &batch)?;
    if batch.len() == 1 {
        Ok(json!({
            "target_lang": lang,
            "model": "google/madlad400-3b-mt",
            "text": translated.first().cloned().unwrap_or_default(),
            "translations": translated,
        }))
    } else {
        Ok(json!({
            "target_lang": lang,
            "model": "google/madlad400-3b-mt",
            "translations": translated,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_put_english_and_russian_first() {
        assert_eq!(TRANSLATION_LANGUAGES[0].code, "en");
        assert_eq!(TRANSLATION_LANGUAGES[1].code, "ru");
    }

    #[test]
    fn normalizes_bcp47_and_rejects_junk() {
        assert_eq!(normalize_target_lang("EN-US").unwrap(), "en");
        assert_eq!(normalize_target_lang("ru").unwrap(), "ru");
        assert!(normalize_target_lang("").is_err());
        assert!(normalize_target_lang("1").is_err());
        assert!(normalize_target_lang("toolongcode").is_err());
    }

    #[test]
    fn fake_engine_translates_plain_text_and_srt() {
        let engine = FakeTranslationEngine::new();
        let plain = translate_request(&engine, "en", Some("你好"), None, None, None).unwrap();
        assert_eq!(plain["text"], "[en] 你好");

        let srt = "1\n00:00:00,000 --> 00:00:01,000\n你好\n\n2\n00:00:01,000 --> 00:00:02,000\n世界\n";
        let result = translate_request(&engine, "ru", None, None, Some(srt), None).unwrap();
        assert_eq!(result["segments"][0]["text"], "[ru] 你好");
        assert_eq!(result["segments"][1]["text"], "[ru] 世界");
        assert!(result["srt"].as_str().unwrap().contains("[ru] 你好"));
    }

    #[test]
    fn rejects_oversized_batch() {
        let texts: Vec<String> = (0..MAX_TRANSLATE_SEGMENTS + 1)
            .map(|i| format!("line {i}"))
            .collect();
        assert!(validate_texts(&texts).is_err());
    }
}
