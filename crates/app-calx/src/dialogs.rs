//! Calx's boxes: every dialog the grid can put up, drawn from the draft in
//! `Calx::dialog`, and the tabs of Format Cells.
//!
//! One method draws whichever box is up and answers it; each answers Enter
//! and Escape through `ui_kit::dialog`, and the grid below is kept from the
//! frame's keys while one is up and for the frame after it closes.

use super::*;

impl Calx {
    /// Whichever modal is open, drawn and answered.
    ///
    /// The dialog is taken out of `self` for the duration so that its own state
    /// can be edited while the application is borrowed to act on it, and put
    /// back unless something closed it.
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let mut keep = true;
        match &mut dialog {
            Dialog::RenameSheet { index, text } => {
                let index = *index;
                let mut accept = false;
                modal(ctx, "Rename sheet", |ui| {
                    let chars = text.chars().count();
                    let field = dialog::field(ui, text, ui.available_width());
                    dialog::focus_on_open(
                        ui,
                        egui::Id::new(("calx-modal", "Rename sheet")),
                        &field,
                        chars,
                    );
                    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        accept = true;
                    }
                    ui.add_space(4.0);
                    // Told *before* pressing OK rather than after: the refusal
                    // is about the name being typed, and finding out on submit
                    // means retyping it.
                    if let Some(why) = self
                        .doc
                        .workbook
                        .sheet_name_refusal(text.trim(), Some(index))
                    {
                        ui.colored_label(egui::Color32::from_rgb(0xB0, 0x30, 0x20), why);
                    }
                    match dialog::submit(ui, "Rename") {
                        Some(true) => accept = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if accept {
                    let name = text.clone();
                    self.rename_sheet(index, &name);
                    keep = false;
                }
            }

            Dialog::GoTo { text } => {
                let mut accept = false;
                modal(ctx, "Go to", |ui| {
                    let field = ui.text_edit_singleline(text);
                    if text.is_empty() {
                        field.request_focus();
                    }
                    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        accept = true;
                    }
                    ui.add_space(2.0);
                    ui.weak("A cell, a range, or a defined name — B12, A1:D9, Sales.");
                    ui.add_space(4.0);
                    match dialog::submit(ui, "Go") {
                        Some(true) => accept = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if accept {
                    let target = text.clone();
                    self.go_to(&target);
                    keep = false;
                }
            }

            Dialog::Validation { rule, existing } => {
                use ss_model::cond::{DvKind, DvOperator, DvSeverity};
                let existing = *existing;
                let (mut accept, mut remove) = (false, false);
                modal(ctx, "Data validation", |ui| {
                    ui.label(
                        egui::RichText::new(format!("Applies to {}", ranges_label(&rule.ranges)))
                            .weak()
                            .small(),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Allow:");
                        egui::ComboBox::from_id_salt("calx-dv-kind")
                            .selected_text(dv_kind_name(rule.kind))
                            .show_ui(ui, |ui| {
                                for kind in [
                                    DvKind::List,
                                    DvKind::Whole,
                                    DvKind::Decimal,
                                    DvKind::Date,
                                    DvKind::Time,
                                    DvKind::TextLength,
                                    DvKind::Custom,
                                ] {
                                    ui.selectable_value(&mut rule.kind, kind, dv_kind_name(kind));
                                }
                            });
                        ui.checkbox(&mut rule.allow_blank, "Ignore blank");
                    });
                    match rule.kind {
                        DvKind::List => {
                            ui.horizontal(|ui| {
                                ui.label("Source:");
                                ui.add(
                                    egui::TextEdit::singleline(&mut rule.formula1)
                                        .hint_text("\"Yes,No\" or A1:A9")
                                        .desired_width(220.0),
                                );
                            });
                            ui.checkbox(&mut rule.show_dropdown, "In-cell dropdown");
                        }
                        DvKind::Custom => {
                            ui.horizontal(|ui| {
                                ui.label("Formula:");
                                ui.add(
                                    egui::TextEdit::singleline(&mut rule.formula1)
                                        .hint_text("=ISNUMBER(A1)")
                                        .desired_width(220.0),
                                );
                            });
                        }
                        DvKind::None => {}
                        _ => {
                            ui.horizontal(|ui| {
                                ui.label("Data:");
                                egui::ComboBox::from_id_salt("calx-dv-op")
                                    .selected_text(dv_op_name(rule.operator))
                                    .show_ui(ui, |ui| {
                                        for op in [
                                            DvOperator::Between,
                                            DvOperator::NotBetween,
                                            DvOperator::Equal,
                                            DvOperator::NotEqual,
                                            DvOperator::GreaterThan,
                                            DvOperator::LessThan,
                                            DvOperator::GreaterThanOrEqual,
                                            DvOperator::LessThanOrEqual,
                                        ] {
                                            ui.selectable_value(
                                                &mut rule.operator,
                                                op,
                                                dv_op_name(op),
                                            );
                                        }
                                    });
                            });
                            let two = matches!(
                                rule.operator,
                                DvOperator::Between | DvOperator::NotBetween
                            );
                            ui.horizontal(|ui| {
                                ui.label(if two { "Minimum:" } else { "Value:" });
                                ui.add(
                                    egui::TextEdit::singleline(&mut rule.formula1)
                                        .desired_width(120.0),
                                );
                                if two {
                                    ui.label("Maximum:");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut rule.formula2)
                                            .desired_width(120.0),
                                    );
                                }
                            });
                        }
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("On error:");
                        egui::ComboBox::from_id_salt("calx-dv-severity")
                            .selected_text(match rule.severity {
                                DvSeverity::Stop => "Stop",
                                DvSeverity::Warning => "Warning",
                                DvSeverity::Information => "Information",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut rule.severity, DvSeverity::Stop, "Stop");
                                ui.selectable_value(
                                    &mut rule.severity,
                                    DvSeverity::Warning,
                                    "Warning",
                                );
                                ui.selectable_value(
                                    &mut rule.severity,
                                    DvSeverity::Information,
                                    "Information",
                                );
                            });
                        ui.add(
                            egui::TextEdit::singleline(&mut rule.error_title)
                                .hint_text("Error title")
                                .desired_width(140.0),
                        );
                    });
                    ui.add(
                        egui::TextEdit::singleline(&mut rule.error_message)
                            .hint_text("Error message")
                            .desired_width(320.0),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut rule.prompt_title)
                                .hint_text("Prompt title")
                                .desired_width(140.0),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut rule.prompt_message)
                                .hint_text("Prompt message")
                                .desired_width(172.0),
                        );
                    });
                    // Right to left, which is the order `dialog::row` lays a
                    // group out in: Cancel ends up on the right.
                    dialog::row(ui, |ui| {
                        keep &= !dialog::button(ui, "Cancel", false).clicked();
                        ui.add_enabled_ui(existing.is_some(), |ui| {
                            remove = dialog::button(ui, "Remove", false)
                                .on_hover_text("Delete this rule from every cell it covers")
                                .clicked();
                        });
                        accept = dialog::button(ui, "OK", true).clicked();
                    });
                    match dialog::answered(ui) {
                        Some(true) => accept = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                let sheet = self.grid.sheet_index;
                if accept {
                    if let Some(s) = self.doc.workbook.sheet(sheet) {
                        let mut list = s.validations.clone();
                        match existing {
                            Some(i) if i < list.len() => list[i] = rule.clone(),
                            _ => list.push(rule.clone()),
                        }
                        self.perform(Change::new(
                            "Data validation",
                            vec![Patch::Validations {
                                sheet,
                                validations: list,
                            }],
                        ));
                        self.grid.invalidate();
                    }
                    keep = false;
                }
                if remove {
                    if let (Some(s), Some(i)) = (self.doc.workbook.sheet(sheet), existing) {
                        let mut list = s.validations.clone();
                        if i < list.len() {
                            list.remove(i);
                        }
                        self.perform(Change::new(
                            "Remove validation",
                            vec![Patch::Validations {
                                sheet,
                                validations: list,
                            }],
                        ));
                        self.grid.invalidate();
                    }
                    keep = false;
                }
            }

            Dialog::CondFormat {
                formats,
                kind,
                operator,
                value1,
                value2,
                bold,
                italic,
                use_text_color,
                text_color,
                use_fill,
                fill_color,
            } => {
                use ss_model::cond::{CfKind, CfOperator, CfRule, CfValue, CfValueKind};
                const KINDS: [&str; 9] = [
                    "Cell value",
                    "Text contains",
                    "Duplicate values",
                    "Unique values",
                    "Top 10",
                    "Above average",
                    "Formula is true",
                    "Colour scale",
                    "Data bar",
                ];
                let mut accept = false;
                let mut add = false;
                let selection = self.grid.selection.ranges().to_vec();
                modal(ctx, "Conditional formatting", |ui| {
                    // The sheet's rules, each with its own delete.
                    let mut delete: Option<(usize, usize)> = None;
                    if formats.is_empty() {
                        ui.weak("No rules on this sheet yet.");
                    }
                    for (b, block) in formats.iter().enumerate() {
                        for (r, rule) in block.rules.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.small_button("✕").on_hover_text("Delete rule").clicked() {
                                    delete = Some((b, r));
                                }
                                ui.small(format!(
                                    "{} — {}",
                                    ranges_label(&block.ranges),
                                    cf_rule_label(rule)
                                ));
                            });
                        }
                    }
                    if let Some((b, r)) = delete {
                        formats[b].rules.remove(r);
                        if formats[b].rules.is_empty() {
                            formats.remove(b);
                        }
                    }
                    ui.separator();
                    ui.label("New rule over the selection:");
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("calx-cf-kind")
                            .selected_text(KINDS[*kind])
                            .show_ui(ui, |ui| {
                                for (i, name) in KINDS.iter().enumerate() {
                                    ui.selectable_value(kind, i, *name);
                                }
                            });
                        match *kind {
                            0 => {
                                egui::ComboBox::from_id_salt("calx-cf-op")
                                    .selected_text(cf_op_name(*operator))
                                    .show_ui(ui, |ui| {
                                        for op in [
                                            CfOperator::GreaterThan,
                                            CfOperator::GreaterThanOrEqual,
                                            CfOperator::LessThan,
                                            CfOperator::LessThanOrEqual,
                                            CfOperator::Equal,
                                            CfOperator::NotEqual,
                                            CfOperator::Between,
                                            CfOperator::NotBetween,
                                        ] {
                                            ui.selectable_value(operator, op, cf_op_name(op));
                                        }
                                    });
                                ui.add(egui::TextEdit::singleline(value1).desired_width(80.0));
                                if matches!(operator, CfOperator::Between | CfOperator::NotBetween)
                                {
                                    ui.label("and");
                                    ui.add(egui::TextEdit::singleline(value2).desired_width(80.0));
                                }
                            }
                            1 => {
                                ui.add(
                                    egui::TextEdit::singleline(value1)
                                        .hint_text("text")
                                        .desired_width(140.0),
                                );
                            }
                            4 => {
                                ui.label("rank");
                                ui.add(
                                    egui::TextEdit::singleline(value1)
                                        .hint_text("10")
                                        .desired_width(50.0),
                                );
                            }
                            6 => {
                                ui.add(
                                    egui::TextEdit::singleline(value1)
                                        .hint_text("=A1>B1")
                                        .desired_width(180.0),
                                );
                            }
                            _ => {}
                        }
                    });
                    // The format the rule paints with, for the dxf kinds.
                    if *kind <= 6 {
                        ui.horizontal(|ui| {
                            ui.checkbox(bold, "Bold");
                            ui.checkbox(italic, "Italic");
                            ui.checkbox(use_text_color, "Text");
                            if *use_text_color {
                                ui.color_edit_button_srgb(text_color);
                            }
                            ui.checkbox(use_fill, "Fill");
                            if *use_fill {
                                ui.color_edit_button_srgb(fill_color);
                            }
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.label("Colour:");
                            ui.color_edit_button_srgb(fill_color);
                        });
                    }
                    dialog::row(ui, |ui| {
                        keep &= !dialog::button(ui, "Cancel", false).clicked();
                        accept = dialog::button(ui, "OK", true).clicked();
                        // Set apart from the two answers: it adds a rule to the
                        // list rather than answering the dialog.
                        ui.add_space(12.0);
                        add = dialog::button(ui, "Add rule", false).clicked();
                    });
                    match dialog::answered(ui) {
                        Some(true) => accept = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if add && !selection.is_empty() {
                    let wins = formats
                        .iter()
                        .flat_map(|f| f.rules.iter())
                        .map(|r| r.priority)
                        .min()
                        .unwrap_or(2)
                        - 1;
                    let dxf =
                        if *kind <= 6 && (*bold || *italic || *use_text_color || *use_fill) {
                            let [r, g, b] = *text_color;
                            let [fr, fg, fb] = *fill_color;
                            Some(self.doc.workbook.styles.add_dxf(ss_model::style::Dxf {
                                bold: bold.then_some(true),
                                italic: italic.then_some(true),
                                color: use_text_color.then_some(Color::rgb(r, g, b)),
                                fill: use_fill.then_some(ss_model::style::Fill::solid(Color::rgb(
                                    fr, fg, fb,
                                ))),
                                ..Default::default()
                            }))
                        } else {
                            None
                        };
                    let [fr, fg, fb] = *fill_color;
                    let visual_color = Color::rgb(fr, fg, fb);
                    let stop = |kind: CfValueKind| CfValue {
                        kind,
                        value: String::new(),
                    };
                    let rule_kind = match *kind {
                        0 => {
                            let mut formulas = vec![value1.clone()];
                            if matches!(operator, CfOperator::Between | CfOperator::NotBetween) {
                                formulas.push(value2.clone());
                            }
                            CfKind::CellIs {
                                operator: *operator,
                                formulas,
                            }
                        }
                        1 => CfKind::Text {
                            op: ss_model::cond::TextOp::Contains,
                            text: value1.clone(),
                        },
                        2 => CfKind::Duplicates { unique: false },
                        3 => CfKind::Duplicates { unique: true },
                        4 => CfKind::Top10 {
                            rank: value1.trim().parse().unwrap_or(10),
                            percent: false,
                            bottom: false,
                        },
                        5 => CfKind::AboveAverage {
                            above: true,
                            equal_average: false,
                            std_dev: None,
                        },
                        6 => CfKind::Expression {
                            formula: value1.trim_start_matches('=').to_string(),
                        },
                        7 => CfKind::ColorScale {
                            stops: vec![stop(CfValueKind::Min), stop(CfValueKind::Max)],
                            colors: vec![Color::rgb(0xFF, 0xFF, 0xFF), visual_color],
                        },
                        _ => CfKind::DataBar {
                            min: stop(CfValueKind::Min),
                            max: stop(CfValueKind::Max),
                            color: visual_color,
                            show_value: true,
                        },
                    };
                    formats.push(ss_model::cond::ConditionalFormat {
                        ranges: selection,
                        rules: vec![CfRule {
                            kind: rule_kind,
                            dxf,
                            priority: wins,
                            stop_if_true: false,
                        }],
                    });
                    value1.clear();
                    value2.clear();
                }
                if accept {
                    let sheet = self.grid.sheet_index;
                    self.perform(Change::new(
                        "Conditional formatting",
                        vec![Patch::ConditionalFormats {
                            sheet,
                            formats: formats.clone(),
                        }],
                    ));
                    self.grid.invalidate();
                    keep = false;
                }
            }

            Dialog::MoveSheet {
                index,
                before,
                copy,
            } => {
                let (index, mut go) = (*index, false);
                let names: Vec<String> = self
                    .doc
                    .workbook
                    .sheets
                    .iter()
                    .map(|s| s.name.clone())
                    .collect();
                modal(ctx, "Move or copy sheet", |ui| {
                    ui.label("Before sheet:");
                    ui_kit::scroll::show(
                        ui,
                        egui::ScrollArea::vertical().max_height(180.0),
                        |ui| {
                            for (i, name) in names.iter().enumerate() {
                                if ui.selectable_label(*before == i, name).clicked() {
                                    *before = i;
                                }
                            }
                            if ui
                                .selectable_label(*before >= names.len(), "(move to end)")
                                .clicked()
                            {
                                *before = names.len();
                            }
                        },
                    );
                    ui.checkbox(copy, "Create a copy");
                    match dialog::submit(ui, "OK") {
                        Some(true) => go = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if go {
                    let (before, copy) = (*before, *copy);
                    self.move_sheet(index, before, copy);
                    keep = false;
                }
            }

            Dialog::Size { axis, text } => {
                let axis = *axis;
                let (title, unit, default, ceiling) = match axis {
                    Axis::Columns => (
                        "Column width",
                        "characters",
                        grid::axis::DEFAULT_COLUMN_CHARS,
                        255,
                    ),
                    Axis::Rows => ("Row height", "points", grid::axis::DEFAULT_ROW_POINTS, 409),
                };
                let mut accept = false;
                modal(ctx, title, |ui| {
                    ui.horizontal(|ui| {
                        let chars = text.chars().count();
                        let field = dialog::field(ui, text, 90.0);
                        dialog::focus_on_open(
                            ui,
                            egui::Id::new(("calx-modal", title)),
                            &field,
                            chars,
                        );
                        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            accept = true;
                        }
                        ui.label(unit);
                    });
                    // The units are the file's, not the screen's, so the number
                    // typed here is the number the next reader will see. Saying
                    // what the default is turns "8.43" from an odd number into
                    // the one everything else on the sheet already is.
                    ui.label(
                        egui::RichText::new(format!("Default {default}"))
                            .weak()
                            .small(),
                    );
                    if parse_size(text, axis).is_none() {
                        ui.colored_label(
                            egui::Color32::from_rgb(0xB0, 0x30, 0x20),
                            format!("A number from 0 to {ceiling}. Zero hides."),
                        );
                    }
                    match dialog::submit(ui, "OK") {
                        Some(true) => accept = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if accept {
                    match parse_size(text, axis) {
                        Some(size) => {
                            self.resize_selection(axis, size);
                            keep = false;
                        }
                        // OK on an unreadable number: the dialog stays, and
                        // the status line says why — a button that silently
                        // does nothing reads as a broken button.
                        None => {
                            self.status =
                                format!("\"{}\" is not a number from 0 to {ceiling}", text.trim());
                        }
                    }
                }
            }

            Dialog::Zoom { text, fresh } => {
                let mut accept = false;
                modal(ctx, "Zoom", |ui| {
                    ui.label("Magnification");
                    let mut preset: Option<i32> = None;
                    ui.horizontal(|ui| {
                        for percent in [200, 100, 75, 50, 25] {
                            if ui.button(format!("{percent}%")).clicked() {
                                preset = Some(percent);
                            }
                        }
                    });
                    if let Some(percent) = preset {
                        *text = percent.to_string();
                        *fresh = true;
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Custom:");
                        // While the field is untouched its number stays
                        // selected, re-seeded every frame — egui clears the
                        // selection on frames the field is not yet focused,
                        // and a keystroke can arrive on any frame. First
                        // touch ends it.
                        if *fresh {
                            select_zoom_percent(ui.ctx(), text);
                        }
                        let chars = text.chars().count();
                        let field = ui.add(
                            egui::TextEdit::singleline(text)
                                .id(egui::Id::new("calx-zoom-percent"))
                                .desired_width(56.0),
                        );
                        ui.label("%");
                        if field.changed() || field.clicked() || field.dragged() {
                            *fresh = false;
                        }
                        dialog::focus_on_open(
                            ui,
                            egui::Id::new(("calx-modal", "Zoom")),
                            &field,
                            chars,
                        );
                    });
                    match dialog::submit(ui, "OK") {
                        Some(true) => accept = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if accept {
                    // A percent that does not parse keeps the zoom it had.
                    if let Ok(percent) = text.trim().trim_end_matches('%').trim().parse::<f64>() {
                        self.grid.set_zoom(percent.clamp(10.0, 400.0) / 100.0);
                    }
                    keep = false;
                }
            }

            Dialog::Names { names, editing } => {
                let sheet_names: Vec<String> = self
                    .doc
                    .workbook
                    .sheets
                    .iter()
                    .map(|s| s.name.clone())
                    .collect();
                let here = self.grid.sheet_index;
                let selection = format!(
                    "{}!{}",
                    ss_formula::translate::quote_sheet(
                        &sheet_names[here.min(sheet_names.len() - 1)]
                    ),
                    absolute(self.grid.selection.active_range())
                );
                let mut save = false;
                let mut remove: Option<usize> = None;
                // A row opened this frame, by a button the Enter key pressed:
                // that Enter is spent, and must not also finish the row.
                let before = *editing;
                modal(ctx, "Names", |ui| {
                    ui.set_min_width(560.0);
                    ui_kit::scroll::show(
                        ui,
                        egui::ScrollArea::vertical().max_height(300.0),
                        |ui| {
                            egui::Grid::new("calx-names")
                                .num_columns(5)
                                .spacing([8.0, 6.0])
                                .striped(true)
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new("Name").strong());
                                    ui.label(egui::RichText::new("Refers to").strong());
                                    ui.label(egui::RichText::new("Scope").strong());
                                    ui.label("");
                                    ui.end_row();
                                    for (index, entry) in names.iter_mut().enumerate() {
                                        let open = *editing == Some(index);
                                        if open {
                                            // The row just opened — by New or
                                            // by Edit — takes the keyboard in
                                            // its name, selected, as Excel's
                                            // New Name box does: what is typed
                                            // next is the name.
                                            dialog::first_field(
                                                ui,
                                                egui::Id::new(("calx-names-row", index)),
                                                &mut entry.name,
                                                120.0,
                                            );
                                            ui.add(
                                                egui::TextEdit::singleline(&mut entry.refers_to)
                                                    .desired_width(220.0)
                                                    .font(egui::TextStyle::Monospace),
                                            );
                                            egui::ComboBox::from_id_salt((
                                                "calx-name-scope",
                                                index,
                                            ))
                                            .selected_text(match entry.scope {
                                                None => "Workbook".to_string(),
                                                Some(i) => sheet_names
                                                    .get(i)
                                                    .cloned()
                                                    .unwrap_or_else(|| "?".into()),
                                            })
                                            .width(110.0)
                                            .show_ui(
                                                ui,
                                                |ui| {
                                                    ui.selectable_value(
                                                        &mut entry.scope,
                                                        None,
                                                        "Workbook",
                                                    );
                                                    for (i, name) in sheet_names.iter().enumerate()
                                                    {
                                                        ui.selectable_value(
                                                            &mut entry.scope,
                                                            Some(i),
                                                            name,
                                                        );
                                                    }
                                                },
                                            );
                                            if ui.button("Done").clicked() {
                                                *editing = None;
                                            }
                                        } else {
                                            ui.label(&entry.name);
                                            ui.label(
                                                egui::RichText::new(&entry.refers_to).monospace(),
                                            );
                                            ui.label(match entry.scope {
                                                None => "Workbook".to_string(),
                                                Some(i) => sheet_names
                                                    .get(i)
                                                    .cloned()
                                                    .unwrap_or_else(|| "?".into()),
                                            });
                                            ui.horizontal(|ui| {
                                                if ui.button("Edit").clicked() {
                                                    *editing = Some(index);
                                                }
                                                if ui.button("Delete").clicked() {
                                                    remove = Some(index);
                                                }
                                            });
                                        }
                                        ui.end_row();
                                    }
                                });
                        },
                    );
                    if names.is_empty() {
                        ui.label(
                            egui::RichText::new("This workbook has no names yet")
                                .weak()
                                .small(),
                        );
                    }
                    ui.add_space(6.0);
                    // Told about a clash while it is being typed rather than on
                    // submit, which is the only moment the answer is useful.
                    for (index, entry) in names.iter().enumerate() {
                        if let Some(why) = self.doc.workbook.defined_name_refusal(
                            entry.name.trim(),
                            entry.scope,
                            Some(index),
                        ) {
                            ui.colored_label(
                                egui::Color32::from_rgb(0xB0, 0x30, 0x20),
                                format!("{}: {why}", entry.name),
                            );
                        }
                    }
                    dialog::row(ui, |ui| {
                        keep &= !dialog::button(ui, "Cancel", false).clicked();
                        save = dialog::button(ui, "Save", true).clicked();
                        ui.add_space(12.0);
                        if dialog::button(ui, "New", false)
                            .on_hover_text(format!("Refers to {selection}"))
                            .clicked()
                        {
                            names.push(ss_model::DefinedName {
                                name: unused_name(names),
                                refers_to: selection.clone(),
                                scope: None,
                            });
                            *editing = Some(names.len() - 1);
                        }
                    });
                    // Enter finishes the row being edited, and saves the list
                    // when no row is: the first Enter is "this name is done",
                    // the second "these names are done". Before, Enter did
                    // nothing while a row was open, and the only key that
                    // left it was Escape, which threw the whole list away.
                    let opened = editing.is_some() && *editing != before;
                    match dialog::answered(ui) {
                        Some(true) if opened => {}
                        Some(true) if editing.is_none() => save = true,
                        Some(true) => *editing = None,
                        Some(false) => keep = false,
                        _ => {}
                    }
                });
                if let Some(index) = remove {
                    names.remove(index);
                    *editing = None;
                }
                if save {
                    let names: Vec<ss_model::DefinedName> = names
                        .iter()
                        .filter(|n| !n.name.trim().is_empty())
                        .cloned()
                        .collect();
                    self.perform(Change::new("Names", vec![Patch::DefinedNames { names }]));
                    keep = false;
                }
            }

            Dialog::FormatCells { look, tab } => {
                let mut apply = false;
                // Cloned out before the closure, which borrows `look`: a theme
                // is a dozen colours, and the alternative is to hand the whole
                // workbook to a function that wants three of them.
                let theme = self.doc.workbook.styles.theme().clone();
                modal(ctx, "Format cells", |ui| {
                    ui.set_min_width(460.0);
                    ui.horizontal(|ui| {
                        for (which, label) in FormatTab::ALL {
                            if ui.selectable_label(*tab == which, label).clicked() {
                                *tab = which;
                            }
                        }
                    });
                    ui.separator();
                    ui_kit::scroll::show(
                        ui,
                        egui::ScrollArea::vertical().max_height(320.0),
                        |ui| match tab {
                            FormatTab::Number => number_tab(ui, look),
                            FormatTab::Alignment => alignment_tab(ui, look),
                            FormatTab::Font => font_tab(ui, &theme, look),
                            FormatTab::Border => border_tab(ui, &theme, look),
                            FormatTab::Fill => fill_tab(ui, &theme, look),
                            FormatTab::Protection => protection_tab(ui, look),
                        },
                    );
                    match dialog::submit(ui, "OK") {
                        Some(true) => apply = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if apply {
                    let look = look.clone();
                    self.format(Format::Whole(look));
                    keep = false;
                }
            }

            Dialog::Note { at, author, text } => {
                let mut apply = false;
                let title = format!("Note on {}", at.to_a1());
                modal(ctx, &title, |ui| {
                    ui.set_width(360.0);
                    ui.horizontal(|ui| {
                        ui.label("Author");
                        ui.add(egui::TextEdit::singleline(author).desired_width(220.0));
                    });
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .desired_width(f32::INFINITY)
                            .desired_rows(5),
                    );
                    ui.add_space(4.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        ui.small("An empty note is no note: clearing the text removes it.");
                    });
                    match dialog::confirm(ui, "OK") {
                        Some(true) => apply = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if apply {
                    let (at, author, text) = (*at, author.clone(), text.clone());
                    self.set_note(at, &author, &text);
                    keep = false;
                }
            }

            Dialog::TextToColumns { how, other } => {
                let mut go = false;
                modal(ctx, "Text to columns", |ui| {
                    ui.set_width(340.0);
                    ui.label("Split the selected column at:");
                    ui.add_space(4.0);
                    for (label, ch) in [
                        ("Tab", '\t'),
                        ("Semicolon", ';'),
                        ("Comma", ','),
                        ("Space", ' '),
                    ] {
                        let mut on = how.delimiters.contains(&ch);
                        if ui.checkbox(&mut on, label).changed() {
                            if on {
                                how.delimiters.push(ch);
                            } else {
                                how.delimiters.retain(|d| *d != ch);
                            }
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label("Other");
                        // One character, because a delimiter is one: a box
                        // holding `, ` would quietly split on neither.
                        if ui
                            .add(egui::TextEdit::singleline(other).desired_width(40.0))
                            .changed()
                        {
                            other.truncate(other.chars().count().min(1));
                        }
                    });
                    ui.add_space(6.0);
                    ui.checkbox(&mut how.merge, "Treat consecutive delimiters as one");
                    let mut quoted = how.quote.is_some();
                    if ui
                        .checkbox(&mut quoted, "Text in quotes stays together")
                        .changed()
                    {
                        how.quote = quoted.then_some('"');
                    }
                    ui.add_space(6.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        for line in [
                            "The fields land in the column itself and the ones",
                            "to its right, over whatever is already there.",
                        ] {
                            ui.small(line);
                        }
                    });
                    match dialog::submit(ui, "Split") {
                        Some(true) => go = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if go {
                    let mut how = how.clone();
                    if let Some(c) = other.chars().next() {
                        if !how.delimiters.contains(&c) {
                            how.delimiters.push(c);
                        }
                    }
                    self.text_to_columns(&how);
                    keep = false;
                }
            }

            Dialog::RemoveDuplicates {
                range,
                columns,
                header,
            } => {
                let mut go = false;
                let where_ = format!("{}:{}", range.start.to_a1(), range.end.to_a1());
                modal(ctx, "Remove duplicates", |ui| {
                    ui.set_width(300.0);
                    ui.label(format!("Looking through {where_}"));
                    ui.add_space(4.0);
                    ui.checkbox(header, "My data has headers");
                    ui.add_space(4.0);
                    ui.label("Rows repeat when these columns match:");
                    ui.horizontal(|ui| {
                        if ui.small_button("Select all").clicked() {
                            for (_, on) in columns.iter_mut() {
                                *on = true;
                            }
                        }
                        if ui.small_button("Select none").clicked() {
                            for (_, on) in columns.iter_mut() {
                                *on = false;
                            }
                        }
                    });
                    ui_kit::scroll::show(
                        ui,
                        egui::ScrollArea::vertical().max_height(280.0),
                        |ui| {
                            for (col, on) in columns.iter_mut() {
                                ui.checkbox(on, ss_model::column_name(*col));
                            }
                        },
                    );
                    match dialog::submit(ui, "Remove") {
                        Some(true) => go = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if go {
                    let chosen: Vec<u32> = columns
                        .iter()
                        .filter(|(_, on)| *on)
                        .map(|(c, _)| *c)
                        .collect();
                    let (range, header) = (*range, *header);
                    self.remove_duplicates(range, &chosen, header);
                    keep = false;
                }
            }

            Dialog::Protect { allow } => {
                let mut go = false;
                modal(ctx, "Protect sheet", |ui| {
                    ui.set_width(340.0);
                    ui.label("Allow everyone who uses this sheet to:");
                    ui.add_space(4.0);
                    ui_kit::scroll::show(
                        ui,
                        egui::ScrollArea::vertical().max_height(360.0),
                        |ui| {
                            for (label, field) in protection_fields(allow) {
                                ui.checkbox(field, label);
                            }
                        },
                    );
                    ui.add_space(6.0);
                    // Broken by hand and laid out left to right explicitly: a
                    // modal stretches its children and centres what is in them,
                    // which turns a paragraph into a monument.
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        for line in [
                            "Protection guards against accidents rather than",
                            "against anyone determined: Calx sets no password,",
                            "and a sheet protected here can be unprotected here.",
                        ] {
                            ui.small(line);
                        }
                    });
                    match dialog::submit(ui, "Protect") {
                        Some(true) => go = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if go {
                    let allow = (**allow).clone();
                    self.protect(Some(allow));
                    keep = false;
                }
            }

            Dialog::PasteSpecial { how } => {
                let mut go = false;
                modal(ctx, "Paste special", |ui| {
                    for kind in ss_formula::clip::PasteKind::ALL {
                        ui.radio_value(&mut how.kind, kind, kind.label());
                    }
                    ui.add_space(6.0);
                    ui.checkbox(&mut how.transpose, "Transpose");
                    ui.checkbox(&mut how.skip_blanks, "Skip blanks")
                        .on_hover_text("A blank in the copy leaves what is already there");
                    match dialog::submit(ui, "Paste") {
                        Some(true) => go = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if go {
                    let how = *how;
                    let text = self.os_clipboard_text();
                    self.paste_how(text, how);
                    keep = false;
                }
            }

            Dialog::Trouble {
                title,
                severity,
                message,
                detail,
                offer_save_as,
            } => {
                let offer = *offer_save_as;
                // Save As is the way out of a refused save, so it is the
                // default when it is offered at all: the box is not asking
                // whether the news was received, it is offering somewhere else
                // to put the work.
                let choices: &[dialog::Choice] = if offer {
                    &[
                        dialog::Choice::new("Save As…").primary(),
                        dialog::Choice::new("OK").escapes(),
                    ]
                } else {
                    &[dialog::Choice::new("OK").primary().escapes()]
                };
                let answer = dialog::message(
                    ctx,
                    "trouble",
                    *severity,
                    title,
                    message,
                    Some(detail.as_str()),
                    choices,
                );
                if answer.is_some() {
                    keep = false;
                }
                if offer && answer == Some(0) {
                    self.save_as();
                }
            }

            Dialog::Find {
                query,
                with,
                replacing,
                whole_workbook,
                report,
            } => {
                let mut command: Option<FindCommand> = None;
                let title = if *replacing { "Replace" } else { "Find" };
                modal(ctx, title, |ui| {
                    egui::Grid::new("calx-find-fields")
                        .num_columns(2)
                        .spacing([8.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("Find what");
                            let field = ui.add(
                                egui::TextEdit::singleline(&mut query.needle).desired_width(240.0),
                            );
                            if query.needle.is_empty() {
                                field.request_focus();
                            }
                            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                command = Some(FindCommand::Next);
                            }
                            ui.end_row();
                            if *replacing {
                                ui.label("Replace with");
                                ui.add(
                                    egui::TextEdit::singleline(with)
                                        .desired_width(240.0)
                                        .hint_text("(nothing)"),
                                );
                                ui.end_row();
                            }
                        });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut query.match_case, "Match case");
                        ui.checkbox(&mut query.whole_cell, "Whole cell");
                    });
                    ui.horizontal(|ui| {
                        ui.checkbox(whole_workbook, "All sheets");
                        // Excel calls this Look in, and the choice decides what
                        // a replacement can reach: only the source of a cell is
                        // ever rewritten.
                        ui.label("Look in");
                        egui::ComboBox::from_id_salt("calx-find-in")
                            .selected_text(if query.in_formulas {
                                "Formulas"
                            } else {
                                "Values"
                            })
                            .width(90.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut query.in_formulas, true, "Formulas");
                                ui.selectable_value(&mut query.in_formulas, false, "Values");
                            });
                    });
                    ui.label(
                        egui::RichText::new("* matches any run, ? any one character")
                            .weak()
                            .small(),
                    );
                    if !report.is_empty() {
                        ui.label(egui::RichText::new(report.as_str()).small());
                    }
                    // Right to left, so this reads backwards: Close on the
                    // right, then the searches, with Find next — the one Enter
                    // in the field also does — filled and furthest left.
                    dialog::row(ui, |ui| {
                        keep &= !dialog::button(ui, "Close", false).clicked();
                        ui.add_space(12.0);
                        if *replacing {
                            if dialog::button(ui, "Replace all", false).clicked() {
                                command = Some(FindCommand::ReplaceAll);
                            }
                            if dialog::button(ui, "Replace", false).clicked() {
                                command = Some(FindCommand::ReplaceOne);
                            }
                        }
                        if dialog::button(ui, "Find previous", false).clicked() {
                            command = Some(FindCommand::Previous);
                        }
                        if dialog::button(ui, "Find next", true).clicked() {
                            command = Some(FindCommand::Next);
                        }
                    });
                    match dialog::answered(ui) {
                        Some(true) if command.is_none() => command = Some(FindCommand::Next),
                        Some(false) => keep = false,
                        _ => {}
                    }
                });
                if let Some(command) = command {
                    let outcome = self.find_command(command, query, with, *whole_workbook);
                    *report = outcome;
                }
            }

            Dialog::Sort {
                range,
                header,
                levels,
            } => {
                let (range, mut go) = (*range, false);
                let columns: Vec<u32> = (range.start.col..=range.end.col).collect();
                let names: Vec<String> = columns
                    .iter()
                    .map(|col| self.column_label(range, *header, *col))
                    .collect();
                modal(ctx, "Sort", |ui| {
                    ui.label(format!("Range {}", range_label(range)));
                    ui.checkbox(header, "My data has headers");
                    ui.add_space(6.0);
                    egui::Grid::new("calx-sort-levels")
                        .num_columns(3)
                        .spacing([8.0, 6.0])
                        .show(ui, |ui| {
                            for (n, level) in levels.iter_mut().enumerate() {
                                ui.label(if n == 0 { "Sort by" } else { "Then by" });
                                let selected = match level.col {
                                    Some(col) => columns
                                        .iter()
                                        .position(|c| *c == col)
                                        .map_or("(none)".to_string(), |i| names[i].clone()),
                                    None => "(none)".to_string(),
                                };
                                egui::ComboBox::from_id_salt(("calx-sort-col", n))
                                    .selected_text(selected)
                                    .width(180.0)
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(level.col.is_none(), "(none)")
                                            .clicked()
                                        {
                                            level.col = None;
                                        }
                                        for (col, name) in columns.iter().zip(&names) {
                                            if ui
                                                .selectable_label(level.col == Some(*col), name)
                                                .clicked()
                                            {
                                                level.col = Some(*col);
                                            }
                                        }
                                    });
                                egui::ComboBox::from_id_salt(("calx-sort-dir", n))
                                    .selected_text(if level.descending {
                                        "Z to A"
                                    } else {
                                        "A to Z"
                                    })
                                    .width(90.0)
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(!level.descending, "A to Z")
                                            .clicked()
                                        {
                                            level.descending = false;
                                        }
                                        if ui.selectable_label(level.descending, "Z to A").clicked()
                                        {
                                            level.descending = true;
                                        }
                                    });
                                ui.end_row();
                            }
                        });
                    match dialog::submit(ui, "Sort") {
                        Some(true) => go = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if go {
                    let (header, levels) = (*header, *levels);
                    self.sort_by(range, header, &levels);
                    keep = false;
                }
            }

            Dialog::Filter {
                col,
                offered,
                has_blanks,
                ticked,
                blanks,
                search,
            } => {
                let (col, has_blanks) = (*col, *has_blanks);
                let mut go = false;
                let mut clear = false;
                modal(ctx, "Filter", |ui| {
                    ui.text_edit_singleline(search)
                        .on_hover_text("Narrows the list below; it does not filter by itself");
                    let needle = search.to_lowercase();
                    let visible: Vec<&String> = offered
                        .iter()
                        .filter(|v| needle.is_empty() || v.to_lowercase().contains(&needle))
                        .collect();

                    ui.horizontal(|ui| {
                        if ui.small_button("Select all").clicked() {
                            for value in &visible {
                                ticked.insert((*value).clone());
                            }
                            *blanks = has_blanks;
                        }
                        if ui.small_button("Select none").clicked() {
                            for value in &visible {
                                ticked.remove(*value);
                            }
                            *blanks = false;
                        }
                    });
                    ui.separator();

                    ui_kit::scroll::show(
                        ui,
                        egui::ScrollArea::vertical().max_height(260.0),
                        |ui| {
                            for value in visible {
                                let mut on = ticked.contains(value);
                                if ui.checkbox(&mut on, value).changed() {
                                    if on {
                                        ticked.insert(value.clone());
                                    } else {
                                        ticked.remove(value);
                                    }
                                }
                            }
                            if has_blanks && needle.is_empty() {
                                ui.checkbox(blanks, "(Blanks)");
                            }
                        },
                    );
                    dialog::row(ui, |ui| {
                        keep &= !dialog::button(ui, "Cancel", false).clicked();
                        go = dialog::button(ui, "OK", true).clicked();
                        ui.add_space(12.0);
                        clear = dialog::button(ui, "Clear this column", false).clicked();
                    });
                    match dialog::answered(ui) {
                        Some(true) => go = true,
                        Some(false) => keep = false,
                        None => {}
                    }
                });
                if go || clear {
                    let (ticked, blanks) = if clear {
                        (offered.iter().cloned().collect(), has_blanks)
                    } else {
                        (ticked.clone(), *blanks)
                    };
                    let offered = offered.clone();
                    self.set_filter_column(col, ticked, blanks, &offered, has_blanks);
                    keep = false;
                }
            }
        }
        // Escape cancels whichever dialog is up, the way Cancel would —
        // every one of them, because a window Escape cannot leave is a trap.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            keep = false;
        }
        if keep {
            self.dialog = Some(dialog);
        }
    }
}

/// Format Cells ▸ Number. A category list and the code behind it.
///
/// The code box is not a nicety: `#,##0.00;[Red](#,##0.00)` is the only way to
/// say some of what a spreadsheet says, and a fixed list of a dozen entries
/// cannot cover a format language.
pub(super) fn number_tab(ui: &mut egui::Ui, look: &mut ss_model::Look) {
    ui.label("Category");
    for (label, code) in NUMBER_FORMATS {
        if ui
            .selectable_label(look.number_format == *code, *label)
            .clicked()
        {
            look.number_format = code.to_string();
        }
    }
    ui.add_space(6.0);
    ui.label("Format code");
    ui.add(
        egui::TextEdit::singleline(&mut look.number_format)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace),
    );
    // What the code does to a number, before the dialog is closed over it.
    let sample = ss_model::numfmt::NumberFormat::parse(&look.number_format)
        .format(ss_model::numfmt::FormatValue::Number(-1234.567))
        .text;
    ui.label(
        egui::RichText::new(format!("−1234.567 shows as  {sample}"))
            .weak()
            .small(),
    );
}

/// Format Cells ▸ Alignment.
pub(super) fn alignment_tab(ui: &mut egui::Ui, look: &mut ss_model::Look) {
    egui::Grid::new("calx-align")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("Horizontal");
            egui::ComboBox::from_id_salt("calx-halign")
                .selected_text(halign_name(look.alignment.horizontal))
                .width(160.0)
                .show_ui(ui, |ui| {
                    for h in [
                        HAlign::General,
                        HAlign::Left,
                        HAlign::Center,
                        HAlign::Right,
                        HAlign::Fill,
                        HAlign::Justify,
                        HAlign::CenterContinuous,
                        HAlign::Distributed,
                    ] {
                        ui.selectable_value(&mut look.alignment.horizontal, h, halign_name(h));
                    }
                });
            ui.end_row();

            ui.label("Vertical");
            egui::ComboBox::from_id_salt("calx-valign")
                .selected_text(valign_name(look.alignment.vertical))
                .width(160.0)
                .show_ui(ui, |ui| {
                    for v in [
                        VAlign::Top,
                        VAlign::Center,
                        VAlign::Bottom,
                        VAlign::Justify,
                        VAlign::Distributed,
                    ] {
                        ui.selectable_value(&mut look.alignment.vertical, v, valign_name(v));
                    }
                });
            ui.end_row();

            ui.label("Indent");
            ui.add(egui::DragValue::new(&mut look.alignment.indent).range(0..=250));
            ui.end_row();

            // Excel stores 0–90 for anticlockwise and 91–180 for the clockwise
            // mirror, so the number in the file is not the angle. The dialog
            // shows the angle and converts, because −45 is what anybody means.
            ui.label("Rotation");
            let mut degrees = rotation_degrees(look.alignment.rotation);
            let stacked = look.alignment.rotation == 255;
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!stacked, |ui| {
                    if ui
                        .add(
                            egui::DragValue::new(&mut degrees)
                                .range(-90..=90)
                                .suffix("°"),
                        )
                        .changed()
                    {
                        look.alignment.rotation = rotation_stored(degrees);
                    }
                });
                let mut on = stacked;
                if ui
                    .checkbox(&mut on, "Stacked")
                    .on_hover_text("One character above the next, which is rotation 255")
                    .changed()
                {
                    look.alignment.rotation = if on { 255 } else { 0 };
                }
            });
            ui.end_row();
        });
    ui.add_space(4.0);
    ui.checkbox(&mut look.alignment.wrap, "Wrap text");
    ui.checkbox(&mut look.alignment.shrink, "Shrink to fit");
}

