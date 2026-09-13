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

/// **A face is filed under the name Word calls it by first.** A font carries
/// two family names: the legacy one (name 1), which is at most four faces —
/// regular, bold, italic, bold italic — and is what Word and every document it
/// writes use, and the typographic one (name 16), which gathers every weight
/// and width into one family. Filed by the second, "Aptos Display" was filed
/// as Aptos Regular and "Calibri Light" as Calibri Regular: a document naming
/// either found nothing and was laid in a stand-in, and whether plain Aptos
/// then got the Aptos file or the Display one depended on the order a
/// directory happened to list them in. The typographic name still answers,
/// but only for a face no file claims by its legacy name.
fn build(dirs: &[PathBuf]) -> BTreeMap<FaceKey, PathBuf> {
    let mut faces = Vec::new();
    for dir in dirs {
        gather(dir, 0, &mut faces);
    }
    // The first directory wins, which puts the system's own fonts ahead of a
    // user's separately installed copy of the same name.
    let mut found = BTreeMap::new();
    for (names, path) in &faces {
        found
            .entry(names.legacy.clone())
            .or_insert_with(|| path.clone());
    }
    for (names, path) in &faces {
        if let Some(typographic) = &names.typographic {
            found
                .entry(typographic.clone())
                .or_insert_with(|| path.clone());
        }
    }
    found
}

/// Every font file under a directory, described.
///
/// Into subdirectories too, because that is how a Linux distribution keeps
/// its fonts: `/usr/share/fonts/truetype/crosextra/Carlito-Regular.ttf`, one
/// package to a folder, and a machine without Office has Carlito nowhere else.
/// Windows and macOS keep theirs flat, and lose nothing by the walk. Three
/// levels is one more than any distribution uses, and stops a link loop.
fn gather(dir: &Path, depth: usize, into: &mut Vec<(Names, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < 3 {
                gather(&path, depth + 1, into);
            }
            continue;
        }
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(extension.as_deref(), Some("ttf" | "otf")) {
            continue;
        }
        if let Some(names) = describe(&path) {
            into.push((names, path));
        }
    }
}

/// The names one file answers to: its legacy family and style, and its
/// typographic ones where it states them.
struct Names {
    legacy: FaceKey,
    typographic: Option<FaceKey>,
    /// The legacy family as the file spells it, for a notice to a person.
    display: String,
}

/// The family name a file states, in its own case — for telling a user which
/// face a nameless fallback turned out to be.
pub fn display_family(path: &Path) -> Option<String> {
    describe(path).map(|names| names.display)
}

