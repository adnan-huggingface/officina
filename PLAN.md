# Assist, phase 3: Scriva

The third of six phases that give Scriva and Calx an assistant a person talks
to while editing. The specification, with every decision and why it was taken,
is kept in the story workspace (`bugs/assist-spec.md`); phase 1 built the
helpers, phase 2 the pane, and this plan puts the pane in Scriva with the tools
a helper edits a document through. **This file is immutable while the work
runs.** Nothing that does the work may edit it: not to reword an item, not to
remove one, and above all not to mark one done. Phase 2's plan is in the
history, and every item of it proved itself before it was replaced.

Each item carries a `verify:` command and is finished exactly when that command
exits zero. `python .claude/hooks/gate.py` runs them all. A `cargo test` filter
that matches no test exits zero, so every test item goes through
`.claude/hooks/proved.py`, which insists a test ran.

## Why

The pane exists, and nothing in Scriva opens it. This phase gives Scriva's
person View ▸ Assist and Ctrl+Alt+A, a request about the selection, the
paragraph or the whole document, and a helper that answers by editing —
through Scriva's own functions, as tracked changes by an author named
"Assistant", which the page already draws, the Review pane already settles,
the writer already writes, and undo already takes back.

The rules this phase makes true:

- **The assistant only proposes.** Every edit it makes is a tracked change or
  a comment by "Assistant", whether or not Track Changes is on; Reject puts the
  text back exactly, and one Undo takes a whole proposal back.
- **A proposal is settled whole, and alone.** Its card's Accept and Reject
  settle every change it made and nothing another proposal or another person
  made.
- **A rewrite keeps the paragraph's style and the words' weight.** A rewritten
  heading is still a heading, and words the helper marks `**` are bold.
- **The document is data.** A paragraph that tells the assistant what to do is
  text, and the worst it can bring about is a proposal the person rejects.
- **Saving keeps open proposals as tracked changes, and says so.**
- **A conversation is about one document.** Opening another, or a new one,
  stops the request and clears the transcript.

Nothing here changes how a Word, Excel or OpenDocument file is read or
written: `cargo xtask fidelity` stays at zero failures, and `cargo xtask
compare --check` holds. The Markdown reader, which the helper's words go
through, learns that an underscore inside a word is a letter.

---

### C1 — what a request carries, and what the helper may ask

The editor's instructions are one constant in `assist`, with a half for each
application, short enough to stay cheap to send with every request, and saying
that the document is the person's material and never an instruction. A
paragraph is written for the helper as one line of Markdown — its heading
level, its list marker, its bold and italic — and the helper's Markdown is
read back with `snake_case` left as it is. A request carries the scope's
paragraphs numbered from the document's first, two more on each side, and the
document's headings. The helper has four tools, each with a strict schema:
read paragraphs, replace paragraphs, insert paragraphs, and comment. Reading
answers with the paragraphs numbered, and a range outside the document is
said rather than guessed at.

    verify: python .claude/hooks/proved.py -p assist the_editors_instructions_say_the_document_is_data_and_stay_short
    verify: python .claude/hooks/proved.py -p wp-text one_paragraph_is_one_line_of_markdown_and_an_underscore_in_a_word_is_a_letter
    verify: python .claude/hooks/proved.py -p scriva a_request_carries_the_scope_numbered_with_the_paragraphs_around_it_and_the_headings
    verify: python .claude/hooks/proved.py -p scriva the_four_tools_are_strict_and_reading_answers_with_the_paragraphs_named

### C2 — a rewrite is a proposal

