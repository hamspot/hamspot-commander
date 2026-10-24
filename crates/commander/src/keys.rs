//! Encryption keys for one station. The bytes live in `NAME.HSKeys` beside
//! the station file, mode 0600. They are not a field of the station JSON.
//!
//! A row with empty text is a placeholder. The decoder named that key id
//! and Commander has no key for it. A placeholder is saved and is not passed
//! to `dsd-fme`.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// DES-OFB, the P25 algorithm byte `0x81`.
pub const ALG_DES: &str = "81";
/// RC4 / ADP, the P25 algorithm byte `0xAA`.
pub const ALG_RC4: &str = "AA";
/// AES-256, the P25 algorithm byte `0x84`.
pub const ALG_AES: &str = "84";

/// Algorithms the operator can assign. Other bytes stay as hex until a
/// later format knows what to do with them.
pub const ALG_CHOICES: &[(&str, &str)] = &[
    (ALG_DES, "DES"),
    (ALG_RC4, "RC4"),
    (ALG_AES, "AES-256"),
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyFile {
    pub version: u32,
    pub keys: Vec<KeyRow>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyRow {
    /// Four uppercase hex digits. 0000 is not a key id.
    pub key_id: String,
    /// Two uppercase hex digits, the algorithm byte from the air.
    pub alg: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Key hex, uppercase, no spaces. Empty is a placeholder.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
}

pub fn empty() -> KeyFile {
    KeyFile {
        version: 1,
        keys: Vec::new(),
    }
}

/// `SAFE-T.HSCommander` keeps its keys in `SAFE-T.HSKeys`.
pub fn path_beside(station: &Path) -> PathBuf {
    station.with_extension("HSKeys")
}

pub fn alg_label(alg: &str) -> String {
    ALG_CHOICES
        .iter()
        .find(|(byte, _)| *byte == alg)
        .map(|(_, label)| (*label).to_string())
        .unwrap_or_else(|| alg.to_string())
}

/// Random hex for a local radio programmed with the same value.
/// DES and RC4 are 8 bytes. AES-256 is 32 bytes. The bytes come from
/// `/dev/urandom`. An algorithm this file cannot pass to `dsd-fme` is refused.
pub fn generate_text(alg: &str) -> Result<String, String> {
    let alg = normalize_alg(alg)?;
    let len = expected_len(&alg).ok_or_else(|| {
        format!("algorithm {} is not used yet", alg_label(&alg))
    })?;
    let mut bytes = vec![0u8; len / 2];
    let mut source = fs::File::open("/dev/urandom").map_err(|err| format!("urandom: {err}"))?;
    source
        .read_exact(&mut bytes)
        .map_err(|err| format!("urandom: {err}"))?;
    let mut text = String::with_capacity(len);
    for byte in bytes {
        text.push_str(&format!("{byte:02X}"));
    }
    Ok(text)
}

/// Hex digits this algorithm needs. Other algorithms are not a filled key.
pub fn expected_len(alg: &str) -> Option<usize> {
    match alg {
        ALG_DES | ALG_RC4 => Some(16),
        ALG_AES => Some(64),
        _ => None,
    }
}

impl KeyRow {
    /// Text matches the length of DES, RC4, or AES-256. A placeholder does not.
    pub fn is_filled(&self) -> bool {
        expected_len(&self.alg).is_some_and(|len| self.text.len() == len)
    }
}

pub fn has_filled(file: &KeyFile) -> bool {
    file.keys.iter().any(KeyRow::is_filled)
}

pub fn load_file(path: &Path) -> Result<KeyFile, String> {
    if !path.exists() {
        return Ok(empty());
    }
    let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    parse(&text)
}

pub fn parse(text: &str) -> Result<KeyFile, String> {
    let file: KeyFile = serde_json::from_str(text).map_err(|err| err.to_string())?;
    validate(&file)?;
    Ok(file)
}

pub fn save_file(path: &Path, file: &KeyFile) -> Result<(), String> {
    validate(file)?;
    let text = serde_json::to_string_pretty(file).map_err(|err| err.to_string())?;
    write_private(path, &(text + "\n"))
}

pub fn validate(file: &KeyFile) -> Result<(), String> {
    if file.version != 1 {
        return Err(format!("key file version {} is not 1", file.version));
    }
    let mut seen = Vec::new();
    for row in &file.keys {
        if row.key_id.len() != 4 || !row.key_id.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(format!("key id {} is not four hex digits", row.key_id));
        }
        if row.key_id == "0000" {
            return Err("key id 0000 is not a key".into());
        }
        if seen.contains(&row.key_id) {
            return Err(format!("key id {} is already in the list", row.key_id));
        }
        seen.push(row.key_id.clone());
        if row.alg.len() != 2 || !row.alg.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(format!("algorithm {} is not one byte", row.alg));
        }
        if !row.text.is_empty() && !row.text.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(format!("key {} is not hex", row.key_id));
        }
    }
    Ok(())
}

