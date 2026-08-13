//! The OPC package: a zip container of parts, plus the Preservation Vault.
//!
//! See `DESIGN.md` §3. The rule this module exists to enforce:
//!
//! > Parse what we understand. Preserve verbatim what we do not. Never write a
//! > part we did not either author or faithfully retain.
//!
//! Concretely: opening a package reads *every* entry into memory and remembers its
//! original zip ordering. Saving writes every one of them back. A part we have no
//! model for is written byte-for-byte as it arrived, so an unsupported feature
//! survives the round trip rather than being silently destroyed.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::content_types::{ContentTypes, CONTENT_TYPES_PATH, FALLBACK_CONTENT_TYPE};
use crate::error::{Error, Result};
use crate::name::PartName;
use crate::rels::Relationships;
use crate::PartClass;

/// A single part of the package.
#[derive(Debug, Clone)]
pub struct Part {
    pub name: PartName,
    pub content_type: String,
    pub class: PartClass,
    /// Position in the original zip's central directory.
    ///
    /// Preserved so a save reproduces the producer's ordering. Word does not care,
    /// but a byte-level diff against the original does, and keeping the diff small
    /// is what makes preservation bugs visible in review.
    order: u32,
    data: Vec<u8>,
}

impl Part {
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Replaces this part's bytes and marks it as modeled — i.e. authored by us.
    ///
    /// This is how a serializer hands back a part it owns. Anything that has not
    /// been through here stays retained and is written back untouched.
    pub fn set_modeled_data(&mut self, data: Vec<u8>) {
        self.data = data;
        self.class = PartClass::Modeled;
    }

    pub fn is_rels(&self) -> bool {
        self.name
            .as_str()
            .rsplit('/')
            .next()
            .is_some_and(|f| f.ends_with(".rels"))
            && self.name.as_str().contains("/_rels/")
    }
}

pub struct Package {
    parts: BTreeMap<PartName, Part>,
    content_types: ContentTypes,
    /// Highest `order` seen on open, so parts added later append rather than collide.
    max_order: u32,
}

impl Package {
    /// A package with no parts, ready to be authored into.
    ///
    /// The two extension defaults are conventional rather than required, but
    /// every OPC producer writes them and a package that declares each `.xml`
    /// part with its own `<Override>` is a package that looks nothing like the
    /// ones it will sit next to.
    pub fn empty() -> Self {
        let mut content_types = ContentTypes::new();
        content_types.set_default(
            "rels",
            "application/vnd.openxmlformats-package.relationships+xml",
        );
        content_types.set_default("xml", "application/xml");
        Package {
            parts: BTreeMap::new(),
            content_types,
            max_order: 0,
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = std::fs::read(path.as_ref())?;
        Self::read(Cursor::new(bytes))
    }

    pub fn read<R: Read + Seek>(reader: R) -> Result<Self> {
        let mut zip = ZipArchive::new(reader).map_err(|e| Error::NotAPackage(e.to_string()))?;

        // Pass 1: pull every entry into memory, including the ones we will never
        // understand. Nothing is filtered here — filtering is how data gets lost.
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

        // Pass 2: content types must be resolved before parts can be classified.
        let ct_bytes = raw
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(CONTENT_TYPES_PATH))
            .map(|(_, d)| d.clone())
            .ok_or(Error::MissingContentTypes)?;
        let content_types = ContentTypes::parse(&ct_bytes)?;

        // Pass 3: classify.
        let mut parts = BTreeMap::new();
        let mut max_order = 0u32;
        for (order, (name, data)) in raw.into_iter().enumerate() {
            if name.eq_ignore_ascii_case(CONTENT_TYPES_PATH) {
                continue; // derived; regenerated from `content_types` on save
            }
            let part_name = match PartName::new(&name) {
                Ok(p) => p,
                // A zip entry that is not a legal part name is not addressable by
                // any consumer, so it cannot be referenced and cannot be preserved
                // meaningfully. Skipping is the only safe option; the alternative
                // is writing back a name that could escape the package root.
                Err(_) => continue,
            };
            let order = order as u32;
            max_order = max_order.max(order);

            let content_type = content_types
                .get(&part_name)
                .unwrap_or(FALLBACK_CONTENT_TYPE)
                .to_string();

            let is_rels =
                part_name.as_str().contains("/_rels/") && part_name.as_str().ends_with(".rels");
            let class = if is_rels {
                PartClass::Derived
            } else {
                PartClass::default_for_unknown()
            };

            parts.insert(
                part_name.clone(),
                Part {
                    name: part_name,
                    content_type,
                    class,
                    order,
                    data,
                },
            );
        }

        Ok(Package {
            parts,
            content_types,
            max_order,
        })
    }

    pub fn part(&self, name: &PartName) -> Option<&Part> {
        self.parts.get(name)
    }

