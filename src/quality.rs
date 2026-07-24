use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cover::CoverSpec;
use crate::provenance::sha256_file;
use crate::timeline::{variant_key, AspectRatio, TimelineEdl, VariantSpec};
use crate::vpe::VpeDocument;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    PreRender,
    PrePackage,
    Acceptance,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreRender => "pre-render",
            Self::PrePackage => "pre-package",
            Self::Acceptance => "acceptance",
        }
    }

    pub fn vpe_name(self) -> &'static str {
        match self {
            Self::PreRender => "pre_render",
            Self::PrePackage => "pre_package",
            Self::Acceptance => "acceptance",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidatePhaseRequest {
    pub phase: Phase,
    #[serde(default)]
    pub input_manifest: BTreeMap<String, String>,
    #[serde(default)]
    pub subtitle_overflow: Option<SubtitleOverflowEvidence>,
    #[serde(default)]
    pub deliverable_stem: Option<String>,
    #[serde(default)]
    pub master_output: Option<String>,
    #[serde(default)]
    pub variant_outputs: BTreeMap<String, String>,
    #[serde(default)]
    pub cover_jobs: BTreeMap<String, String>,
    #[serde(default)]
    pub copy_consistency: Option<CopyConsistencyEvidence>,
    #[serde(default)]
    pub job_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubtitleOverflowEvidence {
    pub checked: bool,
    pub overflow_count: u32,
    #[serde(default)]
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedCoverAttestation {
    pub project_id: String,
    pub revision: i64,
    pub document_sha256: String,
    pub variant_key: String,
    pub variant: VariantSpec,
    pub spec: CoverSpec,
    pub original_relative: String,
    pub final_relative: String,
    pub original_sha256: String,
    pub final_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CopyConsistencyEvidence {
    pub document_sha256: String,
    pub variant_sha256: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedRenderAttestation {
    pub report_relative: String,
    pub report_sha256: String,
    pub bundle_sha256: String,
    pub output_sha256: BTreeMap<String, String>,
    pub replay_sha256: BTreeMap<String, String>,
    pub replay_verified: bool,
    pub master_relative: String,
    pub variants: BTreeMap<String, TrustedVariant>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TrustedVariant {
    pub index: usize,
    pub language: String,
    pub aspect: AspectRatio,
    pub watermark: Option<String>,
    pub cta: Option<String>,
    pub output_relative: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub phase: Phase,
    pub passed: bool,
    pub checks: Vec<GateCheck>,
    pub artifact_sha256: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateCheck {
    pub gate: String,
    pub passed: bool,
    pub message: String,
}

pub struct GateContext<'a> {
    pub project_dir: &'a Path,
    pub document: &'a VpeDocument,
    pub project_id: &'a str,
    pub revision: i64,
    pub document_sha256: &'a str,
    pub request: &'a ValidatePhaseRequest,
    pub attestation: Option<&'a TrustedRenderAttestation>,
    pub cover_attestations: &'a BTreeMap<String, TrustedCoverAttestation>,
}

impl GateContext<'_> {
    fn timeline(&self) -> &TimelineEdl {
        &self.document.timeline
    }
}

pub trait Gate: Send + Sync {
    fn name(&self) -> &'static str;
    fn applies(&self, phase: Phase) -> bool;
    fn check(&self, context: &GateContext<'_>) -> Result<GateCheck>;
}

pub struct Registry {
    gates: Vec<Box<dyn Gate>>,
}

impl Registry {
    pub fn built_in() -> Self {
        Self {
            gates: vec![
                Box::new(DeclaredGate),
                Box::new(InputManifestGate),
                Box::new(TimelineGate),
                Box::new(OpeningHookGate),
                Box::new(SubtitleOverflowGate),
                Box::new(VariantGate),
                Box::new(OutputSpecificationGate),
                Box::new(CoverMatchGate),
                Box::new(CopyConsistencyGate),
                Box::new(RenderReportGate),
                Box::new(DeterministicReplayGate),
                Box::new(FaststartGate),
            ],
        }
    }

    pub fn validate(&self, context: &GateContext<'_>) -> ValidationReport {
        let checks = self
            .gates
            .iter()
            .filter(|gate| gate.applies(context.request.phase))
            .map(|gate| {
                gate.check(context).unwrap_or_else(|error| GateCheck {
                    gate: gate.name().into(),
                    passed: false,
                    message: error.to_string(),
                })
            })
            .collect::<Vec<_>>();
        let artifact_sha256 = artifact_paths(context)
            .into_iter()
            .filter_map(|(name, path)| sha256_file(&path).ok().map(|hash| (name, hash)))
            .collect();
        ValidationReport {
            phase: context.request.phase,
            passed: !checks.is_empty() && checks.iter().all(|check| check.passed),
            checks,
            artifact_sha256,
        }
    }
}

struct DeclaredGate;
impl Gate for DeclaredGate {
    fn name(&self) -> &'static str {
        "declared_required_gates"
    }
    fn applies(&self, _phase: Phase) -> bool {
        true
    }
    fn check(&self, context: &GateContext<'_>) -> Result<GateCheck> {
        let declaration = context
            .document
            .gates
            .iter()
            .find(|gate| gate.phase == context.request.phase.vpe_name())
            .context("VPE must declare required gates for this phase")?;
        let declared = declaration
            .requirements
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mandatory_gates = mandatory_gate_names(context.request.phase);
        for mandatory in mandatory_gates {
            if !declared.contains(mandatory) {
                bail!("VPE gate declaration is missing mandatory gate '{mandatory}'");
            }
        }
        for required in &declared {
            if !mandatory_gates.contains(required) {
                bail!(
                    "declared gate '{required}' is not executable in phase '{}'",
                    context.request.phase.as_str()
                );
            }
        }
        Ok(pass(
            self.name(),
            "declared gates are recognized and include every mandatory gate",
        ))
    }
}

fn mandatory_gate_names(phase: Phase) -> &'static [&'static str] {
    match phase {
        Phase::PreRender => &[
            "input_manifest",
            "continuous_timeline",
            "opening_hook",
            "subtitle_overflow",
        ],
        Phase::PrePackage => &["output_specifications", "cover_match", "copy_consistency"],
        Phase::Acceptance => &["deterministic_replay", "faststart"],
    }
}

struct InputManifestGate;
impl Gate for InputManifestGate {
    fn name(&self) -> &'static str {
        "input_manifest"
    }
    fn applies(&self, phase: Phase) -> bool {
        phase == Phase::PreRender
    }
    fn check(&self, context: &GateContext<'_>) -> Result<GateCheck> {
        if context.request.input_manifest.len() != context.document.sources.len() {
            bail!("input_manifest must bind every VPE source identity");
        }
        for (identity, relative) in &context.document.sources {
            let actual = sha256_file(&checked_project_file(context.project_dir, relative)?)?;
            if context.request.input_manifest.get(identity) != Some(&actual) {
                bail!("input_manifest hash mismatch for source '{identity}'");
            }
        }
        Ok(pass(
            self.name(),
            "all source identities and hashes match project assets",
        ))
    }
}

struct TimelineGate;
impl Gate for TimelineGate {
    fn name(&self) -> &'static str {
        "continuous_timeline"
    }
    fn applies(&self, phase: Phase) -> bool {
        phase == Phase::PreRender
    }
    fn check(&self, context: &GateContext<'_>) -> Result<GateCheck> {
        context.timeline().validate()?;
        Ok(pass(
            self.name(),
            "timeline is continuous and structurally valid",
        ))
    }
}

struct OpeningHookGate;
impl Gate for OpeningHookGate {
    fn name(&self) -> &'static str {
        "opening_hook"
    }
    fn applies(&self, phase: Phase) -> bool {
        phase == Phase::PreRender
    }
    fn check(&self, context: &GateContext<'_>) -> Result<GateCheck> {
        let window = &context.timeline().opening_hook;
        context
            .timeline()
            .validate_opening_hook((window.min_seconds, window.max_seconds))?;
        Ok(pass(
            self.name(),
            "opening hook marker is inside its declared window",
        ))
    }
}

struct SubtitleOverflowGate;
impl Gate for SubtitleOverflowGate {
    fn name(&self) -> &'static str {
        "subtitle_overflow"
    }
    fn applies(&self, phase: Phase) -> bool {
        phase == Phase::PreRender
    }
    fn check(&self, context: &GateContext<'_>) -> Result<GateCheck> {
        let evidence = context
            .request
            .subtitle_overflow
            .as_ref()
            .context("subtitle overflow evidence is required")?;
        if !evidence.checked || evidence.overflow_count != 0 {
            bail!("subtitle overflow evidence must be checked and report zero overflow");
        }
        let mut checked = 0usize;
        for variant in &context.timeline().variants {
            if let Some(relative) = &variant.subtitles {
                validate_ass_safe_area(&checked_project_file(context.project_dir, relative)?)?;
                checked += 1;
            }
        }
        if checked == 0 {
            bail!("at least one real ASS subtitle track must be checked");
        }
        Ok(pass(
            self.name(),
            "real ASS subtitle tracks fit the declared canvas",
        ))
    }
}