/// The family and style of one font file, from its `name` and `head` tables.
fn describe(path: &Path) -> Option<Names> {
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
    for entry in directory.as_chunks::<16>().0 {
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

    // `head.macStyle` is the flag every file sets; the subfamily name is the
    // fallback for one whose flags disagree with its own name.
    let mut flags = (false, false);
    if let Some((offset, length)) = head_table {
        if length >= 46 {
            if let Some(bytes) = read_at(&file, offset + 44, 2) {
                let style = u16::from_be_bytes([bytes[0], bytes[1]]);
                flags = (style & 1 != 0, style & 2 != 0);
            }
        }
    }
    let key = |family: String, subfamily: Option<String>| -> FaceKey {
        let (mut bold, mut italic) = flags;
        if let Some(subfamily) = subfamily {
            let lower = subfamily.to_ascii_lowercase();
            bold |= lower.contains("bold");
            italic |= lower.contains("italic") || lower.contains("oblique");
        }
        (family.to_ascii_lowercase(), bold, italic)
    };
    let typographic = string(&names, 16).map(|family| key(family, string(&names, 17)));
    let display = string(&names, 1).or_else(|| string(&names, 16))?;
    let legacy = match string(&names, 1) {
        Some(family) => key(family, string(&names, 2)),
        None => typographic.clone()?,
    };
    Some(Names {
        legacy,
        typographic,
        display,
    })
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
                    .as_chunks::<2>()
                    .0
                    .iter()
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

    /// A font file with just enough in it to be catalogued: a `head` table
    /// whose `macStyle` says `bold`, and Windows `name` records for `names`.
    fn face(names: &[(u16, &str)], bold: bool) -> Vec<u8> {
        let mut records = Vec::new();
        let mut storage = Vec::new();
        for (id, text) in names {
            let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
            for value in [3, 1, 0x409, *id, bytes.len() as u16, storage.len() as u16] {
                records.extend_from_slice(&u16::to_be_bytes(value));
            }
            storage.extend_from_slice(&bytes);
        }
        let mut name = Vec::new();
        for value in [0, names.len() as u16, 6 + 12 * names.len() as u16] {
            name.extend_from_slice(&u16::to_be_bytes(value));
        }
        name.extend_from_slice(&records);
        name.extend_from_slice(&storage);
        let mut head = vec![0u8; 54];
        head[44..46].copy_from_slice(&u16::to_be_bytes(u16::from(bold)));

        let mut file = vec![0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0];
        let (head_at, name_at) = (12 + 32, 12 + 32 + head.len());
        for (tag, at, length) in [
            (b"head", head_at, head.len()),
            (b"name", name_at, name.len()),
        ] {
            file.extend_from_slice(tag);
            file.extend_from_slice(&[0; 4]);
            file.extend_from_slice(&(at as u32).to_be_bytes());
            file.extend_from_slice(&(length as u32).to_be_bytes());
        }
        file.extend_from_slice(&head);
        file.extend_from_slice(&name);
        file
    }

    /// A distribution keeps each font package in a folder of its own under
    /// the fonts directory, so the face a document needs is two levels down
    /// from anywhere a bare file name would be looked for.
    #[test]
    fn a_face_in_a_packages_own_folder_is_catalogued() {
        let root = std::env::temp_dir().join("ui-kit-catalogue-folders");
        let _ = std::fs::remove_dir_all(&root);
        let deep = root.join("truetype").join("crosextra");
        std::fs::create_dir_all(&deep).expect("a scratch directory");
        std::fs::write(
            deep.join("Carlito-Regular.ttf"),
            face(&[(1, "Carlito"), (2, "Regular")], false),
        )
        .expect("written");
        std::fs::write(
            deep.join("Carlito-Bold.ttf"),
            face(&[(1, "Carlito"), (2, "Bold")], true),
        )
        .expect("written");
        let found = build(std::slice::from_ref(&root));
        assert_eq!(
            found.get(&("carlito".to_owned(), false, false)),
            Some(&deep.join("Carlito-Regular.ttf"))
        );
        assert_eq!(
            found.get(&("carlito".to_owned(), true, false)),
            Some(&deep.join("Carlito-Bold.ttf"))
        );
        assert_eq!(
            display_family(&deep.join("Carlito-Regular.ttf")).as_deref(),
            Some("Carlito"),
            "and the notice gets the name as the file spells it"
        );
    }

    /// "Aptos Display" is family 1 of its file and "Aptos" with a "Display"
    /// style is family 16. Filed by 16, the Display face answered for plain
    /// Aptos — here because its directory is looked in first — and nothing
    /// answered for the name a document actually uses for its headings.
    #[test]
    fn a_face_is_found_by_the_name_word_uses_before_its_typographic_one() {
        let root = std::env::temp_dir().join("ui-kit-catalogue-names");
        let _ = std::fs::remove_dir_all(&root);
        let (first, second) = (root.join("first"), root.join("second"));
        std::fs::create_dir_all(&first).expect("a scratch directory");
        std::fs::create_dir_all(&second).expect("a scratch directory");
        let put = |dir: &Path, file: &str, names: &[(u16, &str)], bold: bool| {
            std::fs::write(dir.join(file), face(names, bold)).expect("written");
        };
        put(
            &first,
            "display.ttf",
            &[
                (1, "Aptos Display"),
                (2, "Regular"),
                (16, "Aptos"),
                (17, "Display"),
            ],
            false,
        );
        put(
            &first,
            "light.ttf",
            &[
                (1, "Calibri Light"),
                (2, "Regular"),
                (16, "Calibri"),
                (17, "Light"),
            ],
            false,
        );
        put(&second, "plain.ttf", &[(1, "Aptos"), (2, "Regular")], false);
        put(&second, "bold.ttf", &[(1, "Aptos"), (2, "Bold")], true);
        put(
            &second,
            "calibri.ttf",
            &[(1, "Calibri"), (2, "Regular")],
            false,
        );
        put(
            &second,
            "sourcelight.ttf",
            &[
                (1, "Source Light"),
                (2, "Regular"),
                (16, "Source"),
                (17, "Light"),
            ],
            false,
        );

        let found = build(&[first.clone(), second.clone()]);
        let at = |family: &str, bold: bool| {
            found
                .get(&(family.to_owned(), bold, false))
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
        };
        assert_eq!(at("aptos", false).as_deref(), Some("plain.ttf"));
        assert_eq!(at("aptos", true).as_deref(), Some("bold.ttf"));
        assert_eq!(at("aptos display", false).as_deref(), Some("display.ttf"));
        assert_eq!(at("calibri", false).as_deref(), Some("calibri.ttf"));
        assert_eq!(at("calibri light", false).as_deref(), Some("light.ttf"));
        assert_eq!(
            at("source", false).as_deref(),
            Some("sourcelight.ttf"),
            "a typographic family no file claims otherwise still answers"
        );
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
