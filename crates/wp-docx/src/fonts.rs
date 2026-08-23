//! `fontTable.xml` — the faces a document carries with it.
//!
//! A document that names Ubuntu on a machine with no Ubuntu is normally laid
//! out in whatever stands in for it, and every line ending moves. Word's answer
//! is to let the author put the type *in the package*, and a reader that
//! ignores it re-wraps every paragraph of such a document — which is why this
//! is here and not an optional nicety.
//!
//! The bytes are obfuscated rather than encrypted: ECMA-376 Part 1 §17.8.1
//! defines the first thirty-two of them as exclusive-ored with the sixteen
//! bytes of the GUID in `w:fontKey`, applied twice. It exists to stop a font
//! being casually lifted out of a document and installed, not to keep anything
//! secret, and undoing it is what the spec expects a consumer to do.

use quick_xml::events::Event;
use quick_xml::Reader;

use ooxml::Package;

use crate::parts::DocumentParts;
use crate::xml::{attr, local_name};

/// One face of one family, ready to hand to a text shaper.
#[derive(Debug, Clone)]
pub struct EmbeddedFace {
    /// The family as the document's runs name it — `Ubuntu`, not `Ubuntu Bold`.
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    /// The de-obfuscated font file.
    pub bytes: Vec<u8>,
}

/// Every face the package carries, de-obfuscated.
///
/// A face whose part is missing, whose key is malformed, or whose bytes are not
/// a font is skipped rather than reported: the document still opens, and the
/// only cost is that this one family falls back to a substitute, exactly as it
/// would have done had it never been embedded.
pub fn embedded(package: &Package, parts: &DocumentParts) -> Vec<EmbeddedFace> {
    let Some(table) = parts.font_table.as_ref() else {
        return Vec::new();
    };
    let Some(part) = package.part(table) else {
        return Vec::new();
    };
    let Ok(rels) = package.relationships(table) else {
        return Vec::new();
    };

    let mut faces = Vec::new();
    for (family, kind, rel_id, key) in declarations(part.data()) {
        let Some(rel) = rels.iter().find(|r| r.id == rel_id) else {
            continue;
        };
        let Some(Ok(target)) = rel.resolve(table) else {
            continue;
        };
        let Some(font) = package.part(&target) else {
            continue;
        };
        let Some(key) = font_key(&key) else { continue };
        let bytes = deobfuscate(font.data(), &key);
        if !is_font(&bytes) {
            continue;
        }
        let (bold, italic) = match kind.as_str() {
            "embedRegular" => (false, false),
            "embedBold" => (true, false),
            "embedItalic" => (false, true),
            "embedBoldItalic" => (true, true),
            _ => continue,
        };
        faces.push(EmbeddedFace {
            family,
            bold,
            italic,
            bytes,
        });
    }
    faces
}

/// The `(family, embed kind, relationship id, font key)` of every embedded face.
fn declarations(xml: &[u8]) -> Vec<(String, String, String, String)> {
    let mut found = Vec::new();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut family = String::new();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) => match local_name(&e) {
                b"font" => family = attr(&e, b"name").unwrap_or_default(),
                name if name.starts_with(b"embed") => {
                    let (Some(id), Some(key)) = (attr(&e, b"id"), attr(&e, b"fontKey")) else {
                        continue;
                    };
                    if family.is_empty() {
                        continue;
                    }
                    found.push((
                        family.clone(),
                        String::from_utf8_lossy(name).into_owned(),
                        id,
                        key,
                    ));
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    found
}

/// The sixteen key bytes of a `{3EEE3167-E5B8-...}` font key.
///
/// The GUID's hexadecimal digits are read as bytes and then taken in reverse,
/// which is what makes the key the *little-endian* reading of the whole GUID
/// rather than of its four separate fields.
fn font_key(text: &str) -> Option<[u8; 16]> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|b| b.is_ascii_hexdigit())
        .map(|b| match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            _ => b - b'A' + 10,
        })
        .collect();
    if digits.len() != 32 {
        return None;
    }
    let mut key = [0u8; 16];
    for (i, pair) in digits.chunks_exact(2).enumerate() {
        key[15 - i] = pair[0] << 4 | pair[1];
    }
    Some(key)
}

/// Undoes §17.8.1: the leading thirty-two bytes carry the key twice.
fn deobfuscate(data: &[u8], key: &[u8; 16]) -> Vec<u8> {
    let mut bytes = data.to_vec();
    for (i, byte) in bytes.iter_mut().take(32).enumerate() {
        *byte ^= key[i % 16];
    }
    bytes
}

/// Whether the bytes open like a font, so a wrong key is caught here rather
/// than by whatever tries to shape with them.
fn is_font(bytes: &[u8]) -> bool {
    matches!(
        bytes.first_chunk::<4>(),
        Some(b"\x00\x01\x00\x00" | b"true" | b"ttcf" | b"OTTO" | b"wOFF")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The key from the demonstration document, whose first bytes are known to
    // undo to a TrueType signature.
    const KEY: &str = "{3EEE3167-E5B8-4798-AE48-EA6B71E31D4D}";

    #[test]
    fn a_font_key_is_the_guid_read_backwards() {
        let key = font_key(KEY).expect("a well-formed key");
        assert_eq!(key[15], 0x3E, "the first digit pair lands last");
        assert_eq!(key[0], 0x4D, "and the last pair first");
    }

    #[test]
    fn a_key_of_the_wrong_length_is_refused() {
        assert!(font_key("{3EEE3167}").is_none());
        assert!(font_key("").is_none());
    }

    #[test]
    fn obfuscation_is_its_own_undoing() {
        let key = font_key(KEY).expect("a well-formed key");
        let mut original: Vec<u8> = (0..80u8).collect();
        original[..4].copy_from_slice(b"\x00\x01\x00\x00");
        let scrambled = deobfuscate(&original, &key);
        assert_ne!(scrambled[..32], original[..32], "the head is disturbed");
        assert_eq!(scrambled[32..], original[32..], "the rest is not touched");
        assert_eq!(deobfuscate(&scrambled, &key), original, "and it reverses");
    }

    #[test]
    fn bytes_that_are_not_a_font_are_recognised() {
        assert!(is_font(b"\x00\x01\x00\x00rest"));
        assert!(is_font(b"OTTOrest"));
        assert!(!is_font(b"<?xml version"));
        assert!(!is_font(b"\x00\x01"));
    }

    #[test]
    fn every_embed_element_names_the_face_it_carries() {
        let xml = br#"<w:fonts><w:font w:name="Ubuntu">
            <w:embedRegular r:id="rId1" w:fontKey="{A}"/>
            <w:embedBoldItalic r:id="rId4" w:fontKey="{B}"/>
            </w:font><w:font w:name="Symbol"/></w:fonts>"#;
        let found = declarations(xml);
        assert_eq!(
            found.len(),
            2,
            "the face with no embedding contributes none"
        );
        assert_eq!(found[0].0, "Ubuntu");
        assert_eq!(found[0].1, "embedRegular");
        assert_eq!(found[0].2, "rId1");
        assert_eq!(found[1].1, "embedBoldItalic");
    }
}
