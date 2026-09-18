# Assist, phase 5: the helper on this computer

The fifth of six phases that give Scriva and Calx an assistant a person talks
to while editing. The specification, with every decision and why it was taken,
is kept in the story workspace (`bugs/assist-spec.md`); phases 1 to 4 built the
helpers, the pane, Scriva's half and Calx's, and every one of them has run
against a helper somewhere else. This plan builds the one the spec promises
first: a model on the person's own computer, which sends nothing anywhere.
**This file is immutable while the work runs.** Nothing that does the work may
edit it: not to reword an item, not to remove one, and above all not to mark
one done. Phase 4's plan is in the history, and every item of it proved itself
before it was replaced.

Each item carries a `verify:` command and is finished exactly when that command
exits zero. `python .claude/hooks/gate.py` runs them all. A `cargo test` filter
that matches no test exits zero, so every test item goes through
`.claude/hooks/proved.py`, which insists a test ran.

## Why

The first row of the first-run card says "The helper on this computer — free,
private, no account", and it is the row a person with no key and no
subscription is meant to take. Until now it has said "not yet". This phase
makes it true: a quantized model read with `candle`, run on the CPU, which
answers in the same events every other helper answers in — so the pane, the
tools and both applications cannot tell it apart.

The rules this phase makes true:

- **Nothing is bundled and nothing is silent.** The weights are downloaded,
  once, after the person has seen the model's name, its licence and its size,
  and pressed the button; the download can be stopped, resumes where it
  stopped, and is refused if what arrives is not what was asked for.
- **The weights live in the cache directory**, never in the repository and
  never beside the settings, and Settings can remove them and say how much
  space comes back.
- **It is a helper like the others.** The same `Provider`: the same words in,
  the same text and tool calls out, the same Stop, the same failures.
- **What it is good at is stated**, and what it is not good at is said in the
  transcript rather than left for the person to work out.
- **A test reaches no network and no weights.** A model of a few dozen
  kilobytes, made by the test itself, is what the runtime's tests run.

Nothing here changes how a file is read or written: `cargo xtask fidelity`
stays at zero failures, and `cargo xtask compare --check` holds.

---

### E1 — the runtime

`assist::local` reads a quantized GGUF with `candle`, holds the pinned model's
name, licence, size, source and hash as one constant, and generates: a prompt
built in the model's own chat template with thinking off, tokens to the end
token or the limit, and Stop honoured between tokens. A tiny model the test
writes itself — a few layers of random weights — is what proves it, so that no
test needs a gigabyte or a network. The model's tool-call syntax becomes the
same `Call` events every other helper's does, and text that is not a tool call
is text.

    verify: python .claude/hooks/proved.py -p assist a_tiny_random_model_generates_tokens_stops_at_the_end_token_and_honours_stop
    verify: python .claude/hooks/proved.py -p assist the_models_tool_call_syntax_becomes_the_same_tool_events

### E2 — the download

The weights come down on a thread of their own, over HTTP, reporting bytes as
they arrive; Stop ends it; an interrupted download resumes where it stopped
rather than starting again; a file whose SHA-256 is not the one the constant
names is refused and removed, so that a half-download or a mirror's mistake
cannot be run. Everything lands in the cache directory, and what is there
already is not fetched twice.

    verify: python .claude/hooks/proved.py -p assist a_download_whose_hash_does_not_match_is_refused_and_removed
    verify: python .claude/hooks/proved.py -p assist an_interrupted_download_resumes_where_it_stopped
    verify: python .claude/hooks/proved.py -p assist the_weights_are_in_the_cache_directory_and_removing_them_says_what_came_back

### E3 — the pane's half

The first-run card's local row says what the helper is, what it is good at,
and what it will download before anything is downloaded; the pane's header
says "on this computer"; the download is a bar with a Stop, and a download that
never answers does not hold the window for a moment. Settings removes the
weights.

    verify: python .claude/hooks/proved.py -p ui-kit a_download_that_never_answers_does_not_hold_the_window
    verify: python .claude/hooks/proved.py -p ui-kit the_local_row_says_what_it_will_download_before_it_downloads_anything

### E4 — what it is not good at

A request the local helper fails twice on gets one added sentence in the
transcript, from the application and not from the model, saying a helper over
the internet would do better and where to choose one. It is said once, not
after every failure, and only for the helper on this computer.

    verify: python .claude/hooks/proved.py -p calx a_local_helper_that_fails_twice_gets_the_sentence_about_a_bigger_one

### E5 — it is a helper like the others

Chosen in the settings, the local helper is what a request goes to, with no
network reached and nothing sent anywhere; a request in a test still reaches
no helper at all. What it costs is nothing, and the pane says so rather than
counting cents.

    verify: python .claude/hooks/proved.py -p assist the_local_helper_answers_a_request_as_any_other_provider_does
    verify: python .claude/hooks/proved.py -p scriva no_test_reaches_a_helper

## The end

### Z1 — the page did not move, and the files did not change

    verify: cargo xtask compare --check
    verify: cargo xtask fidelity