struct VariantGate;
impl Gate for VariantGate {
    fn name(&self) -> &'static str {
        "variant_specs"
    }
    fn applies(&self, phase: Phase) -> bool {
        phase == Phase::PreRender
    }
    fn check(&self, context: &GateContext<'_>) -> Result<GateCheck> {
        if context.timeline().variants.is_empty() {
            bail!("at least one VariantSpec is required");
        }
        context.timeline().validate()?;
        Ok(pass(
            self.name(),
            "variant specifications are present and valid",
        ))
    }
}

struct OutputSpecificationGate;
impl Gate for OutputSpecificationGate {
    fn name(&self) -> &'static str {
        "output_specifications"
    }
    fn applies(&self, phase: Phase) -> bool {
        phase == Phase::PrePackage
    }
    fn check(&self, context: &GateContext<'_>) -> Result<GateCheck> {
        let attestation = context
            .attestation
            .context("a succeeded render job attestation is required")?;
        let master = context
            .request
            .master_output
            .as_deref()
            .context("master_output is required")?;
        if master != attestation.master_relative {
            bail!("master_output does not match the trusted render attestation");
        }
        probe_output(context, "master", master, None)?;
        if context.request.variant_outputs.len() != context.timeline().variants.len() {
            bail!("variant_outputs must bind every declared VariantSpec");
        }
        for (index, variant) in context.timeline().variants.iter().enumerate() {
            let key = variant_key(index, variant);
            let relative = context
                .request
                .variant_outputs
                .get(&key)
                .with_context(|| format!("missing output for variant '{key}'"))?;
            let trusted = attestation
                .variants
                .get(&key)
                .with_context(|| format!("trusted render lacks variant '{key}'"))?;
            if trusted.index != index
                || trusted.language != variant.language
                || trusted.aspect != variant.aspect
                || trusted.watermark != variant.watermark
                || trusted.cta != variant.cta
                || trusted.output_relative != *relative
            {
                bail!("variant '{key}' identity or output path disagrees with attestation");
            }
            probe_output(context, &key, relative, Some(&variant.aspect))?;
        }
        let paths = context
            .request
            .variant_outputs
            .values()
            .collect::<BTreeSet<_>>();
        if paths.len() != context.request.variant_outputs.len() {
            bail!("every variant must use a distinct output path");
        }
        Ok(pass(
            self.name(),
            "master and every variant pass ffprobe specifications",
        ))
    }
}