pub(super) fn halign_name(h: HAlign) -> &'static str {
    match h {
        HAlign::General => "General",
        HAlign::Left => "Left",
        HAlign::Center => "Centre",
        HAlign::Right => "Right",
        HAlign::Fill => "Fill",
        HAlign::Justify => "Justify",
        HAlign::CenterContinuous => "Centre across selection",
        HAlign::Distributed => "Distributed",
    }
}

pub(super) fn valign_name(v: VAlign) -> &'static str {
    match v {
        VAlign::Top => "Top",
        VAlign::Center => "Centre",
        VAlign::Bottom => "Bottom",
        VAlign::Justify => "Justify",
        VAlign::Distributed => "Distributed",
    }
}

/// The angle a stored rotation means. 91–180 is Excel's spelling of −1 to −90.
pub(super) fn rotation_degrees(stored: u32) -> i32 {
    match stored {
        255 => 0,
        r if r > 90 => -((r as i32) - 90),
        r => r as i32,
    }
}

pub(super) fn rotation_stored(degrees: i32) -> u32 {
    if degrees >= 0 {
        degrees.min(90) as u32
    } else {
        (90 + degrees.abs().min(90)) as u32
    }
}

/// Format Cells ▸ Font.
pub(super) fn font_tab(
    ui: &mut egui::Ui,
    theme: &ss_model::color::Theme,
    look: &mut ss_model::Look,
) {
    egui::Grid::new("calx-font-tab")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("Font");
            egui::ComboBox::from_id_salt("calx-font-tab-name")
                .selected_text(look.font.name.clone())
                .width(190.0)
                .show_ui(ui, |ui| {
                    // The workbook's own face first when nobody offers it, so
                    // a document set in something exotic can still be got back
                    // to after the list has been opened.
                    let mut offered: Vec<String> =
                        FONT_NAMES.iter().map(|f| f.to_string()).collect();
                    if !offered.contains(&look.font.name) {
                        offered.insert(0, look.font.name.clone());
                    }
                    for choice in offered {
                        if ui
                            .selectable_label(choice == look.font.name, &choice)
                            .clicked()
                        {
                            look.font.name = choice;
                        }
                    }
                });
            ui.end_row();

            ui.label("Size");
            ui.add(
                egui::DragValue::new(&mut look.font.size)
                    .range(1.0..=409.0)
                    .speed(0.5),
            );
            ui.end_row();

            ui.label("Underline");
            egui::ComboBox::from_id_salt("calx-underline")
                .selected_text(underline_name(look.font.underline))
                .width(190.0)
                .show_ui(ui, |ui| {
                    for u in [
                        Underline::None,
                        Underline::Single,
                        Underline::Double,
                        Underline::SingleAccounting,
                        Underline::DoubleAccounting,
                    ] {
                        ui.selectable_value(&mut look.font.underline, u, underline_name(u));
                    }
                });
            ui.end_row();

            ui.label("Position");
            egui::ComboBox::from_id_salt("calx-vertalign")
                .selected_text(match look.font.vert_align {
                    None => "Normal",
                    Some(ss_model::style::VertAlign::Superscript) => "Superscript",
                    Some(ss_model::style::VertAlign::Subscript) => "Subscript",
                })
                .width(190.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut look.font.vert_align, None, "Normal");
                    ui.selectable_value(
                        &mut look.font.vert_align,
                        Some(ss_model::style::VertAlign::Superscript),
                        "Superscript",
                    );
                    ui.selectable_value(
                        &mut look.font.vert_align,
                        Some(ss_model::style::VertAlign::Subscript),
                        "Subscript",
                    );
                });
            ui.end_row();

            ui.label("Colour");
            color_row(ui, theme, &mut look.font.color);
            ui.end_row();
        });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.checkbox(&mut look.font.bold, "Bold");
        ui.checkbox(&mut look.font.italic, "Italic");
        ui.checkbox(&mut look.font.strike, "Strikethrough");
    });
}

