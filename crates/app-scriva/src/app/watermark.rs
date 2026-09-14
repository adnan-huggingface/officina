//! The watermark: the box that asks for one, and the shape it becomes in a
//! header, written the way Word writes its own so that Word finds it again.

use super::*;

impl Scriva {
    /// Opens the watermark box, prefilled with the watermark the document
    /// already carries.
    pub(super) fn open_watermark_dialog(&mut self) {
        let found = watermark_in(&self.document);
        self.watermark_draft = Some(WatermarkDraft {
            text: found
                .map(|shape| shape.text.to_string())
                .unwrap_or_default(),
            font: found
                .and_then(|shape| shape.font.as_deref().map(str::to_owned))
                .unwrap_or_default(),
            color: match found.and_then(|shape| shape.color) {
                Some(wp_model::Color::Rgb(rgb)) => {
                    format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
                }
                _ => String::new(),
            },
            // A shape turned at all is a diagonal one; Word's own is 315.
            diagonal: found.is_none_or(|shape| shape.rotation != 0.0),
            existing: found.is_some(),
        });
    }

    pub(super) fn watermark_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.watermark_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        let mut remove = false;
        egui::Modal::new(egui::Id::new("scriva-watermark"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(320.0);
                    ui.label(egui::RichText::new("Watermark").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_sized([56.0, 20.0], egui::Label::new("Text:"));
                        dialog::first_field(ui, "scriva-watermark", &mut draft.text, 232.0);
                    });
                    ui.horizontal(|ui| {
                        ui.add_sized([56.0, 20.0], egui::Label::new("Font:"));
                        dialog::field(ui, &mut draft.font, 150.0);
                    });
                    ui.horizontal(|ui| {
                        ui.add_sized([56.0, 20.0], egui::Label::new("Colour:"));
                        dialog::field(ui, &mut draft.color, 64.0);
                        ui.label("hex");
                    });
                    ui.add_space(4.0);
                    ui.checkbox(&mut draft.diagonal, "Diagonal");
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(
                            "It goes in the header, behind the words, on every page.",
                        )
                        .small()
                        .weak(),
                    );
                    ui.add_space(12.0);
                    if draft.existing {
                        if ui.button("Remove watermark").clicked() {
                            remove = true;
                            done = Some(true);
                        }
                        ui.add_space(6.0);
                    }
                    if let Some(answer) = dialog::submit(ui, "Apply") {
                        done = Some(answer);
                    }
                });
            });
        self.watermark_draft = Some(draft.clone());
        match done {
            Some(true) => {
                self.watermark_draft = None;
                if remove {
                    draft.text.clear();
                }
                self.apply_watermark(&draft);
            }
            Some(false) => self.watermark_draft = None,
            None => {}
        }
    }

    /// Puts the watermark into every header the section names, taking away
    /// whatever one was there — one undo step.
    ///
    /// **Into every header, not just the default one.** A document with a
    /// title page names three, and Word stamps all of them; putting the shape
    /// in one alone gives a document whose first page is unmarked, which for a
    /// watermark is the page that most needed marking.
    pub(super) fn apply_watermark(&mut self, draft: &WatermarkDraft) {
        use wp_model::doc::{HeaderFooter, Inline, Piece, Run};
        use wp_model::section::{HeaderId, HeaderKind, HeaderRef};
        self.history
            .push(wp_model::Scope::Body, self.chrome_change());
        let shape = self.watermark_shape(draft);
        let document = &mut self.document;
        for header in document.headers.iter_mut().filter(|header| !header.footer) {
            strip_watermark(&mut header.content);
        }
        let Some(shape) = shape else {
            self.changed();
            return;
        };
        // A document that names no header at all needs one made for the
        // watermark to live in; the writer assigns its part and relationship
        // when it first writes the package.
        if document.section.headers.is_empty() {
            let id = HeaderId(document.headers.iter().map(|h| h.id.0).max().unwrap_or(0) + 1);
            document.headers.push(HeaderFooter {
                id,
                part: None,
                rel: None,
                footer: false,
                content: Vec::new(),
            });
            document.section.headers.push(HeaderRef {
                kind: HeaderKind::Default,
                body: id,
                rel: None,
            });
        }
        // **Only the headers a page will actually show.** A document commonly
        // names three — default, first, even — while `<w:titlePg>` and the
        // document's even/odd setting are both off, in which case two of them
        // are never drawn. Word stamps the watermark on the one that is:
        // measured against a watermark Word wrote itself, which put the shape
        // in the default header and left the other two parts empty. Stamping
        // all three writes shapes into stories no reader ever sees, and Word
        // then reports three watermarks on a document that shows one.
        let title_page = document.section.title_page;
        let even_and_odd = document.settings.even_and_odd_headers;
        let mut wanted: Vec<HeaderId> = document
            .section
            .headers
            .iter()
            .filter(|reference| match reference.kind {
                HeaderKind::Default => true,
                HeaderKind::First => title_page,
                HeaderKind::Even => even_and_odd,
            })
            .map(|reference| reference.body)
            .collect();
        // Two references may name one body — "link to previous" writes that —
        // and a body stamped twice carries the watermark twice.
        wanted.sort_unstable_by_key(|id| id.0);
        wanted.dedup();
        for id in wanted {
            let Some(header) = document
                .headers
                .iter_mut()
                .find(|header| header.id == id && !header.footer)
            else {
                continue;
            };
            let mut paragraph = Paragraph::new();
            paragraph.props.spacing.before = Some(Twips(0));
            paragraph.props.spacing.after = Some(Twips(0));
            paragraph.content.push(Inline::Run(Run {
                content: vec![Piece::Drawing(Box::new(shape.clone()))],
                ..Run::new()
            }));
            header.content.push(Block::Paragraph(paragraph));
        }
        self.changed();
    }

    /// The shape a watermark draft asks for, or `None` when the draft is
    /// blank and the watermark is being taken away.
    ///
    /// **The size is derived rather than asked for.** Word's box does not
    /// offer one either: it fits the words to the page. A turned shape's
    /// bounding box is `(w + h) / root two` across *and* down, so the width
    /// that just fits the text area is `side * root two / (1 + 1/aspect)` —
    /// which on US Letter with "CONFIDENTIAL" gives 529.5 points against the
    /// 527.75 Word itself wrote, a third of a per cent apart. The aspect is
    /// the string's own, measured, so the letters keep their proportions
    /// instead of being stretched into a shape someone guessed.
    pub(super) fn watermark_shape(
        &mut self,
        draft: &WatermarkDraft,
    ) -> Option<wp_model::doc::Drawing> {
        /// The width-to-height proportion of a watermark whose words could
        /// not be measured. Word's own "CONFIDENTIAL" is four to one.
        const DEFAULT_ASPECT: f64 = 4.0;
        use wp_model::doc::{Alignment, DrawingPosition, Offset, RelativeTo, ShapeText, Wrap};
        let text = draft.text.trim();
        if text.is_empty() {
            return None;
        }
        let family = match draft.font.trim() {
            "" => "Calibri".to_owned(),
            named => named.to_owned(),
        };
        let request = wp_layout::FontRequest {
            family: family.clone().into(),
            size: 100.0,
            bold: false,
            italic: false,
            kern: false,
        };
        let section = &self.document.section;
        let across = section.text_width().points();
        let down = (section.page.height.points() - section.margins.top.points())
            - section.margins.bottom.points();
        // Measured when there is a shaper to measure with. There is one
        // whenever a window is open, which is whenever this box can be
        // reached; the fallback is for a document driven without a screen,
        // and is the proportion Word's own "CONFIDENTIAL" comes out at.
        let aspect = self
            .shaper
            .as_mut()
            .map(|shaper| {
                use wp_layout::Shaper as _;
                let metrics = shaper.metrics(&request);
                (
                    shaper.width(text, &request),
                    metrics.ascent + metrics.descent,
                )
            })
            .filter(|(natural, tall)| *natural > 0.0 && *tall > 0.0)
            .map(|(natural, tall)| natural / tall)
            .unwrap_or(DEFAULT_ASPECT);
        let width = match draft.diagonal {
            true => across.min(down) * std::f64::consts::SQRT_2 / (1.0 + 1.0 / aspect),
            false => across,
        };
        let width = width.max(1.0);
        Some(wp_model::doc::Drawing {
            // No source: the writer authors the VML afresh, which is what
            // makes an edited watermark actually reach the file.
            source: Vec::new().into(),
            source_format: wp_model::SourceFormat::Authored,
            anchored: true,
            extent: (
                wp_model::Emu::from_points(width),
                wp_model::Emu::from_points(width / aspect),
            ),
            rel: None,
            chart: None,
            name: None,
            description: None,
            wrap: Wrap::None,
            distance: Default::default(),
            position: Some(Box::new(DrawingPosition {
                horizontal: Offset {
                    relative_to: RelativeTo::Margin,
                    offset: None,
                    align: Some(Alignment::Center),
                },
                vertical: Offset {
                    relative_to: RelativeTo::Margin,
                    offset: None,
                    align: Some(Alignment::Center),
                },
            })),
            behind_text: true,
            tone: None,
            outline: None,
            text: Some(Box::new(ShapeText {
                text: text.into(),
                font: Some(family.into()),
                color: Some(
                    wp_model::Color::from_val(draft.color.trim())
                        .filter(|color| !color.is_auto())
                        // Word's own watermark grey, lightened the way its
                        // half-opaque fill lightens it.
                        .unwrap_or(wp_model::Color::Rgb([0xE0, 0xE0, 0xE0])),
                ),
                bold: false,
                italic: false,
                // What Word draws for a watermark it reads out of a `.docx`,
                // which is where this one is going.
                stretch: true,
                rotation: if draft.diagonal { 315.0 } else { 0.0 },
            })),
        })
    }
}
