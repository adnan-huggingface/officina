//! Editing a table: Tab from cell to cell, the edit every table command goes
//! through, cell margins, column widths, and Insert Table itself.

use super::*;

/// What the strip and the Table menu say about the table the caret is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TableAt {
    pub rows: usize,
    pub columns: usize,
    /// The caret cell's own fill, when it states one in RGB.
    pub shading: Option<[u8; 3]>,
}

impl Scriva {
    /// Runs one edit against the table the caret is in, as one undo step, or
    /// says why nothing happened.
    /// Tab in a table: to the next cell, or with Shift to the one before,
    /// selecting what is in it so that typing replaces it — Word's way of
    /// filling a table in from the keyboard. Tab in the last cell adds a row
    /// and goes to its first cell. Says whether the caret was in a table at
    /// all; the first cell's Shift+Tab goes nowhere, and is still answered.
    ///
    /// A cell that only continues a vertical merge is not a place to stop:
    /// its text belongs to the cell above.
    pub(super) fn tab_to_cell(&mut self, back: bool) -> bool {
        let Some((block, row, cell)) =
            edit::table_cell_at(&self.document, self.scope, self.caret())
        else {
            return false;
        };
        let Some(Block::Table(table)) = self.document.blocks(self.scope).get(block) else {
            return false;
        };
        let stops: Vec<(usize, usize)> = table
            .rows
            .iter()
            .enumerate()
            .flat_map(|(r, row)| {
                row.cells
                    .iter()
                    .enumerate()
                    .filter(|(_, cell)| {
                        cell.props.v_merge != Some(wp_model::table::VMerge::Continue)
                    })
                    .map(move |(c, _)| (r, c))
            })
            .collect();
        let here = stops.iter().position(|&stop| stop == (row, cell));
        let target = match (here, back) {
            (Some(0), true) | (None, true) => return true,
            (Some(at), true) => stops[at - 1],
            (Some(at), false) if at + 1 < stops.len() => stops[at + 1],
            _ => {
                let rows = table.rows.len();
                edit::append_row(&mut self.document, self.scope, &mut self.history, block);
                self.changed();
                (rows, 0)
            }
        };
        let Some(range) =
            edit::cell_paragraphs(&self.document, self.scope, block, target.0, target.1)
        else {
            return true;
        };
        let last = range.end.saturating_sub(1).max(range.start);
        let end = self
            .document
            .paragraphs_in(self.scope)
            .get(last)
            .map(|paragraph| text::len(paragraph))
            .unwrap_or(0);
        self.selection = Selection {
            anchor: Caret {
                paragraph: range.start,
                offset: 0,
            },
            head: Caret {
                paragraph: last,
                offset: end,
            },
        };
        self.reveal = Some(self.caret());
        true
    }

    /// The shape of the table the caret is in — rows, columns and the
    /// caret cell's fill — for the strip and the menu, or nothing.
    pub(crate) fn table_at_caret(&self) -> Option<TableAt> {
        let (index, row, cell) = edit::table_cell_at(&self.document, self.scope, self.caret())?;
        let Block::Table(table) = self.document.blocks(self.scope).get(index)? else {
            return None;
        };
        let shading = table
            .rows
            .get(row)
            .and_then(|r| r.cells.get(cell))
            .and_then(|c| c.props.shading.as_ref())
            .and_then(|shading| match shading.fill {
                Some(wp_model::Color::Rgb(rgb)) => Some(rgb),
                _ => None,
            });
        Some(TableAt {
            rows: table.rows.len(),
            columns: table.columns() as usize,
            shading,
        })
    }

    /// Table ▸ Insert ▸ Row Above / Row Below. The caret stays in its cell:
    /// the new row is empty and beside it, and Tab reaches it.
    pub(super) fn insert_row(&mut self, below: bool) {
        let Some((block, row, _)) = self.table_cell_or_say() else {
            return;
        };
        let at = match below {
            true => row + 1,
            false => row,
        };
        if edit::table_change(
            &mut self.document,
            self.scope,
            &mut self.history,
            block,
            |table| table.insert_row(at, row),
        ) {
            self.changed();
        }
    }

