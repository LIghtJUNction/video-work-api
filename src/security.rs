use hmac::{Hmac, Mac};
use rand::RngCore;
use scrypt::{scrypt, Params};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

const SCRYPT_N_LOG2: u8 = 14; // 2^14
const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;
const SCRYPT_DKLEN: usize = 32;

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    let params = Params::new(SCRYPT_N_LOG2, SCRYPT_R, SCRYPT_P, SCRYPT_DKLEN)
        .map_err(|e| anyhow::anyhow!("scrypt params: {e}"))?;
    let mut digest = [0u8; SCRYPT_DKLEN];
    scrypt(password.as_bytes(), &salt, &params, &mut digest)
        .map_err(|e| anyhow::anyhow!("scrypt: {e}"))?;
    let n = 1u32 << SCRYPT_N_LOG2;
    Ok(format!(
        "scrypt${n}${SCRYPT_R}${SCRYPT_P}${}${}" ,
        hex::encode(salt),
        hex::encode(digest)
    ))
}

pub fn verify_password(password: &str, encoded: &str) -> bool {
    let parts: Vec<&str> = encoded.split('$').collect();
    if parts.len() != 6 || parts[0] != "scrypt" {
        return false;
    }
    let n: u32 = match parts[1].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let r: u32 = match parts[2].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let p: u32 = match parts[3].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let salt = match hex::decode(parts[4]) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let expected = match hex::decode(parts[5]) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let log_n = n.trailing_zeros() as u8;
    if 1u32 << log_n != n {
        return false;
    }
    let params = match Params::new(log_n, r, p, expected.len()) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let mut actual = vec![0u8; expected.len()];
    if scrypt(password.as_bytes(), &salt, &params, &mut actual).is_err() {
        return false;
    }
    bool::from(actual.ct_eq(&expected))
}

pub fn new_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64_url_encode(&bytes)
}

pub fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn constant_time_eq(a: &str, b: &str) -> bool {
    // Length mismatch must not short-circuit in a way that leaks token length
    // via panics; hash both sides then compare digests.
    let ha = token_hash(a);
    let hb = token_hash(b);
    bool::from(ha.as_bytes().ct_eq(hb.as_bytes()))
}

fn base64_url_encode(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Constant-time compare used for MCP bearer tokens.
#[allow(dead_code)]
pub fn hmac_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    // Use HMAC as a belt-and-suspenders constant-time path for equal lengths.
    let mut mac = match HmacSha256::new_from_slice(b"vwa") {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(a);
    let ra = mac.finalize().into_bytes();
    let mut mac = match HmacSha256::new_from_slice(b"vwa") {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(b);
    let rb = mac.finalize().into_bytes();
    bool::from(ra.ct_eq(&rb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrip() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password here", &hash));
    }

    #[test]
    fn session_token_hash_stable() {
        let t = new_session_token();
        assert_eq!(token_hash(&t), token_hash(&t));
        assert_ne!(token_hash(&t), token_hash("other"));
    }
}