/// Record a key id the decoder printed. An existing row is left alone, so a
/// filled key is not cleared and a placeholder is not duplicated.
/// `true` means a placeholder was inserted.
pub fn note_missing(file: &mut KeyFile, alg: u16, key_id: u16) -> bool {
    if key_id == 0 || alg == 0x80 {
        return false;
    }
    let id = format!("{key_id:04X}");
    if file.keys.iter().any(|row| row.key_id == id) {
        return false;
    }
    file.keys.push(KeyRow {
        key_id: id,
        alg: format!("{alg:02X}"),
        name: String::new(),
        text: String::new(),
    });
    true
}

/// Append a blank row with the next free id. The operator pastes the text.
pub fn add_placeholder(file: &mut KeyFile) -> usize {
    let mut n = 1u16;
    let id = loop {
        let id = format!("{n:04X}");
        if file.keys.iter().all(|row| row.key_id != id) {
            break id;
        }
        if n == u16::MAX {
            break "FFFF".to_string();
        }
        n += 1;
    };
    file.keys.push(KeyRow {
        key_id: id,
        alg: ALG_DES.into(),
        name: String::new(),
        text: String::new(),
    });
    file.keys.len() - 1
}

/// Replace one row from the editor. A bad paste leaves the stored text.
pub fn apply_edit(
    file: &mut KeyFile,
    index: usize,
    key_id: &str,
    alg: &str,
    name: &str,
    text: &str,
) -> Result<(), String> {
    if file.keys.get(index).is_none() {
        return Err("that key is not in the list".into());
    }
    let key_id = normalize_id(key_id)?;
    if file
        .keys
        .iter()
        .enumerate()
        .any(|(at, other)| at != index && other.key_id == key_id)
    {
        return Err(format!("key id {key_id} is already in the list"));
    }
    let alg = normalize_alg(alg)?;
    let text = normalize_text(text)?;
    if !text.is_empty() {
        match expected_len(&alg) {
            Some(len) if text.len() == len => {}
            Some(len) => {
                return Err(format!(
                    "key {key_id} needs {len} hex digits for {}",
                    alg_label(&alg)
                ));
            }
            None => {
                return Err(format!(
                    "algorithm {} is not used yet",
                    alg_label(&alg)
                ));
            }
        }
    }
    let row = &mut file.keys[index];
    row.key_id = key_id;
    row.alg = alg;
    row.name = name.trim().to_string();
    row.text = text;
    Ok(())
}

const CSV_HEADER: &str = "key,word0,word1,word2,word3";

/// One filled row in the dsd-fme hex CSV. AES-256 is four 64-bit words.
/// DES and RC4 are one word. A placeholder has no line here.
fn filled_csv_line(row: &KeyRow) -> Option<String> {
    if !row.is_filled() {
        return None;
    }
    let words = if row.alg == ALG_AES {
        format!(
            "{},{},{},{}",
            &row.text[0..16],
            &row.text[16..32],
            &row.text[32..48],
            &row.text[48..64]
        )
    } else {
        row.text.clone()
    };
    Some(format!("{},{words}", row.key_id))
}

