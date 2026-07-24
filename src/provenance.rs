use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};

use anyhow::Context;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AuditReceipt {
    pub source_sha256: BTreeMap<String, String>,
    pub executor_sha256: String,
    pub timestamp: String,
    pub canonical_params: Value,
    pub previous_receipt_hash: Option<String>,
    pub receipt_hash: String,
    pub signature: String,
}

#[derive(Debug, Error)]
pub enum ReceiptError {
    #[error("capability unavailable: receipt signing key is not configured")]
    CapabilityUnavailable,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[derive(Serialize)]
struct UnsignedReceipt<'a> {
    source_sha256: &'a BTreeMap<String, String>,
    executor_sha256: &'a str,
    timestamp: &'a str,
    canonical_params: &'a Value,
    previous_receipt_hash: &'a Option<String>,
}

pub fn create_receipt(
    project_dir: &Path,
    receipt_name: &str,
    source_sha256: BTreeMap<String, String>,
    params: Value,
    previous_receipt_hash: Option<String>,
    key_file: &Path,
) -> Result<(AuditReceipt, PathBuf), ReceiptError> {
    validate_receipt_scope(receipt_name)?;
    let key = load_key(key_file)?;
    let executor = std::env::current_exe().context("resolve current executor")?;
    let executor_sha256 = sha256_file(&executor)?;
    let timestamp = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let canonical_params = canonicalize_value(params);
    let unsigned = UnsignedReceipt {
        source_sha256: &source_sha256,
        executor_sha256: &executor_sha256,
        timestamp: &timestamp,
        canonical_params: &canonical_params,
        previous_receipt_hash: &previous_receipt_hash,
    };
    let receipt_hash = hex::encode(Sha256::digest(
        serde_json::to_vec(&unsigned).context("serialize unsigned receipt")?,
    ));
    let signature = sign_hash(&key, &receipt_hash)?;
    let receipt = AuditReceipt {
        source_sha256,
        executor_sha256,
        timestamp,
        canonical_params,
        previous_receipt_hash,
        receipt_hash,
        signature,
    };
    validate_phase_params(&receipt)?;
    let receipts_dir = checked_receipts_dir(project_dir)?;
    let stage_dir = checked_stage_dir(&receipts_dir, receipt_name)?;
    let destination = stage_dir.join("audit_receipt.json");
    atomic_no_overwrite(
        &destination,
        &serde_json::to_vec_pretty(&receipt).context("serialize audit receipt")?,
    )?;
    Ok((receipt, destination))
}

pub fn signing_available(key_file: &Path) -> Result<(), ReceiptError> {
    load_key(key_file).map(|_| ())
}

pub fn prepare_receipt_scope(
    project_dir: &Path,
    receipt_scope: &str,
) -> Result<PathBuf, ReceiptError> {
    validate_receipt_scope(receipt_scope)?;
    let receipts_dir = checked_receipts_dir(project_dir)?;
    checked_stage_dir(&receipts_dir, receipt_scope)
}