struct CoverMatchGate;
impl Gate for CoverMatchGate {
    fn name(&self) -> &'static str {
        "cover_match"
    }
    fn applies(&self, phase: Phase) -> bool {
        phase == Phase::PrePackage
    }
    fn check(&self, context: &GateContext<'_>) -> Result<GateCheck> {
        if context.request.cover_jobs.len() != context.timeline().variants.len()
            || context.cover_attestations.len() != context.timeline().variants.len()
        {
            bail!("cover_jobs must bind every declared variant");
        }
        let mut paths = BTreeSet::new();
        let mut hashes = BTreeSet::new();
        for (index, variant) in context.timeline().variants.iter().enumerate() {
            let key = variant_key(index, variant);
            let pair = context
                .cover_attestations
                .get(&key)
                .with_context(|| format!("missing cover pair for '{key}'"))?;
            if pair.project_id != context.project_id
                || pair.revision != context.revision
                || pair.document_sha256 != context.document_sha256
                || pair.variant_key != key
                || pair.variant != *variant
            {
                bail!("cover job identity or declared variant brand is stale");
            }
            let original = checked_project_file(context.project_dir, &pair.original_relative)?;
            let jpg = checked_project_file(context.project_dir, &pair.final_relative)?;
            if original.extension().and_then(|value| value.to_str()) != Some("png")
                || jpg.extension().and_then(|value| value.to_str()) != Some("jpg")
            {
                bail!("cover pair must contain original PNG and final JPG");
            }
            let jpg_stem = jpg
                .file_stem()
                .and_then(|value| value.to_str())
                .context("cover JPG filename is invalid")?;
            let expected_original = format!("{jpg_stem}-cover-original.png");
            if original.file_name().and_then(|value| value.to_str())
                != Some(expected_original.as_str())
            {
                bail!("cover original and JPG filenames do not form a matching pair");
            }
            let actual_original = sha256_file(&original)?;
            let actual_jpg = sha256_file(&jpg)?;
            if actual_original != pair.original_sha256 || actual_jpg != pair.final_sha256 {
                bail!("trusted cover attestation hashes do not match actual cover files");
            }
            decode_cover_image(&original, "cover original")?;
            decode_cover_image(&jpg, "cover JPEG")?;
            if !paths.insert(&pair.original_relative)
                || !paths.insert(&pair.final_relative)
                || !hashes.insert(actual_original)
                || !hashes.insert(actual_jpg)
            {
                bail!("every variant must use distinct cover paths and artifacts");
            }
        }
        Ok(pass(
            self.name(),
            "every variant has a hashed original PNG and final JPG",
        ))
    }
}

