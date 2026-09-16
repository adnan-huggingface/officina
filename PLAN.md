# The harness sees what the user sees

Better tools for the tests that drive the applications. **This file is
immutable while the work runs.** Nothing that does the work may edit it: not to
reword an item, not to remove one, and above all not to mark one done. The
redesign's plan that stood here before is in the history, and every item of it
proved itself before it was replaced.

Each item carries a `verify:` command and is finished exactly when that command
exits zero. `python .claude/hooks/gate.py` runs them all. A `cargo test` filter
that matches no test exits zero, so every test item goes through
`.claude/hooks/proved.py`, which insists a test ran.

## Why

One session fixed fifteen bugs, and each fix paid again for the same tools:

- Eight tests walked egui's shapes with helpers of their own, and none could
  read the colour text was painted in.
- Three built screen points by hand from the first page's corner.
- Four spelled out every drag as button events.
- None could see the window grow after it opened, which is where the
  whole-page fit went wrong.
- Frames moved time only when a test invented a clock value.
- The authored corpus document was held to its bytes and never to its intent,
  so a bold space and a plain "bold" passed for months.
- All of it sat in one test file of seven thousand lines.

Real fonts were asked for as well, and cannot be had here: no Office face may
be committed, and a test that uses a machine's copy when it happens to be there
is a test that passes where it looked at nothing. What can be had is a second
face whose metrics differ from egui's own, Hack, which every egui build already
carries under a licence that allows it. Code that quietly assumed egui's
default face fails with it.

None of this changes what is written: `cargo xtask fidelity` stays at zero
failures, and nothing here touches a reader or a writer.

---

### H1 — one view over a painted frame

`ui_kit::drive::Painted` answers for the rectangles, the texts, the colour each
letter was painted in, and the horizontal rules. In Scriva, the colour the
caret types in is the colour the page shows.

    verify: python .claude/hooks/proved.py -p ui-kit painted_reads_rects_texts_and_the_colour_each_letter_is_painted_in
    verify: python .claude/hooks/proved.py -p scriva the_colour_chosen_for_typing_is_the_colour_painted

### H2 — gestures on the driver, and a caret's place on the screen

The driver presses, moves, drags and holds. Scriva says where a caret is in the
window on any page, however far the desk has scrolled.

    verify: python .claude/hooks/proved.py -p ui-kit a_drag_is_a_press_moves_and_a_release_and_a_hold_keeps_the_button_down
    verify: python .claude/hooks/proved.py -p scriva a_caret_is_found_on_screen_on_any_page_however_far_the_desk_has_scrolled

### H3 — a window that opens the way the real one does

The driver can open at the shell's first size and grow to the full window a few
frames in. Scriva shows the whole page at the size the window ends at.

    verify: python .claude/hooks/proved.py -p ui-kit an_opening_window_grows_to_its_full_size_a_few_frames_in
    verify: python .claude/hooks/proved.py -p scriva a_window_that_opens_small_and_grows_shows_the_whole_page_at_its_full_size

### H4 — a clock that advances

    verify: python .claude/hooks/proved.py -p ui-kit waiting_runs_frames_until_an_animation_has_finished

### H5 — the authored document carries what its script meant

The script says which words are bold, italic, underlined or plain. The
committed document, read back from disk, is held to it, and a document that
breaks an expectation is reported.

    verify: python .claude/hooks/proved.py -p scriva the_authored_document_carries_what_its_script_meant
    verify: python .claude/hooks/proved.py -p scriva an_expectation_the_document_breaks_is_reported

### H6 — a second face, and no machine's fonts in a test

A generic face can be given from memory. Scriva's caret and clicks follow Hack
as they follow egui's own face. A headless process sees no installed font, so a
test opening a document lays it the same on every machine.

    verify: python .claude/hooks/proved.py -p ui-kit a_generic_face_given_in_memory_is_the_one_text_is_set_in
    verify: python .claude/hooks/proved.py -p ui-kit a_headless_process_sees_no_installed_fonts
    verify: python .claude/hooks/proved.py -p scriva in_a_face_other_than_eguis_own_the_caret_and_the_click_follow_the_face

### H7 — Scriva's window tests are filed by area

`app/tests.rs` is a directory of files, none past two thousand lines, and at
least as many tests are in it as there were (186).

    verify: python -c "import pathlib,subprocess,sys; d=pathlib.Path('crates/app-scriva/src/app'); out=subprocess.run(['cargo','test','-p','scriva','--lib','--','--list'],capture_output=True,text=True).stdout; n=sum(1 for l in out.splitlines() if l.startswith('app::tests::') and l.endswith(': test')); big=[f.name for f in (d/'tests').glob('*.rs') if len(f.read_text(encoding='utf-8').splitlines())>2000]; sys.exit(0 if not (d/'tests.rs').exists() and (d/'tests').is_dir() and not big and n>=186 else 1)"

## The end

### Z1 — the page did not move

    verify: cargo xtask compare --check