pub fn verify_receipt(path: &Path, key_file: &Path) -> Result<AuditReceipt, ReceiptError> {
    let key = load_key(key_file)?;
    let mut file =
        open_regular_nofollow(path).with_context(|| format!("open receipt {}", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("read receipt {}", path.display()))?;
    let receipt: AuditReceipt =
        serde_json::from_slice(&bytes).map_err(|error| ReceiptError::Invalid(error.to_string()))?;
    validate_phase_params(&receipt)?;
    let unsigned = UnsignedReceipt {
        source_sha256: &receipt.source_sha256,
        executor_sha256: &receipt.executor_sha256,
        timestamp: &receipt.timestamp,
        canonical_params: &receipt.canonical_params,
        previous_receipt_hash: &receipt.previous_receipt_hash,
    };
    let expected_hash = hex::encode(Sha256::digest(
        serde_json::to_vec(&unsigned).context("serialize unsigned receipt")?,
    ));
    if expected_hash != receipt.receipt_hash {
        return Err(ReceiptError::Invalid("receipt hash mismatch".into()));
    }
    let signature = hex::decode(&receipt.signature)
        .map_err(|_| ReceiptError::Invalid("receipt signature is not hexadecimal".into()))?;
    let mut mac = HmacSha256::new_from_slice(&key)
        .map_err(|_| ReceiptError::Invalid("receipt signing key is invalid".into()))?;
    mac.update(receipt.receipt_hash.as_bytes());
    mac.verify_slice(&signature)
        .map_err(|_| ReceiptError::Invalid("receipt signature mismatch".into()))?;
    Ok(receipt)
}

fn validate_phase_params(receipt: &AuditReceipt) -> Result<(), ReceiptError> {
    let Some(phase) = receipt
        .canonical_params
        .get("phase")
        .and_then(Value::as_str)
    else {
        return Ok(());
    };
    if !matches!(phase, "pre-render" | "pre-package" | "acceptance") {
        return Err(ReceiptError::Invalid("receipt phase is invalid".into()));
    }
    let required_string = |name: &str| {
        receipt
            .canonical_params
            .get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ReceiptError::Invalid(format!("receipt lacks canonical {name}")))
    };
    required_string("project_id")?;
    let document = required_string("document_sha256")?;
    let report = required_string("validation_report_sha256")?;
    if !is_sha256(document) || !is_sha256(report) {
        return Err(ReceiptError::Invalid(
            "receipt document/report SHA-256 is malformed".into(),
        ));
    }
    if receipt
        .canonical_params
        .get("revision")
        .and_then(Value::as_i64)
        .is_none_or(|revision| revision < 1)
        || receipt
            .canonical_params
            .get("passed")
            .and_then(Value::as_bool)
            != Some(true)
        || !receipt
            .canonical_params
            .get("gate_results")
            .is_some_and(Value::is_array)
        || !receipt
            .canonical_params
            .get("source_sha256")
            .is_some_and(Value::is_object)
        || !receipt
            .canonical_params
            .get("output_sha256")
            .is_some_and(Value::is_object)
    {
        return Err(ReceiptError::Invalid(
            "receipt canonical phase parameters are incomplete".into(),
        ));
    }
    let bound_previous = receipt
        .canonical_params
        .get("previous_receipt_hash")
        .cloned()
        .unwrap_or(Value::Null);
    let expected_previous = receipt
        .previous_receipt_hash
        .as_ref()
        .map_or(Value::Null, |value| Value::String(value.clone()));
    if bound_previous != expected_previous {
        return Err(ReceiptError::Invalid(
            "canonical previous_receipt_hash mismatch".into(),
        ));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn verify_chain(paths: &[PathBuf], key_file: &Path) -> Result<Vec<AuditReceipt>, ReceiptError> {
    let mut receipts = Vec::with_capacity(paths.len());
    let mut previous = None;
    for path in paths {
        let receipt = verify_receipt(path, key_file)?;
        if receipt.previous_receipt_hash != previous {
            return Err(ReceiptError::Invalid(
                "receipt chain previous hash mismatch".into(),
            ));
        }
        previous = Some(receipt.receipt_hash.clone());
        receipts.push(receipt);
    }
    Ok(receipts)
}

pub fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file =
        open_regular_nofollow(path).with_context(|| format!("open {}", path.display()))?;
    sha256_reader(&mut file)
}

pub fn read_regular_with_sha256(path: &Path) -> anyhow::Result<(Vec<u8>, String)> {
    let mut file =
        open_regular_nofollow(path).with_context(|| format!("open {}", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let hash = hex::encode(Sha256::digest(&bytes));
    Ok((bytes, hash))
}

fn sha256_reader(file: &mut File) -> anyhow::Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

pub fn canonicalize_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted = map
                .into_iter()
                .map(|(key, value)| (key, canonicalize_value(value)))
                .collect::<BTreeMap<_, _>>();
            serde_json::to_value(sorted).expect("BTreeMap serializes")
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_value).collect()),
        other => other,
    }
}

