# The page is the product, and the chrome recedes

The redesign of Scriva's user interface. **This file is immutable while the
work runs.** Nothing that does the work may edit it — not to reword an item,
not to remove one, and above all not to mark one done. A definition of done
that the worker can edit is not a definition of done.

**Nothing here is ticked, because nothing here is ticked by hand.** Each item
carries a `verify:` command, and the item is finished exactly when that command
exits zero. `python .claude/hooks/gate.py` runs them all and reports the ones
that do not. A `cargo test` filter that matches no test exits zero, so every
test item goes through `.claude/hooks/proved.py`, which insists a test ran.

`PROGRESS.md` is where the work is narrated, and `LEARNINGS.md` is where what
egui or a format taught goes. Neither is read by the gate. They are for people.

## Why

An afternoon of driving Scriva by keystroke and reading the screenshots back
found twelve things, worst first: a tracked change is invisible on the page —
the insertion and the deletion run together as one black sentence; a comment
leaves no mark; the caret's style, face and size are nowhere on the screen;
the status bar is Calx's two rows with one row in it; a white blur is painted
along the bottom of every frame; there are two menu systems; a comment with
nothing selected is refused in a box where Word comments the word at the
caret. The design that answers it is written in full in the story's handoff,
and the invariant stands throughout: **`cargo xtask fidelity` stays at zero
failures; none of this changes what is written.** UI never touches a format:
where the window needs the model to do something new, the model gains a
function with its own test and the window calls it.

Ten phases, in the order a user meets them. Each item below names the test
that proves it.

---

## Phase 1 — theme, shell, desk

### T1 — one theme, and its text reads against every fill

`ui_kit::theme` holds every colour and metric the chrome draws with, and the
contrast of the chrome inks against the chrome, field and tinted fills is at
least 4.5 to 1, computed rather than eyeballed.

    verify: python .claude/hooks/proved.py -p ui-kit chrome_text_reads_against_every_fill_it_is_set_on

### T2 — the status panel is as tall as the application says

The shell stops hard-coding Calx's two rows for both applications: a frame at
1600 × 1000 gives a 26-point status panel to an application that asks for one
row and 56 to one that asks for two.

    verify: python .claude/hooks/proved.py -p ui-kit the_status_panel_is_as_tall_as_the_application_says

### T3 — a page casts a shadow on a light desk, and the fade is gone

The desk is the theme's, a page's shapes include a shadow before the paper,
and nothing paints a gradient over the bottom of the desk — the blur was
egui's scroll-area fade, and the theme turns it off.

    verify: python .claude/hooks/proved.py -p scriva a_page_sits_on_the_light_desk_with_a_shadow_and_no_fade

### T4 — the caret blinks, and stands solid after a key

Drawn in the frame after a keystroke, absent a blink's length later, and drawn
again after a full period; never blinking while a selection shows.

    verify: python .claude/hooks/proved.py -p scriva the_caret_blinks_and_stands_solid_after_a_key

## Phase 2 — the toolbar

### B1 — the caret's style, face and size are read from the document

    verify: python .claude/hooks/proved.py -p scriva the_carets_style_face_and_size_are_read_from_the_document

### B2 — every command on the toolbar is reachable at 800 wide

The row folds what does not fit into an overflow menu, and a walk of the row
at 800 points finds every command either on the row or in the overflow.

    verify: python .claude/hooks/proved.py -p scriva every_toolbar_command_is_reachable_at_eight_hundred_wide

### B3 — every tooltip ends with the key the command table gives

    verify: python .claude/hooks/proved.py -p scriva every_toolbar_tooltip_ends_with_the_key_the_table_gives

### B4 — a click on B toggles bold through the one command

    verify: python .claude/hooks/proved.py -p scriva a_click_on_bold_toggles_bold_through_the_command

## Phase 3 — changes and comments on the page

### M1 — a deleted fragment carries its marking and its author

    verify: python .claude/hooks/proved.py -p wp-layout a_deleted_fragment_carries_its_marking_and_author

### M2 — a deletion is struck and an insertion underlined on the page

The painted shapes include a strike across the deleted word and an underline
under the inserted one, and neither under plain text.

    verify: python .claude/hooks/proved.py -p scriva a_deletion_is_struck_and_an_insertion_underlined_on_the_page

### M3 — a comment washes its range and marks the margin

    verify: python .claude/hooks/proved.py -p scriva a_comment_washes_its_range_and_marks_the_margin

## Phase 4 — the Review pane and comments

### V1 — a comment with no selection takes the word at the caret

    verify: python .claude/hooks/proved.py -p scriva a_comment_with_no_selection_takes_the_word_at_the_caret

### V2 — a draft comment is posted with Ctrl+Enter and discarded with Escape

Drafted in the pane, not in a box: Ctrl+Alt+M opens the pane, the typed
text is posted by Ctrl+Enter, and Escape leaves no comment behind.

    verify: python .claude/hooks/proved.py -p scriva a_draft_comment_is_posted_with_ctrl_enter_and_discarded_with_escape

### V3 — one change is settled from its card without touching the others

    verify: python .claude/hooks/proved.py -p scriva one_change_is_settled_from_its_card_without_touching_the_others

