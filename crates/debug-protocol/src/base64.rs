//! Standard base64 (RFC 4648, with padding), so capture chunks travel inside
//! the JSON lines without a dependency.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `bytes` as base64 text.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let n = group
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= group.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn value(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some(u32::from(c - b'A')),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// The bytes `text` encodes, or why it is not base64.
pub fn decode(text: &str) -> Result<Vec<u8>, String> {
    let bytes = text.trim_end_matches('=').as_bytes();
    if bytes.len() % 4 == 1 {
        return Err("base64 text has an impossible length".into());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 2);
    for group in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in group.iter().enumerate() {
            let v = value(c).ok_or_else(|| format!("{:?} is not a base64 character", c as char))?;
            n |= v << (18 - 6 * i);
        }
        for i in 0..group.len() - 1 {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_rfc_vectors() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), coded);
            assert_eq!(decode(coded).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn every_byte_value_round_trips_at_every_length() {
        let data: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        for len in 0..data.len().min(300) {
            assert_eq!(decode(&encode(&data[..len])).unwrap(), &data[..len]);
        }
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(decode("ab$d").is_err());
        assert!(decode("a").is_err());
    }
}