struct CopyConsistencyGate;
impl Gate for CopyConsistencyGate {
    fn name(&self) -> &'static str {
        "copy_consistency"
    }
    fn applies(&self, phase: Phase) -> bool {
        phase == Phase::PrePackage
    }
    fn check(&self, context: &GateContext<'_>) -> Result<GateCheck> {
        let evidence = context
            .request
            .copy_consistency
            .as_ref()
            .context("copy_consistency evidence is required")?;
        if evidence.document_sha256 != context.document_sha256 {
            bail!("copy consistency evidence targets a different document");
        }
        if evidence.variant_sha256.len() != context.timeline().variants.len()
            || context
                .timeline()
                .variants
                .iter()
                .enumerate()
                .any(|(index, variant)| {
                    !evidence
                        .variant_sha256
                        .contains_key(&variant_key(index, variant))
                })
        {
            bail!("copy consistency evidence must bind every declared variant");
        }
        if evidence
            .variant_sha256
            .values()
            .any(|hash| hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            bail!("copy consistency evidence contains a malformed SHA-256");
        }
        let attestation = context
            .attestation
            .context("copy consistency requires a succeeded render attestation")?;
        let mut paths = BTreeSet::new();
        let mut hashes = BTreeSet::new();
        for (index, variant) in context.timeline().variants.iter().enumerate() {
            let key = variant_key(index, variant);
            let relative = context
                .request
                .variant_outputs
                .get(&key)
                .with_context(|| format!("missing output for variant '{key}'"))?;
            if !paths.insert(relative) {
                bail!("variant output paths must be distinct");
            }
            let actual = sha256_file(&checked_project_file(context.project_dir, relative)?)?;
            if evidence.variant_sha256.get(&key) != Some(&actual) {
                bail!("copy consistency hash mismatch for variant '{key}'");
            }
            let trusted = attestation
                .variants
                .get(&key)
                .with_context(|| format!("trusted render lacks variant '{key}'"))?;
            if trusted.output_relative != *relative
                || trusted.sha256 != actual
                || attestation.output_sha256.get(relative) != Some(&actual)
            {
                bail!("variant '{key}' does not match the trusted render attestation");
            }
            if !hashes.insert(actual) {
                bail!("variant artifacts must not alias the same rendered bytes");
            }
        }
        Ok(pass(
            self.name(),
            "copy evidence binds this document and every variant",
        ))
    }
}

