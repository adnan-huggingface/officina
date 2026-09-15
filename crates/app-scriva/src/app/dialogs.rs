//! Scriva's boxes: Insert Table, colour, paragraph, column width, picture
//! size, fonts and zoom. Page Setup, Font, Go To, Word Count and Help have
//! files of their own beside this one.
//!
//! Each is drawn from a draft the command that opened it filled in, and each
//! answers Enter and Escape through `ui_kit::dialog`, with its first field
//! holding the keyboard on opening. Moved out of `app.rs` whole, so that the
//! file a change to one box touches is the file of boxes.

use super::*;

impl Scriva {
    /// The insert-table box: how many columns and rows, the way Word asks.
    pub(super) fn table_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.table_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-table"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(260.0);
                    ui.label(egui::RichText::new("Insert Table").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    for (index, (label, field)) in ["Columns:", "Rows:"]
                        .into_iter()
                        .zip(draft.iter_mut())
                        .enumerate()
                    {
                        ui.horizontal(|ui| {
                            ui.add_sized([72.0, 20.0], egui::Label::new(label));
                            match index == 0 {
                                true => {
                                    dialog::first_field(ui, "scriva-table", field, 64.0);
                                }
                                false => {
                                    dialog::field(ui, field, 64.0);
                                }
                            }
                        });
                    }
                    ui.add_space(12.0);
                    if let Some(answer) = dialog::submit(ui, "Insert") {
                        done = Some(answer);
                    }
                });
            });
        self.table_draft = Some(draft.clone());
        match done {
            Some(true) => {
                self.table_draft = None;
                // Word's own ceiling on columns; a number that does not parse
                // inserts nothing rather than guessing.
                let parse = |text: &str, most: usize| {
                    text.trim()
                        .parse::<usize>()
                        .ok()
                        .filter(|v| (1..=most).contains(v))
                };
                if let (Some(columns), Some(rows)) = (parse(&draft[0], 63), parse(&draft[1], 32767))
                {
                    self.insert_table(rows, columns);
                }
            }
            Some(false) => self.table_draft = None,
            None => {}
        }
    }

    /// More Colours…: Word's standard colours in five tints, a hex field,
    /// three sliders, a well showing the answer, and the last six chosen.
    /// The hex field is the one truth; the grid and the sliders write it.
    pub(super) fn color_dialog(&mut self, ctx: &egui::Context) {
        let Some((target, mut draft)) = self.color_draft.clone() else {
            return;
        };
        let recent = self.recent_colours.clone();
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-color"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(380.0);
                    let title = match target {
                        ColorTarget::Text => "Text Colour",
                        ColorTarget::Borders => "Border Colour",
                    };
                    ui.label(egui::RichText::new(title).font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    let current = wp_model::Color::from_val(draft.trim()).and_then(|c| match c {
                        wp_model::Color::Rgb(rgb) => Some(rgb),
                        _ => None,
                    });
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            dialog::section(ui, "Standard colours");
                            if let Some(rgb) = colour_grid(ui, current) {
                                draft = hex_of(rgb);
                            }
                            if !recent.is_empty() {
                                dialog::section(ui, "Recent");
                                ui.horizontal(|ui| {
                                    for rgb in &recent {
                                        if well(ui, *rgb, current == Some(*rgb), 18.0).clicked() {
                                            draft = hex_of(*rgb);
                                        }
                                    }
                                });
                            }
                        });
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            dialog::section(ui, "Colour");
                            dialog::labelled(ui, "Hex:", |ui| {
                                dialog::first_field(ui, "scriva-color", &mut draft, 72.0);
                            });
                            let mut rgb = current.unwrap_or([0, 0, 0]);
                            let mut moved = false;
                            for (label, channel) in [("R:", 0), ("G:", 1), ("B:", 2)] {
                                dialog::labelled(ui, label, |ui| {
                                    ui.scope(|ui| {
                                        dialog::slider_style(ui.style_mut());
                                        moved |= ui
                                            .add(
                                                egui::Slider::new(&mut rgb[channel], 0..=255)
                                                    .show_value(true),
                                            )
                                            .changed();
                                    });
                                });
                            }
                            if moved {
                                draft = hex_of(rgb);
                            }
                            dialog::labelled(ui, "", |ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(72.0, 28.0),
                                    egui::Sense::hover(),
                                );
                                match current {
                                    Some([r, g, b]) => ui.painter().rect(
                                        rect,
                                        ui_kit::theme::RADIUS_CONTROL as f32,
                                        egui::Color32::from_rgb(r, g, b),
                                        egui::Stroke::new(1.0, ui_kit::theme::FIELD_EDGE),
                                        egui::StrokeKind::Inside,
                                    ),
                                    None => ui.painter().rect_stroke(
                                        rect,
                                        ui_kit::theme::RADIUS_CONTROL as f32,
                                        egui::Stroke::new(1.0, ui_kit::theme::INK_FAINT),
                                        egui::StrokeKind::Inside,
                                    ),
                                }
                            });
                        });
                    });
                    ui.add_space(4.0);
                    if let Some(answer) = dialog::submit(ui, "Apply") {
                        done = Some(answer);
                    }
                });
            });
        self.color_draft = Some((target, draft.clone()));
        match done {
            Some(true) => {
                self.color_draft = None;
                // The same spellings a file's `w:val` may use — six hex
                // digits, with or without the `#`, or the word `auto`.
                // Anything else applies nothing rather than guessing at a
                // colour nobody named.
                if let Some(color) = wp_model::Color::from_val(draft.trim()) {
                    if let wp_model::Color::Rgb(rgb) = color {
                        self.recent_colours.retain(|other| *other != rgb);
                        self.recent_colours.insert(0, rgb);
                        self.recent_colours.truncate(6);
                    }
                    match target {
                        ColorTarget::Text => {
                            self.format_runs(move |props| props.color = Some(color))
                        }
                        ColorTarget::Borders => self.color_borders(color),
                    }
                }
            }
            Some(false) => self.color_draft = None,
            None => {}
        }
    }

    /// Colours every rule of the caret's table, cells' own rules included,
    /// so that the table is one colour after it as it was before.
    ///
    /// A table whose rules are all inherited or all off gets Word's
    /// half-point single lines to carry the colour: a colour on no line is no
    /// colour at all, and a menu choice that did nothing visible would read
    /// as broken.
    pub(super) fn color_borders(&mut self, color: wp_model::Color) {
        use wp_model::prop::BorderStyle;
        self.edit_table(move |table, _, _| {
            let borders = &mut table.props.borders;
            let drawn = |edge: &Option<wp_model::prop::Border>| {
                edge.is_some_and(|b| b.style != BorderStyle::None)
            };
            let any_drawn = [
                &borders.top,
                &borders.start,
                &borders.bottom,
                &borders.end,
                &borders.inside_h,
                &borders.inside_v,
            ]
            .into_iter()
            .any(drawn);
            for edge in [
                &mut borders.top,
                &mut borders.start,
                &mut borders.bottom,
                &mut borders.end,
                &mut borders.inside_h,
                &mut borders.inside_v,
            ] {
                match edge {
                    Some(border) if border.style != BorderStyle::None => border.color = Some(color),
                    _ if !any_drawn => {
                        let mut ruled = ruled_edge();
                        ruled.color = Some(color);
                        *edge = Some(ruled);
                    }
                    _ => {}
                }
            }
            for row in &mut table.rows {
                for cell in &mut row.cells {
                    let borders = &mut cell.props.borders;
                    // Only the four sides: on a cell the "inside" slots are
                    // its diagonals, which a rule colour has no business with.
                    for border in [
                        &mut borders.top,
                        &mut borders.start,
                        &mut borders.bottom,
                        &mut borders.end,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if border.style != BorderStyle::None {
                            border.color = Some(color);
                        }
                    }
                }
            }
        });
    }

    /// Table ▸ Merge Cells: the cells the selection runs across become one,
    /// spanning their columns and holding their paragraphs in order.
    ///
    /// A cell that held nothing but an empty paragraph contributes nothing,
    /// as in Word — merging three blank cells must make one blank cell, not
    /// a cell three lines tall. The merged cell keeps the first cell's
    /// properties and the last cell's right-hand rule, which is the rule the
    /// table's edge now falls on.
    pub(super) fn merge_cells(&mut self) {
        let (start, end) = self.selection.ordered();
        let from = edit::table_cell_at(&self.document, self.scope, start);
        let to = edit::table_cell_at(&self.document, self.scope, end);
        let (Some((index, at_row, first)), Some((index_end, row_end, last))) = (from, to) else {
            self.message = Some((
                "Not in a table".to_owned(),
                "Select across the cells of one row first, then try again.".to_owned(),
            ));
            return;
        };
        if index != index_end || at_row != row_end || first == last {
            self.message = Some((
                "Nothing to merge".to_owned(),
                "Select across two or more cells of one row first, then try again.".to_owned(),
            ));
            return;
        }
        self.history.push(
            self.scope,
            edit::Change::Blocks {
                index,
                before: vec![self.document.body[index].clone()],
                now: 1,
            },
        );
        if let Block::Table(table) = &mut self.document.body[index] {
            let column = starting_column(table, at_row, first);
            let row = &mut table.rows[at_row];
            let taken: Vec<wp_model::table::Cell> = row.cells.drain(first + 1..=last).collect();
            let cell = &mut row.cells[first];
            let blank = |content: &[Block]| {
                content.iter().all(|block| match block {
                    Block::Paragraph(p) => p.content.is_empty() && p.props.numbering.is_none(),
                    _ => false,
                })
            };
            let mut content = std::mem::take(&mut cell.content);
            if blank(&content) {
                content.clear();
            }
            let mut span = cell.props.span() as usize;
            for other in taken {
                span += other.props.span() as usize;
                if other.props.borders.end.is_some() {
                    cell.props.borders.end = other.props.borders.end;
                }
                if !blank(&other.content) {
                    content.extend(other.content);
                }
            }
            if content.is_empty() {
                content.push(Block::Paragraph(Paragraph::new()));
            }
            cell.content = content;
            cell.props.grid_span = span as u32;
            let total: i32 = table.grid
                [column.min(table.grid.len())..(column + span).min(table.grid.len())]
                .iter()
                .map(|t| t.0)
                .sum();
            cell.props.width = wp_model::table::Width::Fixed(Twips(total));
        }
        self.selection = Selection::at(clamp(&self.document, self.scope, start));
        self.reveal = Some(self.caret());
        self.changed();
    }

    /// Opens the paragraph box on the caret's paragraph, showing what that
    /// paragraph states itself — not what it inherits, which is the style's
    /// to show.
    pub(super) fn open_paragraph_dialog(&mut self) {
        let caret = self.caret();
        let Some(paragraph) = edit::paragraph_at(&self.document, self.scope, caret.paragraph)
        else {
            return;
        };
        let props = &paragraph.props;
        let points = |t: Option<Twips>| {
            t.map(|t| trim_number(t.0 as f64 / 20.0))
                .unwrap_or_default()
        };
        let inches = |t: Option<Twips>| {
            t.map(|t| trim_number(t.0 as f64 / 1440.0))
                .unwrap_or_default()
        };
        let (line, line_value) = match props.spacing.line {
            Some(LineSpacing::Multiple(Line240::SINGLE)) => (0, String::new()),
            Some(LineSpacing::Multiple(Line240::ONE_AND_A_HALF)) => (1, String::new()),
            Some(LineSpacing::Multiple(Line240::DOUBLE)) => (2, String::new()),
            Some(LineSpacing::Exact(t)) => (3, trim_number(t.0 as f64 / 20.0)),
            Some(LineSpacing::AtLeast(t)) => (4, trim_number(t.0 as f64 / 20.0)),
            Some(LineSpacing::Multiple(line)) => (5, trim_number(line.0 as f64 / 240.0)),
            None => (6, String::new()),
        };
        let draft = ParagraphDraft {
            before: points(props.spacing.before),
            after: points(props.spacing.after),
            left: inches(props.indent.start),
            right: inches(props.indent.end),
            first_line: inches(props.indent.first_line),
            hanging: inches(props.indent.hanging),
            justify: props.justify,
            line,
            line_value,
        };
        self.paragraph_draft = Some((draft.clone(), draft));
    }

    pub(super) fn paragraph_dialog(&mut self, ctx: &egui::Context) {
        let Some((opened, mut draft)) = self.paragraph_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-paragraph"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(320.0);
                    ui.label(egui::RichText::new("Paragraph").font(dialog::heading_font(16.0)));
                    ui.add_space(4.0);
                    dialog::section(ui, "Alignment");
                    dialog::labelled(ui, "", |ui| {
                        use crate::icons::{self, Icon};
                        for (icon, justify, tip) in [
                            (Icon::AlignLeft, Justify::Start, "Left"),
                            (Icon::AlignCenter, Justify::Center, "Centre"),
                            (Icon::AlignRight, Justify::End, "Right"),
                            (Icon::Justify, Justify::Both, "Justify"),
                        ] {
                            let on = draft.justify == Some(justify);
                            if icons::button(ui, icon, on, tip).clicked() {
                                draft.justify = match on {
                                    true => None,
                                    false => Some(justify),
                                };
                            }
                        }
                    });
                    dialog::section(ui, "Spacing");
                    dialog::labelled(ui, "Before:", |ui| {
                        dialog::first_unit_field(
                            ui,
                            "scriva-paragraph",
                            &mut draft.before,
                            "pt",
                            64.0,
                        );
                    });
                    dialog::labelled(ui, "After:", |ui| {
                        dialog::unit_field(ui, &mut draft.after, "pt", 64.0);
                    });
                    dialog::labelled(ui, "Line spacing:", |ui| {
                        egui::ComboBox::from_id_salt("scriva-paragraph-line")
                            .selected_text(LINE_KINDS[draft.line.min(6)])
                            .width(110.0)
                            .show_ui(ui, |ui| {
                                for (index, name) in LINE_KINDS.iter().enumerate() {
                                    if ui.selectable_label(draft.line == index, *name).clicked() {
                                        draft.line = index;
                                    }
                                }
                            });
                        match draft.line {
                            3 | 4 => {
                                dialog::unit_field(ui, &mut draft.line_value, "pt", 56.0);
                            }
                            5 => {
                                dialog::unit_field(ui, &mut draft.line_value, "lines", 56.0);
                            }
                            _ => {}
                        }
                    });
                    dialog::section(ui, "Indentation");
                    for (label, field) in [
                        ("Left:", &mut draft.left),
                        ("Right:", &mut draft.right),
                        ("First line:", &mut draft.first_line),
                        ("Hanging:", &mut draft.hanging),
                    ] {
                        dialog::labelled(ui, label, |ui| {
                            dialog::unit_field(ui, field, "in", 64.0);
                        });
                    }
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Blank leaves it to the style.")
                            .small()
                            .weak(),
                    );
                    ui.add_space(12.0);
                    if let Some(answer) = dialog::submit(ui, "Apply") {
                        done = Some(answer);
                    }
                });
            });
        self.paragraph_draft = Some((opened.clone(), draft.clone()));
        match done {
            Some(true) => {
                self.paragraph_draft = None;
                self.apply_paragraph(&opened, &draft);
            }
            Some(false) => self.paragraph_draft = None,
            None => {}
        }
    }

    /// Applies the paragraph box to the selection: only the fields that
    /// changed since it opened, so that a box opened on one paragraph and
    /// applied to several states one thing about them and leaves the rest of
    /// each as it was.
    pub(super) fn apply_paragraph(&mut self, opened: &ParagraphDraft, draft: &ParagraphDraft) {
        // A blank is a stated absence; a number is twips; a typo changes
        // nothing, as with the margins box.
        let value = |was: &str, now: &str, per_unit: f64| -> Option<Option<Twips>> {
            if was == now {
                return None;
            }
            let now = now.trim();
            if now.is_empty() {
                return Some(None);
            }
            now.parse::<f64>()
                .ok()
                .filter(|v| (-22.0 * per_unit..=22.0 * per_unit).contains(&(v * per_unit)))
                .map(|v| Some(Twips((v * per_unit).round() as i32)))
        };
        let before = value(&opened.before, &draft.before, 20.0);
        let after = value(&opened.after, &draft.after, 20.0);
        let left = value(&opened.left, &draft.left, 1440.0);
        let right = value(&opened.right, &draft.right, 1440.0);
        let first_line = value(&opened.first_line, &draft.first_line, 1440.0);
        let hanging = value(&opened.hanging, &draft.hanging, 1440.0);
        let justify = (opened.justify != draft.justify).then_some(draft.justify);
        let line = ((opened.line, &opened.line_value) != (draft.line, &draft.line_value))
            .then(|| line_spacing_of(draft.line, &draft.line_value))
            .flatten();
        if [before, after, left, right, first_line, hanging]
            .iter()
            .all(Option::is_none)
            && justify.is_none()
            && line.is_none()
        {
            return;
        }
        self.format_paragraphs(move |props| {
            if let Some(justify) = justify {
                props.justify = justify;
            }
            if let Some(line) = line {
                props.spacing.line = line;
            }
            if let Some(v) = before {
                props.spacing.before = v;
            }
            if let Some(v) = after {
                props.spacing.after = v;
            }
            if let Some(v) = left {
                props.indent.start = v;
            }
            if let Some(v) = right {
                props.indent.end = v;
            }
            if let Some(v) = first_line {
                props.indent.first_line = v;
            }
            if let Some(v) = hanging {
                props.indent.hanging = v;
            }
        });
    }

    pub(super) fn column_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.column_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-column"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(260.0);
                    ui.label(egui::RichText::new("Column Width").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_sized([72.0, 20.0], egui::Label::new("Inches:"));
                        dialog::first_field(ui, "scriva-column", &mut draft, 64.0);
                    });
                    ui.add_space(12.0);
                    if let Some(answer) = dialog::submit(ui, "Apply") {
                        done = Some(answer);
                    }
                });
            });
        self.column_draft = Some(draft.clone());
        match done {
            Some(true) => {
                self.column_draft = None;
                // Word's own bounds: nothing narrower than it could hold a
                // character, nothing wider than its widest paper.
                if let Ok(inches) = draft.trim().parse::<f64>() {
                    if (0.05..=22.0).contains(&inches) {
                        self.apply_column_width(Twips((inches * 1440.0).round() as i32));
                    }
                }
            }
            Some(false) => self.column_draft = None,
            None => {}
        }
    }

    /// Word's Size box, for a picture or a chart: two numbers, in inches.
    ///
    /// Dragging a handle is the fast way and this is the exact one. Typing a
    /// width with the ratio locked moves the height with it, which is what the
    /// lock means — Word recomputes the other field as you leave the one you
    /// typed in, and so does this.
    pub(super) fn size_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.size_draft.take() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-size"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(260.0);
                    ui.label(egui::RichText::new("Size").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    let mut typed: Option<bool> = None;
                    for (label, horizontal) in [("Width:", true), ("Height:", false)] {
                        ui.horizontal(|ui| {
                            ui.add_sized([56.0, 20.0], egui::Label::new(label));
                            let field = match horizontal {
                                true => &mut draft.width,
                                false => &mut draft.height,
                            };
                            let field = match horizontal {
                                true => dialog::first_field(ui, "scriva-size", field, 64.0),
                                false => dialog::field(ui, field, 64.0),
                            };
                            if field.changed() {
                                typed = Some(horizontal);
                            }
                            ui.label("in");
                        });
                    }
                    // The locked field follows the typed one, so the box always
                    // shows the size it would set.
                    if let Some(horizontal) = typed.filter(|_| draft.locked) {
                        let (from, to) = match horizontal {
                            true => (&draft.width, draft.ratio.recip()),
                            false => (&draft.height, draft.ratio),
                        };
                        if let Some(value) = from.trim().parse::<f64>().ok().filter(|v| *v > 0.0) {
                            let other = inches(value * to * 72.0);
                            match horizontal {
                                true => draft.height = other,
                                false => draft.width = other,
                            }
                        }
                    }
                    ui.add_space(4.0);
                    ui.checkbox(&mut draft.locked, "Lock aspect ratio");
                    if let Some((width, height)) = draft.natural {
                        ui.add_space(4.0);
                        if ui.button("Original size").clicked() {
                            draft.width = inches(width);
                            draft.height = inches(height);
                        }
                    }
                    ui.add_space(12.0);
                    if let Some(answer) = dialog::confirm(ui, "OK") {
                        done = Some(answer);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        done = Some(true);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        done = Some(false);
                    }
                });
            });
        match done {
            Some(true) => {
                // A field that does not parse keeps the size it had: the box is
                // not the place to argue about a typo.
                let read = |text: &str, was: f64| {
                    text.trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|inches| (0.01..=22.0).contains(inches))
                        .map(|inches| inches * 72.0)
                        .unwrap_or(was)
                };
                let (was_w, was_h) = self
                    .picked_drawing()
                    .map(|drawing| (drawing.extent.0.points(), drawing.extent.1.points()))
                    .unwrap_or((0.0, 0.0));
                let width = read(&draft.width, was_w);
                let height = read(&draft.height, was_h);
                if (width, height) != (was_w, was_h) {
                    self.resize_drawing(draft.picked, width, height);
                }
            }
            Some(false) => {}
            None => self.size_draft = Some(draft),
        }
    }

    /// Word's Zoom box: presets, the two fits, and a percent you can type.
    /// The faces the document asks for that this machine draws in others,
    /// and what that means for the page. Word's Font Substitution box, with
    /// the one thing it leaves out: whether the lines still break where Word
    /// broke them.
    pub(super) fn fonts_dialog(&mut self, ctx: &egui::Context) {
        use ui_kit::fonts::Shown;
        let mut close = false;
        egui::Modal::new(egui::Id::new("scriva-fonts"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(520.0);
                    ui.label(
                        egui::RichText::new("Fonts this document asks for")
                            .font(dialog::heading_font(16.0)),
                    );
                    ui.add_space(8.0);
                    dialog::paragraph(
                        ui,
                        "These faces are not installed on this computer. Each is shown in the face beside it.",
                    );
                    ui.add_space(10.0);
                    egui::Grid::new("scriva-fonts-grid")
                        .num_columns(3)
                        .spacing([18.0, 6.0])
                        .show(ui, |ui| {
                            for shown in &self.substitutions {
                                ui.label(egui::RichText::new(&shown.asked).strong());
                                ui.label(&shown.shown);
                                ui.label(match shown.how {
                                    Shown::Twin => "same widths: lines and pages break as in Word",
                                    Shown::Embedded => "the copy carried in the document",
                                    Shown::StandIn => "the face Word itself stands in",
                                    Shown::Pitched => {
                                        "laid at its own measured widths: lines break as in Word"
                                    }
                                    Shown::Generic => "different widths: lines may break elsewhere",
                                });
                                ui.end_row();
                            }
                        });
                    ui.add_space(12.0);
                    dialog::paragraph(
                        ui,
                        "Installing a face the document names makes it look as it does in Word.\n\
                         A face with the same widths already lays the document out as Word does.",
                    );
                    // Nothing to decide, so one button, answering Enter and
                    // Escape alike.
                    if dialog::row(ui, |ui| dialog::button(ui, "OK", true).clicked()) {
                        close = true;
                    }
                    let keyed = ui.input_mut(|i| {
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                            || i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                    });
                    close |= keyed;
                });
            });
        if close {
            self.fonts_listing = false;
        }
    }

    pub(super) fn zoom_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.zoom_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-zoom"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(240.0);
                    ui.label(egui::RichText::new("Zoom").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    ui.label("Zoom to");
                    let mut preset: Option<i32> = None;
                    ui.horizontal(|ui| {
                        for percent in [200, 100, 75] {
                            if ui.button(format!("{percent}%")).clicked() {
                                preset = Some(percent);
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Page width").clicked() {
                            preset = self.fit_percent(true);
                        }
                        if ui.button("Whole page").clicked() {
                            preset = self.fit_percent(false);
                        }
                    });
                    if let Some(percent) = preset {
                        draft = percent.to_string();
                        self.zoom_fresh = true;
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Percent:");
                        // While the field is untouched its number stays selected,
                        // re-seeded every frame — egui clears the selection on
                        // frames the field is not yet focused, and a keystroke
                        // can arrive on any frame. First touch ends it.
                        if self.zoom_fresh {
                            select_percent(ui.ctx(), &draft);
                        }
                        let field = ui.add(
                            egui::TextEdit::singleline(&mut draft)
                                .id(egui::Id::new("scriva-zoom-percent"))
                                .desired_width(56.0),
                        );
                        ui.label("%");
                        if field.changed() || field.clicked() || field.dragged() {
                            self.zoom_fresh = false;
                        }
                        field.request_focus();
                    });
                    ui.add_space(12.0);
                    if let Some(answer) = dialog::confirm(ui, "OK") {
                        done = Some(answer);
                    }
                    // The percent field is the only thing to type into, so Enter
                    // anywhere in the box means OK — the way Word's box reads it.
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        done = Some(true);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        done = Some(false);
                    }
                });
            });
        self.zoom_draft = Some(draft.clone());
        if done.is_some() {
            // The box closes this frame, and the typing gate has already been
            // decided for it — without this, the very Enter that confirmed
            // the zoom would fall through into the document as a new line.
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
        }
        match done {
            Some(true) => {
                self.zoom_draft = None;
                // A percent that does not parse keeps the zoom it had.
                if let Ok(percent) = draft.trim().trim_end_matches('%').trim().parse::<f64>() {
                    self.view.zoom = percent.clamp(10.0, 500.0) / 100.0;
                }
            }
            Some(false) => self.zoom_draft = None,
            None => {}
        }
    }

    /// The zoom that fits the paper to the desk — its width, or the whole
    /// first page — leaving a little air for the scrollbar and the edges.
    pub(crate) fn fit_percent(&self, width_only: bool) -> Option<i32> {
        let geometry = &self.view.pages().first()?.geometry;
        // The paper's size on the glass is its points times [`view::SCALE`],
        // so the percent that fits is measured against that.
        let fit_w = (self.viewport.x as f64 - 32.0).max(60.0) / (geometry.width * view::SCALE);
        let percent = if width_only {
            fit_w
        } else {
            fit_w.min((self.viewport.y as f64 - 24.0).max(60.0) / (geometry.height * view::SCALE))
        };
        Some(((percent * 100.0).floor() as i32).clamp(10, 500))
    }
}

