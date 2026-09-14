//! The page surface: the scrolling desk the pages sit on, what the pointer
//! does there — carets, selections, a picture picked, dragged and resized —
//! and where on a page a point lands.

use super::*;

impl Scriva {
    /// The page surface: a scrolling desk with the pages on it.
    pub(super) fn surface(&mut self, ui: &mut egui::Ui) {
        // The percent, taken to the glass: 100% is Word's — a document inch
        // on 96 logical pixels — not a point per point.
        let zoom = (self.view.zoom * view::SCALE) as f32;
        let (extent_w, extent_h) = self.view.extent();
        let outer = ui.available_rect_before_wrap();
        self.viewport = outer.size();
        ui.painter().rect_filled(outer, 0.0, view::desk());
        // At least as wide as the window, so a page narrower than the desk is
        // centred on it rather than pinned to the left edge.
        let desired = egui::vec2(
            (extent_w as f32 * zoom).max(outer.width() - 2.0),
            extent_h as f32 * zoom,
        );

        let scroll = egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
                self.surface_id = Some(response.id);
                // The surface is an editor, not a button. Without this filter
                // egui reads a bare arrow key as "move keyboard focus to the
                // neighbouring widget" — Up walked the focus onto the toolbar,
                // and the caret vanished with it.
                ui.ctx().memory_mut(|m| {
                    m.set_focus_lock_filter(
                        response.id,
                        egui::EventFilter {
                            tab: true,
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            escape: true,
                        },
                    );
                });
                // The pages are laid out against their own width; the extra
                // width the desk has goes half to each side.
                let slack = ((rect.width() - extent_w as f32 * zoom) / 2.0).max(0.0);
                let origin = rect.min + egui::vec2(slack, 0.0);
                let painter = ui.painter_at(rect);
                // Over the paper the pointer is a text cursor, which is how a
                // window says "this is a place where clicking means something".
                // Over a picture it says a different thing, and over one of a
                // selected picture's handles it says which way that handle
                // pulls — the only way a user finds out a picture can be
                // resized at all is the pointer changing shape over it.
                // A handle is a fixed-size target on the glass however far
                // the page is zoomed out; reach is that target measured on
                // the page.
                let reach = crate::drawings::GRIP / zoom.max(0.05) as f64;
                if response.hovered() || self.dragging.is_some() {
                    let over = ui
                        .ctx()
                        .pointer_hover_pos()
                        .and_then(|pointer| self.spot_at(pointer, origin, zoom));
                    // A link is announced the two ways Word announces one: a
                    // tooltip saying where it goes and how to go there, and,
                    // while the key that follows it is held, the hand. Without
                    // either, a link is text that happens to be blue and the
                    // only way to find out it can be followed is to guess.
                    let link = over
                        .and_then(|spot| view::character_over(&self.view, self.scope, spot))
                        .and_then(|caret| self.link_at(caret));
                    match (&link, ui.input(|i| i.modifiers.command)) {
                        (Some(_), true) => ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand),
                        _ => ui.ctx().set_cursor_icon(self.pointer_icon(over, reach)),
                    }
                    // Not while the right-click menu is up: the menu is the
                    // answer to the same question and the tooltip only sits
                    // under it repeating itself.
                    let menu_up = egui::Popup::is_any_open(ui.ctx());
                    if let Some(destination) = link.as_ref().filter(|_| !menu_up) {
                        let where_to = match destination {
                            crate::links::Destination::Away(url) => url.clone(),
                            crate::links::Destination::Here(name) => {
                                format!("{name} (in this document)")
                            }
                        };
                        egui::Tooltip::always_open(
                            ui.ctx().clone(),
                            ui.layer_id(),
                            egui::Id::new("scriva-link"),
                            egui::PopupAnchor::Pointer,
                        )
                        .show(|ui| {
                            ui.label(where_to);
                            ui.label("Ctrl+click to follow link");
                        });
                    }
                }