struct RenderReportGate;
impl Gate for RenderReportGate {
    fn name(&self) -> &'static str {
        "render_report"
    }
    fn applies(&self, phase: Phase) -> bool {
        phase == Phase::Acceptance
    }
    fn check(&self, context: &GateContext<'_>) -> Result<GateCheck> {
        let attestation = context
            .attestation
            .context("trusted render attestation is required")?;
        let path = checked_project_file(context.project_dir, &attestation.report_relative)?;
        if sha256_file(&path)? != attestation.report_sha256 {
            bail!("render report hash differs from the worker attestation");
        }
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        if value
            .get("document_sha256")
            .and_then(|value| value.as_str())
            != Some(context.document_sha256)
            || value.get("job_id").and_then(|value| value.as_str())
                != context.request.job_id.as_deref()
        {
            bail!("render report is not bound to this document and job");
        }
        if value.get("project_id").and_then(Value::as_str) != Some(context.project_id)
            || value.get("revision").and_then(Value::as_i64) != Some(context.revision)
            || value
                .get("replay_bundle_sha256")
                .and_then(Value::as_str)
                .is_none_or(|hash| {
                    hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
        {
            bail!("render report project, revision, or replay bundle binding mismatch");
        }
        let outputs = value
            .get("output_sha256")
            .and_then(|value| value.as_object())
            .context("render report must bind output_sha256")?;
        if outputs.is_empty() {
            bail!("render report has no output hashes");
        }
        for (relative, expected) in outputs {
            let actual = sha256_file(&checked_project_file(context.project_dir, relative)?)?;
            if expected.as_str() != Some(&actual) {
                bail!("render report output hash mismatch for '{relative}'");
            }
        }
        let canonical_mapping: BTreeMap<String, String> =
            serde_json::from_value(value["output_sha256"].clone())?;
        if canonical_mapping != attestation.output_sha256 {
            bail!("render report output map differs from the worker attestation");
        }
        let mut hasher = sha2::Sha256::new();
        use sha2::Digest;
        hasher.update(serde_json::to_vec(&canonical_mapping)?);
        if value.get("canonical_output_sha256").and_then(Value::as_str)
            != Some(format!("{:x}", hasher.finalize()).as_str())
        {
            bail!("render report canonical output hash mismatch");
        }
        Ok(pass(
            self.name(),
            "render report binds this job, document, and output hashes",
        ))
    }
}

struct DeterministicReplayGate;
impl Gate for DeterministicReplayGate {
    fn name(&self) -> &'static str {
        "deterministic_replay"
    }
    fn applies(&self, phase: Phase) -> bool {
        phase == Phase::Acceptance
    }
    fn check(&self, context: &GateContext<'_>) -> Result<GateCheck> {
        let attestation = context
            .attestation
            .context("trusted render attestation is required")?;
        if !attestation.replay_verified
            || attestation.output_sha256.is_empty()
            || attestation.output_sha256 != attestation.replay_sha256
        {
            bail!("deterministic replay hashes differ");
        }
        Ok(pass(
            self.name(),
            "canonical replay bundle and executor policy are verified",
        ))
    }
}

struct FaststartGate;
impl Gate for FaststartGate {
    fn name(&self) -> &'static str {
        "faststart"
    }
    fn applies(&self, phase: Phase) -> bool {
        phase == Phase::Acceptance
    }
    fn check(&self, context: &GateContext<'_>) -> Result<GateCheck> {
        let attestation = context
            .attestation
            .context("trusted render attestation is required")?;
        for relative in std::iter::once(&attestation.master_relative).chain(
            attestation
                .variants
                .values()
                .map(|variant| &variant.output_relative),
        ) {
            let mp4 = checked_project_file(context.project_dir, relative)?;
            if !is_faststart(&mp4)? {
                bail!("every final MP4 must contain ftyp with moov before mdat");
            }
        }
        Ok(pass(
            self.name(),
            "MP4 ftyp is valid and moov precedes mdat",
        ))
    }
}

fn pass(gate: &str, message: &str) -> GateCheck {
    GateCheck {
        gate: gate.into(),
        passed: true,
        message: message.into(),
    }
}

