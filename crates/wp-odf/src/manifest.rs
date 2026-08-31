//! `META-INF/manifest.xml` — what an ODF package says is in it.
//!
//! The seat `[Content_Types].xml` occupies for OPC, and classified the same
//! way: **derived**, regenerated on every save rather than copied. What makes
//! that safe is that it is regenerated from what was read rather than from
//! guesswork — every entry keeps the media type it arrived with, and the
//! attributes this reader has no use for are kept beside it and written back.
//! A manifest rebuilt from a list of parts alone would quietly drop the
//! directory entries LibreOffice writes and the `manifest:version` a consumer
//! reads to decide which version of the standard it is looking at.
//!
//! **An encrypted package is refused rather than half-read.** The bytes of a
//! part under `manifest:encryption-data` are ciphertext, and a reader that
//! shrugged and treated them as XML would report a broken document rather than
//! a locked one.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::xml::{attr, local_name};
use crate::{Error, Result};

/// One line of the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// As the manifest spells it: `content.xml`, `Pictures/one.png`, or `/`
    /// for the document itself. Not a [`ooxml::PartName`] — `/` and a trailing
    /// separator are both legal here and neither is a part.
    pub full_path: String,
    pub media_type: String,
    /// Everything else the entry carried, in order, so that a version or a
    /// size attribute survives a save this crate had no opinion about.
    pub rest: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct Manifest {
    version: Option<String>,
    entries: Vec<Entry>,
    encrypted: bool,
}

impl Manifest {
    /// The manifest a document with nothing in it but itself would have.
    pub fn for_document(mimetype: &str) -> Manifest {
        Manifest {
            version: Some(crate::ODF_VERSION.to_string()),
            entries: vec![Entry {
                full_path: "/".into(),
                media_type: mimetype.into(),
                rest: vec![("manifest:version".into(), crate::ODF_VERSION.into())],
            }],
            encrypted: false,
        }
    }

    pub fn encrypted(&self) -> bool {
        self.encrypted
    }

    pub fn media_type(&self, full_path: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.full_path == full_path)
            .map(|e| e.media_type.as_str())
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Records a part, keeping its place if it was already listed.
    pub fn declare(&mut self, full_path: &str, media_type: &str) {
        match self.entries.iter_mut().find(|e| e.full_path == full_path) {
            Some(entry) => entry.media_type = media_type.to_string(),
            None => self.entries.push(Entry {
                full_path: full_path.to_string(),
                media_type: media_type.to_string(),
                rest: Vec::new(),
            }),
        }
    }

    pub fn withdraw(&mut self, full_path: &str) {
        self.entries.retain(|e| e.full_path != full_path);
    }

    pub fn to_xml(&self) -> Vec<u8> {
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <manifest:manifest \
             xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"",
        );
        if let Some(version) = &self.version {
            out.push_str(&format!(" manifest:version=\"{}\"", escape(version)));
        }
        out.push_str(">\n");
        for entry in &self.entries {
            out.push_str(&format!(
                " <manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"{}\"",
                escape(&entry.full_path),
                escape(&entry.media_type)
            ));
            for (name, value) in &entry.rest {
                out.push_str(&format!(" {}=\"{}\"", name, escape(value)));
            }
            out.push_str("/>\n");
        }
        out.push_str("</manifest:manifest>\n");
        out.into_bytes()
    }
}

pub fn parse(bytes: &[u8]) -> Result<Manifest> {
    let text = String::from_utf8_lossy(bytes);
    let mut reader = Reader::from_str(&text);
    let mut manifest = Manifest::default();
    loop {
        let event = reader
            .read_event()
            .map_err(|e| Error::Xml(format!("{}: {e}", crate::container::MANIFEST_PATH)))?;
        match event {
            Event::Start(e) | Event::Empty(e) => match local_name(&e) {
                b"manifest" => manifest.version = attr(&e, b"version"),
                b"file-entry" => {
                    if let Some(entry) = file_entry(&e) {
                        manifest.entries.push(entry);
                    }
                }
                b"encryption-data" => manifest.encrypted = true,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(manifest)
}

fn file_entry(e: &BytesStart<'_>) -> Option<Entry> {
    let full_path = attr(e, b"full-path")?;
    let media_type = attr(e, b"media-type").unwrap_or_default();
    let rest = crate::xml::attributes(e)
        .filter(|a| {
            let name = crate::xml::strip_prefix(a.key.as_ref());
            name != b"full-path" && name != b"media-type"
        })
        .filter_map(|a| {
            let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
            let value = a.normalized_value(crate::xml::XML_VERSION).ok()?;
            Some((key, value.into_owned()))
        })
        .collect();
    Some(Entry {
        full_path,
        media_type,
        rest,
    })
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.4">"#,
        r#"<manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text" manifest:version="1.4"/>"#,
        r#"<manifest:file-entry manifest:full-path="Pictures/" manifest:media-type=""/>"#,
        r#"<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>"#,
        r#"</manifest:manifest>"#
    );

    #[test]
    fn a_manifest_read_and_written_back_says_the_same_thing() {
        let manifest = parse(SAMPLE.as_bytes()).expect("a manifest parses");
        assert_eq!(manifest.media_type("content.xml"), Some("text/xml"));
        assert_eq!(manifest.entries().len(), 3);

        let again = parse(&manifest.to_xml()).expect("and parses again");
        assert_eq!(again.entries(), manifest.entries());
        assert_eq!(again.version, manifest.version);
    }

    /// A directory entry is not a part and never will be, and rebuilding the
    /// manifest from the parts alone would lose it.
    #[test]
    fn an_entry_that_is_not_a_part_survives_the_rebuild() {
        let manifest = parse(SAMPLE.as_bytes()).expect("a manifest parses");
        let written = String::from_utf8(manifest.to_xml()).expect("utf-8");
        assert!(
            written.contains(r#"manifest:full-path="Pictures/""#),
            "{written}"
        );
        assert!(
            written.contains(r#"manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text" manifest:version="1.4""#),
            "the root entry keeps the version attribute nothing here reads: {written}"
        );
    }

    #[test]
    fn a_locked_document_is_named_as_locked_rather_than_read_as_broken() {
        let locked = concat!(
            r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0">"#,
            r#"<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml">"#,
            r#"<manifest:encryption-data manifest:checksum-type="SHA1/1K" manifest:checksum="x">"#,
            r#"</manifest:encryption-data></manifest:file-entry></manifest:manifest>"#
        );
        assert!(parse(locked.as_bytes())
            .expect("it still parses")
            .encrypted());
    }
}
