# Assist, phase 8: the helper on the graphics processor

Phase 7 set the bar and measured it. On a processor alone no model of the
catalogue meets its sixth item — 39 to 48 seconds before the first word on a
fast desktop — and the same 8B on this workstation's graphics processor,
reached through Ollama, met the whole bar: 17 of 20, 4.2 seconds to the first
word at the median. The user's verdict at the desk: *"excellent speed and also
wrote 2 paragraphs as asked."* So the model, the requests and the numbers are
settled, and what is missing is Officina's own runtime on that hardware — a
normal user has no Ollama. This phase gives the helper on this computer the
graphics processor, and offers it only where one that meets the bar is found.

**This file is immutable while the work runs.** Nothing that does the work may
edit it: not to reword an item, not to remove one, and above all not to mark one
done. Phase 7's plan is in the history, and every item of it proved itself
before it was replaced.

Each item carries a `verify:` command and is finished exactly when that command
exits zero. `python3 .claude/hooks/gate.py` runs them all. A `cargo test`
filter that matches no test exits zero, so every test item goes through
`.claude/hooks/proved.py`, which insists a test ran.

## Why

Reading a thousand-token request is arithmetic over all of it. A processor does
it at ten to twenty tokens a second; a graphics processor at thousands. No
smaller model closes that gap, and the bar (ADR 0005) does not bend for it. So
the helper runs where the request is read fast, or it is not offered. The rules
this phase makes true:

- **The runtime reaches the graphics processor**: candle's CUDA kernels, behind
  a Cargo feature that is off by default, so that the portable build and the
  gate know nothing of it.
- **The helper reads the model onto the graphics processor when it can, and
  onto the processor when it cannot**, and says which in the pane.
- **The computer decides with the card in view.** A graphics processor the
  build can use, with memory enough for a model, offers that model at the
  card's measured wait; a processor alone is judged as phase 7 left it.
- **Two builds, said plainly.** A binary linked against the driver does not
  start without it, so the release is two archives per platform: one for a
  computer with an NVIDIA graphics processor, one for every other. The guide
  says which to take, in words.
- **Measured before believed**: the deck against the 4B and the 8B on this
  workstation's card through Officina's own runtime, and the thresholds and
  the words set from those numbers.
- **No test touches a graphics processor, downloads, or reaches a helper.**
  The gate builds and tests without the feature; what it proves about the
  device choice is the processor half and the words.

Nothing here changes how a file is read or written: `cargo xtask fidelity`
stays at zero failures, and `cargo xtask compare --check` holds.

---

### H1 — the runtime

`assist` gains the feature `cuda` (`candle-core/cuda`, `candle-nn/cuda`,
`candle-transformers/cuda`), and both applications and `xtask` pass it
through. `assist::local` chooses its device once: the first CUDA device when
the feature is in and the driver answers, else the processor; the choice is
made without loading a model and can be forced to the processor. The model,
the prompt tensors, the cache truncation and the sampling all run on the
chosen device. The spike and the deck say which device answered and take
`--cpu` to force the other.

    verify: python3 -c "import pathlib,sys; t=pathlib.Path('crates/assist/Cargo.toml').read_text(encoding='utf-8'); sys.exit(0 if 'cuda = [' in t else 1)"
    verify: python3 .claude/hooks/proved.py -p assist without_a_graphics_processor_the_helper_reads_onto_the_processor_and_says_so
    verify: python3 -c "import pathlib,sys; t=pathlib.Path('xtask/src/eval.rs').read_text(encoding='utf-8'); sys.exit(0 if '--cpu' in t else 1)"

### H2 — the computer decides with the card in view

`Hardware::graphics` says whether the card is one this build can use — the
driver answered for it — besides its name and memory. `Model` carries two
measured waits, the processor's and the graphics processor's, and `tier`
judges a usable card by its memory (the model's, with room over) and the
card's wait, and a processor as before. The row, the header and Settings say
where the helper will run: "on this computer's graphics processor". A card
that is there but not usable by this build is named, and the guide's words
about the other build are pointed at.

    verify: python3 .claude/hooks/proved.py -p assist a_usable_graphics_processor_offers_the_largest_model_it_holds_at_the_cards_wait
    verify: python3 .claude/hooks/proved.py -p assist a_graphics_processor_this_build_cannot_use_is_named_and_the_other_build_pointed_at
    verify: python3 .claude/hooks/proved.py -p ui-kit the_header_and_the_card_say_where_the_helper_runs

### H3 — measured

The deck against the 4B and the 8B on this workstation's RTX 3090 through
Officina's own runtime, thinking off, in the graphics build; the tables in
the story's `bugs/assist-bar.md` beside phase 7's, the graphics waits in
`Model` set from them, and the note saying which items the bar holds at.

    verify: python3 -c "import pathlib,sys; p=pathlib.Path.home()/'dev/stories/st29-officina/bugs/assist-bar.md'; t=p.read_text(encoding='utf-8') if p.exists() else ''; sys.exit(0 if 'through Officina' in t and 'own runtime' in t else 1)"
    verify: python3 .claude/hooks/proved.py -p assist the_graphics_waits_are_measured_and_within_the_bar

### H4 — two builds, said plainly

`cargo xtask dist` builds the portable archive as before and, where the CUDA
toolkit is on the build machine, a second archive for computers with an NVIDIA
graphics processor, named so. `cargo xtask install` takes `--graphics` to
install that one. The guide's section on what the assistant costs says which
archive to take and what happens on a computer without the driver.

    verify: python3 .claude/hooks/proved.py -p xtask the_archive_name_says_what_it_will_and_will_not_run_on
    verify: python3 .claude/hooks/proved.py -p scriva the_guide_says_which_build_to_take_for_a_graphics_processor

### H5 — the record

ADR 0005 gains a postscript, or ADR 0006 stands beside it: the helper runs
where the request is read fast. PROGRESS.md, LEARNINGS.md, GUIDE.md,
README.md, THIRD-PARTY-NOTICES.yml if the tree grew, and the story's HANDOFF.

    verify: python3 -c "import pathlib,sys; t=pathlib.Path('adr').glob('000*.md'); t=''.join(p.read_text(encoding='utf-8') for p in t); sys.exit(0 if 'graphics processor' in t and 'two' in t else 1)"
    verify: python3 .claude/hooks/proved.py -p ui-kit the_notices_name_every_crate_the_suite_ships

## The end

### Z1 — the page did not move, and the files did not change

    verify: cargo xtask compare --check
    verify: cargo xtask fidelity