pub fn deliverable_paths(context: &GateContext<'_>) -> Result<Vec<(String, PathBuf)>> {
    let stem = context
        .request
        .deliverable_stem
        .as_deref()
        .context("deliverable_stem evidence is required")?;
    validate_single_component(stem)?;
    Ok([
        format!("{stem}.mp4"),
        format!("{stem}.txt"),
        format!("{stem}.jpg"),
        format!("{stem}-cover-original.png"),
    ]
    .into_iter()
    .map(|name| (name.clone(), context.project_dir.join("exports").join(name)))
    .collect())
}

fn artifact_paths(context: &GateContext<'_>) -> Vec<(String, PathBuf)> {
    let mut paths = Vec::new();
    if let Some(path) = &context.request.master_output {
        paths.push((path.clone(), context.project_dir.join(path)));
    }
    paths.extend(
        context
            .request
            .variant_outputs
            .iter()
            .map(|(id, path)| (format!("variant:{id}"), context.project_dir.join(path))),
    );
    for (id, pair) in context.cover_attestations {
        paths.push((
            format!("cover-original:{id}"),
            context.project_dir.join(&pair.original_relative),
        ));
        paths.push((
            format!("cover-jpg:{id}"),
            context.project_dir.join(&pair.final_relative),
        ));
    }
    if let Some(attestation) = context.attestation {
        paths.push((
            "render-report".into(),
            context.project_dir.join(&attestation.report_relative),
        ));
    }
    paths
}

fn checked_project_file(project_dir: &Path, relative: &str) -> Result<PathBuf> {
    let relative_path = Path::new(relative);
    if relative.is_empty()
        || relative_path.is_absolute()
        || !relative_path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        bail!("artifact path must be a safe project-relative path");
    }
    let path = project_dir.join(relative_path);
    let metadata =
        std::fs::symlink_metadata(&path).with_context(|| format!("missing artifact {relative}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        bail!("artifact {relative} must be a non-empty regular non-symlink file");
    }
    sha256_file(&path)?;
    Ok(path)
}

fn validate_single_component(value: &str) -> Result<()> {
    if value.is_empty()
        || Path::new(value).components().count() != 1
        || !Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        bail!("value must be a safe filename component");
    }
    Ok(())
}

fn validate_ass_safe_area(path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(path).context("read ASS subtitle track")?;
    let play_x = ass_header_number(&text, "PlayResX").context("ASS PlayResX is required")?;
    let play_y = ass_header_number(&text, "PlayResY").context("ASS PlayResY is required")?;
    if play_x <= 0 || play_y <= 0 || !text.lines().any(|line| line.starts_with("Dialogue:")) {
        bail!("ASS subtitle track lacks valid resolution or dialogue");
    }
    let styles = ass_styles(&text)?;
    for line in text.lines().filter(|line| line.starts_with("Dialogue:")) {
        if line.len() > 4096 {
            bail!("ASS dialogue is unbounded");
        }
        let fields = line
            .trim_start_matches("Dialogue:")
            .splitn(10, ',')
            .collect::<Vec<_>>();
        if fields.len() != 10 {
            bail!("ASS dialogue does not match the standard event format");
        }
        let style = styles
            .get(fields[3].trim())
            .with_context(|| format!("ASS dialogue references unknown style '{}'", fields[3]))?;
        let available_width = play_x - style.margin_l - style.margin_r;
        let available_height = play_y - (2 * style.margin_v);
        if available_width <= 0 || available_height < style.font_size * 2 {
            bail!("ASS style safe area is invalid");
        }
        let visible_lines = fields[9].split("\\N").collect::<Vec<_>>();
        if visible_lines.len() as f64 * style.font_size as f64 * 1.3 > available_height as f64 {
            bail!("ASS dialogue is estimated to overflow the vertical safe area");
        }
        for visible_line in visible_lines {
            let visible = strip_ass_overrides(visible_line);
            let estimated_width = visible.chars().count() as f64 * style.font_size as f64 * 0.55;
            if estimated_width > available_width as f64 {
                bail!("ASS dialogue is estimated to overflow the horizontal safe area");
            }
        }
        if let Some(position) = line
            .split("\\pos(")
            .nth(1)
            .and_then(|part| part.split(')').next())
        {
            let mut values = position
                .split(',')
                .filter_map(|value| value.trim().parse::<i64>().ok());
            let (Some(x), Some(y)) = (values.next(), values.next()) else {
                bail!("invalid ASS position override");
            };
            if x < 0 || x > play_x || y < 0 || y > play_y {
                bail!("ASS position is outside the canvas");
            }
        }
    }
    Ok(())
}