pub(super) fn underline_name(u: Underline) -> &'static str {
    match u {
        Underline::None => "None",
        Underline::Single => "Single",
        Underline::Double => "Double",
        Underline::SingleAccounting => "Single, accounting",
        Underline::DoubleAccounting => "Double, accounting",
    }
}

/// A colour, with a way back to "automatic".
///
/// Automatic is not a colour and no picker can express it, which is why it
/// needs a checkbox of its own: without one, a border once given a colour
/// could never be handed back to the theme.
pub(super) fn color_row(ui: &mut egui::Ui, theme: &ss_model::color::Theme, color: &mut Color) {
    ui.horizontal(|ui| {
        let mut rgb = color.resolve(theme).unwrap_or([0, 0, 0]);
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            let [r, g, b] = rgb;
            *color = Color::rgb(r, g, b);
        }
        let mut automatic = *color == Color::Auto;
        if ui.checkbox(&mut automatic, "Automatic").changed() {
            *color = if automatic {
                Color::Auto
            } else {
                let [r, g, b] = rgb;
                Color::rgb(r, g, b)
            };
        }
    });
}

/// Format Cells ▸ Border, one edge at a time.
///
/// Per edge rather than by preset, because the presets on the toolbar cannot
/// say "a thick red line under this and a hairline down the side", and that is
/// the whole reason to open a dialog rather than press a button.
pub(super) fn border_tab(
    ui: &mut egui::Ui,
    theme: &ss_model::color::Theme,
    look: &mut ss_model::Look,
) {
    let presets = [
        ("All", BorderPreset::All),
        ("Outline", BorderPreset::Outline),
        ("None", BorderPreset::None),
    ];
    ui.horizontal(|ui| {
        ui.label("Quick");
        for (label, preset) in presets {
            if ui.button(label).clicked() {
                preset.apply(&mut look.border);
            }
        }
    });
    ui.add_space(6.0);
    egui::Grid::new("calx-border-edges")
        .num_columns(3)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            let edges: [BorderEdge; 5] = [
                ("Left", |b| &mut b.left),
                ("Right", |b| &mut b.right),
                ("Top", |b| &mut b.top),
                ("Bottom", |b| &mut b.bottom),
                ("Diagonal", |b| &mut b.diagonal),
            ];
            for (name, pick) in edges {
                let edge = pick(&mut look.border);
                ui.label(name);
                egui::ComboBox::from_id_salt(("calx-border", name))
                    .selected_text(border_style_name(edge.style))
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for style in BORDER_STYLES {
                            ui.selectable_value(&mut edge.style, *style, border_style_name(*style));
                        }
                    });
                color_row(ui, theme, &mut edge.color);
                ui.end_row();
            }
        });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.checkbox(&mut look.border.diagonal_up, "Diagonal up");
        ui.checkbox(&mut look.border.diagonal_down, "Diagonal down");
    });
}