fn sign_hash(key: &[u8], receipt_hash: &str) -> Result<String, ReceiptError> {
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|_| ReceiptError::Invalid("receipt signing key is invalid".into()))?;
    mac.update(receipt_hash.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn load_key(path: &Path) -> Result<Vec<u8>, ReceiptError> {
    let mut file = match open_regular_nofollow(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ReceiptError::CapabilityUnavailable);
        }
        Err(error) => return Err(anyhow::Error::from(error).into()),
    };
    let mut key = Vec::new();
    file.read_to_end(&mut key)
        .context("read receipt signing key")?;
    if key.is_empty() {
        return Err(ReceiptError::Invalid(
            "receipt signing key must not be empty".into(),
        ));
    }
    Ok(key)
}

fn checked_receipts_dir(project_dir: &Path) -> Result<PathBuf, ReceiptError> {
    let project = project_dir
        .canonicalize()
        .context("canonicalize video project")?;
    let receipts = project.join("receipts");
    reject_symlink(&receipts)?;
    let receipts = receipts
        .canonicalize()
        .context("canonicalize receipts directory")?;
    if !receipts.starts_with(&project) || !receipts.is_dir() {
        return Err(ReceiptError::Invalid(
            "receipts directory escaped the project".into(),
        ));
    }
    Ok(receipts)
}

fn checked_stage_dir(receipts_dir: &Path, stage: &str) -> Result<PathBuf, ReceiptError> {
    let mut path = receipts_dir.to_path_buf();
    for component in Path::new(stage).components() {
        let Component::Normal(name) = component else {
            return Err(ReceiptError::Invalid("invalid receipt scope".into()));
        };
        path.push(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(ReceiptError::Invalid(
                    "receipt stage path must be a regular non-symlink directory".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&path).context("create receipt stage directory")?;
                File::open(path.parent().expect("created directory has parent"))
                    .context("open receipt directory parent")?
                    .sync_all()
                    .context("sync receipt directory parent")?;
            }
            Err(error) => return Err(anyhow::Error::from(error).into()),
        }
    }
    let canonical = path
        .canonicalize()
        .context("canonicalize receipt stage directory")?;
    if !canonical.starts_with(receipts_dir) {
        return Err(ReceiptError::Invalid(
            "receipt stage directory escaped receipts root".into(),
        ));
    }
    Ok(canonical)
}

fn validate_receipt_scope(name: &str) -> Result<(), ReceiptError> {
    let path = Path::new(name);
    if name.is_empty()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '/'))
    {
        return Err(ReceiptError::Invalid("invalid receipt scope".into()));
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), ReceiptError> {
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(ReceiptError::Invalid(
            "receipt path must not be a symlink".into(),
        ));
    }
    Ok(())
}

