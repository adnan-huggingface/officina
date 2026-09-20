# Assist, phase 7: the bar, and the hardware under it

Six phases gave Scriva and Calx an assistant, and two days of a person using
it found what no review had: the helper that ships — Qwen3 1.7B on the
processor — is below the bar a person holds an assistant to. It claims work it
did not do, it makes the smallest edit it can call by the name asked for, and
it takes minutes. The user's judgement on 2026-09-20, in one sentence: *"My
complaint is with the quality of the assistant's output."* And their decision:
the assistant is not a toy; it must do all six of the things below, and
Officina must look at the computer it is on and say whether a local helper is
even an option there.

**This file is immutable while the work runs.** Nothing that does the work may
edit it: not to reword an item, not to remove one, and above all not to mark one
done. Phase 6's plan is in the history, and every item of it proved itself
before it was replaced.

Each item carries a `verify:` command and is finished exactly when that command
exits zero. `python3 .claude/hooks/gate.py` runs them all. A `cargo test`
filter that matches no test exits zero, so every test item goes through
`.claude/hooks/proved.py`, which insists a test ran.

## The bar

A minimum viable assistant, for this suite, does these six things. Not one of
them is optional, and a helper that fails one is not offered as an assistant.

1. **Does what it is asked, through the tools, and never claims otherwise.** A
   chip pressed is a card made. A request for a change that ends with no card
   is a failure of the helper, whatever it said.
2. **A paragraph rewritten is better than the original and means the same.**
   Not a "shorter" sentence that is the same sentence.
3. **Writes a coherent passage on request** — a few hundred words, not one.
4. **Reads a whole short document** for a summary or an answer, and says so
   when it cannot hold it.
5. **In Calx, a formula that computes**, and its own `#NAME?` fixed when the
   engine says so.
6. **First word within about five seconds, a paragraph proposal within about
   twenty**, on the computer it is offered on.

Items 1–5 are what the model is. Item 6 is the computer, and it is the one the
programme's first spec underestimated: a request here is about a thousand
tokens in and a hundred or two out, so its cost is *reading*, which is
arithmetic over the whole prompt, not writing. Seven hundred of those thousand
tokens — the brief and the tools — are the same on every request of a session.

## Why

The spec's premise was *no subscription, no GPU, and still a good default*. The
premise bought a model below the bar, and the design then grew a sentence
("Nothing was changed") to catch the model lying — a symptom mistaken for a
feature. This phase replaces the premise with a rule: **a local helper is
offered only where the computer can run one that meets the bar**, and where it
cannot, the person is told so in words, and told what can answer instead. The
rules it makes true:

- **The bar is written down** where decisions live, and every candidate helper
  is measured against it by the same deck, by hand, against the real weights.
- **The computer is examined**, and the local helper offered is the largest
  the computer can hold and run within the bar — or none.
- **What never changes is read once.** The brief and the tools are kept in the
  model's cache across a session's requests, so that a request costs the
  reading of the document and the words, not of everything.
- **No test downloads, loads real weights, reaches a helper or reaches the
  network.** The deck is hand-run and never in the gate; what the gate proves
  about it is that its checks catch a helper below the bar, against the
  scripted one.

Nothing here changes how a file is read or written: `cargo xtask fidelity`
stays at zero failures, and `cargo xtask compare --check` holds.

---

### G1 — the bar, written where decisions live

ADR 0005, *An assistant below the bar is not offered*: the six items, why the
first premise failed, the rule that the computer decides whether a local helper
is offered, and what a person is told where it is not. The spec in the story
workspace gains the same as "The bar (2026-09-20)". The guide says what the
assistant needs of a computer, in words without "GPU" or "token".

    verify: python3 -c "import pathlib,sys; sys.exit(0 if pathlib.Path('adr/0005-an-assistant-below-the-bar-is-not-offered.md').exists() else 1)"
    verify: python3 .claude/hooks/proved.py -p scriva the_guide_and_the_decisions_say_what_assist_does
    verify: python3 .claude/hooks/proved.py -p scriva the_guide_says_what_the_assistant_needs_of_a_computer

### G2 — the deck

`cargo xtask assist-eval <folder>` — hand-run, never in the gate — runs about
twenty requests through the real `Session` loop against fixtures built in
memory: Scriva's tools against documents (a one-paragraph story, a clumsy
letter, a report of some fifteen hundred words with headings and planted facts,
an empty document) and Calx's against a workbook (a header row and figures, an
empty sheet, a cell with a broken formula). Each request has a mechanical check
for the bar item it stands for — the tool called or not called, words landed
and their count, a planted fact in the reply, a formula that evaluates, a
claim made with no card — and the deck records first word and total time for
every one, then prints a table and, for the items a person must judge, the
words themselves. The gate proves the checks, not the model: against the
scripted helper, the deck fails a helper that writes one sentence for a story,
that claims a change with no card, that writes a formula the engine rejects,
and that answers a question with a tool.

    verify: python3 .claude/hooks/proved.py -p xtask the_deck_fails_a_helper_below_the_bar_and_passes_one_that_meets_it
    verify: python3 -c "import pathlib,sys; t=pathlib.Path('xtask/src/main.rs').read_text(encoding='utf-8'); sys.exit(0 if 'assist-eval' in t else 1)"

