use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimelineEdl {
    pub main_tracks: Vec<MainTrack>,
    #[serde(default)]
    pub overlay_tracks: Vec<OverlayTrack>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    #[serde(default)]
    pub transitions: Vec<Transition>,
    #[serde(default)]
    pub variants: Vec<VariantSpec>,
    #[serde(default)]
    pub opening_hook: OpeningHook,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MainTrack {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub source: String,
    pub source_in: f64,
    pub source_out: f64,
    pub timeline_in: f64,
    pub timeline_out: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OverlayTrack {
    Broll {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<String>,
        clip: Clip,
    },
    Pip {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<String>,
        clip: Clip,
    },
    Effect {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<String>,
        timeline_in: f64,
        timeline_out: f64,
        name: String,
    },
    Hold {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<String>,
        source_time: f64,
        timeline_in: f64,
        timeline_out: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub name: String,
    pub timeline_time: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    pub kind: String,
    pub timeline_time: f64,
    pub duration: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VariantSpec {
    pub language: String,
    pub aspect: AspectRatio,
    #[serde(default)]
    pub subtitles: Option<String>,
    #[serde(default)]
    pub watermark: Option<String>,
    #[serde(default)]
    pub cta: Option<String>,
}

pub fn variant_key(index: usize, variant: &VariantSpec) -> String {
    let slug = variant
        .language
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() { "variant" } else { &slug };
    let aspect = match variant.aspect {
        AspectRatio::Portrait => "9x16",
        AspectRatio::Landscape => "16x9",
        AspectRatio::Square => "1x1",
    };
    format!("v{:03}-{slug}-{aspect}", index + 1)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AspectRatio {
    #[serde(rename = "9:16")]
    Portrait,
    #[serde(rename = "16:9")]
    Landscape,
    #[serde(rename = "1:1")]
    Square,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OpeningHook {
    pub min_seconds: f64,
    pub max_seconds: f64,
}

impl Default for OpeningHook {
    fn default() -> Self {
        Self {
            min_seconds: 2.8,
            max_seconds: 3.2,
        }
    }
}

impl TimelineEdl {
    pub fn duration(&self) -> f64 {
        self.main_tracks
            .iter()
            .flat_map(|track| &track.clips)
            .map(|clip| clip.timeline_out)
            .fold(0.0, f64::max)
    }

    pub fn validate(&self) -> Result<()> {
        if self.main_tracks.is_empty() {
            bail!("main_tracks must not be empty");
        }
        validate_range(
            self.opening_hook.min_seconds,
            self.opening_hook.max_seconds,
            "opening_hook",
        )?;
        let mut duration = None;
        for (index, track) in self.main_tracks.iter().enumerate() {
            if track
                .name
                .as_ref()
                .is_some_and(|name| name.trim().is_empty())
            {
                bail!("main track name must not be empty");
            }
            if track.clips.is_empty() {
                bail!("main track clips must not be empty");
            }
            let mut clips = track.clips.iter().collect::<Vec<_>>();
            clips.sort_by(|a, b| a.timeline_in.total_cmp(&b.timeline_in));
            for clip in &clips {
                validate_clip(clip)?;
            }
            if clips[0].timeline_in != 0.0 {
                bail!("main track {index} must start at zero");
            }
            for pair in clips.windows(2) {
                if (pair[0].timeline_out - pair[1].timeline_in).abs() > 0.000_001 {
                    bail!("main track {index} must be continuous");
                }
            }
            let track_duration = clips.last().unwrap().timeline_out;
            if duration.is_some_and(|value: f64| (value - track_duration).abs() > 0.000_001) {
                bail!("all main tracks must have the same duration");
            }
            duration = Some(track_duration);
        }
        let duration = duration.unwrap();
        for overlay in &self.overlay_tracks {
            let (start, end) = match overlay {
                OverlayTrack::Broll { clip, .. } | OverlayTrack::Pip { clip, .. } => {
                    validate_clip(clip)?;
                    (clip.timeline_in, clip.timeline_out)
                }
                OverlayTrack::Effect {
                    timeline_in,
                    timeline_out,
                    ..
                } => (*timeline_in, *timeline_out),
                OverlayTrack::Hold {
                    source_time,
                    timeline_in,
                    timeline_out,
                    ..
                } => {
                    finite_nonnegative(*source_time, "hold source_time")?;
                    (*timeline_in, *timeline_out)
                }
            };
            validate_range(start, end, "overlay")?;
            if end > duration {
                bail!("overlay exceeds timeline duration");
            }
        }
        for marker in &self.markers {
            finite_nonnegative(marker.timeline_time, "marker")?;
            if marker.timeline_time > duration {
                bail!("marker exceeds timeline duration");
            }
        }
        for transition in &self.transitions {
            finite_nonnegative(transition.timeline_time, "transition")?;
            finite_nonnegative(transition.duration, "transition duration")?;
            if transition.kind.trim().is_empty() || transition.duration == 0.0 {
                bail!("transition kind and positive duration are required");
            }
            let transition_end = transition.timeline_time + transition.duration;
            if !transition_end.is_finite() || transition_end > duration {
                bail!("transition exceeds timeline duration");
            }
        }
        for variant in &self.variants {
            if variant.language.trim().is_empty() {
                bail!("variant language must not be empty");
            }
            if variant
                .subtitles
                .as_ref()
                .is_some_and(|path| path.trim().is_empty())
            {
                bail!("variant subtitles must not be empty");
            }
            if variant
                .subtitles
                .as_ref()
                .is_some_and(|path| !is_safe_relative_path(path))
            {
                bail!("variant subtitles must be a sandboxed relative path");
            }
            if variant
                .watermark
                .as_ref()
                .is_some_and(|path| path.trim().is_empty())
            {
                bail!("variant watermark must not be empty");
            }
            if variant
                .watermark
                .as_ref()
                .is_some_and(|path| !is_safe_relative_path(path))
            {
                bail!("variant watermark must be a sandboxed relative path");
            }
            if variant
                .cta
                .as_ref()
                .is_some_and(|cta| cta.trim().is_empty())
            {
                bail!("variant cta must not be empty");
            }
            if variant.subtitles.is_none() && variant.watermark.is_none() && variant.cta.is_none() {
                bail!("variant must declare subtitles, watermark, or cta");
            }
        }
        Ok(())
    }

    pub fn canonical_json(&self) -> Result<String> {
        self.validate()?;
        Ok(serde_json::to_string(self)?)
    }

    pub fn canonical_sha256(&self) -> Result<String> {
        Ok(hex::encode(Sha256::digest(
            self.canonical_json()?.as_bytes(),
        )))
    }

    pub fn validate_opening_hook(&self, timeline_window: (f64, f64)) -> Result<()> {
        validate_range(timeline_window.0, timeline_window.1, "opening hook window")?;
        if self.markers.iter().any(|marker| {
            marker.timeline_time >= timeline_window.0 && marker.timeline_time <= timeline_window.1
        }) {
            Ok(())
        } else {
            bail!(
                "opening hook requires a marker between {:.3}s and {:.3}s",
                timeline_window.0,
                timeline_window.1
            )
        }
    }
}

fn finite_nonnegative(value: f64, label: &str) -> Result<()> {
    if !value.is_finite() || value < 0.0 {
        bail!("{label} must be finite and nonnegative");
    }
    Ok(())
}

fn is_safe_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn validate_range(start: f64, end: f64, label: &str) -> Result<()> {
    finite_nonnegative(start, label)?;
    finite_nonnegative(end, label)?;
    if end <= start {
        bail!("{label} must have a positive range");
    }
    Ok(())
}

fn validate_clip(clip: &Clip) -> Result<()> {
    if clip.source.trim().is_empty() {
        bail!("clip source must not be empty");
    }
    validate_range(clip.source_in, clip.source_out, "clip source")?;
    validate_range(clip.timeline_in, clip.timeline_out, "clip timeline")?;
    if ((clip.source_out - clip.source_in) - (clip.timeline_out - clip.timeline_in)).abs()
        > 0.000_001
    {
        bail!("clip source and timeline durations must match");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edl() -> TimelineEdl {
        TimelineEdl {
            main_tracks: vec![MainTrack {
                name: None,
                clips: vec![
                    Clip {
                        source: "a.mp4".into(),
                        source_in: 0.0,
                        source_out: 3.0,
                        timeline_in: 0.0,
                        timeline_out: 3.0,
                    },
                    Clip {
                        source: "b.mp4".into(),
                        source_in: 1.0,
                        source_out: 3.0,
                        timeline_in: 3.0,
                        timeline_out: 5.0,
                    },
                ],
            }],
            overlay_tracks: vec![OverlayTrack::Hold {
                track: None,
                source_time: 1.0,
                timeline_in: 1.0,
                timeline_out: 2.0,
            }],
            markers: vec![],
            transitions: vec![],
            variants: vec![],
            opening_hook: OpeningHook::default(),
        }
    }
    #[test]
    fn deterministic_hash() {
        assert_eq!(
            edl().canonical_sha256().unwrap(),
            edl().canonical_sha256().unwrap()
        );
    }
    #[test]
    fn rejects_gap() {
        let mut value = edl();
        value.main_tracks[0].clips[1].timeline_in = 3.1;
        value.main_tracks[0].clips[1].source_out = 2.9;
        assert!(value
            .validate()
            .unwrap_err()
            .to_string()
            .contains("continuous"));
    }
    #[test]
    fn rejects_overlay_escape() {
        let mut value = edl();
        value.overlay_tracks[0] = OverlayTrack::Hold {
            track: None,
            source_time: 1.0,
            timeline_in: 4.0,
            timeline_out: 6.0,
        };
        assert!(value.validate().is_err());
    }

    #[test]
    fn validates_each_main_track_and_requires_equal_durations() {
        let mut value = edl();
        value.main_tracks.push(MainTrack {
            name: Some("alternate".into()),
            clips: vec![Clip {
                source: "c.mp4".into(),
                source_in: 0.0,
                source_out: 4.0,
                timeline_in: 0.0,
                timeline_out: 4.0,
            }],
        });
        assert!(value
            .validate()
            .unwrap_err()
            .to_string()
            .contains("same duration"));
        value.main_tracks[1].clips[0].source_out = 5.0;
        value.main_tracks[1].clips[0].timeline_out = 5.0;
        assert!(value.validate().is_ok());
    }

    #[test]
    fn rejects_transition_whose_end_exceeds_timeline() {
        let mut value = edl();
        value.transitions.push(Transition {
            track: None,
            kind: "cross_dissolve".into(),
            timeline_time: 4.8,
            duration: 0.3,
        });
        assert!(value
            .validate()
            .unwrap_err()
            .to_string()
            .contains("exceeds timeline"));
        value.transitions[0].timeline_time = f64::MAX;
        value.transitions[0].duration = f64::MAX;
        assert!(value.validate().is_err());
    }

    #[test]
    fn variant_keys_distinguish_same_language_outputs() {
        let portrait = VariantSpec {
            language: "EN".into(),
            aspect: AspectRatio::Portrait,
            subtitles: Some("en.ass".into()),
            watermark: None,
            cta: None,
        };
        let square = VariantSpec {
            aspect: AspectRatio::Square,
            ..portrait.clone()
        };
        assert_eq!(variant_key(0, &portrait), "v001-en-9x16");
        assert_eq!(variant_key(1, &square), "v002-en-1x1");
        assert_ne!(variant_key(0, &portrait), variant_key(1, &square));
    }
}
