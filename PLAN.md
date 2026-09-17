# Assist, phase 2: the pane

The second of six phases that give Scriva and Calx an assistant a person talks
to while editing. The specification, with every decision and why it was taken,
is kept in the story workspace (`bugs/assist-spec.md`); phase 1 built the
helpers, and this plan is the pane they answer into. **This file is immutable
while the work runs.** Nothing that does the work may edit it: not to reword an
item, not to remove one, and above all not to mark one done. Phase 1's plan is
in the history, and every item of it proved itself before it was replaced.

Each item carries a `verify:` command and is finished exactly when that command
exits zero. `python .claude/hooks/gate.py` runs them all. A `cargo test` filter
that matches no test exits zero, so every test item goes through
`.claude/hooks/proved.py`, which insists a test ran.

## Why

A helper that answers is half an assistant. The other half is where a person
asks, sees the answer arrive, sees what the helper looked at and changed, and
stops it — and where the helper is chosen, once, by someone who has never heard
the word "model". This phase builds that half in `ui_kit::assist`, once, for
both applications, with nothing of either document in it: each application
gives the pane its tools, its scopes and its quick verbs, runs the tools the
helper asks for against its own document, and hands back what came of each.
Wiring it into Scriva is phase 3, and into Calx phase 4; here it is driven in
ui-kit's own tests, hosted by a small application of the tests' own.

The rules this phase makes true for the rest:

- **The document never leaves the window's thread.** A request runs on a
  thread of its own; a tool call comes back to the window, at most one a
  frame, and the application runs it there.
- **The window never waits on a helper.** Stop is immediate on the screen,
  and a helper that cannot hear it yet is let go, not waited for.
- **Nothing leaves the computer unannounced.** The first request to a helper
  over the internet says what will be sent, and to whom, and waits.
- **A key is checked before it is kept, and never shown.**

Nothing here touches a reader or a writer: `cargo xtask fidelity` stays at zero
failures.

---

### B1 — a request, on a thread of its own

A request typed into the composer runs on a thread of its own while the window
goes on painting, and the helper's words arrive in the transcript as they are
said. When the helper wants a tool, the call is handed to the application on a
frame, never more than one a frame, and the application's result goes back to
the helper. A request that does not finish leaves its words on the screen,
marked as not kept, since the next request does not build on them. A helper
refused under a test is refused on the request's thread, and the refusal is
counted again when it reaches the window's thread, where a test reads the
count.

    verify: python .claude/hooks/proved.py -p ui-kit a_request_typed_into_the_composer_streams_its_answer_into_the_transcript
    verify: python .claude/hooks/proved.py -p ui-kit a_tool_call_is_handed_to_the_host_one_per_frame_and_its_result_goes_back_to_the_helper
    verify: python .claude/hooks/proved.py -p ui-kit words_from_a_request_that_did_not_finish_are_marked_as_not_kept
    verify: python .claude/hooks/proved.py -p ui-kit a_refusal_on_the_requests_thread_is_counted_on_the_windows

### B2 — Stop

Stop ends a request at once as far as the window is concerned: what arrived
stays in the transcript, the composer is ready for the next request, and a tool
call waiting for the document is answered as stopped. A helper that has not
begun to answer reads Stop only when its first word comes, which can be
minutes; the window lets go of it rather than wait, and the next request goes
to a helper of its own, from the conversation as it stood before the stopped
request.

    verify: python .claude/hooks/proved.py -p ui-kit stop_leaves_the_transcript_with_what_arrived_and_the_composer_ready
    verify: python .claude/hooks/proved.py -p ui-kit stop_lets_go_of_a_helper_that_has_not_begun_to_answer

### B3 — the first-run card

Until a helper is chosen, the pane asks which, once, in words. The rows come
from what the computer has, looked for on a thread of its own, in the order the
ladder gives them: what was found first, and preselected; the helper on this
computer first when nothing was found. The words "model", "LLM", "token",
"inference", "endpoint" and "GPU" are nowhere on the card. Ollama's row lists
every model the server has with its size and says which are large, and the one
it offers is the most recent that is not. Once a helper is chosen, the pane's
header names it in words.