pub(super) fn border_style_name(style: BorderStyle) -> &'static str {
    match style {
        BorderStyle::None => "None",
        BorderStyle::Hair => "Hair",
        BorderStyle::Thin => "Thin",
        BorderStyle::Medium => "Medium",
        BorderStyle::Thick => "Thick",
        BorderStyle::Double => "Double",
        BorderStyle::Dotted => "Dotted",
        BorderStyle::Dashed => "Dashed",
        BorderStyle::DashDot => "Dash-dot",
        BorderStyle::DashDotDot => "Dash-dot-dot",
        BorderStyle::MediumDashed => "Medium dashed",
        BorderStyle::MediumDashDot => "Medium dash-dot",
        BorderStyle::MediumDashDotDot => "Medium dash-dot-dot",
        BorderStyle::SlantDashDot => "Slant dash-dot",
    }
}

/// The Protect Sheet checkboxes, in the order Excel lists them.
///
/// Borrowed rather than copied so the dialog edits the model value directly:
/// fifteen `checkbox` calls with fifteen field names is fifteen chances to
/// wire one to the wrong flag.
pub(super) fn protection_fields(p: &mut ss_model::Protection) -> Vec<(&'static str, &mut bool)> {
    vec![
        ("Select locked cells", &mut p.select_locked),
        ("Select unlocked cells", &mut p.select_unlocked),
        ("Format cells", &mut p.format_cells),
        ("Format columns", &mut p.format_columns),
        ("Format rows", &mut p.format_rows),
        ("Insert columns", &mut p.insert_columns),
        ("Insert rows", &mut p.insert_rows),
        ("Insert hyperlinks", &mut p.insert_hyperlinks),
        ("Delete columns", &mut p.delete_columns),
        ("Delete rows", &mut p.delete_rows),
        ("Sort", &mut p.sort),
        ("Use AutoFilter", &mut p.filter),
        ("Use PivotTable reports", &mut p.pivot_tables),
        ("Edit objects", &mut p.objects),
        ("Edit scenarios", &mut p.scenarios),
    ]
}