fn atomic_no_overwrite(path: &Path, bytes: &[u8]) -> Result<(), ReceiptError> {
    let parent = path
        .parent()
        .ok_or_else(|| ReceiptError::Invalid("receipt path has no parent".into()))?;
    let temporary = parent.join(format!(".receipt-{}.tmp", Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::hard_link(&temporary, path)
            .with_context(|| format!("create no-overwrite receipt {}", path.display()))?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result.map_err(Into::into)
}

pub fn write_immutable(path: &Path, bytes: &[u8]) -> Result<(), ReceiptError> {
    atomic_no_overwrite(path, bytes)
}

pub fn verify_report_binding(
    receipt: &AuditReceipt,
    report_path: &Path,
) -> Result<(), ReceiptError> {
    let expected = receipt
        .canonical_params
        .get("validation_report_sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ReceiptError::Invalid("receipt does not bind validation_report_sha256".into())
        })?;
    let (bytes, actual) = read_regular_with_sha256(report_path)?;
    if expected != actual {
        return Err(ReceiptError::Invalid(
            "validation report hash does not match receipt".into(),
        ));
    }
    let report: Value = serde_json::from_slice(&bytes)
        .map_err(|error| ReceiptError::Invalid(format!("invalid validation report: {error}")))?;
    if report.get("passed").and_then(Value::as_bool) != Some(true) {
        return Err(ReceiptError::Invalid(
            "receipt-bound validation report is not PASS".into(),
        ));
    }
    Ok(())
}

pub fn open_regular_nofollow(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path must be a regular non-symlink file",
        ));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn signs_verifies_and_detects_tampering() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("receipts")).unwrap();
        let key = root.path().join("key");
        fs::write(&key, b"test-only-secret").unwrap();
        let (_, path) = create_receipt(
            root.path(),
            "pre-render",
            BTreeMap::from([("project.vpe".into(), "abc".into())]),
            serde_json::json!({"b": 2, "a": 1}),
            None,
            &key,
        )
        .unwrap();
        verify_receipt(&path, &key).unwrap();
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["canonical_params"]["a"] = serde_json::json!(99);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(verify_receipt(&path, &key).is_err());
    }

    #[test]
    fn missing_key_is_capability_unavailable() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("receipts")).unwrap();
        assert!(matches!(
            create_receipt(
                root.path(),
                "test",
                BTreeMap::new(),
                Value::Null,
                None,
                &root.path().join("missing")
            ),
            Err(ReceiptError::CapabilityUnavailable)
        ));
    }

    #[test]
    fn verifies_an_ordered_receipt_chain() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("receipts")).unwrap();
        let key = root.path().join("key");
        fs::write(&key, b"test-only-secret").unwrap();
        let (first, first_path) = create_receipt(
            root.path(),
            "one",
            BTreeMap::new(),
            serde_json::json!({"phase": 1}),
            None,
            &key,
        )
        .unwrap();
        let (_, second_path) = create_receipt(
            root.path(),
            "two",
            BTreeMap::new(),
            serde_json::json!({"phase": 2}),
            Some(first.receipt_hash),
            &key,
        )
        .unwrap();
        assert_eq!(
            verify_chain(&[first_path, second_path], &key)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn report_binding_rejects_a_forged_pass_report() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("receipts/rev-1/acceptance")).unwrap();
        let report = root
            .path()
            .join("receipts/rev-1/acceptance/validation_report.json");
        fs::write(&report, br#"{"phase":"acceptance","passed":true}"#).unwrap();
        let report_hash = sha256_file(&report).unwrap();
        let key = root.path().join("key");
        fs::write(&key, b"test-only-secret").unwrap();
        let (receipt, _) = create_receipt(
            root.path(),
            "rev-1/acceptance",
            BTreeMap::new(),
            serde_json::json!({
                "phase": "acceptance",
                "project_id": "test",
                "revision": 1,
                "document_sha256": "00".repeat(32),
                "validation_report_sha256": report_hash,
                "passed": true,
                "gate_results": [],
                "source_sha256": {},
                "output_sha256": {},
                "previous_receipt_hash": null
            }),
            None,
            &key,
        )
        .unwrap();
        verify_report_binding(&receipt, &report).unwrap();
        fs::write(
            &report,
            br#"{"phase":"acceptance","passed":true,"forged":true}"#,
        )
        .unwrap();
        assert!(verify_report_binding(&receipt, &report).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn key_and_receipt_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("receipts")).unwrap();
        let real_key = root.path().join("real-key");
        let key = root.path().join("key");
        fs::write(&real_key, b"test-only-secret").unwrap();
        symlink(&real_key, &key).unwrap();
        assert!(create_receipt(
            root.path(),
            "pre-render",
            BTreeMap::new(),
            Value::Null,
            None,
            &key,
        )
        .is_err());

        fs::remove_file(&key).unwrap();
        fs::write(&key, b"test-only-secret").unwrap();
        let (_, receipt) = create_receipt(
            root.path(),
            "pre-render",
            BTreeMap::new(),
            Value::Null,
            None,
            &key,
        )
        .unwrap();
        let real_receipt = root.path().join("real-receipt");
        fs::rename(&receipt, &real_receipt).unwrap();
        symlink(&real_receipt, &receipt).unwrap();
        assert!(verify_receipt(&receipt, &key).is_err());
    }
}