    pub fn part_mut(&mut self, name: &PartName) -> Option<&mut Part> {
        self.parts.get_mut(name)
    }

    pub fn parts(&self) -> impl Iterator<Item = &Part> {
        self.parts.values()
    }

    pub fn content_types(&self) -> &ContentTypes {
        &self.content_types
    }

    /// Parses the relationships owned by `part`.
    ///
    /// Returns an empty set when the part has no `.rels` companion, which is the
    /// normal case for most parts.
    pub fn relationships(&self, part: &PartName) -> Result<Relationships> {
        let rels_name = part.rels_part();
        match self.parts.get(&rels_name) {
            Some(p) => Relationships::parse(&rels_name, p.data()),
            None => Ok(Relationships::new()),
        }
    }

    /// The package-level relationships at `/_rels/.rels`, which name the root
    /// document part.
    pub fn root_relationships(&self) -> Result<Relationships> {
        let name = PartName::new("/_rels/.rels")?;
        match self.parts.get(&name) {
            Some(p) => Relationships::parse(&name, p.data()),
            None => Ok(Relationships::new()),
        }
    }

    /// Adds or replaces a part. Used by serializers; the part is marked modeled.
    pub fn put_part(&mut self, name: PartName, content_type: &str, data: Vec<u8>) {
        self.content_types.declare(&name, content_type);
        match self.parts.get_mut(&name) {
            Some(existing) => existing.set_modeled_data(data),
            None => {
                self.max_order += 1;
                let order = self.max_order;
                self.parts.insert(
                    name.clone(),
                    Part {
                        name,
                        content_type: content_type.to_string(),
                        class: PartClass::Modeled,
                        order,
                        data,
                    },
                );
            }
        }
    }

    /// Removes a part, its `.rels` companion, and its content-type override.
    ///
    /// The three go together because leaving any one behind produces a package
    /// Excel objects to: an override naming a part that is not there is invalid
    /// by the OPC spec, and a `.rels` part whose owner is gone is a set of
    /// relationships from nowhere.
    ///
    /// What it deliberately does *not* do is follow those relationships. A
    /// worksheet points at drawings, tables, and pivot definitions, and some of
    /// them are shared; deciding which are now unreachable is a graph traversal,
    /// and getting it wrong deletes a part another sheet still needs. An
    /// orphaned part is untidy and opens; a missing one does not.
    pub fn remove_part(&mut self, name: &PartName) -> bool {
        let rels = name.rels_part();
        self.parts.remove(&rels);
        self.content_types.remove_override(&rels);
        self.content_types.remove_override(name);
        self.parts.remove(name).is_some()
    }

