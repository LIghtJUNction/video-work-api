use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WordTimestamp {
    pub word: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VadInterval {
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub breath: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrimRequest {
    pub requested_start: f64,
    pub requested_end: f64,
    #[serde(default = "default_search_radius")]
    pub search_radius: f64,
    #[serde(default)]
    pub words: Vec<WordTimestamp>,
    #[serde(default)]
    pub voiced_intervals: Vec<VadInterval>,
    #[serde(default)]
    pub silent_intervals: Vec<VadInterval>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalyzeSafeTrimsRequest {
    pub video_path: String,
    pub requested_start: f64,
    pub requested_end: f64,
    #[serde(default = "default_search_radius")]
    pub search_radius: f64,
    #[serde(default)]
    pub words: Vec<WordTimestamp>,
}

impl AnalyzeSafeTrimsRequest {
    pub fn validate(&self) -> Result<()> {
        if !self.requested_start.is_finite()
            || !self.requested_end.is_finite()
            || !self.search_radius.is_finite()
            || self.requested_start < 0.0
            || self.requested_end <= self.requested_start
            || self.search_radius < 0.0
        {
            bail!("requested trim range and search_radius are invalid");
        }
        for word in &self.words {
            if !word.start.is_finite()
                || !word.end.is_finite()
                || word.start < 0.0
                || word.end <= word.start
            {
                bail!("word timestamps must be genuine finite increasing intervals");
            }
        }
        Ok(())
    }
}

fn default_search_radius() -> f64 {
    0.5
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TrimResult {
    pub trim_start: f64,
    pub trim_end: f64,
    pub start_interval: VadInterval,
    pub end_interval: VadInterval,
    pub capability: AlignmentCapability,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AlignmentCapability {
    pub vad_alignment: &'static str,
    pub word_level_asr_backend: &'static str,
    pub segment_level_asr_backend: &'static str,
}

pub fn trim_silence_safe(request: &TrimRequest) -> Result<TrimResult> {
    validate_request(request)?;
    let word_backend = if request.words.is_empty() {
        "unavailable_no_word_timestamps"
    } else {
        "genuine_supplied_or_funclip_timestamps"
    };
    let (trim_start, start_interval) = choose_boundary(
        request.requested_start,
        request.search_radius,
        &request.silent_intervals,
        &request.words,
        &request.voiced_intervals,
        "start",
    )?;
    let (trim_end, end_interval) = choose_boundary(
        request.requested_end,
        request.search_radius,
        &request.silent_intervals,
        &request.words,
        &request.voiced_intervals,
        "end",
    )?;
    if trim_end <= trim_start {
        bail!("safe trim boundaries would produce an empty or reversed interval");
    }
    Ok(TrimResult {
        trim_start,
        trim_end,
        start_interval,
        end_interval,
        capability: AlignmentCapability {
            vad_alignment: "available_with_supplied_intervals",
            word_level_asr_backend: word_backend,
            segment_level_asr_backend: "funclip",
        },
    })
}

fn validate_request(request: &TrimRequest) -> Result<()> {
    if !request.requested_start.is_finite()
        || !request.requested_end.is_finite()
        || !request.search_radius.is_finite()
        || request.requested_start < 0.0
        || request.requested_end <= request.requested_start
        || request.search_radius < 0.0
    {
        bail!("requested trim range and search_radius are invalid");
    }
    for (start, end) in request
        .words
        .iter()
        .map(|item| (item.start, item.end))
        .chain(
            request
                .voiced_intervals
                .iter()
                .chain(&request.silent_intervals)
                .map(|item| (item.start, item.end)),
        )
    {
        if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
            bail!("alignment intervals must be finite, nonnegative, and increasing");
        }
    }
    Ok(())
}

fn choose_boundary(
    requested: f64,
    radius: f64,
    silence: &[VadInterval],
    words: &[WordTimestamp],
    voiced: &[VadInterval],
    label: &str,
) -> Result<(f64, VadInterval)> {
    let mut candidates = silence
        .iter()
        .filter(|interval| {
            !words
                .iter()
                .any(|word| overlaps(interval.start, interval.end, word.start, word.end))
                && !voiced
                    .iter()
                    .any(|voice| overlaps(interval.start, interval.end, voice.start, voice.end))
        })
        .filter_map(|interval| {
            let lower = interval.start.max(requested - radius);
            let upper = interval.end.min(requested + radius);
            if upper < lower {
                return None;
            }
            let point = requested.clamp(lower, upper);
            is_clear(point, words, voiced).then_some((
                (point - requested).abs(),
                point,
                interval.clone(),
            ))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    candidates
        .into_iter()
        .next()
        .map(|(_, point, interval)| (point, interval))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no safe {label} boundary within {radius:.3}s; refusing to cut a word or voiced interval"
            )
        })
}

fn overlaps(a_start: f64, a_end: f64, b_start: f64, b_end: f64) -> bool {
    a_start < b_end && b_start < a_end
}

fn is_clear(point: f64, words: &[WordTimestamp], voiced: &[VadInterval]) -> bool {
    !words
        .iter()
        .any(|word| point > word.start && point < word.end)
        && !voiced
            .iter()
            .any(|interval| point > interval.start && point < interval.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> TrimRequest {
        TrimRequest {
            requested_start: 1.0,
            requested_end: 9.0,
            search_radius: 0.5,
            words: vec![],
            voiced_intervals: vec![],
            silent_intervals: vec![
                VadInterval {
                    start: 0.8,
                    end: 1.2,
                    breath: true,
                },
                VadInterval {
                    start: 8.8,
                    end: 9.2,
                    breath: false,
                },
            ],
        }
    }

    #[test]
    fn preserves_requested_boundaries_inside_silence() {
        let result = trim_silence_safe(&base()).unwrap();
        assert_eq!((result.trim_start, result.trim_end), (1.0, 9.0));
    }

    #[test]
    fn rejects_silence_that_overlaps_word_and_voice_at_candidate() {
        let mut request = base();
        request.words.push(WordTimestamp {
            word: "cut".into(),
            start: 0.9,
            end: 1.1,
        });
        request.voiced_intervals.push(VadInterval {
            start: 0.8,
            end: 1.2,
            breath: false,
        });
        assert!(trim_silence_safe(&request)
            .unwrap_err()
            .to_string()
            .contains("no safe start boundary"));
    }

    #[test]
    fn errors_when_no_silent_point_exists() {
        let mut request = base();
        request.silent_intervals.clear();
        assert!(trim_silence_safe(&request).is_err());
    }
}