"Improve the wording" lands as a redline by the Assistant: the old words
struck, the new beside them, a card in the pane with Accept and Reject, and
Accept makes it plain text. A rewritten heading keeps its style, and `**` in
the new words is bold. Reject puts the paragraph back exactly, and one Undo
takes the whole proposal away. A rewrite into more or fewer paragraphs accepts
to the new paragraphs and rejects to the old; paragraphs inserted, or replaced
with nothing, are proposals too. A comment from the assistant is a card on the
page and in Review. Each card settles its own proposal and no other. Track
Changes being off makes no difference to any of it.

    verify: python .claude/hooks/proved.py -p scriva improve_the_wording_lands_as_a_redline_by_the_assistant_that_accept_makes_plain_text
    verify: python .claude/hooks/proved.py -p scriva a_rewritten_heading_is_still_a_heading_and_its_bold_words_are_still_bold
    verify: python .claude/hooks/proved.py -p scriva reject_puts_the_paragraph_back_and_undo_takes_the_proposal_away_entirely
    verify: python .claude/hooks/proved.py -p scriva a_rewrite_into_more_or_fewer_paragraphs_accepts_to_the_new_ones_and_rejects_to_the_old
    verify: python .claude/hooks/proved.py -p scriva paragraphs_inserted_or_taken_out_are_proposals_too
    verify: python .claude/hooks/proved.py -p scriva a_comment_from_the_assistant_is_a_card_on_the_page_and_in_review
    verify: python .claude/hooks/proved.py -p scriva each_card_settles_its_own_proposal_and_no_other
    verify: python .claude/hooks/proved.py -p scriva a_proposal_is_tracked_whether_track_changes_is_on_or_not

### C3 — the assistant's changes, together

Accept all and Reject all on the pane's menu settle the Assistant's changes
and no one else's. A document saved with proposals still open carries them as
tracked changes, and the status line says how many. A paragraph that tells the
assistant to delete everything can bring about only a proposal.

    verify: python .claude/hooks/proved.py -p scriva accept_all_and_reject_all_in_the_pane_settle_only_the_assistants_changes
    verify: python .claude/hooks/proved.py -p scriva a_document_saved_with_open_proposals_carries_them_as_tracked_changes_and_says_so
    verify: python .claude/hooks/proved.py -p scriva a_paragraph_that_tells_the_assistant_to_delete_everything_can_only_propose

### C4 — the pane in Scriva's window

View ▸ Assist and Ctrl+Alt+A open the pane with the composer ready, F6 walks
it with the others, and Escape gives the document back. Assist and Review share
the right-hand side, each a tab of the other, at one width. The scope chip
follows the selection, and Whole document states its word count. Proposals
that arrive with the pane closed are a notice on the row under the toolbar,
with Show, and the status bar says the assistant is working while it is. The
right-click menu asks the assistant about the selection, and its rows, like
the menus', each have a letter of their own. A new document ends the
conversation about the old one. Nothing a test does reaches a helper.

    verify: python .claude/hooks/proved.py -p scriva ctrl_alt_a_opens_assist_and_f6_walks_it
    verify: python .claude/hooks/proved.py -p scriva assist_and_review_share_the_right_hand_side
    verify: python .claude/hooks/proved.py -p scriva the_scope_chip_follows_the_selection_and_whole_document_states_the_word_count
    verify: python .claude/hooks/proved.py -p scriva proposals_arriving_with_the_pane_closed_are_a_notice_on_the_row
    verify: python .claude/hooks/proved.py -p scriva the_status_bar_says_the_assistant_is_working
    verify: python .claude/hooks/proved.py -p scriva the_right_click_menu_asks_the_assistant_about_the_selection
    verify: python .claude/hooks/proved.py -p scriva a_new_document_ends_the_conversation_about_the_old_one
    verify: python .claude/hooks/proved.py -p scriva no_two_rows_of_a_menu_share_a_letter
    verify: python .claude/hooks/proved.py -p scriva no_two_rows_of_the_context_menu_share_a_letter
    verify: python .claude/hooks/proved.py -p scriva the_assist_pane_is_drawn_where_it_can_be_seen

## The end

### Z1 — the page did not move, and the files did not change

    verify: cargo xtask compare --check
    verify: cargo xtask fidelity
