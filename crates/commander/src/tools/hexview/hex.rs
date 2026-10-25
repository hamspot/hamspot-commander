//! Pure hex-viewer logic: row formatting, parsing, search and value decoding.

pub const ROW: usize = 16;

pub fn ascii(b: u8) -> char {
    if (0x20..0x7F).contains(&b) {
        b as char
    } else {
        '.'
    }
}

/// `000010  48 65 6C 6C 6F 20 77 6F 72 6C 64 0A 00 01 02 03  Hello world.....`
/// A short last row is padded so the text column lines up.
pub fn format_row(offset: usize, bytes: &[u8]) -> String {
    let mut s = format!("{offset:06X}  ");
    for i in 0..ROW {
        match bytes.get(i) {
            Some(b) => s.push_str(&format!("{b:02X} ")),
            None => s.push_str("   "),
        }
    }
    s.push(' ');
    s.extend(bytes.iter().take(ROW).map(|b| ascii(*b)));
    s
}

/// A plain hex dump of one row without offset or text, for the clipboard (`48 65 6C`).
pub fn row_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(ROW)
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn row_count(len: usize) -> usize {
    len.div_ceil(ROW)
}

/// `0x1F0` (hex) or `496` (decimal).
pub fn parse_offset(s: &str) -> Result<usize, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("enter an offset".into());
    }
    let r = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        usize::from_str_radix(h, 16)
    } else {
        t.parse::<usize>()
    };
    r.map_err(|_| format!("'{t}' is not an offset (use 496 or 0x1F0)"))
}

/// Search pattern: hex bytes (`DE AD be ef` or `deadbeef`) or quoted text (`"RIFF"`).
pub fn parse_pattern(s: &str) -> Result<Vec<u8>, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("enter hex bytes or \"text\"".into());
    }
    if let Some(inner) = t.strip_prefix('"') {
        let text = inner.strip_suffix('"').ok_or("missing closing quote")?;
        if text.is_empty() {
            return Err("empty text".into());
        }
        return Ok(text.as_bytes().to_vec());
    }
    let digits: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("hex digits only (or \"quoted text\")".into());
    }
    if !digits.len().is_multiple_of(2) {
        return Err("hex needs an even number of digits".into());
    }
    Ok((0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).unwrap())
        .collect())
}

/// First match at or after `from`, wrapping to the start once.
pub fn find(data: &[u8], pat: &[u8], from: usize) -> Option<usize> {
    if pat.is_empty() || pat.len() > data.len() {
        return None;
    }
    let scan =
        |range: std::ops::Range<usize>| range.into_iter().find(|&i| data[i..].starts_with(pat));
    let last = data.len() - pat.len();
    if from <= last {
        if let Some(i) = scan(from..last + 1) {
            return Some(i);
        }
    }
    scan(0..from.min(last + 1))
}

/// The value at `off` read several ways. Entries that run past the end read `n/a`.
pub fn inspect(data: &[u8], off: usize) -> Vec<(&'static str, String)> {
    let at = |n: usize| data.get(off..off + n);
    let na = || "n/a".to_owned();
    let mut v = vec![];
    let u8v = at(1).map(|b| b[0]);
    v.push(("U8", u8v.map_or_else(na, |b| b.to_string())));
    v.push(("I8", u8v.map_or_else(na, |b| (b as i8).to_string())));
    v.push(("BINARY", u8v.map_or_else(na, |b| format!("{b:08b}"))));
    let a2 = at(2).map(|b| [b[0], b[1]]);
    v.push((
        "U16 LE",
        a2.map_or_else(na, |b| u16::from_le_bytes(b).to_string()),
    ));
    v.push((
        "U16 BE",
        a2.map_or_else(na, |b| u16::from_be_bytes(b).to_string()),
    ));
    let a4 = at(4).map(|b| [b[0], b[1], b[2], b[3]]);
    v.push((
        "U32 LE",
        a4.map_or_else(na, |b| u32::from_le_bytes(b).to_string()),
    ));
    v.push((
        "U32 BE",
        a4.map_or_else(na, |b| u32::from_be_bytes(b).to_string()),
    ));
    v.push((
        "F32 LE",
        a4.map_or_else(na, |b| format!("{:e}", f32::from_le_bytes(b))),
    ));
    v.push((
        "F32 BE",
        a4.map_or_else(na, |b| format!("{:e}", f32::from_be_bytes(b))),
    ));
    v
}