/// The line spacing the box's kind and number mean: `Some(None)` is the
/// style's own, `None` a number that does not parse.
fn line_spacing_of(kind: usize, value: &str) -> Option<Option<LineSpacing>> {
    let number = || value.trim().parse::<f64>().ok().filter(|v| *v > 0.0);
    Some(match kind {
        0 => Some(LineSpacing::Multiple(Line240::SINGLE)),
        1 => Some(LineSpacing::Multiple(Line240::ONE_AND_A_HALF)),
        2 => Some(LineSpacing::Multiple(Line240::DOUBLE)),
        3 => Some(LineSpacing::Exact(Twips((number()? * 20.0).round() as i32))),
        4 => Some(LineSpacing::AtLeast(Twips(
            (number()? * 20.0).round() as i32
        ))),
        5 => Some(LineSpacing::Multiple(Line240(
            (number()? * 240.0).round() as i32
        ))),
        _ => None,
    })
}

/// A colour as the hex field spells it.
fn hex_of(rgb: [u8; 3]) -> String {
    format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

/// Word's ten standard colours across, each in five tints down — the
/// colour itself, then mixed with white by a fifth, two fifths, three and
/// four — with the chosen one outlined. The colours are the palette's; the
/// pattern is Word's own gallery.
fn colour_grid(ui: &mut egui::Ui, current: Option<[u8; 3]>) -> Option<[u8; 3]> {
    let bases: Vec<[u8; 3]> = crate::toolbar::PALETTE
        .iter()
        .filter(|(name, _)| !matches!(*name, "Black" | "White" | "Gray"))
        .map(|(_, rgb)| *rgb)
        .collect();
    let mut chosen = None;
    ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
    for tint in 0..5 {
        ui.horizontal(|ui| {
            for base in &bases {
                let mix = tint as f32 / 5.0;
                let rgb = base.map(|c| (c as f32 + (255.0 - c as f32) * mix).round() as u8);
                if well(ui, rgb, current == Some(rgb), 18.0).clicked() {
                    chosen = Some(rgb);
                }
            }
        });
    }
    chosen
}

/// One square of colour, outlined in the ink when it is the chosen one.
fn well(ui: &mut egui::Ui, rgb: [u8; 3], on: bool, side: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
    ui.painter()
        .rect_filled(rect, 2.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
    let stroke = match (on, response.hovered()) {
        (true, _) => egui::Stroke::new(2.0, ui_kit::theme::INK),
        (_, true) => egui::Stroke::new(1.0, ui_kit::theme::INK_SOFT),
        _ => egui::Stroke::new(1.0, ui_kit::theme::FIELD_EDGE),
    };
    ui.painter()
        .rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
    response.on_hover_text(hex_of(rgb))
}