/// `dsd-fme -K` CSV. The header is skipped by the decoder. A placeholder is
/// omitted. AES-256 is four 64-bit words. DES and RC4 are one word.
pub fn runtime_csv(file: &KeyFile) -> Option<String> {
    let mut lines = vec![CSV_HEADER.to_string()];
    for row in &file.keys {
        if let Some(line) = filled_csv_line(row) {
            lines.push(line);
        }
    }
    if lines.len() == 1 {
        return None;
    }
    Some(lines.join("\n") + "\n")
}

/// The file Import reads. Filled rows match `runtime_csv`. A placeholder is
/// the key id alone, which import stores as blank text. Names are not in
/// this format. Returns how many key rows were written.
pub fn export_csv(file: &KeyFile) -> String {
    let mut lines = vec![CSV_HEADER.to_string()];
    for row in &file.keys {
        if let Some(line) = filled_csv_line(row) {
            lines.push(line);
        } else {
            lines.push(row.key_id.clone());
        }
    }
    lines.join("\n") + "\n"
}

/// Write `export_csv` to `path` as mode 0600. The count is the key rows.
pub fn export_file(path: &Path, file: &KeyFile) -> Result<usize, String> {
    let text = export_csv(file);
    write_private(path, &text)?;
    Ok(file.keys.len())
}

/// Write the private CSV the voice decoder reads, or remove it when no row
/// is filled. The returned path is empty when there is nothing to pass.
pub fn publish_runtime(file: &KeyFile) -> Result<String, String> {
    let path = std::env::temp_dir().join("hamspot-commander-keys.csv");
    match runtime_csv(file) {
        Some(text) => {
            write_private(&path, &text)?;
            Ok(path.display().to_string())
        }
        None => {
            if path.exists() {
                fs::remove_file(&path).map_err(|err| format!("{}: {err}", path.display()))?;
            }
            Ok(String::new())
        }
    }
}

/// One `dsd-fme` hex CSV. The first row is the header and is skipped. A word
/// of all zeros is empty text, so a file of ids alone becomes placeholders.
pub fn import_csv(file: &mut KeyFile, text: &str) -> Result<usize, String> {
    let mut count = 0usize;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if index == 0 {
            continue;
        }
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        if fields.is_empty() || fields[0].is_empty() {
            continue;
        }
        let key_id = normalize_id(fields[0])?;
        let mut words = Vec::new();
        for field in fields.iter().skip(1).take(4) {
            let word = normalize_text(field)?;
            if word.is_empty() || word.chars().all(|ch| ch == '0') {
                continue;
            }
            words.push(word);
        }
        let text = words.concat();
        let alg = if text.len() == 64 {
            ALG_AES.to_string()
        } else if let Some(row) = file.keys.iter().find(|row| row.key_id == key_id) {
            row.alg.clone()
        } else {
            ALG_DES.to_string()
        };
        if !text.is_empty() && expected_len(&alg).is_none_or(|len| text.len() != len) {
            return Err(format!("key {key_id} does not match {}", alg_label(&alg)));
        }
        if let Some(row) = file.keys.iter_mut().find(|row| row.key_id == key_id) {
            row.alg = alg;
            row.text = text;
        } else {
            file.keys.push(KeyRow {
                key_id,
                alg,
                name: String::new(),
                text,
            });
        }
        count += 1;
    }
    validate(file)?;
    Ok(count)
}

/// `dsd-fme` prints each imported key on stderr as `Key [`. That line is
/// not a trunking event and must not reach the window.
pub fn is_key_dump(line: &str) -> bool {
    let mut plain = String::new();
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            for next in chars.by_ref() {
                if next == 'm' {
                    break;
                }
            }
            continue;
        }
        plain.push(ch);
    }
    plain.trim().starts_with("Key [")
}

pub fn normalize_id(raw: &str) -> Result<String, String> {
    let mut rest = raw.trim();
    if let Some(stripped) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        rest = stripped;
    }
    if rest.is_empty() || rest.len() > 4 || !rest.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("key id {raw} is not four hex digits"));
    }
    let value = u16::from_str_radix(rest, 16).map_err(|err| err.to_string())?;
    if value == 0 {
        return Err("key id 0000 is not a key".into());
    }
    Ok(format!("{value:04X}"))
}

