//! The zip an OpenDocument file is, and everything in it.
//!
//! A deliberate twin of `ooxml::package`, down to the three-way classification
//! and to reading every entry into memory and filtering none of them. What is
//! *not* shared is the container itself, and that is not tidiness: an ODF
//! package differs from an OPC one in two places that are both stated in zip
//! terms rather than in XML.
//!
//! **`mimetype` comes first and is stored uncompressed** (ODF 1.4 part 2 §3.3),
//! so that a file can be identified by the bytes at a fixed offset without
//! inflating anything. `ooxml::Package::write` puts `[Content_Types].xml` first
//! and deflates everything, which is the same rule pointing the other way.
//!
//! **`META-INF/manifest.xml` lists what is in the package**, with a media type
//! for each entry, where OPC has content types and a graph of relationships.
//! ODF has no relationships at all: `content.xml` names a picture by its path
//! inside the package, so there is nothing here for a `parts.rs` to do.
//!
//! Everything else is the same discipline. A part this crate models is
//! re-serialized from the model; a part it does not understand is held as it
//! arrived and written back byte for byte; the manifest is regenerated, which
//! is what `[Content_Types].xml` is for OPC. An entry the manifest gives no
//! media type — Word's ODF export leaves one behind — is still a part, still
//! retained, and still written back. "Unsupported" has to mean "survives".

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;

use ooxml::{PartClass, PartName};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::manifest::{self, Manifest};
use crate::{Error, Result};

/// Where the mime type is kept, and the only entry whose position in the zip
/// carries meaning.
pub const MIMETYPE_PATH: &str = "mimetype";

/// Where the list of what is in the package is kept.
pub const MANIFEST_PATH: &str = "META-INF/manifest.xml";

/// What a text document says it is.
pub const TEXT_MIMETYPE: &str = "application/vnd.oasis.opendocument.text";

/// The media type an entry the manifest never mentions is given.
///
/// Word's ODF export leaves `word/webextensions/taskpanes.xml` in the package
/// with an empty media type, and a reader that treated an unlisted entry as
/// absent would drop it on the next save.
const UNKNOWN_MEDIA_TYPE: &str = "application/octet-stream";

/// One entry of the package.
#[derive(Debug, Clone)]
pub struct Part {
    name: PartName,
    media_type: String,
    class: PartClass,
    /// Where it stood in the zip when it arrived, so that a save which changes
    /// one part does not reshuffle the rest and turn a small edit into a
    /// whole-file diff.
    order: u32,
    data: Vec<u8>,
}

impl Part {
    pub fn name(&self) -> &PartName {
        &self.name
    }

    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    pub fn class(&self) -> PartClass {
        self.class
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Replaces the bytes of a part this crate models, and says so.
    pub fn set_modeled_data(&mut self, data: Vec<u8>) {
        self.class = PartClass::Modeled;
        self.data = data;
    }
}

#[derive(Debug, Clone)]
pub struct Container {
    parts: BTreeMap<PartName, Part>,
    manifest: Manifest,
    /// The `mimetype` entry's own bytes. Kept as it arrived rather than
    /// rebuilt, because a document that is not a text document must not be
    /// written back claiming to be one.
    mimetype: String,
    max_order: u32,
}

impl Container {
    /// A package holding nothing but the two entries every ODF file has.
    pub fn empty(mimetype: &str) -> Container {
        Container {
            parts: BTreeMap::new(),
            manifest: Manifest::for_document(mimetype),
            mimetype: mimetype.to_string(),
            max_order: 0,
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Container> {
        let bytes = std::fs::read(path.as_ref())?;
        Container::read(Cursor::new(bytes))
    }

    pub fn read<R: Read + Seek>(reader: R) -> Result<Container> {
        let mut zip = ZipArchive::new(reader).map_err(|e| Error::NotAPackage(e.to_string()))?;

        // Pass 1: everything into memory, filtering nothing. Filtering is how
        // data gets lost, and the manifest cannot be trusted to be complete.
        let mut raw: Vec<(String, Vec<u8>)> = Vec::with_capacity(zip.len());
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|e| Error::NotAPackage(format!("entry {i}: {e}")))?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().to_string();
            let mut data = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut data)?;
            raw.push((name, data));
        }

        let mimetype = raw
            .iter()
            .find(|(n, _)| n == MIMETYPE_PATH)
            .map(|(_, d)| String::from_utf8_lossy(d).trim().to_string())
            .ok_or(Error::NoMimetype)?;

        let manifest = raw
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(MANIFEST_PATH))
            .map(|(_, d)| manifest::parse(d))
            .transpose()?
            .unwrap_or_else(|| Manifest::for_document(&mimetype));
        if manifest.encrypted() {
            return Err(Error::Encrypted);
        }

