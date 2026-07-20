//! Human-readable download basenames derived from generation text.

/// Build a filesystem-safe `.wav` download name from target text.
///
/// Long text becomes `前缀…后缀-a1b2.wav` (Unicode ellipsis + 4-hex short hash).
/// Short text is used as-is after sanitization, still with `-` + short hash.
/// The on-disk generation file remains a UUID; this is only for
/// `Content-Disposition` / browser `download` attributes.
pub fn download_name_from_text(text: &str) -> String {
    let cleaned = sanitize_display_text(text);
    let label = if cleaned.is_empty() {
        "speech".to_string()
    } else {
        abbreviate_text(&cleaned, 8, 6)
    };
    // Hash the original target text so identical copy shares a stable suffix,
    // and near-identical abbreviations still differ when the full text does.
    let hash = short_hash4(text.trim());
    format!("{label}-{hash}.wav")
}

/// ASCII-only fallback for the classic `filename=` parameter.
pub fn download_name_ascii_fallback(text: &str) -> String {
    let name = download_name_from_text(text);
    let stem = name.trim_end_matches(".wav");
    let ascii: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else if c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let ascii = ascii.trim_matches('_');
    if ascii.is_empty() {
        "speech.wav".into()
    } else {
        format!("{ascii}.wav")
    }
}

/// `Content-Disposition` value with both ASCII `filename` and RFC 5987 `filename*`.
pub fn content_disposition_attachment(text: &str) -> String {
    let utf8_name = download_name_from_text(text);
    let ascii_name = download_name_ascii_fallback(text);
    // Escape double quotes in the ASCII token (should not occur after sanitize).
    let ascii_safe = ascii_name.replace('\\', "_").replace('"', "_");
    let encoded = percent_encode_filename(&utf8_name);
    format!("attachment; filename=\"{ascii_safe}\"; filename*=UTF-8''{encoded}")
}

fn sanitize_display_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_space = false;
    for ch in text.chars() {
        if ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0')
        {
            continue;
        }
        // Collapse whitespace to a single full-width-friendly space for readability.
        if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
            continue;
        }
        prev_space = false;
        // Strip characters that confuse Content-Disposition parsers.
        if ch == ';' || ch == '=' {
            continue;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

fn abbreviate_text(text: &str, head: usize, tail: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let raw = if n <= head + tail + 1 {
        chars.into_iter().collect::<String>()
    } else {
        let mut s: String = chars[..head].iter().collect();
        s.push('…');
        s.extend(chars[n - tail..].iter());
        s
    };
    // Drop dangling leading/trailing punctuation so names don't end with "。."
    raw.trim_matches(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '。' | '，' | '、' | '；' | '：' | '！' | '？' | '.' | ',' | '!' | '?' | ';' | ':'
            )
    })
    .to_string()
}

fn percent_encode_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len() * 3);
    for b in name.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            _ => {
                out.push('%');
                out.push(hex_digit(b >> 4));
                out.push(hex_digit(b & 0xf));
            }
        }
    }
    out
}

fn hex_digit(n: u8) -> char {
    b"0123456789ABCDEF"[n as usize] as char
}

/// FNV-1a over UTF-8 bytes → 4 lowercase hex digits (stable, no extra crate).
fn short_hash4(text: &str) -> String {
    let mut h: u32 = 2_166_136_261;
    for b in text.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(16_777_619);
    }
    format!("{:04x}", h & 0xffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_gets_hash_suffix() {
        let name = download_name_from_text("你好世界");
        assert!(name.starts_with("你好世界-"));
        assert!(name.ends_with(".wav"));
        assert_eq!(name.len(), "你好世界-".len() + 4 + ".wav".len());
        let hash = name
            .trim_start_matches("你好世界-")
            .trim_end_matches(".wav");
        assert_eq!(hash.len(), 4);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn long_text_uses_ellipsis_and_hash() {
        let text = "车况已经介绍的差不多了适不适合购买还是看实际需求和预算欢迎到店亲自试驾再做决定";
        let name = download_name_from_text(text);
        assert!(name.ends_with(".wav"));
        assert!(name.contains('…'));
        assert!(name.starts_with("车况已经介绍的差"));
        assert!(name.contains("再做决定-"));
        // …suffix-xxxx.wav
        let stem = name.trim_end_matches(".wav");
        let hash = stem.rsplit_once('-').map(|(_, h)| h).unwrap();
        assert_eq!(hash.len(), 4);
    }

    #[test]
    fn empty_falls_back() {
        let a = download_name_from_text("   ");
        let b = download_name_from_text("");
        assert!(a.starts_with("speech-") && a.ends_with(".wav"));
        assert!(b.starts_with("speech-") && b.ends_with(".wav"));
        // empty and whitespace-only share the same trimmed hash input
        assert_eq!(a, b);
    }

    #[test]
    fn strips_path_chars() {
        let name = download_name_from_text("a/b\\c:d*e?.wav");
        assert!(!name.contains('/'));
        assert!(!name.contains('\\'));
        assert!(!name.contains(':'));
        assert!(name.ends_with(".wav"));
    }

    #[test]
    fn same_text_stable_hash() {
        let t = "选购二手车不能只看外观";
        assert_eq!(download_name_from_text(t), download_name_from_text(t));
        assert_ne!(
            download_name_from_text(t),
            download_name_from_text("选购二手车不能只看内饰")
        );
    }

    #[test]
    fn disposition_has_filename_star() {
        let d = content_disposition_attachment("测试文案较长需要省略号处理的内容结尾");
        assert!(d.contains("filename="));
        assert!(d.contains("filename*=UTF-8''"));
        assert!(d.contains(".wav"));
    }
}