    /// Table ▸ Insert ▸ Column Left / Column Right of the caret's cell. A
    /// cell spanning several columns gets the new one at its outer edge.
    pub(super) fn insert_column(&mut self, after: bool) {
        let Some((block, row, cell)) = self.table_cell_or_say() else {
            return;
        };
        let Some(Block::Table(table)) = self.document.blocks(self.scope).get(block) else {
            return;
        };
        let column = starting_column(table, row, cell);
        let span = table.rows[row].cells[cell].props.span() as usize;
        let at = match after {
            true => column + span,
            false => column,
        };
        if edit::table_change(
            &mut self.document,
            self.scope,
            &mut self.history,
            block,
            |table| table.insert_column(at),
        ) {
            self.changed();
        }
    }

    /// Table ▸ Delete ▸ Row: the caret's. The caret lands in the row that
    /// takes its place, or the one above when it was the last; deleting the
    /// only row deletes the table, as Word's does.
    pub(super) fn delete_row(&mut self) {
        let Some((block, row, cell)) = self.table_cell_or_say() else {
            return;
        };
        let rows = match self.document.blocks(self.scope).get(block) {
            Some(Block::Table(table)) => table.rows.len(),
            _ => return,
        };
        if rows <= 1 {
            self.delete_table();
            return;
        }
        if edit::table_change(
            &mut self.document,
            self.scope,
            &mut self.history,
            block,
            |table| table.delete_row(row),
        ) {
            self.land_in_cell(block, row.min(rows - 2), cell);
            self.changed();
        }
    }

    /// Table ▸ Delete ▸ Column: the caret's, spans and all. The caret lands
    /// in the cell that takes its place; deleting the only column deletes
    /// the table.
    pub(super) fn delete_column(&mut self) {
        let Some((block, row, cell)) = self.table_cell_or_say() else {
            return;
        };
        let Some(Block::Table(table)) = self.document.blocks(self.scope).get(block) else {
            return;
        };
        if table.grid.len() <= 1 {
            self.delete_table();
            return;
        }
        let column = starting_column(table, row, cell);
        let span = table.rows[row].cells[cell].props.span() as usize;
        let changed = edit::table_change(
            &mut self.document,
            self.scope,
            &mut self.history,
            block,
            |table| (0..span).all(|_| table.delete_column(column)),
        );
        if changed {
            self.land_in_cell(block, row, cell);
            self.changed();
        }
    }

    /// Table ▸ Delete ▸ Table: the caret's, leaving an empty paragraph where
    /// it stood, with the caret on it.
    pub(super) fn delete_table(&mut self) {
        let Some((block, _, _)) = self.table_cell_or_say() else {
            return;
        };
        if let Some(caret) =
            edit::delete_table(&mut self.document, self.scope, &mut self.history, block)
        {
            self.selection = Selection::at(clamp(&self.document, self.scope, caret));
            self.reveal = Some(self.caret());
            self.changed();
        }
    }

    /// The caret to the start of a cell, or the nearest cell the row still
    /// has, or the table's first paragraph when the row is gone.
    fn land_in_cell(&mut self, block: usize, row: usize, cell: usize) {
        let Some(Block::Table(table)) = self.document.blocks(self.scope).get(block) else {
            return;
        };
        let row = row.min(table.rows.len().saturating_sub(1));
        let cell = cell.min(
            table
                .rows
                .get(row)
                .map_or(0, |r| r.cells.len().saturating_sub(1)),
        );
        let paragraph = edit::cell_paragraphs(&self.document, self.scope, block, row, cell)
            .map(|range| range.start)
            .unwrap_or(0);
        self.selection = Selection::at(clamp(
            &self.document,
            self.scope,
            Caret {
                paragraph,
                offset: 0,
            },
        ));
        self.reveal = Some(self.caret());
    }