fn normalize_alg(raw: &str) -> Result<String, String> {
    let mut rest = raw.trim();
    if let Some(stripped) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        rest = stripped;
    }
    if rest.len() != 2 || !rest.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("algorithm {raw} is not one byte"));
    }
    Ok(rest.to_ascii_uppercase())
}

/// Strip spaces, newlines, and one leading `0x`. Anything else that is not
/// a hex digit is refused.
pub fn normalize_text(raw: &str) -> Result<String, String> {
    let mut text = String::new();
    let compact: String = raw.chars().filter(|ch| !ch.is_ascii_whitespace()).collect();
    let compact = compact
        .strip_prefix("0x")
        .or_else(|| compact.strip_prefix("0X"))
        .unwrap_or(compact.as_str());
    for ch in compact.chars() {
        if !ch.is_ascii_hexdigit() {
            return Err("key text is not hex".into());
        }
        text.push(ch.to_ascii_uppercase());
    }
    Ok(text)
}

fn write_private(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(|err| err.to_string())?;
        }
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|err| format!("{}: {err}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|err| format!("{}: {err}", path.display()))?;
    // umask can widen the mode OpenOptions asked for. The key file stays 0600.
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_loads_as_no_rows() {
        let path = std::env::temp_dir().join("hamspot-keys-missing-test.HSKeys");
        let _ = fs::remove_file(&path);
        let file = load_file(&path).expect("missing");
        assert!(file.keys.is_empty());
    }

    #[test]
    fn a_placeholder_round_trips_with_blank_text() {
        let dir = std::env::temp_dir().join("hamspot-keys-round-trip");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("Station.HSKeys");
        let mut file = empty();
        assert!(note_missing(&mut file, 0x84, 0x1A2B));
        assert!(!note_missing(&mut file, 0x84, 0x1A2B));
        save_file(&path, &file).expect("save");
        let mode = fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let text = fs::read_to_string(&path).expect("read");
        assert!(!text.contains("\"text\""));
        let back = load_file(&path).expect("load");
        assert_eq!(back.keys.len(), 1);
        assert_eq!(back.keys[0].key_id, "1A2B");
        assert_eq!(back.keys[0].alg, "84");
        assert!(back.keys[0].text.is_empty());
        assert!(!back.keys[0].is_filled());
        assert!(!has_filled(&back));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_filled_row_is_not_wiped_by_a_second_sight() {
        let mut file = empty();
        file.keys.push(KeyRow {
            key_id: "1A2B".into(),
            alg: ALG_DES.into(),
            name: String::new(),
            text: "0011223344556677".into(),
        });
        assert!(!note_missing(&mut file, 0x81, 0x1A2B));
        assert_eq!(file.keys[0].text, "0011223344556677");
        assert!(file.keys[0].is_filled());
    }

    #[test]
    fn the_private_csv_splits_aes_and_omits_a_blank_row() {
        let mut file = empty();
        note_missing(&mut file, 0x84, 0x0002);
        file.keys.push(KeyRow {
            key_id: "0003".into(),
            alg: ALG_AES.into(),
            name: String::new(),
            text: "11".repeat(32),
        });
        file.keys.push(KeyRow {
            key_id: "0004".into(),
            alg: ALG_DES.into(),
            name: String::new(),
            text: "0011223344556677".into(),
        });
        let csv = runtime_csv(&file).expect("csv");
        assert!(csv.starts_with("key,word0,word1,word2,word3\n"));
        assert!(!csv.contains("0002"));
        assert!(csv.contains("0003,1111111111111111,1111111111111111,1111111111111111,1111111111111111"));
        assert!(csv.contains("0004,0011223344556677\n"));
    }

    #[test]
    fn a_key_dump_line_is_recognized() {
        assert!(is_key_dump("Key [1A2B] [0011223344556677]"));
        assert!(is_key_dump("  Key [0004] [0000000000000000]"));
        assert!(!is_key_dump(" ALG ID: 0x81 KEY ID: 0x1A2B"));
    }

    #[test]
    fn a_generated_key_matches_the_algorithm() {
        let des = generate_text(ALG_DES).expect("des");
        assert_eq!(des.len(), 16);
        assert!(des.chars().all(|ch| ch.is_ascii_hexdigit()));
        let aes = generate_text(ALG_AES).expect("aes");
        assert_eq!(aes.len(), 64);
        assert!(aes.chars().all(|ch| ch.is_ascii_hexdigit()));
        let err = generate_text("21").expect_err("dmr");
        assert!(err.contains("not used yet"));
    }

    #[test]
    fn a_bad_paste_does_not_replace_the_stored_text() {
        let mut file = empty();
        let index = add_placeholder(&mut file);
        apply_edit(&mut file, index, "1A2B", ALG_DES, "", "0011223344556677").expect("edit");
        let err = apply_edit(&mut file, index, "1A2B", ALG_DES, "", "zz").expect_err("bad");
        assert!(err.contains("not hex"));
        assert_eq!(file.keys[0].text, "0011223344556677");
    }

    #[test]
    fn a_csv_of_ids_alone_becomes_placeholders() {
        let mut file = empty();
        let n = import_csv(&mut file, "key,word0\n1A2B,0000000000000000\n").expect("import");
        assert_eq!(n, 1);
        assert!(file.keys[0].text.is_empty());
        assert!(!file.keys[0].is_filled());
    }

    #[test]
    fn an_export_imports_as_the_same_keys() {
        let mut file = empty();
        note_missing(&mut file, 0x84, 0x0002);
        file.keys.push(KeyRow {
            key_id: "0003".into(),
            alg: ALG_AES.into(),
            name: "site".into(),
            text: "11".repeat(32),
        });
        file.keys.push(KeyRow {
            key_id: "0004".into(),
            alg: ALG_DES.into(),
            name: "local".into(),
            text: "0011223344556677".into(),
        });
        file.keys.push(KeyRow {
            key_id: "0005".into(),
            alg: ALG_RC4.into(),
            name: String::new(),
            text: "AABBCCDDEEFF0011".into(),
        });
        let csv = export_csv(&file);
        assert!(csv.starts_with("key,word0,word1,word2,word3\n"));
        assert!(csv.contains("\n0002\n"));
        assert!(csv.contains(
            "0003,1111111111111111,1111111111111111,1111111111111111,1111111111111111\n"
        ));
        assert!(csv.contains("0004,0011223344556677\n"));
        assert!(csv.contains("0005,AABBCCDDEEFF0011\n"));
        assert!(!csv.contains("site"));
        let mut fresh = empty();
        assert_eq!(import_csv(&mut fresh, &csv).expect("import"), 4);
        let aes = fresh.keys.iter().find(|row| row.key_id == "0003").expect("aes");
        assert_eq!(aes.alg, ALG_AES);
        assert_eq!(aes.text, "11".repeat(32));
        let des = fresh.keys.iter().find(|row| row.key_id == "0004").expect("des");
        assert_eq!(des.alg, ALG_DES);
        assert_eq!(des.text, "0011223344556677");
        let placeholder = fresh
            .keys
            .iter()
            .find(|row| row.key_id == "0002")
            .expect("placeholder");
        assert!(placeholder.text.is_empty());
        let mut same = file.clone();
        for row in &mut same.keys {
            row.text.clear();
        }
        import_csv(&mut same, &csv).expect("reimport");
        assert_eq!(same.keys[3].alg, ALG_RC4);
        assert_eq!(same.keys[3].text, "AABBCCDDEEFF0011");
        assert_eq!(same.keys[1].name, "site");
        let dir = std::env::temp_dir().join("hamspot-keys-export");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("keys.csv");
        assert_eq!(export_file(&path, &file).expect("write"), 4);
        let mode = fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = fs::remove_dir_all(&dir);
    }
}