        let mut parts = BTreeMap::new();
        let mut max_order = 0u32;
        for (order, (name, data)) in raw.into_iter().enumerate() {
            if name == MIMETYPE_PATH || name.eq_ignore_ascii_case(MANIFEST_PATH) {
                continue; // both derived; regenerated on save
            }
            let Ok(part_name) = PartName::new(&name) else {
                // Not addressable by anything inside the document, so it cannot
                // be referenced and cannot be preserved meaningfully. The
                // alternative is writing back a name that could escape the
                // package root.
                continue;
            };
            let order = order as u32;
            max_order = max_order.max(order);
            let media_type = manifest
                .media_type(&name)
                .filter(|t| !t.is_empty())
                .unwrap_or(UNKNOWN_MEDIA_TYPE)
                .to_string();
            parts.insert(
                part_name.clone(),
                Part {
                    name: part_name,
                    media_type,
                    class: PartClass::default_for_unknown(),
                    order,
                    data,
                },
            );
        }

        Ok(Container {
            parts,
            manifest,
            mimetype,
            max_order,
        })
    }

    pub fn mimetype(&self) -> &str {
        &self.mimetype
    }

    pub fn part(&self, name: &str) -> Option<&Part> {
        PartName::new(name).ok().and_then(|n| self.parts.get(&n))
    }

    pub fn part_mut(&mut self, name: &str) -> Option<&mut Part> {
        PartName::new(name)
            .ok()
            .and_then(move |n| self.parts.get_mut(&n))
    }

    pub fn parts(&self) -> impl Iterator<Item = &Part> {
        self.parts.values()
    }

    /// Every part, as the comparison behind `cargo xtask fidelity` wants them.
    ///
    /// Through `ooxml`'s own comparison rather than one of this crate's, so
    /// that "faithful" has one definition in the repository and not two.
    pub fn entries(&self) -> Vec<ooxml::compare::Entry<'_>> {
        self.parts
            .values()
            .map(|part| ooxml::compare::Entry {
                name: &part.name,
                kind: &part.media_type,
                data: &part.data,
            })
            .collect()
    }

    /// The bytes of a part, or nothing where there is no such part.
    pub fn data(&self, name: &str) -> Option<&[u8]> {
        self.part(name).map(Part::data)
    }

    pub fn put_part(&mut self, name: &str, media_type: &str, data: Vec<u8>) -> Result<()> {
        let part_name = PartName::new(name)?;
        let order = match self.parts.get(&part_name) {
            Some(existing) => existing.order,
            None => {
                self.max_order += 1;
                self.max_order
            }
        };
        self.manifest.declare(part_name.zip_entry(), media_type);
        self.parts.insert(
            part_name.clone(),
            Part {
                name: part_name,
                media_type: media_type.to_string(),
                class: PartClass::Modeled,
                order,
                data,
            },
        );
        Ok(())
    }

    pub fn remove_part(&mut self, name: &str) -> bool {
        let Ok(part_name) = PartName::new(name) else {
            return false;
        };
        self.manifest.withdraw(part_name.zip_entry());
        self.parts.remove(&part_name).is_some()
    }

    /// Writes the package to `path`, all of it or none of it.
    ///
    /// Beside the target first and then renamed over it, for the reason
    /// `ooxml::Package::save` gives at length: the rename is the one step that
    /// is atomic, so a disk that fills up leaves the user with one whole
    /// document rather than neither.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let mut buf = Vec::new();
        self.write(Cursor::new(&mut buf))?;

        let temporary = temporary_beside(path);
        if let Err(e) = std::fs::write(&temporary, &buf) {
            let _ = std::fs::remove_file(&temporary);
            return Err(e.into());
        }
        if let Err(e) = std::fs::rename(&temporary, path) {
            let _ = std::fs::remove_file(&temporary);
            return Err(e.into());
        }
        Ok(())
    }

    pub fn write<W: Write + Seek>(&self, writer: W) -> Result<()> {
        let mut zip = ZipWriter::new(writer);

        // First, and stored rather than deflated, so that the file can be
        // identified by the bytes at a fixed offset. This is the one thing
        // about an ODF package that a byte offset depends on.
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file(MIMETYPE_PATH, stored)
            .map_err(|e| Error::NotAPackage(e.to_string()))?;
        zip.write_all(self.mimetype.as_bytes())?;

        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file(MANIFEST_PATH, opts)
            .map_err(|e| Error::NotAPackage(e.to_string()))?;
        zip.write_all(&self.manifest.to_xml())?;

        let mut ordered: Vec<&Part> = self.parts.values().collect();
        ordered.sort_by_key(|p| (p.order, p.name.clone()));
        for part in ordered {
            zip.start_file(part.name.zip_entry(), opts)
                .map_err(|e| Error::NotAPackage(e.to_string()))?;
            zip.write_all(part.data())?;
        }

        zip.finish()
            .map_err(|e| Error::NotAPackage(e.to_string()))?;
        Ok(())
    }
}

