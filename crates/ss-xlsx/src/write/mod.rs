//! Writing a workbook back out.
//!
//! The rule from `DESIGN.md` §1 — *never write a part we did not either author
//! or faithfully retain* — decides the shape of this entirely. There is no
//! serializer here in the usual sense, because a serializer would need to
//! understand every part it emits, and this crate understands four of the
//! twenty-odd parts in a real workbook. What there is instead is a set of
//! *edits*: the original bytes go out unless a specific, named piece of them
//! disagrees with the model, and then only that piece is replaced.
//!
//! Three parts are edited. Worksheets get their changed cells replaced.
//! `sharedStrings.xml` gets the text those cells introduced appended to it.
//! `styles.xml` gets any number format they needed. Everything else in the
//! package — including the parts of those three files outside the edited
//! regions — is the producer's bytes, untouched.
//!
//! Every part goes through the edit on every save, even one with no changes to
//! make. That is deliberate: it means the no-edit fidelity check is a test of
//! this code rather than a test of code that skipped it.

mod blank;
mod cells;
mod chart_out;
mod drawing_out;
mod sheet_out;
mod splice;
mod strings_out;
mod styles_out;

use std::io::{Seek, Write};
use std::path::Path;

use ooxml::PartName;
use ss_model::{SheetKind, Workbook};

use crate::error::Result;
use crate::{parts, workbook_part, XlsxDocument};

impl XlsxDocument {
    /// A document for a workbook that has never been in a file.
    ///
    /// The package is authored here rather than copied from a template, so
    /// every byte in it is one this crate is answerable for.
    pub fn new(workbook: Workbook) -> Result<Self> {
        let package = blank::package_for(&workbook)?;
        Ok(XlsxDocument { workbook, package })
    }

    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.flush()?;
        self.package.save(path)?;
        Ok(())
    }

    pub fn write_to<W: Write + Seek>(&mut self, writer: W) -> Result<()> {
        self.flush()?;
        self.package.write(writer)?;
        Ok(())
    }

    /// Brings the package's edited parts in line with the model.
    ///
    /// Separate from writing so that a caller can inspect the result — the
    /// fidelity harness compares packages, not files.
    pub fn flush(&mut self) -> Result<()> {
        self.flush_with(false)
    }

    /// Flushes, writing every cell out of the model rather than only the ones
    /// that changed.
    ///
    /// No save should call this: the whole point of the writer is that it leaves
    /// untouched cells alone, cached values and unknown attributes included, and
    /// regenerating them throws that protection away for nothing.
    ///
    /// It exists because it asks the harness a stronger question than a save
    /// does. A save proves the bytes we copied survived being copied. This
    /// proves we could have written them ourselves — that our idea of a
    /// worksheet and Excel's agree everywhere in the corpus, not just in the
    /// places an edit happened to land. When those two answers diverge, the
    /// difference is a feature of real files we have not understood yet, and it
    /// is much better to hear about it from the harness than from a user whose
    /// edit landed on that row.
    pub fn flush_regenerating(&mut self) -> Result<()> {
        self.flush_with(true)
    }

    fn flush_with(&mut self, regenerate: bool) -> Result<()> {
        let located = parts::locate(&self.package)?;

        let mut sst = match &located.shared_strings {
            Some(name) => match self.package.part(name) {
                Some(part) => strings_out::Sst::read(name.as_str(), part.data())?,
                None => strings_out::Sst::absent(),
            },
            None => strings_out::Sst::absent(),
        };

        // Sheets first: they are what discovers new text and new styles.
        let meta = {
            let part =
                self.package
                    .part(&located.workbook)
                    .ok_or_else(|| crate::Error::MissingPart {
                        referenced_by: "/_rels/.rels".to_owned(),
                        rel_id: "officeDocument".to_owned(),
                    })?;
            workbook_part::parse(located.workbook.as_str(), part.data())?
        };

        let mut written: Vec<(PartName, String, Vec<u8>)> = Vec::new();
        for (index, entry) in meta.sheets.iter().enumerate() {
            let Some(sheet) = self.workbook.sheet(index) else {
                continue;
            };
            if !matches!(sheet.kind, SheetKind::Worksheet) {
                continue;
            }
            let Some(name) = entry
                .rel_id
                .as_deref()
                .and_then(|id| located.sheet_target(id))
                .map(|(_, name)| name.clone())
            else {
                continue;
            };
            let Some(part) = self.package.part(&name) else {
                continue;
            };
            let content_type = part.content_type.clone();
            let mut ctx = sheet_out::Context {
                sheet,
                strings: &self.workbook.strings,
                sst: &mut sst,
                regenerate,
            };
            let data = sheet_out::rewrite(name.as_str(), part.data(), &mut ctx)?;
            written.push((name.clone(), content_type, data));

            // Pictures the user moved, resized or deleted. Compared against the
            // *file*, re-read, rather than against a flag set when the drag
            // ended: a picture dragged and dragged back has not changed, and a
            // part we rewrite for nothing is a part we could get wrong for
            // nothing.
            if let Some(drawing) = crate::drawing_of(&self.package, &name) {
                let original = crate::picture_anchors(&self.package, &drawing)?;
                let current: std::collections::BTreeMap<usize, &ss_model::Anchor> = sheet
                    .pictures
                    .iter()
                    .filter(|p| p.drawing_part == drawing.as_str())
                    .map(|p| (p.anchor_index, &p.anchor))
                    .collect();

                let mut wanted = drawing_out::Wanted::new();
                for (index, was) in &original {
                    match current.get(index) {
                        None => {
                            wanted.insert(*index, None);
                        }
                        Some(now) if *now != was => {
                            wanted.insert(*index, Some((*now).clone()));
                        }
                        Some(_) => {}
                    }
                }
                if !wanted.is_empty() {
                    if let Some(part) = self.package.part(&drawing) {
                        let content_type = part.content_type.clone();
                        let data = drawing_out::rewrite(drawing.as_str(), part.data(), &wanted)?;
                        written.push((drawing, content_type, data));
                    }
                }
            }
        }

        // Chart titles. A chart is otherwise entirely preserved, so this is the
        // one thing in it the model can disagree with the file about — and the
        // comparison is against the *file*, re-read, rather than against a flag
        // set when the user typed.
        for sheet in &self.workbook.sheets {
            for chart in &sheet.charts {
                let Ok(name) = PartName::new(&chart.part) else {
                    continue;
                };
                let Some(part) = self.package.part(&name) else {
                    continue;
                };
                let body = crate::chart::parse(name.as_str(), part.data())?;
                if body.title == chart.title {
                    continue;
                }
                let content_type = part.content_type.clone();
                let data = chart_out::retitle(name.as_str(), part.data(), chart.title.as_deref())?;
                written.push((name, content_type, data));
            }
        }

        if let Some(name) = &located.shared_strings {
            if let Some(part) = self.package.part(name) {
                let content_type = part.content_type.clone();
                let data = strings_out::rewrite(name.as_str(), part.data(), &sst)?;
                written.push((name.clone(), content_type, data));
            }
        }

        if let Some(name) = &located.styles {
            if let Some(part) = self.package.part(name) {
                let content_type = part.content_type.clone();
                let additions =
                    styles_out::additions(name.as_str(), part.data(), &self.workbook.styles)?;
                let data = styles_out::rewrite(name.as_str(), part.data(), &additions)?;
                written.push((name.clone(), content_type, data));
            }
        }

        for (name, content_type, data) in written {
            self.package.put_part(name, &content_type, data);
        }
        Ok(())
    }
}