Until the helper on this computer exists (phase 5), its row stays where the
user's answer put it — preselected when nothing was found — and choosing it
says that it is not ready yet, instead of keeping a choice that cannot answer.

A settings file that cannot be read is said, with the reason, in place of the
card, and choosing a helper again keeps the old file beside the new one.

    verify: python .claude/hooks/proved.py -p assist ollamas_models_come_with_their_sizes_and_the_small_ones_first
    verify: python .claude/hooks/proved.py -p ui-kit the_first_run_card_preselects_the_first_thing_found_and_the_local_helper_when_nothing_is
    verify: python .claude/hooks/proved.py -p ui-kit the_card_says_model_llm_token_endpoint_and_gpu_nowhere
    verify: python .claude/hooks/proved.py -p ui-kit the_ollama_row_lists_every_model_and_says_which_are_large
    verify: python .claude/hooks/proved.py -p ui-kit the_helper_on_this_computer_is_not_kept_as_the_choice_until_it_is_ready
    verify: python .claude/hooks/proved.py -p ui-kit settings_that_cannot_be_read_are_said_in_place_of_the_card_and_set_aside_when_a_helper_is_chosen

### B4 — the settings box

Settings is one box: the helper, in the card's words; for Claude the sign-in,
the key, the three models in words with what a paragraph costs on each, the
fallback, and what this session has spent, in words and cents; for Ollama and
another service the address, the key and the model's name. A key is checked
with one request that costs nothing before it is kept, the box says what the
service answered, and a key is never painted. Under a test the check is
refused, and counted, like any other reach for a helper.

    verify: python .claude/hooks/proved.py -p assist a_key_is_checked_with_a_request_that_costs_nothing
    verify: python .claude/hooks/proved.py -p ui-kit a_headless_process_refuses_every_helper_but_the_scripted_one_and_counts_it
    verify: python .claude/hooks/proved.py -p ui-kit the_settings_box_offers_each_claude_model_with_its_cost_and_says_what_the_session_spent
    verify: python .claude/hooks/proved.py -p ui-kit a_key_is_kept_only_once_the_service_has_accepted_it_and_is_never_painted

### B5 — before anything leaves the computer

The first request to a helper over the internet says, in the pane, what will
be sent and to whom, and waits for Send; Not this time puts the words back in
the composer. A later request to the same helper does not ask again, and a new
helper does. A helper that sends nothing away — the one on this computer, or
Ollama at an address on this computer — never asks.

    verify: python .claude/hooks/proved.py -p ui-kit the_first_request_to_a_helper_over_the_internet_says_what_will_be_sent_and_waits_for_send

### B6 — the pane in a window

The pane is a panel beside the document that any `DocumentApp` hosts. Escape
in the pane hands the keyboard back to the document, and what is typed next is
the document's. A quick verb sends at once, asking for little effort; one that
needs words of its own (Translate…) puts its start in the composer and waits.
The scope chip follows what the application says the selection is, until the
person chooses another. The pane's menu opens Settings, clears the
conversation, and carries the application's own rows. Every part of the pane is
painted inside a clip that has room in it: the find bar was once reachable by
every key and never on the screen.

    verify: python .claude/hooks/proved.py -p ui-kit escape_in_the_pane_hands_the_keyboard_back
    verify: python .claude/hooks/proved.py -p ui-kit a_quick_verb_sends_at_once_with_little_effort_and_one_that_needs_words_waits_for_them
    verify: python .claude/hooks/proved.py -p ui-kit the_scope_chip_follows_the_host_until_the_person_chooses_another
    verify: python .claude/hooks/proved.py -p ui-kit the_panes_menu_opens_settings_clears_the_conversation_and_carries_the_hosts_rows
    verify: python .claude/hooks/proved.py -p ui-kit every_part_of_the_pane_is_painted_where_it_can_be_seen

### B7 — the download's bar

Drawn now, used in phase 5: a bar that fills as the bytes arrive, says in words
how much of how much has come, and offers Stop.

    verify: python .claude/hooks/proved.py -p ui-kit the_download_bar_fills_as_the_bytes_arrive_and_offers_stop

## The end

### Z1 — the page did not move, and the files did not change

    verify: cargo xtask compare --check
    verify: cargo xtask fidelity