fn temporary_beside(path: &Path) -> std::path::PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".to_string());
    path.with_file_name(format!(".{name}.scriva-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A package with the awkwardness a real one has: an entry the manifest
    /// never mentions, beside media types it does.
    fn sample() -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut zip = ZipWriter::new(Cursor::new(&mut buf));
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            zip.start_file(MIMETYPE_PATH, stored).unwrap();
            zip.write_all(TEXT_MIMETYPE.as_bytes()).unwrap();
            zip.start_file(MANIFEST_PATH, opts).unwrap();
            zip.write_all(MANIFEST_XML.as_bytes()).unwrap();
            zip.start_file("content.xml", opts).unwrap();
            zip.write_all(b"<office:document-content/>").unwrap();
            zip.start_file("Pictures/one.png", opts).unwrap();
            zip.write_all(b"\x89PNG not really").unwrap();
            zip.start_file("word/webextensions/taskpanes.xml", opts)
                .unwrap();
            zip.write_all(b"<wetp:taskpanes/>").unwrap();
            zip.finish().unwrap();
        }
        buf
    }

    const MANIFEST_XML: &str = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.4">"#,
        r#"<manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text" manifest:version="1.4"/>"#,
        r#"<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>"#,
        r#"<manifest:file-entry manifest:full-path="Pictures/one.png" manifest:media-type="image/png"/>"#,
        r#"</manifest:manifest>"#
    );

    #[test]
    fn every_entry_is_kept_including_the_one_the_manifest_never_names() {
        let container = Container::read(Cursor::new(sample())).expect("a package reads");
        assert_eq!(container.mimetype(), TEXT_MIMETYPE);
        assert_eq!(container.parts().count(), 3);
        let stray = container
            .part("word/webextensions/taskpanes.xml")
            .expect("an unlisted entry is still a part");
        assert_eq!(stray.data(), b"<wetp:taskpanes/>");
        assert_eq!(stray.class(), PartClass::Retained);
        assert_eq!(
            container.part("Pictures/one.png").unwrap().media_type(),
            "image/png"
        );
    }

    /// The rule that is stated in zip terms rather than in XML, and the reason
    /// this container cannot be an `ooxml::Package`.
    #[test]
    fn the_mimetype_is_written_first_and_uncompressed() {
        let container = Container::read(Cursor::new(sample())).expect("a package reads");
        let mut out = Vec::new();
        container.write(Cursor::new(&mut out)).expect("written");

        let mut zip = ZipArchive::new(Cursor::new(&out)).expect("a zip comes back");
        let first = zip.by_index(0).expect("there is a first entry");
        assert_eq!(first.name(), MIMETYPE_PATH);
        assert_eq!(first.compression(), CompressionMethod::Stored);
        drop(first);
        // And at the fixed offset the identification depends on: a local header
        // is thirty bytes plus the name, with no extra field.
        assert_eq!(&out[30..30 + MIMETYPE_PATH.len()], MIMETYPE_PATH.as_bytes());
        let at = 30 + MIMETYPE_PATH.len();
        assert_eq!(&out[at..at + TEXT_MIMETYPE.len()], TEXT_MIMETYPE.as_bytes());
    }

    #[test]
    fn a_package_written_back_holds_what_it_held() {
        let container = Container::read(Cursor::new(sample())).expect("a package reads");
        let mut out = Vec::new();
        container.write(Cursor::new(&mut out)).expect("written");
        let again = Container::read(Cursor::new(&out)).expect("and reads again");

        assert_eq!(again.mimetype(), container.mimetype());
        let mut before: Vec<_> = container
            .parts()
            .map(|p| (p.name().as_str().to_string(), p.data().to_vec()))
            .collect();
        let mut after: Vec<_> = again
            .parts()
            .map(|p| (p.name().as_str().to_string(), p.data().to_vec()))
            .collect();
        before.sort();
        after.sort();
        assert_eq!(before, after);
        assert_eq!(
            again
                .part("word/webextensions/taskpanes.xml")
                .map(Part::media_type),
            Some(UNKNOWN_MEDIA_TYPE),
            "an entry with no media type keeps standing for itself"
        );
    }

    #[test]
    fn a_file_that_is_not_an_odf_package_says_so_rather_than_panicking() {
        assert!(matches!(
            Container::read(Cursor::new(b"not a zip at all".to_vec())),
            Err(Error::NotAPackage(_))
        ));
    }
}