    /// The table cell the caret is in, or the message every table command
    /// gives when it is not in one.
    fn table_cell_or_say(&mut self) -> Option<(usize, usize, usize)> {
        let found = edit::table_cell_at(&self.document, self.scope, self.caret());
        if found.is_none() {
            self.message = Some((
                "Not in a table".to_owned(),
                "Put the caret in a table cell first, then try again.".to_owned(),
            ));
        }
        found
    }

    pub(super) fn edit_table(
        &mut self,
        change: impl FnOnce(&mut wp_model::table::Table, usize, usize),
    ) {
        let caret = self.caret();
        let Some((index, row, cell)) = edit::table_cell_at(&self.document, self.scope, caret)
        else {
            self.message = Some((
                "Not in a table".to_owned(),
                "Put the caret in a table cell first, then try again.".to_owned(),
            ));
            return;
        };
        self.history.push(
            self.scope,
            edit::Change::Blocks {
                index,
                before: vec![self.document.body[index].clone()],
                now: 1,
            },
        );
        if let Block::Table(table) = &mut self.document.body[index] {
            change(table, row, cell);
        }
        self.changed();
    }

    /// Opens the cell-padding box on the caret's table, prefilled with what
    /// the table states itself — blank where it says nothing and takes the
    /// padding from its style, which is where Word keeps its own 0.08in.
    pub(super) fn open_cell_margin_dialog(&mut self) {
        let caret = self.caret();
        let Some((index, _, _)) = edit::table_cell_at(&self.document, self.scope, caret) else {
            self.message = Some((
                "Not in a table".to_owned(),
                "Put the caret in a table cell first, then try again.".to_owned(),
            ));
            return;
        };
        let Block::Table(table) = &self.document.body[index] else {
            return;
        };
        let points = |w: Option<wp_model::table::Width>| match w {
            Some(wp_model::table::Width::Fixed(t)) => trim_number(t.0 as f64 / 20.0),
            _ => String::new(),
        };
        let margins = table.props.cell_margins;
        self.cell_margin_draft = Some([
            points(margins.top),
            points(margins.start),
            points(margins.bottom),
            points(margins.end),
        ]);
    }