                // A click on the desk gives keyboard focus to the surface itself
                // (below), so that typing goes somewhere. The caret must stay
                // visible when the surface holds its own focus — only some other
                // widget (a dialog's text field) holding it should hide the caret.
                self.focused = ui
                    .ctx()
                    .memory(|m| m.focused().is_none_or(|id| id == response.id));
                // The caret blinks — on for a beat, off for a beat — and stands
                // solid from every key or click, so that it is always showing
                // where the next letter goes at the moment that matters. Never
                // while a selection shows: the selection says where the caret
                // is, and a caret blinking at the end of it is noise. A frame
                // is asked for at the next change of phase and not before;
                // the window sleeps between.
                let caret_shown = if self.focused && self.selection.is_empty() {
                    let since = ui.input(|i| i.time) - self.blink_from;
                    let beats = (since / ui_kit::theme::BLINK).max(0.0);
                    let remaining = ui_kit::theme::BLINK * (1.0 - beats.fract());
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs_f64(remaining));
                    (beats as u64).is_multiple_of(2)
                } else {
                    true
                };
                // Decode before painting: the painter borrows the pages, and the
                // cache cannot be borrowed mutably at the same time.
                self.pictures.prepare(
                    ui.ctx(),
                    self.package.as_ref(),
                    self.parts.as_ref(),
                    view::image_rels(&self.view).into_iter(),
                );
                self.pictures.prepare_charts(
                    self.package.as_ref(),
                    self.parts.as_ref(),
                    view::chart_rels(&self.view).into_iter(),
                );
                let washes = self.comment_washes();
                let markers = view::paint(
                    &painter,
                    &self.view,
                    self.scope,
                    self.selection,
                    if self.finder.is_some() {
                        &self.find_matches
                    } else {
                        &[]
                    },
                    &washes,
                    caret_shown.then_some(self.caret()),
                    self.focused,
                    zoom,
                    origin,
                    self.shaper.as_mut().expect("a shaper by now"),
                    &self.pictures,
                    self.picked,
                );

                // A comment's marker: hovered, it says who and what; clicked,
                // it selects the words the comment is about.
                let over_marker = ui
                    .ctx()
                    .pointer_latest_pos()
                    .and_then(|pointer| markers.iter().find(|m| m.rect.contains(pointer)))
                    .copied();
                if let Some(marker) = over_marker {
                    if let Some(comment) = self.document.comment(marker.comment) {
                        let first = comment.text().lines().next().unwrap_or("").to_owned();
                        egui::Tooltip::always_open(
                            ui.ctx().clone(),
                            ui.layer_id(),
                            egui::Id::new("scriva-comment-marker"),
                            egui::PopupAnchor::Pointer,
                        )
                        .show(|ui| {
                            ui.label(egui::RichText::new(comment.author.to_string()).strong());
                            ui.label(first);
                        });
                    }
                    if response.clicked() {
                        if let Some(wash) = washes.iter().find(|w| w.comment == marker.comment) {
                            self.scope = wash.scope;
                            self.selection = wash.range;
                            self.picked = None;
                        }
                    }
                }

                // A press decides what the drag is: a picture under the pointer
                // is dragged as an object, and anything else sweeps a selection.
                if over_marker.is_none() && (response.drag_started() || response.clicked()) {
                    // The grip is chosen by where the press landed, not where
                    // the pointer is now: a drag is only reported once it has
                    // moved a few pixels, and a quick pull would already be
                    // off the handle it took hold of.
                    let spot = ui
                        .input(|i| i.pointer.press_origin())
                        .or_else(|| response.interact_pointer_pos())
                        .and_then(|pointer| self.spot_at(pointer, origin, zoom));
                    self.dragging = None;
                    match spot.and_then(|spot| {
                        view::drawing_at(&self.view, self.scope, spot, reach)
                            .map(|found| (spot, found))
                    }) {
                        Some((spot, (picked, rect))) => {
                            let already = self.picked == Some(picked);
                            self.picked = Some(picked);
                            // A handle can only be pulled once it is on the
                            // screen to aim at: the first press on a picture
                            // selects it and drags it about, and the press
                            // after that can take hold of a corner.
                            self.dragging = match already {
                                true => crate::drawings::grip_at(rect, spot.x, spot.y, reach),
                                false => Some(crate::drawings::Grip::Body),
                            };
                            self.drag_from = Some((spot.x, spot.y));
                            ui.ctx().memory_mut(|m| m.request_focus(response.id));
                        }
                        None => self.picked = None,
                    }
                }
                if self.picked.is_none() && over_marker.is_none() {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        if let Some(spot) = self.spot_at(pointer, origin, zoom) {
                            // A click on the part of the page that is *not*
                            // being edited is not a place to put the caret.
                            // While a header is open the text is showing and
                            // not editable — which is what the wash over it
                            // says — and dragging the caret out from under the
                            // keyboard would make a liar of it.
                            if self.click_lands_here(spot) {
                                if let Some(caret) = view::caret_at(&self.view, self.scope, spot) {
                                    let extend = ui.input(|i| i.modifiers.shift) || self.sweeping;
                                    self.set_caret(caret, extend);
                                }
                            }
                        }
                    }
                    // Ctrl+click follows the link under the pointer, which is
                    // Word's gesture and Word's reason for it: the letters of
                    // a link are still text to put a caret in and edit, so a
                    // bare click cannot mean "go there".
                    if response.clicked() && ui.input(|i| i.modifiers.command) {
                        if let Some(destination) = response
                            .interact_pointer_pos()
                            .and_then(|pointer| self.spot_at(pointer, origin, zoom))
                            .and_then(|spot| view::character_over(&self.view, self.scope, spot))
                            .and_then(|caret| self.link_at(caret))
                        {
                            self.follow_link(destination);
                        }
                    }
                    // A second click takes the word and a third takes the
                    // paragraph, the way every word processor since has —
                    // except in the margins. There a double-click opens the
                    // band drawn there, and once one is open a double-click on
                    // the page closes it again and puts the caret where it
                    // landed. Both gestures are Word's, and they are the only
                    // way most people ever reach a header.
                    if response.double_clicked() {
                        let spot = response
                            .interact_pointer_pos()
                            .and_then(|pointer| self.spot_at(pointer, origin, zoom));
                        let band = spot.and_then(|spot| self.band_at(spot));
                        // A margin that is not the band already open — the
                        // footer while the header is up, or either of them
                        // from the text — opens the one drawn there. A margin
                        // that *is* the open band is ordinary text, and a
                        // double-click in it takes a word like anywhere else.
                        let elsewhere = band.is_some_and(|footer| {
                            !self.editing_band() || footer != self.in_footer()
                        });
                        match (elsewhere, self.scope, band) {
                            (true, _, Some(footer)) => {
                                if let Some(spot) = spot {
                                    self.enter_band(spot.page, footer);
                                }
                            }
                            (_, wp_model::Scope::Chrome(_), None) => {
                                self.close_band();
                                if let Some(caret) = spot
                                    .and_then(|spot| view::caret_at(&self.view, self.scope, spot))
                                {
                                    self.set_caret(caret, false);
                                }
                            }
                            _ => {
                                let caret = self.caret();
                                let content = self.paragraph_text(caret.paragraph);
                                let word = text::word_at(&content, caret.offset);
                                self.selection = Selection {
                                    anchor: Caret {
                                        paragraph: caret.paragraph,
                                        offset: word.start,
                                    },
                                    head: Caret {
                                        paragraph: caret.paragraph,
                                        offset: word.end,
                                    },
                                };
                            }
                        }
                    }
                    if response.triple_clicked() {
                        let caret = self.caret();
                        let length = self.paragraph_text(caret.paragraph).len();
                        self.selection = Selection {
                            anchor: Caret {
                                paragraph: caret.paragraph,
                                offset: 0,
                            },
                            head: Caret {
                                paragraph: caret.paragraph,
                                offset: length,
                            },
                        };
                    }
                    // A right-click outside the selection moves the caret
                    // there first, so the menu acts on what was clicked.
                    if response.secondary_clicked() {
                        if let Some(caret) = response
                            .interact_pointer_pos()
                            .and_then(|pointer| self.spot_at(pointer, origin, zoom))
                            .and_then(|spot| view::caret_at(&self.view, self.scope, spot))
                        {
                            let (start, end) = self.selection.ordered();
                            let inside =
                                !self.selection.is_empty() && caret >= start && caret <= end;
                            if !inside {
                                self.set_caret(caret, false);
                            }
                        }
                        ui.ctx().memory_mut(|m| m.request_focus(response.id));
                    }
                }
                // A right-click on a picture selects it, the same as a left one:
                // the menu that comes up is about what was clicked.
                if response.secondary_clicked() {
                    if let Some(found) = response
                        .interact_pointer_pos()
                        .and_then(|pointer| self.spot_at(pointer, origin, zoom))
                        .and_then(|spot| view::drawing_at(&self.view, self.scope, spot, reach))
                    {
                        self.picked = Some(found.0);
                    }
                    // The menu outlives the click that opened it, so what was
                    // under that click has to be remembered rather than asked
                    // for again when the menu is drawn.
                    let clicked = response
                        .interact_pointer_pos()
                        .and_then(|pointer| self.spot_at(pointer, origin, zoom))
                        .and_then(|spot| view::character_over(&self.view, self.scope, spot))
                        .and_then(|caret| self.link_at(caret));
                    self.menu_link = clicked;
                }
                let has_selection = !self.selection.is_empty();
                let picture = self.picked.is_some();
                let mut chosen: Option<Command> = None;
                let mut follow: Option<crate::links::Destination> = None;
                response.context_menu(|ui| {
                    ui.set_min_width(160.0);
                    // A picked picture has its own menu: Cut and Copy are the
                    // text's, and a picture is not a stretch of text.
                    if picture {
                        if ui.button("Cut").clicked() {
                            chosen = Some(Command::Cut);
                            ui.close();
                        }
                        if ui.button("Copy").clicked() {
                            chosen = Some(Command::Copy);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Size…").clicked() {
                            chosen = Some(Command::PictureSize);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Delete").clicked() {
                            chosen = Some(Command::DeletePicture);
                            ui.close();
                        }
                        return;
                    }
                    // Word offers this too, and it is how a reader who never
                    // hears about the modifier follows a link.
                    if let Some(destination) = &self.menu_link {
                        if ui.button("Open Hyperlink").clicked() {
                            follow = Some(destination.clone());
                            ui.close();
                        }
                        ui.separator();
                    }
                    if ui
                        .add_enabled(has_selection, egui::Button::new("Cut"))
                        .clicked()
                    {
                        chosen = Some(Command::Cut);
                        ui.close();
                    }
                    if ui
                        .add_enabled(has_selection, egui::Button::new("Copy"))
                        .clicked()
                    {
                        chosen = Some(Command::Copy);
                        ui.close();
                    }
                    if ui.button("Paste").clicked() {
                        chosen = Some(Command::Paste);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Select All").clicked() {
                        chosen = Some(Command::SelectAll);
                        ui.close();
                    }
                });
                if let Some(command) = chosen {
                    self.run(command);
                }
                if let Some(destination) = follow {
                    self.follow_link(destination);
                }
                if response.drag_started() {
                    self.sweeping = false;
                }
                if response.dragged() {
                    self.sweeping = self.picked.is_none();
                    // The drag is applied a step at a time, from where the
                    // pointer was last frame, so the model always says what is
                    // on the screen and an undo puts back one whole drag.
                    if let (Some(grip), Some(from), Some(pointer)) = (
                        self.dragging,
                        self.drag_from,
                        response.interact_pointer_pos(),
                    ) {
                        if let Some(spot) = self.spot_at(pointer, origin, zoom) {
                            // Shift breaks a corner's hold on the aspect
                            // ratio, for the user who means to stretch.
                            let keep = !ui.input(|i| i.modifiers.shift);
                            self.drag_drawing(grip, spot.x - from.0, spot.y - from.1, keep);
                            self.drag_from = Some((spot.x, spot.y));
                        }
                    }
                }
                if response.drag_stopped() || response.clicked() {
                    self.sweeping = false;
                    self.dragging = None;
                    self.drag_from = None;
                    self.dragged = false;
                }
                // A click anywhere on the desk puts the caret in the document,
                // which is what makes typing go somewhere.
                if response.clicked() && self.picked.is_none() {
                    ui.ctx().memory_mut(|m| m.request_focus(response.id));
                }
                // Scroll to wherever asked for, once the layout is current —
                // a caret has no place on the page until the page exists.
                if self.reveal.is_some() && self.view.is_stale(self.stamp) {
                    ui.ctx().request_repaint();
                } else if let Some(caret) = self.reveal.take() {
                    let prefer = self.reveal_on.take();
                    if let Some((page, rect)) =
                        view::caret_rect_on(&self.view, self.scope, caret, prefer)
                    {
                        let (page_x, page_y) = self.view.page_origin(page);
                        let min = origin
                            + egui::vec2(
                                (page_x as f32 + rect.min.x) * zoom,
                                (page_y as f32 + rect.min.y) * zoom,
                            );
                        let target =
                            egui::Rect::from_min_size(min, egui::vec2(2.0, rect.height() * zoom))
                                .expand2(egui::vec2(0.0, 24.0));
                        ui.scroll_to_rect(target, None);
                    }
                }
                response
            });
        self.scroll = scroll.state.offset.y;
    }

    /// Moves or resizes the picked drawing by one step of a drag.
    ///
    /// The whole drag is one undo entry: the first step records the paragraph as
    /// it was, and the rest change it further without recording again.
    pub(super) fn drag_drawing(
        &mut self,
        grip: crate::drawings::Grip,
        dx: f64,
        dy: f64,
        keep: bool,
    ) {
        use crate::drawings::{moved, resized, Grip};

        let Some(picked) = self.picked else {
            return;
        };
        let Some((page, _)) = view::rect_of(&self.view, self.scope, picked) else {
            return;
        };
        let geometry = match self.view.pages().get(page) {
            Some(page) => page.geometry,
            None => return,
        };
        // Where the paragraph starts on the page, which is what an offset
        // relative to the paragraph is measured from.
        let origin = view::rect_of(&self.view, self.scope, picked)
            .map(|(_, rect)| (rect.0, rect.1))
            .unwrap_or((geometry.start, geometry.top));
        let before = match self
            .document
            .paragraphs_in(self.scope)
            .get(picked.paragraph)
        {
            Some(paragraph) => (*paragraph).clone(),
            None => return,
        };
        let mut paragraphs = self.document.paragraphs_in_mut(self.scope);
        let Some(drawing) = paragraphs
            .get_mut(picked.paragraph)
            .and_then(|paragraph| paragraph.drawing_mut(picked.nth))
        else {
            return;
        };
        let changed = match grip {
            Grip::Body => moved(drawing, &geometry, origin, dx, dy),
            grip => resized(drawing, grip, dx, dy, keep),
        };
        drop(paragraphs);
        if !changed {
            return;
        }
        if !self.dragged {
            self.history.push(
                self.scope,
                crate::edit::Change::Paragraph {
                    index: picked.paragraph,
                    before: Box::new(before),
                },
            );
            self.dragged = true;
        }
        self.changed();
    }

    /// What the pointer should look like where it is.
    ///
    /// The text cursor over text, an arrow over a picture — an object is not a
    /// place to type — and over the handles of the *selected* picture, the
    /// arrow that says which way that handle pulls. A picture nobody has
    /// clicked yet shows no handles, so it must not claim any either.
    pub(super) fn pointer_icon(&self, over: Option<view::Spot>, reach: f64) -> egui::CursorIcon {
        use crate::drawings::Grip;
        let icon = |grip: Grip| match grip {
            Grip::Corner { right, bottom } => match right == bottom {
                true => egui::CursorIcon::ResizeNwSe,
                false => egui::CursorIcon::ResizeNeSw,
            },
            Grip::Edge {
                horizontal: true, ..
            } => egui::CursorIcon::ResizeHorizontal,
            Grip::Edge { .. } => egui::CursorIcon::ResizeVertical,
            // Only an anchored drawing can be dragged about; an inline one is
            // held in place by the words around it.
            Grip::Body => match self.picked_drawing().is_some_and(|d| d.anchored) {
                true => egui::CursorIcon::Move,
                false => egui::CursorIcon::Default,
            },
        };
        // Mid-drag the pointer keeps the shape it started with, wherever it has
        // wandered to — including off the picture, which every drag does.
        if let Some(grip) = self.dragging {
            return icon(grip);
        }
        let Some(spot) = over else {
            return egui::CursorIcon::Text;
        };
        let Some((found, rect)) = view::drawing_at(&self.view, self.scope, spot, reach) else {
            return egui::CursorIcon::Text;
        };
        match self.picked == Some(found) {
            true => crate::drawings::grip_at(rect, spot.x, spot.y, reach)
                .map(icon)
                .unwrap_or(egui::CursorIcon::Default),
            false => egui::CursorIcon::Default,
        }
    }

    /// The drawing the selection names, if it still exists.
    pub(super) fn picked_drawing(&self) -> Option<&wp_model::doc::Drawing> {
        let picked = self.picked?;
        let paragraph = *self
            .document
            .paragraphs_in(self.scope)
            .get(picked.paragraph)?;
        paragraph.drawings().get(picked.nth).copied()
    }

    /// Opens the Size box for the selected picture.
    pub(super) fn open_size_dialog(&mut self) {
        let (Some(picked), Some(drawing)) = (self.picked, self.picked_drawing()) else {
            // The box is about a picture, so say which one is missing rather
            // than doing nothing and leaving the user to guess.
            self.message = Some((
                "Nothing selected".to_owned(),
                "Click the picture or chart to size, then try again.\n\n\
                 A selected picture shows eight handles, and dragging one \
                 resizes it."
                    .to_owned(),
            ));
            return;
        };
        let (width, height) = (drawing.extent.0.points(), drawing.extent.1.points());
        // A picture's own pixels, at the 96 to the inch a screen shot is
        // measured in — the size Reset puts it back to. A chart has no pixels.
        let natural = drawing
            .rel
            .as_deref()
            .filter(|_| drawing.chart.is_none())
            .and_then(|rel| self.pictures.texture(rel))
            .map(|texture| {
                let [w, h] = texture.size();
                (w as f64 * 0.75, h as f64 * 0.75)
            });
        self.size_draft = Some(SizeDraft {
            picked,
            width: inches(width),
            height: inches(height),
            locked: true,
            ratio: match height > 0.0 {
                true => width / height,
                false => 1.0,
            },
            natural,
        });
    }

    /// Sets the selected picture's size, in points. One undo entry.
    pub(super) fn resize_drawing(
        &mut self,
        picked: crate::drawings::Picked,
        width: f64,
        height: f64,
    ) {
        let before = match self
            .document
            .paragraphs_in(self.scope)
            .get(picked.paragraph)
        {
            Some(paragraph) => (*paragraph).clone(),
            None => return,
        };
        let changed = {
            let mut paragraphs = self.document.paragraphs_in_mut(self.scope);
            paragraphs
                .get_mut(picked.paragraph)
                .and_then(|paragraph| paragraph.drawing_mut(picked.nth))
                .is_some_and(|drawing| crate::drawings::set_size(drawing, width, height))
        };
        if !changed {
            return;
        }
        self.history.push(
            self.scope,
            crate::edit::Change::Paragraph {
                index: picked.paragraph,
                before: Box::new(before),
            },
        );
        self.changed();
    }

    /// Takes the picked drawing out of the document.
    pub(super) fn delete_drawing(&mut self) -> bool {
        let Some(picked) = self.picked else {
            return false;
        };
        let before = match self
            .document
            .paragraphs_in(self.scope)
            .get(picked.paragraph)
        {
            Some(paragraph) => (*paragraph).clone(),
            None => return false,
        };
        let removed = {
            let mut paragraphs = self.document.paragraphs_in_mut(self.scope);
            paragraphs
                .get_mut(picked.paragraph)
                .is_some_and(|paragraph| paragraph.remove_drawing(picked.nth))
        };
        if !removed {
            return false;
        }
        self.history.push(
            self.scope,
            crate::edit::Change::Paragraph {
                index: picked.paragraph,
                before: Box::new(before),
            },
        );
        self.picked = None;
        self.changed();
        true
    }

    /// The text every comment is about, with its author's place in the
    /// document's order — nothing while comments are hidden. Worked out once
    /// per document revision, not per frame: the walk is over every paragraph.
    fn comment_washes(&mut self) -> Vec<view::Wash> {
        if !self.view.show_comments {
            return Vec::new();
        }
        if self.washes_for != self.stamp {
            self.washes_for = self.stamp;
            self.comment_ranges = crate::revise::comment_ranges(&self.document);
        }
        self.comment_ranges
            .iter()
            .filter_map(|range| {
                let comment = self.document.comment(range.id)?;
                let author = self
                    .view
                    .authors
                    .iter()
                    .position(|known| *known == comment.author)
                    .unwrap_or(0);
                Some(view::Wash {
                    comment: range.id,
                    scope: range.scope,
                    range: range.range,
                    author,
                })
            })
            .collect()
    }

    /// Turns a window point into a point on a page.
    pub(super) fn spot_at(
        &self,
        pointer: egui::Pos2,
        origin: egui::Pos2,
        zoom: f32,
    ) -> Option<view::Spot> {
        // `origin` is already the top-left of the *pages*, slack included.
        let local = (pointer - origin) / zoom;
        // The same gap the pages were stacked with: this once said sixteen on
        // its own, and a gap widened in one place would have moved every
        // click on the second page eight points up the paper.
        let mut y = view::GAP as f64;
        let width = self.view.extent().0;
        for (index, page) in self.view.pages().iter().enumerate() {
            let left = (width - page.geometry.width) / 2.0;
            let bottom = y + page.geometry.height;
            if (local.y as f64) < bottom || index + 1 == self.view.pages().len() {
                return Some(view::Spot {
                    page: index,
                    x: local.x as f64 - left,
                    y: local.y as f64 - y,
                });
            }
            y = bottom + view::GAP as f64;
        }
        None
    }
}