pub(crate) fn decode_cover_image(path: &Path, identity: &str) -> Result<()> {
    let probe = Command::new("/usr/bin/ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_type,width,height",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .with_context(|| format!("probe {identity}"))?;
    if !probe.status.success() {
        bail!("{identity} is not a decodable image");
    }
    let decoded = Command::new("/usr/bin/ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "null", "-"])
        .output()
        .with_context(|| format!("decode {identity}"))?;
    if !decoded.status.success() {
        bail!("{identity} failed fixed FFmpeg decode");
    }
    Ok(())
}

#[derive(Debug)]
struct AssStyle {
    font_size: i64,
    margin_l: i64,
    margin_r: i64,
    margin_v: i64,
}

fn ass_styles(text: &str) -> Result<BTreeMap<String, AssStyle>> {
    let mut in_styles = false;
    let mut format = Vec::<String>::new();
    let mut styles = BTreeMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_styles = trimmed.eq_ignore_ascii_case("[V4+ Styles]");
            continue;
        }
        if !in_styles {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("Format:") {
            format = value
                .split(',')
                .map(|item| item.trim().to_owned())
                .collect();
        } else if let Some(value) = trimmed.strip_prefix("Style:") {
            if format.is_empty() {
                bail!("ASS Style appears before its Format declaration");
            }
            let values = value.split(',').map(str::trim).collect::<Vec<_>>();
            if values.len() != format.len() {
                bail!("ASS Style does not match its Format declaration");
            }
            let field = |name: &str| -> Result<&str> {
                let index = format
                    .iter()
                    .position(|field| field.eq_ignore_ascii_case(name))
                    .with_context(|| format!("ASS style format lacks {name}"))?;
                values
                    .get(index)
                    .copied()
                    .context("ASS style value missing")
            };
            let name = field("Name")?.to_owned();
            styles.insert(
                name,
                AssStyle {
                    font_size: field("Fontsize")?.parse()?,
                    margin_l: field("MarginL")?.parse()?,
                    margin_r: field("MarginR")?.parse()?,
                    margin_v: field("MarginV")?.parse()?,
                },
            );
        }
    }
    if styles.is_empty() {
        bail!("ASS subtitle track has no V4+ styles");
    }
    Ok(styles)
}

fn strip_ass_overrides(text: &str) -> String {
    let mut visible = String::new();
    let mut in_override = false;
    for character in text.chars() {
        match character {
            '{' => in_override = true,
            '}' => in_override = false,
            _ if !in_override => visible.push(character),
            _ => {}
        }
    }
    visible
}

fn ass_header_number(text: &str, name: &str) -> Option<i64> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == name)
            .then(|| value.trim().parse().ok())
            .flatten()
    })
}

fn probe_output(
    context: &GateContext<'_>,
    identity: &str,
    relative: &str,
    aspect: Option<&AspectRatio>,
) -> Result<()> {
    let path = checked_project_file(context.project_dir, relative)?;
    let output = Command::new("/usr/bin/ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=width,height",
            "-of",
            "json",
        ])
        .arg(&path)
        .output()
        .with_context(|| format!("run ffprobe for {identity}"))?;
    if !output.status.success() {
        bail!("ffprobe rejected output '{identity}'");
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let duration = value
        .pointer("/format/duration")
        .and_then(|value| value.as_str())
        .and_then(|value| value.parse::<f64>().ok())
        .context("output duration is missing")?;
    if (duration - context.timeline().duration()).abs() > 0.12 {
        bail!("output '{identity}' duration is outside tolerance");
    }
    if let Some(aspect) = aspect {
        let stream = value
            .get("streams")
            .and_then(|value| value.as_array())
            .and_then(|streams| streams.iter().find(|item| item.get("width").is_some()))
            .context("output video stream is missing")?;
        let width = stream
            .get("width")
            .and_then(|value| value.as_u64())
            .context("output width is missing")?;
        let height = stream
            .get("height")
            .and_then(|value| value.as_u64())
            .context("output height is missing")?;
        let expected = match aspect {
            AspectRatio::Portrait => 9.0 / 16.0,
            AspectRatio::Landscape => 16.0 / 9.0,
            AspectRatio::Square => 1.0,
        };
        if ((width as f64 / height as f64) - expected).abs() > 0.02 {
            bail!("output '{identity}' aspect ratio does not match VariantSpec");
        }
    }
    Ok(())
}