    pub(super) fn cell_margin_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.cell_margin_draft.clone() else {
            return;
        };
        let mut done: Option<bool> = None;
        egui::Modal::new(egui::Id::new("scriva-cell-margins"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(280.0);
                    ui.label(egui::RichText::new("Cell Margins").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    for (index, (label, field)) in ["Top:", "Left:", "Bottom:", "Right:"]
                        .into_iter()
                        .zip(draft.iter_mut())
                        .enumerate()
                    {
                        ui.horizontal(|ui| {
                            ui.add_sized([64.0, 20.0], egui::Label::new(label));
                            match index == 0 {
                                true => {
                                    dialog::first_field(ui, "scriva-cell-margins", field, 64.0);
                                }
                                false => {
                                    dialog::field(ui, field, 64.0);
                                }
                            }
                            ui.label("pt");
                        });
                    }
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Blank leaves it to the table's style.")
                            .small()
                            .weak(),
                    );
                    ui.add_space(12.0);
                    if let Some(answer) = dialog::submit(ui, "Apply") {
                        done = Some(answer);
                    }
                });
            });
        self.cell_margin_draft = Some(draft.clone());
        match done {
            Some(true) => {
                self.cell_margin_draft = None;
                self.apply_cell_margins(&draft);
            }
            Some(false) => self.cell_margin_draft = None,
            None => {}
        }
    }

    /// States the caret's table's cell padding. A blank field states nothing,
    /// which is not the same as nothing at all: it puts the side back in the
    /// hands of the table's style.
    pub(super) fn apply_cell_margins(&mut self, draft: &[String; 4]) {
        let side = |text: &str| {
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            text.parse::<f64>()
                .ok()
                .filter(|v| (0.0..=100.0).contains(v))
                .map(|v| wp_model::table::Width::Fixed(Twips((v * 20.0).round() as i32)))
        };
        let margins = wp_model::table::CellMargins {
            top: side(&draft[0]),
            start: side(&draft[1]),
            bottom: side(&draft[2]),
            end: side(&draft[3]),
        };
        self.edit_table(move |table, _, _| table.props.cell_margins = margins);
    }

    /// Opens the width box on the caret's column, prefilled in inches.
    pub(super) fn open_column_dialog(&mut self) {
        let caret = self.caret();
        let Some((index, row, cell)) = edit::table_cell_at(&self.document, self.scope, caret)
        else {
            self.message = Some((
                "Not in a table".to_owned(),
                "Put the caret in a table cell first, then try again.".to_owned(),
            ));
            return;
        };
        let Block::Table(table) = &self.document.body[index] else {
            return;
        };
        let column = starting_column(table, row, cell);
        let width = table.grid.get(column).copied().unwrap_or(Twips(0));
        self.column_draft = Some(format!("{:.2}", width.0 as f64 / 1440.0));
    }

    /// Sets the caret's column to `width`, and restates the width of every
    /// cell the column passes through — the grid and the cells disagreeing is
    /// what makes Word redraw a table differently than it was saved.
    pub(super) fn apply_column_width(&mut self, width: Twips) {
        self.edit_table(move |table, at_row, at_cell| {
            let column = starting_column(table, at_row, at_cell);
            let Some(entry) = table.grid.get_mut(column) else {
                return;
            };
            *entry = width;
            for row in &mut table.rows {
                let mut at = row.props.grid_before as usize;
                for cell in &mut row.cells {
                    let span = cell.props.grid_span.max(1) as usize;
                    if (at..at + span).contains(&column) {
                        let total: i32 = table.grid[at..(at + span).min(table.grid.len())]
                            .iter()
                            .map(|t| t.0)
                            .sum();
                        cell.props.width = wp_model::table::Width::Fixed(Twips(total));
                    }
                    at += span;
                }
            }
        });
    }

    /// Builds an evenly divided, fully ruled table and puts it above the
    /// caret's paragraph.
    pub(crate) fn insert_table(&mut self, rows: usize, columns: usize) {
        use wp_model::table::{Cell, Row, Table, TableBorders, TableProps};
        let margins = &self.document.section.margins;
        let text_width = self.document.section.page.width.0 - margins.start.0 - margins.end.0;
        let each = Twips((text_width / columns as i32).max(144));
        // Word's default grid: half-point single lines on every edge.
        let edge = ruled_edge();
        let table = Table {
            props: TableProps {
                borders: TableBorders {
                    top: Some(edge),
                    start: Some(edge),
                    bottom: Some(edge),
                    end: Some(edge),
                    inside_h: Some(edge),
                    inside_v: Some(edge),
                },
                ..TableProps::default()
            },
            grid: vec![each; columns],
            rows: (0..rows)
                .map(|_| Row {
                    props: Default::default(),
                    // Each cell states its width as well as the grid, as
                    // Word's own new table does. Measured: a cell that
                    // states none is laid to its *content* by Word, whatever
                    // the grid says — a 468pt table came back 28pt wide,
                    // its second column as wide as "B1", and an empty
                    // column under a point.
                    cells: (0..columns)
                        .map(|_| {
                            let mut cell = Cell::new();
                            cell.props.width = wp_model::table::Width::Fixed(each);
                            cell
                        })
                        .collect(),
                })
                .collect(),
        };
        let caret = edit::insert_block(
            &mut self.document,
            self.scope,
            &mut self.history,
            self.selection,
            Block::Table(table),
        );
        self.selection = Selection::at(clamp(&self.document, self.scope, caret));
        self.changed();
        self.reveal = Some(self.caret());
    }
}
