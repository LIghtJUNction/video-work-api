use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Component, Path};

use serde::Serialize;

use crate::timeline::{
    AspectRatio, Clip, MainTrack, Marker, OverlayTrack, TimelineEdl, Transition, VariantSpec,
};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VpeDocument {
    pub project_name: String,
    pub canvas: CanvasSpec,
    pub sources: BTreeMap<String, String>,
    pub timeline: TimelineEdl,
    pub cuts: Vec<CutSpec>,
    pub hold_sources: Vec<HoldSource>,
    pub gates: Vec<GateSpec>,
    pub export: Option<XryExport>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CanvasSpec {
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CutSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    pub timeline_time: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HoldSource {
    pub source: String,
    pub timeline_in: f64,
    pub timeline_out: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GateSpec {
    pub phase: String,
    pub requirements: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct XryExport {
    pub task_dir: String,
    pub subject_id: String,
    pub encoder_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpeError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl VpeError {
    fn new(location: Location, message: impl Into<String>) -> Self {
        Self {
            line: location.line,
            column: location.column,
            message: message.into(),
        }
    }
}

impl fmt::Display for VpeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for VpeError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Location {
    line: usize,
    column: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Word(String),
    String(String),
    LeftBrace,
    RightBrace,
    LeftParen,
    RightParen,
    Equals,
    Comma,
    At,
    Range,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
struct Token {
    kind: TokenKind,
    location: Location,
}

pub fn parse(input: &str) -> Result<VpeDocument, VpeError> {
    let tokens = lex(input)?;
    Parser::new(tokens).parse_document()
}

fn lex(input: &str) -> Result<Vec<Token>, VpeError> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    let mut line = 1;
    let mut column = 1;
    while index < chars.len() {
        let ch = chars[index];
        if ch.is_whitespace() {
            advance(ch, &mut line, &mut column);
            index += 1;
            continue;
        }
        let location = Location { line, column };
        let single = match ch {
            '{' => Some(TokenKind::LeftBrace),
            '}' => Some(TokenKind::RightBrace),
            '(' => Some(TokenKind::LeftParen),
            ')' => Some(TokenKind::RightParen),
            '=' => Some(TokenKind::Equals),
            ',' => Some(TokenKind::Comma),
            '@' => Some(TokenKind::At),
            _ => None,
        };
        if let Some(kind) = single {
            tokens.push(Token { kind, location });
            advance(ch, &mut line, &mut column);
            index += 1;
            continue;
        }
        if ch == '.' && chars.get(index + 1) == Some(&'.') {
            tokens.push(Token {
                kind: TokenKind::Range,
                location,
            });
            index += 2;
            column += 2;
            continue;
        }
        if ch == '"' {
            index += 1;
            column += 1;
            let mut value = String::new();
            let mut closed = false;
            while index < chars.len() {
                let current = chars[index];
                if current == '"' {
                    index += 1;
                    column += 1;
                    closed = true;
                    break;
                }
                if current == '\n' || current == '\r' {
                    return Err(VpeError::new(
                        location,
                        "quoted strings cannot contain newlines",
                    ));
                }
                if current == '\\' {
                    let escape = chars.get(index + 1).copied().ok_or_else(|| {
                        VpeError::new(location, "unterminated escape in quoted string")
                    })?;
                    let decoded = match escape {
                        '"' => '"',
                        '\\' => '\\',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        _ => {
                            return Err(VpeError::new(
                                Location { line, column },
                                format!("unsupported escape sequence \\{escape}"),
                            ));
                        }
                    };
                    value.push(decoded);
                    index += 2;
                    column += 2;
                    continue;
                }
                value.push(current);
                index += 1;
                column += 1;
            }
            if !closed {
                return Err(VpeError::new(location, "unterminated quoted string"));
            }
            tokens.push(Token {
                kind: TokenKind::String(value),
                location,
            });
            continue;
        }
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.') {
            let start = index;
            while index < chars.len() {
                let current = chars[index];
                if current == '.' && chars.get(index + 1) == Some(&'.') {
                    break;
                }
                if !(current.is_ascii_alphanumeric() || matches!(current, '_' | '-' | ':' | '.')) {
                    break;
                }
                index += 1;
                column += 1;
            }
            tokens.push(Token {
                kind: TokenKind::Word(chars[start..index].iter().collect()),
                location,
            });
            continue;
        }
        return Err(VpeError::new(
            location,
            format!("unexpected character '{ch}'"),
        ));
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        location: Location { line, column },
    });
    Ok(tokens)
}

fn advance(ch: char, line: &mut usize, column: &mut usize) {
    if ch == '\n' {
        *line += 1;
        *column = 1;
    } else {
        *column += 1;
    }
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            position: 0,
        }
    }

    fn parse_document(mut self) -> Result<VpeDocument, VpeError> {
        self.expect_word("project")?;
        let project_name = self.expect_string("project name")?;
        if project_name.trim().is_empty() {
            return self.fail_current("project name cannot be empty");
        }
        self.expect(TokenKind::LeftBrace, "'{' after project name")?;

        let mut canvas = None;
        let mut sources = BTreeMap::new();
        let mut timeline = None;
        let mut cuts = Vec::new();
        let mut hold_sources = Vec::new();
        let mut gates = Vec::new();
        let mut export = None;

        while !self.consume(&TokenKind::RightBrace) {
            let keyword = self.peek_word()?.to_string();
            match keyword.as_str() {
                "canvas" => {
                    if canvas.is_some() {
                        return self.fail_current("canvas may be declared only once");
                    }
                    canvas = Some(self.parse_canvas()?);
                }
                "source" => self.parse_source(&mut sources)?,
                "timeline" => {
                    if timeline.is_some() {
                        return self.fail_current("timeline may be declared only once");
                    }
                    let result = self.parse_timeline(canvas.as_ref())?;
                    cuts = result.cuts;
                    hold_sources = result.hold_sources;
                    timeline = Some(result.edl);
                }
                "marker" => {
                    let timeline = timeline
                        .as_mut()
                        .ok_or_else(|| self.error_current("timeline must precede markers"))?;
                    timeline.markers.push(self.parse_marker()?);
                }
                "variant" => {
                    let timeline = timeline
                        .as_mut()
                        .ok_or_else(|| self.error_current("timeline must precede variants"))?;
                    timeline.variants.push(self.parse_variant()?);
                }
                "gate" => gates.push(self.parse_gate()?),
                "export" => {
                    if export.is_some() {
                        return self.fail_current("export may be declared only once");
                    }
                    export = Some(self.parse_export()?);
                }
                unknown => {
                    return self.fail_current(format!("unknown project statement '{unknown}'"));
                }
            }
        }
        self.expect(TokenKind::Eof, "end of file")?;

        let canvas = canvas.ok_or_else(|| self.error_current("missing canvas declaration"))?;
        let timeline = timeline.ok_or_else(|| self.error_current("missing timeline block"))?;
        let document = VpeDocument {
            project_name,
            canvas,
            sources,
            timeline,
            cuts,
            hold_sources,
            gates,
            export,
        };
        document
            .validate()
            .map_err(|message| VpeError::new(Location { line: 1, column: 1 }, message))?;
        Ok(document)
    }

    fn parse_canvas(&mut self) -> Result<CanvasSpec, VpeError> {
        self.expect_word("canvas")?;
        let location = self.current().location;
        let dimensions = self.expect_any_word("canvas dimensions")?;
        let (width, height) = dimensions
            .split_once('x')
            .ok_or_else(|| VpeError::new(location, "canvas must use WIDTHxHEIGHT"))?;
        let width = parse_positive_u32(width, location, "canvas width")?;
        let height = parse_positive_u32(height, location, "canvas height")?;
        self.expect(TokenKind::At, "'@' before frame rate")?;
        let rate_location = self.current().location;
        let rate = self.expect_any_word("frame rate")?;
        let rate = rate
            .strip_suffix("fps")
            .ok_or_else(|| VpeError::new(rate_location, "frame rate must end with fps"))?;
        let frame_rate = rate
            .parse::<f64>()
            .map_err(|_| VpeError::new(rate_location, "invalid frame rate"))?;
        if !frame_rate.is_finite() || frame_rate <= 0.0 || frame_rate > 240.0 {
            return Err(VpeError::new(
                rate_location,
                "frame rate must be greater than 0 and at most 240fps",
            ));
        }
        Ok(CanvasSpec {
            width,
            height,
            frame_rate,
        })
    }

    fn parse_source(&mut self, sources: &mut BTreeMap<String, String>) -> Result<(), VpeError> {
        self.expect_word("source")?;
        let location = self.current().location;
        let alias = self.expect_identifier("source alias")?;
        self.expect(TokenKind::Equals, "'=' after source alias")?;
        let path = self.expect_string("source path")?;
        if path.trim().is_empty() {
            return Err(VpeError::new(location, "source path cannot be empty"));
        }
        if !is_safe_asset_path(&path) {
            return Err(VpeError::new(
                location,
                "source path must be a sandboxed relative path",
            ));
        }
        if sources.insert(alias.clone(), path).is_some() {
            return Err(VpeError::new(
                location,
                format!("source alias '{alias}' is declared more than once"),
            ));
        }
        Ok(())
    }

    fn parse_timeline(&mut self, canvas: Option<&CanvasSpec>) -> Result<TimelineResult, VpeError> {
        self.expect_word("timeline")?;
        self.expect(TokenKind::LeftBrace, "'{' after timeline")?;
        let mut main_tracks = Vec::new();
        let mut overlay_tracks = Vec::new();
        let mut transitions = Vec::new();
        let mut cuts = Vec::new();
        let mut hold_sources = Vec::new();
        let mut track_names = HashSet::new();
        while !self.consume(&TokenKind::RightBrace) {
            self.expect_word("track")?;
            let location = self.current().location;
            let category = self.expect_identifier("track category")?;
            let (name, overlay_kind) = match category.as_str() {
                "main" => {
                    let name = if self.current().kind == TokenKind::LeftBrace {
                        None
                    } else {
                        Some(self.expect_identifier("main track name")?)
                    };
                    (name, None)
                }
                "overlay" => {
                    if self.current().kind == TokenKind::LeftBrace {
                        (None, Some(OverlayKind::Broll))
                    } else {
                        let name = Some(self.expect_identifier("overlay track name")?);
                        let kind = if self.consume_word("type") {
                            let kind_location = self.current().location;
                            match self.expect_identifier("overlay track type")?.as_str() {
                                "broll" => OverlayKind::Broll,
                                "pip" => OverlayKind::Pip,
                                "effect" => OverlayKind::Effect,
                                _ => {
                                    return Err(VpeError::new(
                                        kind_location,
                                        "overlay track type must be broll, pip, or effect",
                                    ));
                                }
                            }
                        } else {
                            OverlayKind::Broll
                        };
                        (name, Some(kind))
                    }
                }
                _ => {
                    return Err(VpeError::new(
                        location,
                        "track category must be main or overlay",
                    ));
                }
            };
            let track_key = format!(
                "{category}:{}",
                name.as_deref().unwrap_or(if category == "main" {
                    "main"
                } else {
                    "overlay"
                })
            );
            if !track_names.insert(track_key) {
                return Err(VpeError::new(
                    location,
                    format!(
                        "{category} track '{}' is declared more than once",
                        name.as_deref().unwrap_or(&category)
                    ),
                ));
            }
            self.expect(TokenKind::LeftBrace, "'{' after track declaration")?;
            let mut clips = Vec::new();
            while !self.consume(&TokenKind::RightBrace) {
                let keyword = self.peek_word()?.to_string();
                match keyword.as_str() {
                    "clip" => {
                        let clip = self.parse_clip()?;
                        if category == "main" {
                            clips.push(clip);
                        } else {
                            match overlay_kind.unwrap() {
                                OverlayKind::Broll => overlay_tracks.push(OverlayTrack::Broll {
                                    track: name.clone(),
                                    clip,
                                }),
                                OverlayKind::Pip => overlay_tracks.push(OverlayTrack::Pip {
                                    track: name.clone(),
                                    clip,
                                }),
                                OverlayKind::Effect => {
                                    return self.fail_previous(
                                        "effect overlay tracks cannot contain clips",
                                    );
                                }
                            }
                        }
                    }
                    "cut" if category == "main" => cuts.push(self.parse_cut(name.clone())?),
                    "transition" => {
                        if category != "main" {
                            return self.fail_current("transitions are only valid in main tracks");
                        }
                        transitions.push(
                            self.parse_transition(name.clone(), canvas.map(|c| c.frame_rate))?,
                        )
                    }
                    "hold" => {
                        if category != "overlay" || overlay_kind == Some(OverlayKind::Effect) {
                            return self.fail_current(
                                "holds are only valid in broll or pip overlay tracks",
                            );
                        }
                        let (hold, binding) = self.parse_hold(name.clone())?;
                        overlay_tracks.push(hold);
                        hold_sources.push(binding);
                    }
                    "effect" if overlay_kind == Some(OverlayKind::Effect) => {
                        overlay_tracks.push(self.parse_effect(name.clone())?);
                    }
                    unknown => {
                        return self.fail_current(format!("unknown track statement '{unknown}'"));
                    }
                }
            }
            if category == "main" {
                main_tracks.push(MainTrack { name, clips });
            }
        }
        Ok(TimelineResult {
            edl: TimelineEdl {
                main_tracks,
                overlay_tracks,
                markers: Vec::new(),
                transitions,
                variants: Vec::new(),
                opening_hook: Default::default(),
            },
            cuts,
            hold_sources,
        })
    }

    fn parse_clip(&mut self) -> Result<Clip, VpeError> {
        self.expect_word("clip")?;
        let source = self.expect_identifier("clip source alias")?;
        self.expect_word("source")?;
        let source_in = self.expect_time("clip source in")?;
        self.expect(TokenKind::Range, "'..' in clip source range")?;
        let source_out = self.expect_time("clip source out")?;
        self.expect_word("at")?;
        let timeline_in = self.expect_time("clip timeline in")?;
        Ok(Clip {
            source,
            source_in,
            source_out,
            timeline_in,
            timeline_out: timeline_in + (source_out - source_in),
        })
    }

    fn parse_cut(&mut self, track: Option<String>) -> Result<CutSpec, VpeError> {
        self.expect_word("cut")?;
        self.expect_word("at")?;
        Ok(CutSpec {
            track,
            timeline_time: self.expect_time("cut time")?,
        })
    }

    fn parse_transition(
        &mut self,
        track: Option<String>,
        frame_rate: Option<f64>,
    ) -> Result<Transition, VpeError> {
        self.expect_word("transition")?;
        let kind = self.expect_identifier("transition kind")?;
        self.expect_word("at")?;
        let timeline_time = self.expect_time("transition time")?;
        self.expect_word("duration")?;
        let location = self.current().location;
        let duration = self.expect_any_word("transition duration")?;
        let duration = if let Some(frames) = duration.strip_suffix('f') {
            let frame_rate = frame_rate.ok_or_else(|| {
                VpeError::new(location, "canvas must precede frame-based transitions")
            })?;
            parse_positive_u32(frames, location, "transition frames")? as f64 / frame_rate
        } else {
            parse_time(&duration, location)?
        };
        Ok(Transition {
            track,
            kind,
            timeline_time,
            duration,
        })
    }

    fn parse_hold(
        &mut self,
        track: Option<String>,
    ) -> Result<(OverlayTrack, HoldSource), VpeError> {
        self.expect_word("hold")?;
        let source = self.expect_identifier("hold source alias")?;
        self.expect_word("at")?;
        let timeline_in = self.expect_time("hold timeline in")?;
        self.expect(TokenKind::Range, "'..' in hold range")?;
        let timeline_out = self.expect_time("hold timeline out")?;
        self.expect_word("source_time")?;
        let source_time = self.expect_time("hold source time")?;
        Ok((
            OverlayTrack::Hold {
                track,
                source_time,
                timeline_in,
                timeline_out,
            },
            HoldSource {
                source,
                timeline_in,
                timeline_out,
            },
        ))
    }

    fn parse_effect(&mut self, track: Option<String>) -> Result<OverlayTrack, VpeError> {
        self.expect_word("effect")?;
        let name = self.expect_identifier("effect name")?;
        self.expect_word("at")?;
        let timeline_in = self.expect_time("effect timeline in")?;
        self.expect(TokenKind::Range, "'..' in effect range")?;
        let timeline_out = self.expect_time("effect timeline out")?;
        Ok(OverlayTrack::Effect {
            track,
            timeline_in,
            timeline_out,
            name,
        })
    }

    fn parse_marker(&mut self) -> Result<Marker, VpeError> {
        self.expect_word("marker")?;
        let name = self.expect_string("marker name")?;
        self.expect_word("at")?;
        Ok(Marker {
            name,
            timeline_time: self.expect_time("marker time")?,
        })
    }

    fn parse_variant(&mut self) -> Result<VariantSpec, VpeError> {
        self.expect_word("variant")?;
        let language = self.expect_string("variant language")?;
        self.expect_word("aspect")?;
        let location = self.current().location;
        let aspect = match self.expect_any_word("variant aspect")?.as_str() {
            "9:16" => AspectRatio::Portrait,
            "16:9" => AspectRatio::Landscape,
            "1:1" => AspectRatio::Square,
            _ => {
                return Err(VpeError::new(
                    location,
                    "variant aspect must be 9:16, 16:9, or 1:1",
                ));
            }
        };
        let mut subtitles = None;
        let mut watermark = None;
        let mut cta = None;
        while let TokenKind::Word(key) = &self.current().kind {
            let key = key.clone();
            if !matches!(key.as_str(), "subtitles" | "watermark" | "cta") {
                break;
            }
            let key_location = self.current().location;
            self.position += 1;
            let value = self.expect_string(&format!("variant {key}"))?;
            if value.trim().is_empty() {
                return Err(VpeError::new(
                    key_location,
                    format!("variant {key} cannot be empty"),
                ));
            }
            match key.as_str() {
                "subtitles" if subtitles.is_none() => {
                    if !is_safe_asset_path(&value) {
                        return Err(VpeError::new(
                            key_location,
                            "variant subtitles must be a sandboxed relative path",
                        ));
                    }
                    subtitles = Some(value);
                }
                "watermark" if watermark.is_none() => {
                    if !is_safe_asset_path(&value) {
                        return Err(VpeError::new(
                            key_location,
                            "variant watermark must be a sandboxed relative path",
                        ));
                    }
                    watermark = Some(value);
                }
                "cta" if cta.is_none() => cta = Some(value),
                _ => {
                    return Err(VpeError::new(
                        key_location,
                        format!("duplicate variant field '{key}'"),
                    ));
                }
            }
        }
        if subtitles.is_none() && watermark.is_none() && cta.is_none() {
            return Err(VpeError::new(
                location,
                "variant must declare subtitles, watermark, or cta",
            ));
        }
        Ok(VariantSpec {
            language,
            aspect,
            watermark,
            cta,
            subtitles,
        })
    }

    fn parse_gate(&mut self) -> Result<GateSpec, VpeError> {
        self.expect_word("gate")?;
        let location = self.current().location;
        let phase = self.expect_identifier("gate phase")?;
        if !matches!(phase.as_str(), "pre_render" | "pre_package" | "acceptance") {
            return Err(VpeError::new(
                location,
                "gate phase must be pre_render, pre_package, or acceptance",
            ));
        }
        self.expect_word("require")?;
        let mut requirements = Vec::new();
        loop {
            let requirement = self.expect_identifier("gate requirement")?;
            if !is_known_gate_requirement(&requirement) {
                return self.fail_previous(format!("unknown gate requirement '{requirement}'"));
            }
            if requirements.contains(&requirement) {
                return self.fail_previous(format!("duplicate gate requirement '{requirement}'"));
            }
            requirements.push(requirement);
            if !self.consume(&TokenKind::Comma) {
                break;
            }
        }
        Ok(GateSpec {
            phase,
            requirements,
        })
    }

    fn parse_export(&mut self) -> Result<XryExport, VpeError> {
        self.expect_word("export")?;
        self.expect_word("xry")?;
        self.expect(TokenKind::LeftParen, "'(' after export xry")?;
        let mut task_dir = None;
        let mut subject_id = None;
        let mut encoder_profile = None;
        for index in 0..3 {
            let key_location = self.current().location;
            let key = self.expect_identifier("export argument name")?;
            self.expect(TokenKind::Equals, "'=' after export argument name")?;
            let value = self.expect_string("export argument value")?;
            match key.as_str() {
                "task_dir" if task_dir.is_none() => task_dir = Some(value),
                "subject_id" if subject_id.is_none() => subject_id = Some(value),
                "encoder_profile" if encoder_profile.is_none() => encoder_profile = Some(value),
                "task_dir" | "subject_id" | "encoder_profile" => {
                    return Err(VpeError::new(
                        key_location,
                        format!("duplicate export argument '{key}'"),
                    ));
                }
                _ => {
                    return Err(VpeError::new(
                        key_location,
                        format!("unknown export argument '{key}'"),
                    ));
                }
            }
            if index < 2 {
                self.expect(TokenKind::Comma, "',' between export arguments")?;
            }
        }
        self.expect(TokenKind::RightParen, "')' after export arguments")?;
        Ok(XryExport {
            task_dir: task_dir.ok_or_else(|| self.error_current("missing task_dir"))?,
            subject_id: subject_id.ok_or_else(|| self.error_current("missing subject_id"))?,
            encoder_profile: encoder_profile
                .ok_or_else(|| self.error_current("missing encoder_profile"))?,
        })
    }

    fn expect_time(&mut self, label: &str) -> Result<f64, VpeError> {
        let location = self.current().location;
        let value = self.expect_any_word(label)?;
        parse_time(&value, location)
    }

    fn expect_identifier(&mut self, label: &str) -> Result<String, VpeError> {
        let location = self.current().location;
        let value = self.expect_any_word(label)?;
        if value.is_empty()
            || !value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        {
            return Err(VpeError::new(
                location,
                format!("{label} must be an English identifier"),
            ));
        }
        Ok(value)
    }

    fn expect_string(&mut self, label: &str) -> Result<String, VpeError> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::String(value) => {
                self.position += 1;
                Ok(value)
            }
            _ => Err(VpeError::new(
                token.location,
                format!("expected {label} as a quoted string"),
            )),
        }
    }

    fn expect_any_word(&mut self, label: &str) -> Result<String, VpeError> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::Word(value) => {
                self.position += 1;
                Ok(value)
            }
            _ => Err(VpeError::new(token.location, format!("expected {label}"))),
        }
    }

    fn expect_word(&mut self, expected: &str) -> Result<(), VpeError> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::Word(value) if value == expected => {
                self.position += 1;
                Ok(())
            }
            TokenKind::Word(value) => Err(VpeError::new(
                token.location,
                format!("expected '{expected}', found '{value}'"),
            )),
            _ => Err(VpeError::new(
                token.location,
                format!("expected '{expected}'"),
            )),
        }
    }

    fn peek_word(&self) -> Result<&str, VpeError> {
        match &self.current().kind {
            TokenKind::Word(value) => Ok(value),
            _ => Err(self.error_current("expected an English statement keyword")),
        }
    }

    fn expect(&mut self, expected: TokenKind, label: &str) -> Result<(), VpeError> {
        if self.consume(&expected) {
            Ok(())
        } else {
            self.fail_current(format!("expected {label}"))
        }
    }

    fn consume(&mut self, expected: &TokenKind) -> bool {
        if &self.current().kind == expected {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn consume_word(&mut self, expected: &str) -> bool {
        if matches!(&self.current().kind, TokenKind::Word(value) if value == expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn current(&self) -> &Token {
        &self.tokens[self.position.min(self.tokens.len() - 1)]
    }

    fn error_current(&self, message: impl Into<String>) -> VpeError {
        VpeError::new(self.current().location, message)
    }

    fn fail_current<T>(&self, message: impl Into<String>) -> Result<T, VpeError> {
        Err(self.error_current(message))
    }

    fn fail_previous<T>(&self, message: impl Into<String>) -> Result<T, VpeError> {
        let location = self.tokens[self.position.saturating_sub(1)].location;
        Err(VpeError::new(location, message))
    }
}

struct TimelineResult {
    edl: TimelineEdl,
    cuts: Vec<CutSpec>,
    hold_sources: Vec<HoldSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverlayKind {
    Broll,
    Pip,
    Effect,
}

impl VpeDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self.sources.is_empty() {
            return Err("at least one source must be declared".into());
        }
        self.timeline
            .validate()
            .map_err(|error| error.to_string())?;
        for clip in self
            .timeline
            .main_tracks
            .iter()
            .flat_map(|track| &track.clips)
        {
            if !self.sources.contains_key(&clip.source) {
                return Err(format!("clip references unknown source '{}'", clip.source));
            }
        }
        for overlay in &self.timeline.overlay_tracks {
            if let OverlayTrack::Broll { clip, .. } | OverlayTrack::Pip { clip, .. } = overlay {
                if !self.sources.contains_key(&clip.source) {
                    return Err(format!(
                        "overlay clip references unknown source '{}'",
                        clip.source
                    ));
                }
            }
        }
        for hold in &self.hold_sources {
            if !self.sources.contains_key(&hold.source) {
                return Err(format!("hold references unknown source '{}'", hold.source));
            }
        }
        let duration = self.timeline.duration();
        let track_boundaries = self
            .timeline
            .main_tracks
            .iter()
            .map(|track| {
                (
                    track.name.clone(),
                    track
                        .clips
                        .iter()
                        .take(track.clips.len().saturating_sub(1))
                        .map(|clip| (clip.timeline_out * 1_000_000.0).round() as i64)
                        .collect::<HashSet<_>>(),
                )
            })
            .collect::<Vec<_>>();
        for cut in &self.cuts {
            if !cut.timeline_time.is_finite()
                || cut.timeline_time <= 0.0
                || cut.timeline_time >= duration
            {
                return Err("cut must be strictly inside the timeline".into());
            }
            let boundaries = track_boundaries
                .iter()
                .find(|(name, _)| name == &cut.track)
                .map(|(_, boundaries)| boundaries)
                .ok_or_else(|| "cut references an unknown main track".to_string())?;
            if !boundaries.contains(&((cut.timeline_time * 1_000_000.0).round() as i64)) {
                return Err("every cut must match its main track clip boundary".into());
            }
        }
        for (track, boundaries) in &track_boundaries {
            if boundaries.iter().any(|boundary| {
                !self.cuts.iter().any(|cut| {
                    cut.track == *track
                        && (cut.timeline_time * 1_000_000.0).round() as i64 == *boundary
                })
            }) {
                return Err("every main track clip boundary must have a cut declaration".into());
            }
        }
        for transition in &self.timeline.transitions {
            if transition.kind != "cross_dissolve" {
                return Err(format!(
                    "unsupported transition '{}'; supported: cross_dissolve",
                    transition.kind
                ));
            }
            if !transition.timeline_time.is_finite()
                || !transition.duration.is_finite()
                || transition.timeline_time < 0.0
                || transition.duration <= 0.0
                || !(transition.timeline_time + transition.duration).is_finite()
                || transition.timeline_time + transition.duration > duration
            {
                return Err("transition must have a valid time and positive duration".into());
            }
            let boundaries = track_boundaries
                .iter()
                .find(|(name, _)| name == &transition.track)
                .map(|(_, boundaries)| boundaries)
                .ok_or_else(|| "transition references an unknown main track".to_string())?;
            if !boundaries.contains(&((transition.timeline_time * 1_000_000.0).round() as i64)) {
                return Err("transition must match its main track clip boundary".into());
            }
        }
        for overlay in &self.timeline.overlay_tracks {
            if let OverlayTrack::Effect { name, .. } = overlay {
                if !matches!(name.as_str(), "grayscale" | "vignette") {
                    return Err(format!(
                        "unsupported effect '{name}'; supported: grayscale, vignette"
                    ));
                }
            }
        }
        let has_opening_hook_gate = self
            .gates
            .iter()
            .any(|gate| gate.requirements.iter().any(|item| item == "opening_hook"));
        if has_opening_hook_gate {
            let window = &self.timeline.opening_hook;
            self.timeline
                .validate_opening_hook((window.min_seconds, window.max_seconds))
                .map_err(|error| error.to_string())?;
        }
        if let Some(export) = &self.export {
            if export.task_dir.trim().is_empty() {
                return Err("export task_dir cannot be empty".into());
            }
            if export.subject_id.len() < 3
                || export.subject_id.len() > 4
                || !export.subject_id.starts_with('S')
                || !export.subject_id[1..].chars().all(|ch| ch.is_ascii_digit())
            {
                return Err("export subject_id must match S followed by 2 or 3 digits".into());
            }
            if !matches!(
                export.encoder_profile.as_str(),
                "formal-cpu" | "formal-auto" | "formal-vaapi"
            ) {
                return Err(
                    "export encoder_profile must be formal-cpu, formal-auto, or formal-vaapi"
                        .into(),
                );
            }
        }
        Ok(())
    }
}

fn parse_positive_u32(value: &str, location: Location, label: &str) -> Result<u32, VpeError> {
    let parsed = value
        .parse::<u32>()
        .map_err(|_| VpeError::new(location, format!("invalid {label}")))?;
    if parsed == 0 {
        return Err(VpeError::new(location, format!("{label} must be positive")));
    }
    Ok(parsed)
}

fn parse_time(value: &str, location: Location) -> Result<f64, VpeError> {
    let mut parts = value.split(':');
    let hours = parts.next();
    let minutes = parts.next();
    let seconds = parts.next();
    if parts.next().is_some() || hours.is_none() || minutes.is_none() || seconds.is_none() {
        return Err(VpeError::new(location, "time must use HH:MM:SS.mmm"));
    }
    let hours_text = hours.unwrap();
    let minutes_text = minutes.unwrap();
    let seconds_text = seconds.unwrap();
    if hours_text.len() != 2
        || minutes_text.len() != 2
        || seconds_text.len() != 6
        || seconds_text.as_bytes().get(2) != Some(&b'.')
        || !hours_text
            .chars()
            .all(|character| character.is_ascii_digit())
        || !minutes_text
            .chars()
            .all(|character| character.is_ascii_digit())
        || !seconds_text[..2]
            .chars()
            .all(|character| character.is_ascii_digit())
        || !seconds_text[3..]
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err(VpeError::new(location, "time must use HH:MM:SS.mmm"));
    }
    let hours = hours_text.parse::<u64>();
    let minutes = minutes_text.parse::<u64>();
    let seconds = seconds_text.parse::<f64>();
    let (Ok(hours), Ok(minutes), Ok(seconds)) = (hours, minutes, seconds) else {
        return Err(VpeError::new(location, "invalid HH:MM:SS.mmm time"));
    };
    if minutes >= 60 || !seconds.is_finite() || !(0.0..60.0).contains(&seconds) {
        return Err(VpeError::new(location, "invalid HH:MM:SS.mmm time"));
    }
    Ok(hours as f64 * 3600.0 + minutes as f64 * 60.0 + seconds)
}

fn is_safe_asset_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn is_known_gate_requirement(requirement: &str) -> bool {
    matches!(
        requirement,
        "input_manifest"
            | "continuous_timeline"
            | "subtitle_overflow"
            | "opening_hook"
            | "output_specifications"
            | "cover_match"
            | "copy_consistency"
            | "deterministic_replay"
            | "faststart"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"project "Aurora Launch" {
  canvas 1080x1920 @ 30fps
  source host = "assets/host-take-03.mp4"
  source detail = "assets/product-detail.mp4"

  timeline {
    track main {
      clip host source 00:00:00.600..00:00:06.800 at 00:00:00.000
      cut at 00:00:06.200
      clip detail source 00:00:01.400..00:00:05.900 at 00:00:06.200
      transition cross_dissolve at 00:00:06.200 duration 12f
    }
    track overlay {
      hold host at 00:00:02.500..00:00:03.200 source_time 00:00:02.800
    }
  }

  marker "Opening hook" at 00:00:03.000
  variant "ZH-EN" aspect 9:16 subtitles "subs.zh-en.ass"
  gate pre_render require opening_hook, continuous_timeline
  export xry(task_dir = "campaign/batch-01", subject_id = "S01", encoder_profile = "formal-auto")
}"#;

    #[test]
    fn parses_design_spec_and_export() {
        let document = parse(SAMPLE).unwrap();
        assert_eq!(document.project_name, "Aurora Launch");
        assert_eq!(document.canvas.width, 1080);
        assert_eq!(document.timeline.main_tracks[0].clips.len(), 2);
        assert_eq!(document.cuts[0].timeline_time, 6.2);
        assert_eq!(document.timeline.transitions[0].duration, 0.4);
        assert_eq!(document.timeline.overlay_tracks.len(), 1);
        assert_eq!(
            document.timeline.variants[0].subtitles.as_deref(),
            Some("subs.zh-en.ass")
        );
        assert_eq!(document.export.unwrap().subject_id, "S01");
    }

    #[test]
    fn rejects_unknown_statement_with_location() {
        let input = SAMPLE.replace(
            "  marker \"Opening hook\"",
            "  wobble fast\n  marker \"Opening hook\"",
        );
        let error = parse(&input).unwrap_err();
        assert!(error.line > 1);
        assert_eq!(error.column, 3);
        assert!(error.message.contains("unknown project statement 'wobble'"));
    }

    #[test]
    fn rejects_unknown_export_argument() {
        let input = SAMPLE.replace("task_dir =", "folder =");
        let error = parse(&input).unwrap_err();
        assert!(error.message.contains("unknown export argument 'folder'"));
        assert!(error.to_string().starts_with("line "));
    }

    #[test]
    fn export_arguments_require_equals_with_location() {
        let input = SAMPLE.replace("task_dir =", "task_dir");
        let error = parse(&input).unwrap_err();
        assert!(error.line > 1);
        assert!(error.column > 1);
        assert!(error.message.contains("'=' after export argument name"));
    }

    #[test]
    fn parses_named_main_tracks_typed_overlays_and_all_variant_payloads() {
        let input = r#"project "Multi Track" {
  canvas 1920x1080 @ 30fps
  source host = "assets/host.mp4"
  source detail = "assets/detail.mp4"
  timeline {
    track main primary {
      clip host source 00:00:00.000..00:00:04.000 at 00:00:00.000
    }
    track main alternate {
      clip detail source 00:00:01.000..00:00:05.000 at 00:00:00.000
    }
    track overlay product type broll {
      clip detail source 00:00:00.000..00:00:01.000 at 00:00:01.000
    }
    track overlay presenter type pip {
      hold host at 00:00:02.000..00:00:03.000 source_time 00:00:01.500
    }
    track overlay graphics type effect {
      effect vignette at 00:00:03.000..00:00:03.500
    }
  }
  variant "EN" aspect 16:9 watermark "brand/logo.png" cta "Learn more"
  variant "ZH-EN" aspect 9:16 subtitles "subs/zh-en.ass" watermark "brand/logo.png" cta "Explore now"
}"#;
        let document = parse(input).unwrap();
        assert_eq!(document.timeline.main_tracks.len(), 2);
        assert_eq!(
            document.timeline.main_tracks[1].name.as_deref(),
            Some("alternate")
        );
        assert!(matches!(
            &document.timeline.overlay_tracks[0],
            OverlayTrack::Broll { track, .. } if track.as_deref() == Some("product")
        ));
        assert!(matches!(
            &document.timeline.overlay_tracks[1],
            OverlayTrack::Hold { track, .. } if track.as_deref() == Some("presenter")
        ));
        assert!(matches!(
            &document.timeline.overlay_tracks[2],
            OverlayTrack::Effect { track, .. } if track.as_deref() == Some("graphics")
        ));
        assert_eq!(
            document.timeline.variants[0].watermark.as_deref(),
            Some("brand/logo.png")
        );
        assert_eq!(
            document.timeline.variants[0].cta.as_deref(),
            Some("Learn more")
        );
        assert_eq!(
            document.timeline.variants[1].subtitles.as_deref(),
            Some("subs/zh-en.ass")
        );
    }

    #[test]
    fn rejects_variant_without_payload_and_hold_in_effect_track() {
        let no_payload = SAMPLE.replace(
            "variant \"ZH-EN\" aspect 9:16 subtitles \"subs.zh-en.ass\"",
            "variant \"ZH-EN\" aspect 9:16",
        );
        assert!(parse(&no_payload)
            .unwrap_err()
            .message
            .contains("must declare subtitles, watermark, or cta"));

        let bad_hold = SAMPLE.replace("track overlay {", "track overlay graphics type effect {");
        assert!(parse(&bad_hold)
            .unwrap_err()
            .message
            .contains("holds are only valid"));
    }

    #[test]
    fn rejects_transition_that_runs_past_timeline_end() {
        let input = SAMPLE.replace(
            "transition cross_dissolve at 00:00:06.200 duration 12f",
            "transition cross_dissolve at 00:00:10.600 duration 12f",
        );
        assert!(parse(&input)
            .unwrap_err()
            .message
            .contains("transition exceeds timeline duration"));
    }
}