### G3 — the models, a catalogue

`assist::local::MODELS`: Qwen3 4B and Qwen3 8B at Q4_K_M, from Qwen's own
GGUF repositories, Apache-2.0, each pinned by size and SHA-256, with the memory
it needs while it runs. The 1.7B is no longer offered: it is below the bar, and
the note says so. Every path that named the one model — the folder, `have`,
the download, the first-run card, Settings, the pane's header — takes a model
of the catalogue; the settings say which; weights of a model no longer offered
are removed with the others when a person presses Remove.

    verify: python3 .claude/hooks/proved.py -p assist the_catalogue_pins_every_model_by_size_hash_and_licence_and_the_small_one_is_gone
    verify: python3 .claude/hooks/proved.py -p assist the_download_fetches_what_is_missing_and_no_more
    verify: python3 .claude/hooks/proved.py -p ui-kit settings_say_which_model_on_this_computer_and_remove_takes_every_folder

### G4 — the computer decides whether local is an option

`assist::machine::Hardware`: how much memory the computer has, what its
processor can do (`is_x86_feature_detected!` and the arm64 equivalent), and
whether a graphics processor candle could use is there. `tier(&Hardware)` is
the largest model of the catalogue the computer can hold with room to work, or
none; the thresholds are the measured ones from G6, and the test says which
computer gets which. Where the answer is none, `Row::Local` is not on the
first-run card and the local choice is not in Settings; the card says, in
words, what this computer lacks and what can answer instead. Where a computer
can hold a model but will be slow, the row says how slow, from the same
numbers. Nothing here runs a model to find out.

    verify: python3 .claude/hooks/proved.py -p assist a_computer_below_the_floor_is_not_offered_a_local_helper_and_is_told_why
    verify: python3 .claude/hooks/proved.py -p assist the_tier_is_the_largest_model_the_computer_can_hold_and_says_how_it_will_feel
    verify: python3 .claude/hooks/proved.py -p ui-kit a_computer_that_cannot_run_a_helper_is_not_offered_one_in_settings_either

### G5 — what never changes is read once

The brief and the tools open every request of a session and never change. The
local helper keeps the model's keys and values for that prefix across requests
and reads only what follows it, so that the wait before the first word is the
document's and the words', not the brief's. A request whose prefix differs — a
new session, the other application — starts afresh, as every request did
before. Proved without weights: on the tiny model, decoding greedily, an answer
with the prefix kept is the same answer, token for token, as one read whole;
and the number of tokens the model was made to read is what the request added.
The speed it buys on the real weights is measured in G6.

    verify: python3 .claude/hooks/proved.py -p assist a_request_that_shares_the_last_ones_prefix_reads_only_what_follows_it_and_answers_the_same
    verify: python3 .claude/hooks/proved.py -p assist a_request_with_a_different_prefix_starts_afresh

### G6 — measured, and the thresholds set from the numbers

The deck run on this workstation against the 1.7B, 4B and 8B weights, in a
release build, portable and `target-cpu=native`, with and without G5; the
table in the story's `bugs/assist-bar.md` and its lessons in LEARNINGS.md.
G4's thresholds are those numbers, and the note says which model met which
item of the bar and at what wait. The deck's requests, the fixtures and the
checks are in the repository; the outputs judged by hand are in the note.

    verify: python3 -c "import pathlib,sys; p=pathlib.Path.home()/'dev/stories/st29-officina/bugs/assist-bar.md'; t=p.read_text(encoding='utf-8') if p.exists() else ''; sys.exit(0 if '8B' in t and '4B' in t and 'first word' in t else 1)"

### G7 — the record

PROGRESS.md, LEARNINGS.md, GUIDE.md's section on what the assistant costs and
needs, README.md's paragraph where it promises a helper "on this computer",
`THIRD-PARTY-NOTICES.yml` if anything was brought in or vendored, and the
story's HANDOFF. The guide's numbers are the measured ones.

    verify: python3 .claude/hooks/proved.py -p ui-kit the_notices_name_every_crate_the_suite_ships
    verify: python3 -c "import pathlib,sys; t=pathlib.Path('README.md').read_text(encoding='utf-8'); sys.exit(0 if '1.7B' not in t and '1.3 GB' not in t else 1)"

## The end

### Z1 — the page did not move, and the files did not change

    verify: cargo xtask compare --check
    verify: cargo xtask fidelity
