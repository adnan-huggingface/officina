//! What type the machine actually has, by the name a document calls it.
//!
//! The named-face table in [`crate::fonts`] covers the dozen faces a
//! spreadsheet or a letter names nine times out of ten. It cannot cover the
//! tenth: a document that names Ubuntu Mono is asking for the file of that
//! name, and the only way to know whether this machine has one is to look.
//!
//! This exists because of a rule that is easy to get backwards. **An embedded
//! font is a fallback, not an override.** Word draws with the face the machine
//! has installed and reaches for the copy inside the package only when there
//! is none — so a reader that always prefers the embedded copy lays the
//! document out in metrics Word never used. The demonstration document embeds
//! Ubuntu Mono at 500 units to the em while the copy installed here measures
//! 560, and the difference re-wraps three paragraphs.
//!
//! Only the `name` and `head` tables are read, and only those bytes are ever
//! pulled off the disk: a font folder is several hundred megabytes and none of
//! it is wanted until something asks to draw with it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// A family name, folded to lower case, with the two style bits.
pub type FaceKey = (String, bool, bool);

static CATALOGUE: OnceLock<BTreeMap<FaceKey, PathBuf>> = OnceLock::new();

/// Every installed face, indexed by the name a document would name it.
pub fn installed() -> &'static BTreeMap<FaceKey, PathBuf> {
    CATALOGUE.get_or_init(|| build(&crate::fonts::font_directories()))
}

/// Whether the machine has any face of this family.
pub fn has_family(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    installed().keys().any(|(family, _, _)| *family == name)
}

/// The file for one face: the exact style first, then the family's plain face.
///
/// Word leans and thickens a face it has rather than changing family, and so
/// does this — a family with no italic file is still that family.
pub fn file(name: &str, bold: bool, italic: bool) -> Option<&'static Path> {
    let name = name.to_ascii_lowercase();
    let table = installed();
    table
        .get(&(name.clone(), bold, italic))
        .or_else(|| table.get(&(name.clone(), bold, false)))
        .or_else(|| table.get(&(name, false, false)))
        .map(PathBuf::as_path)
}

fn build(dirs: &[PathBuf]) -> BTreeMap<FaceKey, PathBuf> {
    let mut found = BTreeMap::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            if !matches!(extension.as_deref(), Some("ttf" | "otf")) {
                continue;
            }
            let Some((family, bold, italic)) = describe(&path) else {
                continue;
            };
            // The first directory wins, which puts the system's own fonts
            // ahead of a user's separately installed copy of the same name.
            found.entry((family, bold, italic)).or_insert(path);
        }
    }
    found
}

/// The family and style of one font file, from its `name` and `head` tables.
fn describe(path: &Path) -> Option<(String, bool, bool)> {
    let file = std::fs::File::open(path).ok()?;
    let header = read_at(&file, 0, 12)?;
    // A collection has no single name, and nothing here can draw with one.
    if &header[..4] == b"ttcf" {
        return None;
    }
    let tables = u16::from_be_bytes([header[4], header[5]]) as usize;
    let directory = read_at(&file, 12, tables.checked_mul(16)?)?;
    let mut name_table = None;
    let mut head_table = None;
    for entry in directory.chunks_exact(16) {
        let offset = u32::from_be_bytes([entry[8], entry[9], entry[10], entry[11]]) as u64;
        let length = u32::from_be_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
        match &entry[..4] {
            b"name" => name_table = Some((offset, length)),
            b"head" => head_table = Some((offset, length)),
            _ => {}
        }
    }
    let (offset, length) = name_table?;
    let names = read_at(&file, offset, length)?;
    // 16 is the typographic family, which is what a document names when a
    // family carries more than the four faces the older name 1 can describe.
    let family = string(&names, 16).or_else(|| string(&names, 1))?;
    let subfamily = string(&names, 17).or_else(|| string(&names, 2));

    // `head.macStyle` is the flag every file sets; the subfamily name is the
    // fallback for one whose flags disagree with its own name.
    let mut bold = false;
    let mut italic = false;
    if let Some((offset, length)) = head_table {
        if length >= 46 {
            if let Some(bytes) = read_at(&file, offset + 44, 2) {
                let style = u16::from_be_bytes([bytes[0], bytes[1]]);
                bold = style & 1 != 0;
                italic = style & 2 != 0;
            }
        }
    }
    if let Some(subfamily) = subfamily.as_deref() {
        let lower = subfamily.to_ascii_lowercase();
        bold |= lower.contains("bold");
        italic |= lower.contains("italic") || lower.contains("oblique");
    }
    Some((family.to_ascii_lowercase(), bold, italic))
}