### V4 — the card at the caret is the outlined one

    verify: python .claude/hooks/proved.py -p scriva the_card_at_the_caret_is_the_outlined_one

## Phase 5 — the Navigate pane and F6

### N1 — F6 lands in the Navigate pane when it is open and skips it when closed

    verify: python .claude/hooks/proved.py -p scriva f6_lands_in_the_navigate_pane_when_open_and_skips_it_when_closed

### N2 — Enter on a heading moves the caret and returns the keyboard

    verify: python .claude/hooks/proved.py -p scriva enter_on_a_heading_moves_the_caret_and_returns_the_keyboard_to_the_document

### N3 — Escape closes one thing per press

A band first, the find bar second.

    verify: python .claude/hooks/proved.py -p scriva escape_closes_the_band_first_and_the_find_bar_second

### N4 — the heading containing the caret is the lit row

    verify: python .claude/hooks/proved.py -p scriva the_heading_containing_the_caret_is_the_lit_row

## Phase 6 — table and picture strips, table commands

### A1 — a row inserted below by menu takes Tab into its first cell

    verify: python .claude/hooks/proved.py -p scriva a_row_inserted_below_by_menu_takes_tab_into_its_first_cell

### A2 — deleting the caret's column narrows the grid and nothing else

    verify: python .claude/hooks/proved.py -p scriva deleting_the_caret_column_narrows_the_grid_and_nothing_else

### A3 — deleting a table leaves an empty paragraph, and undo brings it back

    verify: python .claude/hooks/proved.py -p scriva deleting_a_table_leaves_an_empty_paragraph_and_undo_brings_it_back

### A4 — the fidelity harness's edit round-trip inserts a row

Weak check: the harness names the row edit; that it stays at zero is the
fidelity gate's own business.

    verify: python -c "import sys,pathlib; sys.exit(0 if 'insert_row' in pathlib.Path('xtask/src/fidelity.rs').read_text(encoding='utf-8') else 1)"

## Phase 7 — context menus

### C1 — Shift+F10 opens the context menu and Escape closes it without moving the caret

    verify: python .claude/hooks/proved.py -p scriva shift_f10_opens_the_context_menu_and_escape_closes_it_without_moving_the_caret

### C2 — no two rows of the context menu share a letter

    verify: python .claude/hooks/proved.py -p scriva no_two_rows_of_the_context_menu_share_a_letter

## Phase 8 — dialogs

### D1 — a measure is read in inches, centimetres and points

`"3 cm"` is 1.18 in, `"36 pt"` is 0.5 in, `"1.25"` is 1.25 in, and junk is
nothing.

    verify: python .claude/hooks/proved.py -p ui-kit a_measure_is_read_in_inches_centimetres_and_points

### D2 — Page Setup with A4 landscape is one undo step

    verify: python .claude/hooks/proved.py -p scriva page_setup_a4_landscape_is_one_undo_step

### D3 — Ctrl+G, 5, Enter puts the caret on page five

    verify: python .claude/hooks/proved.py -p scriva ctrl_g_five_enter_puts_the_caret_on_page_five

### D4 — every box opened by its key takes the keyboard in its first field

The existing coverage, extended to the new boxes.

    verify: python .claude/hooks/proved.py -p scriva a_box_that_opens_puts_the_keyboard_in_its_first_field

## Phase 9 — find bar and menus

### F1 — match case finds the capital and not the lower

    verify: python .claude/hooks/proved.py -p scriva match_case_finds_the_capital_and_not_the_lower

### F2 — whole word finds "the" and not "then"

    verify: python .claude/hooks/proved.py -p scriva whole_word_finds_the_and_not_then

### F3 — the guide's key tables are the command table's

    verify: python .claude/hooks/proved.py -p scriva the_guides_key_tables_are_the_command_tables

### F4 — the Help menu opens by its letter

    verify: python .claude/hooks/proved.py -p scriva the_help_menu_opens_by_its_letter_and_lists_the_shortcuts

## Phase 10 — notices, drop, badge, polish

### P1 — a save says so in the status for four seconds and not after

    verify: python .claude/hooks/proved.py -p scriva a_save_says_so_in_the_status_for_four_seconds_and_not_after

### P2 — a dropped document opens through the unsaved guard

    verify: python .claude/hooks/proved.py -p scriva a_dropped_document_opens_through_the_unsaved_guard

### P3 — a `.doc` opened shows the notice bar and no box

    verify: python .claude/hooks/proved.py -p scriva a_doc_opened_shows_the_notice_bar_and_no_box

## The end

### Z1 — the page did not move

Every corpus document lays as `LAYOUT.md` records; attribution and paint
moved no word.

    verify: cargo xtask compare --check

### Z2 — the redesign was driven, and what it found is written down

Weak check, for the same reason as the last plan's: the after-tour on the rig
and Calx's screenshot are a person's eyes, and the section is where they
report. The section is not written until every wall it hit is fixed or named.

    verify: python -c "import sys,pathlib; sys.exit(0 if 'What driving the redesign found' in pathlib.Path('PROGRESS.md').read_text(encoding='utf-8') else 1)"