    /// Writes the package to `path`, all of it or none of it.
    ///
    /// Beside the target first, then renamed over it. Writing straight to the
    /// target truncates it before the first byte arrives, so a disk that fills
    /// up, a drive pulled out, or a process killed halfway leaves the user with
    /// neither the old workbook nor the new one. The rename is the one step
    /// that is atomic, so the file on disk is only ever one whole package or
    /// the other.
    ///
    /// The temporary sits in the target's own directory because a rename across
    /// volumes is a copy, and a copy is what this is avoiding. It is removed on
    /// every path out — including the one where the rename is refused because
    /// another program is holding the target open, which is the common failure
    /// and the one that must leave the original untouched.
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
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        // [Content_Types].xml must be the first entry: some consumers, including
        // older Office builds, read it by position rather than by name.
        zip.start_file(CONTENT_TYPES_PATH, opts)
            .map_err(|e| Error::NotAPackage(e.to_string()))?;
        zip.write_all(&self.content_types.to_xml())?;

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

/// A name beside `path` for the half-written file to live under.
///
/// Hidden from a directory listing by the leading dot, told apart from another
/// save of the same file by the process id, and in the same directory so the
/// rename that follows stays on one volume.
fn temporary_beside(path: &Path) -> std::path::PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "workbook".to_string());
    path.with_file_name(format!(".{name}.calx-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal but structurally real package, including a part with a
    /// content type we deliberately do not understand.
    fn sample_package() -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut zip = ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default();

            zip.start_file(CONTENT_TYPES_PATH, opts).unwrap();
            zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/customXml/item1.xml" ContentType="application/vnd.acme.secret-sauce+xml"/>
</Types>"#).unwrap();

            zip.start_file("_rels/.rels", opts).unwrap();
            zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#).unwrap();

            zip.start_file("word/document.xml", opts).unwrap();
            zip.write_all(b"<document>hello</document>").unwrap();

            zip.start_file("customXml/item1.xml", opts).unwrap();
            zip.write_all(b"<secret>do not lose me</secret>").unwrap();

            zip.finish().unwrap();
        }
        buf
    }

    #[test]
    fn opens_and_classifies_parts() {
        let pkg = Package::read(Cursor::new(sample_package())).unwrap();

        let doc = PartName::new("/word/document.xml").unwrap();
        let part = pkg.part(&doc).expect("document part must be present");
        assert!(part.content_type.ends_with("main+xml"));

        // Nothing is modeled yet, so everything that is not a rels part is retained.
        assert_eq!(part.class, PartClass::Retained);

        let rels = PartName::new("/_rels/.rels").unwrap();
        assert_eq!(pkg.part(&rels).unwrap().class, PartClass::Derived);
    }

    #[test]
    fn a_part_we_do_not_understand_survives_the_round_trip_byte_for_byte() {
        // The whole point of the Preservation Vault.
        let original = sample_package();
        let pkg = Package::read(Cursor::new(original)).unwrap();

        let mut out = Vec::new();
        pkg.write(Cursor::new(&mut out)).unwrap();

        let reopened = Package::read(Cursor::new(out)).unwrap();
        let secret = PartName::new("/customXml/item1.xml").unwrap();
        assert_eq!(
            reopened
                .part(&secret)
                .expect("unmodeled part must survive")
                .data(),
            b"<secret>do not lose me</secret>"
        );
    }

    #[test]
    fn every_part_survives_a_save() {
        let pkg = Package::read(Cursor::new(sample_package())).unwrap();
        let before: Vec<String> = pkg.parts().map(|p| p.name.to_string()).collect();

        let mut out = Vec::new();
        pkg.write(Cursor::new(&mut out)).unwrap();
        let after: Vec<String> = Package::read(Cursor::new(out))
            .unwrap()
            .parts()
            .map(|p| p.name.to_string())
            .collect();

        assert_eq!(
            before, after,
            "no part may be added or dropped by a no-op save"
        );
    }

    #[test]
    fn content_types_survive_regeneration() {
        let pkg = Package::read(Cursor::new(sample_package())).unwrap();
        let mut out = Vec::new();
        pkg.write(Cursor::new(&mut out)).unwrap();
        let reopened = Package::read(Cursor::new(out)).unwrap();

        let secret = PartName::new("/customXml/item1.xml").unwrap();
        assert_eq!(
            reopened.part(&secret).unwrap().content_type,
            "application/vnd.acme.secret-sauce+xml",
            "a content type we do not model must still be declared on save"
        );
    }

    #[test]
    fn root_relationship_names_the_document_part() {
        let pkg = Package::read(Cursor::new(sample_package())).unwrap();
        let rels = pkg.root_relationships().unwrap();
        let root = PartName::new("/").err();
        assert!(root.is_some(), "`/` alone is not a part name");

        let office_doc = rels
            .by_type("http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument")
            .next()
            .expect("package must name its root document");
        assert_eq!(office_doc.target, "word/document.xml");
    }

    #[test]
    fn missing_content_types_is_rejected() {
        let mut buf = Vec::new();
        {
            let mut zip = ZipWriter::new(Cursor::new(&mut buf));
            zip.start_file("word/document.xml", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"<document/>").unwrap();
            zip.finish().unwrap();
        }
        assert!(matches!(
            Package::read(Cursor::new(buf)),
            Err(Error::MissingContentTypes)
        ));
    }

    /// A directory of our own under the system temp, removed at the end.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("calx-pkg-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    #[test]
    fn a_saved_package_leaves_no_temporary_beside_it() {
        let dir = scratch("clean");
        let target = dir.join("book.xlsx");
        Package::read(Cursor::new(sample_package()))
            .expect("reads")
            .save(&target)
            .expect("saves");

        let names: Vec<String> = std::fs::read_dir(&dir)
            .expect("lists")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["book.xlsx"], "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_that_fails_leaves_the_file_that_was_there_alone() {
        // The failure is provoked by making the target a *directory*: the
        // rename cannot replace it, which is the same shape as the failure
        // that matters — another program holding the file open — without
        // needing a second process to hold it.
        let dir = scratch("refused");
        let target = dir.join("book.xlsx");
        std::fs::create_dir(&target).expect("a directory in the way");
        std::fs::write(target.join("evidence"), b"still here").expect("writes");

        let package = Package::read(Cursor::new(sample_package())).expect("reads");
        assert!(package.save(&target).is_err(), "the save cannot succeed");

        assert!(
            target.join("evidence").exists(),
            "what was at the path was not touched"
        );
        let strays: Vec<String> = std::fs::read_dir(&dir)
            .expect("lists")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .filter(|n| n != "book.xlsx")
            .collect();
        assert!(
            strays.is_empty(),
            "the half-written file is gone: {strays:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_zip_input_is_rejected_cleanly() {
        let err = Package::read(Cursor::new(b"this is not a zip file".to_vec()));
        assert!(matches!(err, Err(Error::NotAPackage(_))));
    }
}