/// One name-table record, preferring the Windows Unicode encoding every
/// Windows font carries and falling back to the Macintosh Roman one.
fn string(table: &[u8], want: u16) -> Option<String> {
    let count = u16::from_be_bytes([*table.get(2)?, *table.get(3)?]) as usize;
    let storage = u16::from_be_bytes([*table.get(4)?, *table.get(5)?]) as usize;
    let mut best: Option<String> = None;
    for i in 0..count {
        let at = 6 + i * 12;
        let Some(record) = table.get(at..at + 12) else {
            break;
        };
        let platform = u16::from_be_bytes([record[0], record[1]]);
        let name_id = u16::from_be_bytes([record[6], record[7]]);
        if name_id != want {
            continue;
        }
        let length = u16::from_be_bytes([record[8], record[9]]) as usize;
        let offset = u16::from_be_bytes([record[10], record[11]]) as usize;
        let Some(bytes) = table.get(storage + offset..storage + offset + length) else {
            continue;
        };
        let text = match platform {
            0 | 3 => {
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect();
                String::from_utf16(&units).ok()?
            }
            _ => bytes.iter().map(|&b| b as char).collect(),
        };
        let text = text.trim().to_owned();
        if text.is_empty() {
            continue;
        }
        // The Windows record is the authority where a file carries both.
        if platform == 3 {
            return Some(text);
        }
        best.get_or_insert(text);
    }
    best
}

fn read_at(file: &std::fs::File, offset: u64, length: usize) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    // A name table is a few kilobytes; anything claiming a megabyte is a
    // malformed file and is not worth the allocation.
    if length == 0 || length > 1 << 20 {
        return None;
    }
    let mut file = file;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buffer = vec![0u8; length];
    file.read_exact(&mut buffer).ok()?;
    Some(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `name` table holding "Ab" as a Windows record and "Bold" as a Mac one.
    fn table() -> Vec<u8> {
        let mut table = Vec::new();
        table.extend_from_slice(&0u16.to_be_bytes()); // format
        table.extend_from_slice(&2u16.to_be_bytes()); // count
        table.extend_from_slice(&(6u16 + 24).to_be_bytes()); // storage offset
        let record = |platform: u16, id: u16, length: u16, offset: u16, into: &mut Vec<u8>| {
            into.extend_from_slice(&platform.to_be_bytes());
            into.extend_from_slice(&1u16.to_be_bytes());
            into.extend_from_slice(&0u16.to_be_bytes());
            into.extend_from_slice(&id.to_be_bytes());
            into.extend_from_slice(&length.to_be_bytes());
            into.extend_from_slice(&offset.to_be_bytes());
        };
        let mut records = Vec::new();
        record(3, 1, 4, 0, &mut records);
        record(1, 2, 4, 4, &mut records);
        table.extend_from_slice(&records);
        table.extend_from_slice(&[0, b'A', 0, b'b']);
        table.extend_from_slice(b"Bold");
        table
    }

    #[test]
    fn a_name_record_is_read_out_of_its_storage() {
        let table = table();
        assert_eq!(string(&table, 1).as_deref(), Some("Ab"));
        assert_eq!(string(&table, 2).as_deref(), Some("Bold"));
        assert_eq!(string(&table, 16), None, "a name it does not carry");
    }

    #[test]
    fn a_truncated_table_answers_nothing_rather_than_panicking() {
        assert_eq!(string(&[], 1), None);
        assert_eq!(string(&[0, 0, 0, 5, 0, 30], 1), None);
        let mut short = table();
        short.truncate(20);
        assert_eq!(string(&short, 1), None);
    }

    #[test]
    fn the_machines_own_fonts_are_found_by_the_name_a_document_uses() {
        // Nothing is asserted about *which* faces exist — a build machine may
        // have almost none. What is asserted is that looking is safe, that
        // what it finds is really there, and that the name folds case.
        for ((family, _, _), path) in installed().iter().take(50) {
            assert!(!family.is_empty(), "a face with no name was indexed");
            assert!(path.exists(), "{} was indexed but is gone", path.display());
        }
        if has_family("arial") {
            assert!(file("Arial", false, false).is_some(), "found but no file");
            assert!(file("ARIAL", true, false).is_some(), "the name folds case");
            assert!(!has_family("arial "), "the name is not trimmed for callers");
        }
    }
}
