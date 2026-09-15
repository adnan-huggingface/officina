//! Headers and footers — the bands above and below the text of a page:
//! which one the caret is in, making one a section does not have yet,
//! first-page and odd/even kinds, linking to the previous section, the band
//! bar, and the page-number field.

use super::*;

impl Scriva {
    /// Opens the header or footer box, prefilled with what is there now: its
    /// text, one paragraph per line, and the formatting of its first words —
    /// the box sets one voice, so the first is the one it can show.
    /// The page the caret is standing on, which is the page a band command
    /// means: "the header" is always some particular page's header.
    pub(super) fn caret_page(&self) -> usize {
        // A band open on page nine is a band of page nine, whatever the layout
        // says about where its first line is drawn. See [`Scriva::band_page`].
        if let Some(page) = self.band_page.filter(|_| self.editing_band()) {
            return page.min(self.view.pages().len().saturating_sub(1));
        }
        view::caret_rect(&self.view, self.scope, self.caret())
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    /// Whether the band being edited is a footer.
    pub(super) fn in_footer(&self) -> bool {
        match self.scope {
            wp_model::Scope::Body => false,
            wp_model::Scope::Chrome(id) => {
                self.document.header(id).is_some_and(|header| header.footer)
            }
        }
    }

    /// Opens a page's header or footer for editing in place.
    ///
    /// **The band is edited where it is drawn.** A header holding a table, a
    /// logo and three fields cannot be stated in a box and read back, so the
    /// caret moves into it instead and every command that works in the text
    /// works there — which is what Word does, and why its header has never
    /// had a dialog.
    pub(super) fn enter_band(&mut self, page: usize, footer: bool) {
        let Some(id) = self.band_body(page, footer) else {
            return;
        };
        if self.scope == wp_model::Scope::Body {
            self.left_behind = Some(self.selection);
        }
        self.go_to(wp_model::Scope::Chrome(id), Caret::default());
        self.reveal = Some(self.caret());
        // The band of *this* page, not of the first page that happens to show
        // the same one: opening the running head on page nine must not scroll
        // the window back to page one.
        self.reveal_on = Some(page);
        self.band_page = Some(page);
    }

    /// Takes the section's headers or footers away — Word's Remove Header,
    /// which removes every kind the section names and not only the one the
    /// page in front of you happens to show.
    ///
    /// Bodies and references go together in one change, because a reference
    /// pointing at nothing is a document Word calls damaged.
    pub(super) fn remove_band(&mut self, footer: bool) {
        let index = self.caret_section();
        let Some(section) = self.document.section_mut(index) else {
            return;
        };
        let going: Vec<wp_model::HeaderId> = match footer {
            true => &section.footers,
            false => &section.headers,
        }
        .iter()
        .map(|reference| reference.body)
        .collect();
        if going.is_empty() {
            return;
        }
        self.close_band();
        self.history
            .push(wp_model::Scope::Body, self.chrome_change());
        if let Some(section) = self.document.section_mut(index) {
            match footer {
                true => section.footers.clear(),
                false => section.headers.clear(),
            }
        }
        // Only the bodies nothing still points at: another section may have
        // been unlinked onto the very same body, and a body a live reference
        // names is a document Word calls damaged.
        let kept: Vec<wp_model::HeaderId> = self
            .document
            .section_props()
            .iter()
            .flat_map(|section| {
                section
                    .headers
                    .iter()
                    .chain(section.footers.iter())
                    .map(|reference| reference.body)
                    .collect::<Vec<_>>()
            })
            .collect();
        self.document
            .headers
            .retain(|header| !going.contains(&header.id) || kept.contains(&header.id));
        self.changed();
    }

    /// One undo entry covering the bodies, every section's references and the
    /// setting that decides which of them a page uses.
    pub(super) fn chrome_change(&self) -> edit::Change {
        edit::Change::Chrome {
            headers: self.document.headers.clone(),
            sections: self.document.section_props(),
            settings: Box::new(self.document.settings.clone()),
            caret: self.caret(),
        }
    }

    /// Which section the page the caret is on belongs to.
    ///
    /// A band command means *this page's* band, and in a document of several
    /// sections that is not the same as the last section's — which is what
    /// `Document::section` alone would have answered.
    pub(super) fn caret_section(&self) -> usize {
        let page = self.caret_page();
        self.view
            .pages()
            .get(page)
            .map(|page| page.section)
            .unwrap_or_else(|| self.document.sections().len().saturating_sub(1))
    }

    /// Back to the text, where the caret was before the band was opened.
    pub(super) fn close_band(&mut self) {
        self.band_page = None;
        if self.scope == wp_model::Scope::Body {
            return;
        }
        self.scope = wp_model::Scope::Body;
        self.picked = None;
        let back = self.left_behind.take().unwrap_or_default();
        self.selection = Selection {
            anchor: clamp(&self.document, wp_model::Scope::Body, back.anchor),
            head: clamp(&self.document, wp_model::Scope::Body, back.head),
        };
    }

    /// Which kind of band a page asks for: its own if it is a title page or
    /// an even one and the document says those differ, and the default
    /// otherwise.
    pub(super) fn band_kind(&self, page: usize) -> wp_model::HeaderKind {
        let laid = self.view.pages().get(page);
        let number = laid.map(|page| page.number).unwrap_or(1);
        let index = laid
            .map(|page| page.section)
            .unwrap_or_else(|| self.document.sections().len().saturating_sub(1));
        let sections = self.document.sections();
        let section = sections
            .get(index)
            .map(|(_, section)| *section)
            .unwrap_or(&self.document.section);
        section
            .header_for_page(number, self.document.settings.even_and_odd_headers)
            .unwrap_or(wp_model::HeaderKind::Default)
    }

    /// Which body holds the band this page shows — making an empty one, with
    /// the section reference that gives it its identity, when the page has
    /// none.
    ///
    /// The *kind* is the one the page asked for rather than the default: a
    /// title page in a section with a first-page header of its own has to
    /// open that one, and a new header made for a title page has to be the
    /// first-page header or it will not appear on the page it was made from.
    pub(super) fn band_body(&mut self, page: usize, footer: bool) -> Option<wp_model::HeaderId> {
        let kind = self.band_kind(page);
        // The laid-out page first, because it is what the user is looking at;
        // the section after it, because the layout is a frame behind — and it
        // is exactly one frame behind at the moment a band has just been made,
        // which is when making a second one would go unnoticed.
        let laid = self.view.pages().get(page).and_then(|page| match footer {
            true => page.footer_body,
            false => page.header_body,
        });
        match laid {
            Some(id) => Some(id),
            None => self.band_of_kind(page, kind, footer),
        }
    }

    /// The section's band of one kind, made if it has none.
    ///
    /// Asked directly, without the laid-out page, by the two switches that
    /// change *which* kind a page wants: the layout still shows the band the
    /// page wanted a moment ago, and following it would carry on editing the
    /// one that is no longer drawn there.
    pub(super) fn band_of_kind(
        &mut self,
        page: usize,
        kind: wp_model::HeaderKind,
        footer: bool,
    ) -> Option<wp_model::HeaderId> {
        let index = self.page_section(page);
        // What the page shows now, with "Link to Previous" followed: a section
        // that inherits its band already has one to edit, and making a second
        // would silently unlink it.
        let shown = self.document.bands();
        let existing = shown.get(index).and_then(|bands| match footer {
            true => bands.footer(kind),
            false => bands.header(kind),
        });
        if existing.is_some() {
            return existing;
        }
        self.make_band(index, kind, footer, Vec::new())
    }

    /// Makes a band of one kind for one section and points the section at it.
    ///
    /// `content` is what goes in it: nothing for a band being made from
    /// scratch, and a copy of the inherited one for a section being unlinked —
    /// which is what Word does, measured: unlink a section's header and the
    /// words stay on the page while the section before it keeps its own copy.
    ///
    /// A body and the reference to it are one change, because restoring either
    /// without the other leaves a reference pointing at nothing.
    pub(super) fn make_band(
        &mut self,
        index: usize,
        kind: wp_model::HeaderKind,
        footer: bool,
        content: Vec<Block>,
    ) -> Option<wp_model::HeaderId> {
        use wp_model::doc::HeaderFooter;
        use wp_model::section::{HeaderId, HeaderRef};
        self.history
            .push(wp_model::Scope::Body, self.chrome_change());
        let id = HeaderId(
            self.document
                .headers
                .iter()
                .map(|header| header.id.0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let content = match content.is_empty() {
            true => {
                let sections = self.document.sections();
                let section = sections
                    .get(index)
                    .map(|(_, section)| *section)
                    .unwrap_or(&self.document.section);
                vec![Block::Paragraph(band_paragraph(section))]
            }
            false => content,
        };
        self.document.headers.push(HeaderFooter {
            id,
            part: None,
            rel: None,
            footer,
            content,
        });
        let section = self.document.section_mut(index)?;
        let refs = match footer {
            true => &mut section.footers,
            false => &mut section.headers,
        };
        refs.push(HeaderRef {
            kind,
            body: id,
            rel: None,
        });
        self.changed();
        Some(id)
    }

    /// Which section a laid-out page belongs to.
    pub(super) fn page_section(&self, page: usize) -> usize {
        self.view
            .pages()
            .get(page)
            .map(|page| page.section)
            .unwrap_or_else(|| self.document.sections().len().saturating_sub(1))
    }

    /// Word's "Different first page" and "Different odd & even pages" — the
    /// two switches that decide how many bands a section has and which of
    /// them any given page shows.
    ///
    /// Turning either on does not fill the new band in: Word leaves it empty
    /// and so does this, which is why the caret follows to whichever band the
    /// page in front of the user now wants. Turning one off leaves the band
    /// it stops using in the document, exactly as Word does — a switch is not
    /// a delete, and flicking it back has to bring the header back with it.
    pub(super) fn set_band_kinds(&mut self, title_page: bool, even_and_odd: bool) {
        let page = self.caret_page();
        let index = self.page_section(page);
        // `<w:titlePg>` belongs to the section the page is in — a preface may
        // have a title page where the chapters after it do not — while the
        // even/odd flag is the document's and covers all of them.
        let was = self
            .document
            .sections()
            .get(index)
            .map(|(_, section)| section.title_page)
            .unwrap_or(self.document.section.title_page);
        if title_page == was && even_and_odd == self.document.settings.even_and_odd_headers {
            return;
        }
        let footer = self.in_footer();
        self.history
            .push(wp_model::Scope::Body, self.chrome_change());
        if let Some(section) = self.document.section_mut(index) {
            section.title_page = title_page;
        }
        self.document.settings.even_and_odd_headers = even_and_odd;
        self.changed();
        if self.editing_band() {
            let kind = self.band_kind(page);
            if let Some(id) = self.band_of_kind(page, kind, footer) {
                self.go_to(wp_model::Scope::Chrome(id), Caret::default());
            }
        }
    }

    /// Whether the band the caret's page shows is inherited from the section
    /// before it — Word's "Link to Previous".
    ///
    /// `None` in the first section of a document, which has nothing to link
    /// to and so is never offered the switch.
    pub(super) fn linked_to_previous(&self, footer: bool) -> Option<bool> {
        let page = self.caret_page();
        let index = self.page_section(page);
        if index == 0 {
            return None;
        }
        let kind = self.band_kind(page);
        let sections = self.document.sections();
        let (_, section) = sections.get(index)?;
        Some(!wp_model::Bands::is_own(section, kind, footer))
    }

    /// Links the caret's section to the one before it, or breaks the link.
    ///
    /// **Breaking it copies rather than empties.** Word's own answer, measured
    /// over COM: unlink a second section's header and the words are still
    /// there, while the first section keeps a copy of its own — so the two can
    /// then be changed apart. Linking again drops this section's reference and
    /// leaves its body in the document, so that flicking the switch back does
    /// not cost the words that were typed into it.
    pub(super) fn set_link_to_previous(&mut self, linked: bool) {
        let page = self.caret_page();
        let index = self.page_section(page);
        if index == 0 || self.linked_to_previous(self.in_footer()) == Some(linked) {
            return;
        }
        let footer = self.in_footer();
        let kind = self.band_kind(page);
        if linked {
            self.history
                .push(wp_model::Scope::Body, self.chrome_change());
            if let Some(section) = self.document.section_mut(index) {
                let refs = match footer {
                    true => &mut section.footers,
                    false => &mut section.headers,
                };
                refs.retain(|reference| reference.kind != kind);
            }
            self.changed();
            // Whatever the section inherits now is what the page shows, and
            // that is what the caret must be in — or it is editing a band that
            // is no longer drawn anywhere.
            match self
                .document
                .bands()
                .get(index)
                .and_then(|bands| match footer {
                    true => bands.footer(kind),
                    false => bands.header(kind),
                }) {
                Some(id) => self.go_to(wp_model::Scope::Chrome(id), Caret::default()),
                None => self.close_band(),
            }
            return;
        }
        let inherited = self
            .document
            .bands()
            .get(index)
            .and_then(|bands| match footer {
                true => bands.footer(kind),
                false => bands.header(kind),
            })
            .and_then(|id| self.document.header(id))
            .map(|band| band.content.clone())
            .unwrap_or_default();
        if let Some(id) = self.make_band(index, kind, footer, inherited) {
            self.go_to(wp_model::Scope::Chrome(id), Caret::default());
        }
    }

    /// Which band of a page a point is in: the margin above the text, or the
    /// one below it. `None` between them, which is the body.
    pub(super) fn band_at(&self, spot: view::Spot) -> Option<bool> {
        let page = self.view.pages().get(spot.page)?;
        if spot.y < page.geometry.top {
            return Some(false);
        }
        if spot.y > page.geometry.height - page.geometry.bottom {
            return Some(true);
        }
        None
    }

    /// Whether a click there is a click in the flow being edited.
    ///
    /// The rest of the page is showing but not being edited, and a click on it
    /// must not drag the caret out from under the keyboard — the same reason
    /// the veil is drawn over it.
    pub(super) fn click_lands_here(&self, spot: view::Spot) -> bool {
        match self.scope {
            wp_model::Scope::Body => self.band_at(spot).is_none(),
            wp_model::Scope::Chrome(_) => self.band_at(spot) == Some(self.in_footer()),
        }
    }

    /// The bar that stands under the toolbar while a band is open, saying
    /// which one and offering the two ways out of it.
    pub(super) fn band_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let footer = self.in_footer();
        let page = self.caret_page();
        let index = self.page_section(page);
        let kind = match self.band_kind(page) {
            wp_model::HeaderKind::First => "First page",
            wp_model::HeaderKind::Even => "Even pages",
            wp_model::HeaderKind::Default if self.document.settings.even_and_odd_headers => {
                "Odd pages"
            }
            wp_model::HeaderKind::Default => "Every page",
        };
        let was = (
            self.document
                .sections()
                .get(index)
                .map(|(_, section)| section.title_page)
                .unwrap_or(self.document.section.title_page),
            self.document.settings.even_and_odd_headers,
        );
        let (mut title_page, mut even_and_odd) = was;
        // `None` in the first section: there is nothing before it to link to,
        // and a switch that can only ever be off is a switch that misleads.
        let linked = self.linked_to_previous(footer);
        let mut link = linked.unwrap_or(false);
        let mut chosen = None;
        // The application's theme leaves an unchecked box with no outline at
        // all — fine on a menu row, where the tick is the only state worth
        // showing, and wrong here: a switch nobody can see until it is already
        // on is a switch nobody finds.
        {
            let widgets = &mut ui.style_mut().visuals.widgets;
            widgets.inactive.bg_fill = egui::Color32::WHITE;
            widgets.inactive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(0x8C));
            widgets.hovered.bg_fill = egui::Color32::WHITE;
            widgets.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(0x5C));
            widgets.active.bg_fill = egui::Color32::WHITE;
        }
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(match footer {
                    true => "Footer",
                    false => "Header",
                })
                .strong()
                .color(ui_kit::theme::INK),
            );
            // Which band, in the words the page-kind switches use, so that a
            // reader who ticks "Different first page" sees the name change.
            ui.label(
                egui::RichText::new(format!("Section {} · {kind}", index + 1))
                    .color(ui_kit::theme::INK_SOFT),
            );
            ui.add_space(12.0);
            // Word keeps these two on the Header & Footer tab, and they belong
            // here for the same reason: they are only ever wanted while one is
            // open, and they decide which one you are looking at.
            ui.checkbox(&mut title_page, "Different first page")
                .on_hover_text("The first page of the section carries a band of its own");
            ui.checkbox(&mut even_and_odd, "Different odd & even")
                .on_hover_text("Left- and right-hand pages carry different bands");
            if linked.is_some() {
                ui.checkbox(&mut link, "Link to Previous").on_hover_text(
                    "Show the section before this one's band. Turning it off \
                     takes a copy this section can change on its own.",
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                if ui
                    .button("Close  Esc")
                    .on_hover_text("Go back to the text — Esc, or double-click the page")
                    .clicked()
                {
                    chosen = Some(Command::CloseChrome);
                }
                if ui
                    .button(match footer {
                        true => "Go to header",
                        false => "Go to footer",
                    })
                    .clicked()
                {
                    chosen = Some(Command::SwitchBand);
                }
                if ui
                    .button("From edge…")
                    .on_hover_text("How far the header and footer sit from the paper's edges")
                    .clicked()
                {
                    chosen = Some(Command::CustomMargins);
                }
            });
        });
        if (title_page, even_and_odd) != was {
            self.set_band_kinds(title_page, even_and_odd);
        }
        if linked.is_some_and(|was| was != link) {
            self.set_link_to_previous(link);
        }
        chosen
    }

    /// Puts a page number at the caret — Word's Insert ▸ Page Number, which is
    /// a field and not a typed digit, so it counts on every page it is drawn.
    pub(super) fn insert_page_field(&mut self, of_pages: bool) {
        use wp_model::doc::Piece;
        let caret = self.caret();
        let Some(before) = edit::paragraph_at(&self.document, self.scope, caret.paragraph) else {
            return;
        };
        self.history.push(
            self.scope,
            edit::Change::Paragraph {
                index: caret.paragraph,
                before: Box::new(before),
            },
        );
        // The cached "1" between the separator and the end is what a reader
        // that does not evaluate fields shows, and what Word writes.
        let field = |code: &str| {
            [
                Piece::FieldStart {
                    dirty: false,
                    lock: false,
                },
                Piece::Instruction(format!(" {code} ").into()),
                Piece::FieldSeparate,
                Piece::Text("1".into()),
                Piece::FieldEnd,
            ]
        };
        let mut pieces: Vec<Piece> = Vec::new();
        if of_pages {
            pieces.push(Piece::Text("Page ".into()));
        }
        pieces.extend(field("PAGE"));
        if of_pages {
            pieces.push(Piece::Text(" of ".into()));
            pieces.extend(field("NUMPAGES"));
        }
        let mut offset = caret.offset;
        {
            let mut paragraphs = self.document.paragraphs_in_mut(self.scope);
            let Some(target) = paragraphs.get_mut(caret.paragraph) else {
                return;
            };
            offset += crate::text::insert_pieces(target, offset, pieces);
        }
        self.selection = Selection::at(Caret {
            paragraph: caret.paragraph,
            offset,
        });
        self.changed();
    }
}