/// Reads at most `limit` bytes. Returns the bytes and whether the file was longer.
pub fn load(path: &std::path::Path, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    use std::io::Read;
    let f = std::fs::File::open(path)?;
    let mut buf = vec![];
    f.take(limit as u64 + 1).read_to_end(&mut buf)?;
    let truncated = buf.len() > limit;
    buf.truncate(limit);
    Ok((buf, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_format_and_padding() {
        let r = format_row(0x10, b"Hello world\n\x00\x01\x02\x03");
        assert_eq!(
            r,
            "000010  48 65 6C 6C 6F 20 77 6F 72 6C 64 0A 00 01 02 03  Hello world....."
        );
        // a short last row keeps the text column aligned
        let short = format_row(0x20, b"AB");
        assert!(short.starts_with("000020  41 42 "));
        assert_eq!(short.find("AB").unwrap(), r.find("Hello").unwrap());
        assert_eq!(row_hex(b"\x00\xFFa"), "00 FF 61");
        assert_eq!(ascii(0x7F), '.');
        assert_eq!(ascii(b' '), ' ');
        assert_eq!(row_count(0), 0);
        assert_eq!(row_count(16), 1);
        assert_eq!(row_count(17), 2);
    }

    #[test]
    fn offsets_and_patterns() {
        assert_eq!(parse_offset("496").unwrap(), 496);
        assert_eq!(parse_offset(" 0x1F0 ").unwrap(), 496);
        assert_eq!(parse_offset("0X10").unwrap(), 16);
        assert!(parse_offset("").is_err());
        assert!(parse_offset("-1").is_err());
        assert!(parse_offset("0xZZ").is_err());
        assert_eq!(
            parse_pattern("DE AD be ef").unwrap(),
            vec![0xDE, 0xAD, 0xBE, 0xEF]
        );
        assert_eq!(
            parse_pattern("deadbeef").unwrap(),
            vec![0xDE, 0xAD, 0xBE, 0xEF]
        );
        assert_eq!(parse_pattern("\"RIFF\"").unwrap(), b"RIFF".to_vec());
        assert!(parse_pattern("\"RIFF").is_err());
        assert!(parse_pattern("ABC").is_err());
        assert!(parse_pattern("XY").is_err());
        assert!(parse_pattern("").is_err());
        assert!(parse_pattern("\"\"").is_err());
    }

    #[test]
    fn search_wraps_once() {
        let d = b"abcXYZabcXYZ";
        assert_eq!(find(d, b"XYZ", 0), Some(3));
        assert_eq!(find(d, b"XYZ", 4), Some(9));
        assert_eq!(find(d, b"XYZ", 10), Some(3), "wraps to the start");
        assert_eq!(find(d, b"nope", 0), None);
        assert_eq!(find(d, b"", 0), None);
        assert_eq!(find(d, b"abcXYZabcXYZ!", 0), None);
        assert_eq!(find(d, b"Z", 11), Some(11), "match at the very end");
    }

    #[test]
    fn inspector_values() {
        let d = [0x48, 0x65, 0x6C, 0x6C, 0x00, 0x00, 0x80, 0x3F, 0xFF];
        let m: std::collections::HashMap<_, _> = inspect(&d, 0).into_iter().collect();
        assert_eq!(m["U8"], "72");
        assert_eq!(m["BINARY"], "01001000");
        assert_eq!(m["U16 LE"], (0x6548u16).to_string());
        assert_eq!(m["U16 BE"], (0x4865u16).to_string());
        assert_eq!(m["U32 LE"], 0x6C6C6548u32.to_string());
        // 00 00 80 3F little-endian is 1.0f32
        let m: std::collections::HashMap<_, _> = inspect(&d, 4).into_iter().collect();
        assert_eq!(m["F32 LE"], "1e0");
        // signed byte and end-of-data handling
        let m: std::collections::HashMap<_, _> = inspect(&d, 8).into_iter().collect();
        assert_eq!(m["U8"], "255");
        assert_eq!(m["I8"], "-1");
        assert_eq!(m["U16 LE"], "n/a");
        assert_eq!(m["F32 BE"], "n/a");
        let m: std::collections::HashMap<_, _> = inspect(&d, 99).into_iter().collect();
        assert_eq!(m["U8"], "n/a");
    }

    #[test]
    fn load_limits_size() {
        let dir = std::env::temp_dir().join(format!("rustcars-hex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.bin");
        std::fs::write(&p, vec![7u8; 100]).unwrap();
        let (b, t) = load(&p, 100).unwrap();
        assert_eq!((b.len(), t), (100, false));
        let (b, t) = load(&p, 40).unwrap();
        assert_eq!((b.len(), t), (40, true));
        assert!(load(&dir.join("missing"), 10).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
