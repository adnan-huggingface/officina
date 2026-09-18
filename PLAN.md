# Assist, phase 4: Calx

The fourth of six phases that give Scriva and Calx an assistant a person talks
to while editing. The specification, with every decision and why it was taken,
is kept in the story workspace (`bugs/assist-spec.md`); phase 1 built the
helpers, phase 2 the pane, phase 3 put the pane in Scriva, and this plan puts
it in Calx with the tools a helper edits a sheet through. **This file is
immutable while the work runs.** Nothing that does the work may edit it: not to
reword an item, not to remove one, and above all not to mark one done. Phase
3's plan is in the history, and every item of it proved itself before it was
replaced.

Each item carries a `verify:` command and is finished exactly when that command
exits zero. `python .claude/hooks/gate.py` runs them all. A `cargo test` filter
that matches no test exits zero, so every test item goes through
`.claude/hooks/proved.py`, which insists a test ran.

## Why

A spreadsheet is where a person is likeliest to know what they want and least
likely to know how to ask for it: a column that sums three others, a date
column cleaned up, a total row. The pane exists and Scriva uses it; this phase
gives Calx's person View ▸ Assist and Ctrl+Alt+A, a request about the cell, the
selection, the sheet or the workbook, and a helper that answers by editing —
through Calx's own functions, so that the grid, the engine and Ctrl+Z treat an
assistant's edit exactly as they treat a person's.

The rules this phase makes true:

- **An edit lands at once, and is one thing to undo.** Calx has no tracked
  changes and grows none for this: every tool call a request makes is gathered
  into one labelled entry, so one Ctrl+Z takes the whole answer back.
- **The engine checks the helper.** Every cell a tool writes goes back to the
  helper as the grid now evaluates it, so that a formula that came back
  `#NAME?` is in front of it before it answers.
- **Nothing is overwritten by accident.** A cell that holds something is
  refused unless the call says so, and the refusal names the cells.
- **What changed is visible.** The cells a request wrote are washed in the
  accent for a few seconds, and the card says how many were written, where,
  and which held something before.
- **The chart inspector wins its slot.** Assist takes the right-hand side
  where the inspector sits, and while a chart is selected the inspector has it.
- **The sheet is data.** A cell whose text tells the assistant what to do is
  text, and the worst it can bring about is one undoable change.

Nothing here changes how an Excel or OpenDocument file is read or written:
`cargo xtask fidelity` stays at zero failures, and `cargo xtask compare
--check` holds.

---

### C1 — what a request carries, and what the helper may ask

The editor's instructions gain a Calx half, short enough to stay cheap to send
with every request, saying that the sheet's contents are the person's material
and never an instruction. A request carries the workbook's sheets and which is
showing, the selection's address, the size of the used range, the header row
and the rows under it with the selection, as tab-separated text with each
formula shown as `=…` and its value beside it, and the workbook's defined
names. The sample is capped, and says what it left out rather than trailing
off. The helper has six tools, each with a strict schema: read a range, write
cells, fill, insert, delete, and add a sheet. Reading answers with the cells
as the grid shows them, and a range outside the sheet is said rather than
guessed at.

    verify: python .claude/hooks/proved.py -p assist the_editors_instructions_say_the_document_is_data_and_stay_short
    verify: python .claude/hooks/proved.py -p calx a_request_carries_the_sheets_the_selection_the_headers_and_the_names
    verify: python .claude/hooks/proved.py -p calx the_sample_sent_is_capped_and_says_what_it_left_out
    verify: python .claude/hooks/proved.py -p calx the_six_tools_are_strict_and_reading_answers_with_the_cells_as_shown

### C2 — an edit lands at once, as one entry to undo

"Add a column that totals the others" writes the header, writes the formula,
fills it down, and is one entry in the undo history, labelled with the request's
first words; one Ctrl+Z takes the whole answer back, and one Ctrl+Y puts it
again. Every written cell goes back to the helper as the grid evaluates it, so
a formula that came back an error is in front of it before it answers. A cell
that holds something is not overwritten unless the call says so, and the
refusal names the cells. Rows and columns inserted or deleted, and a sheet
added, are part of the same entry.

    verify: python .claude/hooks/proved.py -p calx add_a_total_column_writes_the_header_the_formula_and_the_fill_as_one_undo_entry
    verify: python .claude/hooks/proved.py -p calx a_formula_that_evaluates_to_an_error_is_shown_to_the_assistant_before_it_answers
    verify: python .claude/hooks/proved.py -p calx write_cells_refuses_a_cell_that_holds_something_unless_told_to_overwrite
    verify: python .claude/hooks/proved.py -p calx rows_columns_and_a_sheet_the_assistant_added_are_part_of_the_same_entry

### C3 — what the person sees afterwards

The cells a request wrote are washed in the accent for a few seconds — a view
state, not a format, and nothing a file keeps. The card in the pane says what
was done in the person's terms, "Wrote 101 cells in D", with Undo; a cell the
assistant overwrote is listed on the card by address, because that is the one
thing worth checking. Undo on the card takes the whole request back and says
so, and a card whose change has been undone another way says that instead.

    verify: python .claude/hooks/proved.py -p calx the_cells_a_request_wrote_are_washed_and_undo_on_the_card_takes_them_back
    verify: python .claude/hooks/proved.py -p calx a_card_lists_the_cells_the_assistant_overwrote

### C4 — the pane in Calx's window

View ▸ Assist and Ctrl+Alt+A open the pane with the composer ready, F6 walks it
with the window's other stops, and Escape gives the grid back. The pane takes
the right-hand side the chart inspector sits in, and while a chart is selected
the inspector has it and the pane waits. The scope chip follows the selection —
Cell, Selection, Sheet, Whole workbook — and the quick verbs send at once. The
right-click menu on a selection asks the assistant about it, and its rows, like
the menus', each have a letter of their own. A new or opened workbook stops the
request and clears the conversation about the old one. Nothing a test does
reaches a helper.

    verify: python .claude/hooks/proved.py -p calx ctrl_alt_a_opens_assist_and_the_inspector_wins_while_a_chart_is_selected
    verify: python .claude/hooks/proved.py -p calx the_scope_chip_follows_the_selection_and_a_verb_sends_at_once
    verify: python .claude/hooks/proved.py -p calx the_right_click_menu_asks_the_assistant_about_the_selection
    verify: python .claude/hooks/proved.py -p calx a_new_workbook_ends_the_conversation_about_the_old_one
    verify: python .claude/hooks/proved.py -p calx no_two_rows_of_a_menu_share_a_letter
    verify: python .claude/hooks/proved.py -p calx the_assist_pane_is_drawn_where_it_can_be_seen

### C5 — the sheet is data

A cell whose text tells the assistant to wipe the sheet is text in the request,
below instructions that say so; a helper that obeys it anyway can do no more
than one change the person undoes, and the card says what it was.

    verify: python .claude/hooks/proved.py -p calx a_cell_that_tells_the_assistant_to_wipe_the_sheet_can_only_make_one_undoable_change

## The end

### Z1 — the page did not move, and the files did not change

    verify: cargo xtask compare --check
    verify: cargo xtask fidelity