pub fn is_faststart(path: &Path) -> Result<bool> {
    let mut file = crate::provenance::open_regular_nofollow(path)?;
    let length = file.metadata()?.len();
    let mut offset = 0_u64;
    let mut valid_ftyp = false;
    let mut moov = None;
    let mut mdat = None;
    while offset.saturating_add(8) <= length {
        file.seek(SeekFrom::Start(offset))?;
        let mut header = [0_u8; 8];
        file.read_exact(&mut header)?;
        let size32 = u32::from_be_bytes(header[..4].try_into().expect("four bytes")) as u64;
        let kind = &header[4..8];
        let (size, header_size) = if size32 == 1 {
            let mut extended = [0_u8; 8];
            file.read_exact(&mut extended)?;
            (u64::from_be_bytes(extended), 16)
        } else if size32 == 0 {
            (length - offset, 8)
        } else {
            (size32, 8)
        };
        if size < header_size || offset.saturating_add(size) > length {
            bail!("invalid MP4 box size");
        }
        match kind {
            b"ftyp" if offset == 0 && size >= 16 => valid_ftyp = true,
            b"moov" if moov.is_none() => moov = Some(offset),
            b"mdat" if mdat.is_none() => mdat = Some(offset),
            _ => {}
        }
        offset += size;
    }
    Ok(matches!(
        (valid_ftyp, moov, mdat),
        (true, Some(moov), Some(mdat)) if moov < mdat
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn mp4_box(kind: &[u8; 4]) -> Vec<u8> {
        [8_u32.to_be_bytes().as_slice(), kind].concat()
    }

    #[test]
    fn faststart_requires_ftyp_and_moov_before_mdat() {
        let root = tempdir().unwrap();
        let fast = root.path().join("fast.mp4");
        let ftyp = [
            16_u32.to_be_bytes().as_slice(),
            b"ftyp",
            b"isom",
            0_u32.to_be_bytes().as_slice(),
        ]
        .concat();
        fs::write(&fast, [ftyp, mp4_box(b"moov"), mp4_box(b"mdat")].concat()).unwrap();
        assert!(is_faststart(&fast).unwrap());
        let forged = root.path().join("forged.mp4");
        fs::write(&forged, [mp4_box(b"moov"), mp4_box(b"mdat")].concat()).unwrap();
        assert!(!is_faststart(&forged).unwrap());
    }

    #[test]
    fn caller_asserted_replay_fields_are_rejected() {
        let error = serde_json::from_value::<ValidatePhaseRequest>(serde_json::json!({
            "phase": "acceptance",
            "job_id": "job",
            "render_report": "exports/job/render-report.json",
            "deterministic_executor": false,
            "rerender_sha256": "forged"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn caller_asserted_cover_hashes_and_fake_images_are_rejected() {
        assert!(
            serde_json::from_value::<ValidatePhaseRequest>(serde_json::json!({
                "phase": "pre-package",
                "cover_hashes": {
                    "v001-en-9x16": {
                        "original": "exports/fake.png",
                        "jpg": "exports/fake.jpg",
                        "original_sha256": "00",
                        "jpg_sha256": "00"
                    }
                }
            }))
            .unwrap_err()
            .to_string()
            .contains("unknown field")
        );
        let root = tempdir().unwrap();
        let fake = root.path().join("fake.jpg");
        fs::write(&fake, b"not an image").unwrap();
        assert!(decode_cover_image(&fake, "fake cover").is_err());
    }
}