/// Format Cells ▸ Protection: the tab that does nothing until the sheet is
/// protected, which is the single most confusing thing about it in Excel too.
pub(super) fn protection_tab(ui: &mut egui::Ui, look: &mut ss_model::Look) {
    ui.checkbox(&mut look.locked, "Locked");
    ui.add_space(6.0);
    // A line per widget rather than one label with newlines in it: a modal
    // centres a multi-line galley, and a centred paragraph is a monument.
    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
        for line in [
            "Locking a cell has no effect until the sheet is protected.",
            "Every cell starts locked, so protecting a sheet with nothing",
            "unlocked locks all of it — unlock the cells people are meant",
            "to type in first.",
        ] {
            ui.small(line);
        }
    });
}

pub(super) fn fill_tab(
    ui: &mut egui::Ui,
    theme: &ss_model::color::Theme,
    look: &mut ss_model::Look,
) {
    // The hatches are kept by name and drawn as a blend, so the list here is
    // the handful anybody picks; a file's own `lightTrellis` survives being
    // opened and shown among them because the model never dropped it.
    let offered: Vec<Pattern> = [
        Pattern::None,
        Pattern::Solid,
        Pattern::Named("gray125".into()),
        Pattern::Named("gray0625".into()),
        Pattern::Named("lightGray".into()),
        Pattern::Named("mediumGray".into()),
        Pattern::Named("darkGray".into()),
    ]
    .into_iter()
    .chain(
        (!matches!(look.fill.pattern, Pattern::None | Pattern::Solid))
            .then(|| look.fill.pattern.clone()),
    )
    .collect();

    egui::Grid::new("calx-fill-tab")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("Pattern");
            egui::ComboBox::from_id_salt("calx-pattern")
                .selected_text(pattern_name(&look.fill.pattern))
                .width(190.0)
                .show_ui(ui, |ui| {
                    for pattern in &offered {
                        if ui
                            .selectable_label(look.fill.pattern == *pattern, pattern_name(pattern))
                            .clicked()
                        {
                            look.fill.pattern = pattern.clone();
                        }
                    }
                });
            ui.end_row();
            ui.label("Colour");
            color_row(ui, theme, &mut look.fill.fg);
            ui.end_row();
            ui.label("Pattern colour");
            color_row(ui, theme, &mut look.fill.bg);
            ui.end_row();
        });
    ui.label(
        egui::RichText::new("A solid fill uses the first colour; a hatch uses both")
            .weak()
            .small(),
    );
}

pub(super) fn pattern_name(p: &Pattern) -> String {
    match p {
        Pattern::None => "None".to_string(),
        Pattern::Solid => "Solid".to_string(),
        Pattern::Named(name) => match name.as_str() {
            "gray125" => "12.5% grey".to_string(),
            "gray0625" => "6.25% grey".to_string(),
            "lightGray" => "25% grey".to_string(),
            "mediumGray" => "50% grey".to_string(),
            "darkGray" => "75% grey".to_string(),
            other => other.to_string(),
        },
    }
}
