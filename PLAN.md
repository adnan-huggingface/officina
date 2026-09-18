# Assist, phase 6: the record

The last of six phases that give Scriva and Calx an assistant a person talks
to while editing. Phases 1 to 5 built the helpers, the pane, Scriva's half,
Calx's half and the helper that runs on the person's own computer. This plan
writes down what was built and why, shows it working outside a test, and makes
the licences of what it brought in say so. **This file is immutable while the
work runs.** Nothing that does the work may edit it: not to reword an item,
not to remove one, and above all not to mark one done. Phase 5's plan is in the
history, and every item of it proved itself before it was replaced.

Each item carries a `verify:` command and is finished exactly when that command
exits zero. `python .claude/hooks/gate.py` runs them all. A `cargo test` filter
that matches no test exits zero, so every test item goes through
`.claude/hooks/proved.py`, which insists a test ran.

## Why

A feature nobody can read about is a feature nobody can keep. Five phases of
decisions are in a story workspace that ships with nothing; the applications
themselves say almost nothing about the assistant; and the suite now depends on
a machine-learning runtime whose licence file lists none of it. This phase ends
the programme by making the built thing legible: an architecture decision for
the rule the whole design turns on, a guide a person reads before they use it, a
README that says the documents are theirs to vibe too, a licence file that
names everything, and a tour of the real binaries with shots to prove it works
where no test can reach.

The rules this phase makes true:

- **The decision is written where decisions live**, not only in a plan that the
  next phase replaces.
- **A person can find out what Assist does** from the applications' own guide,
  including what it costs and what never leaves the computer.
- **Everything the suite ships is named and licensed**, the assistant's
  dependencies included.
- **It works outside a test**: the real binaries, driven on the rig, with the
  helper on this computer answering and the shots kept.

Nothing here changes how a file is read or written: `cargo xtask fidelity`
stays at zero failures, and `cargo xtask compare --check` holds.

---

### F1 — the decision, written down

ADR 0004, *The model proposes, the editor disposes*: why an assistant's edit
is a tracked change in Scriva and one labelled, washed change in Calx; why the
helper on this computer is the default and no free hosted service is offered;
why the document is data and the tools are the only way anything happens; and
why no test may reach a helper or the network.

    verify: python3 -c "import pathlib,sys; sys.exit(0 if pathlib.Path('adr/0004-the-model-proposes-the-editor-disposes.md').exists() else 1)"
    verify: python3 .claude/hooks/proved.py -p scriva the_guide_and_the_decisions_say_what_assist_does

### F2 — what a person reads

GUIDE.md gains an Assist section for each application — how to open it, what a
request is about, what happens to what it changes, what it costs, and what
leaves the computer — and the README says, under its vibe-first opening, that
the documents are the person's to vibe as well. Every key and row the guide
names is one the applications have.

    verify: python3 -c "import pathlib,sys; t=pathlib.Path('GUIDE.md').read_text(encoding='utf-8'); sys.exit(0 if 'Assist' in t and 'on this computer' in t else 1)"
    verify: python3 .claude/hooks/proved.py -p scriva every_key_the_guide_names_is_a_key_the_window_reads

### F3 — what the suite ships

`THIRD-PARTY-NOTICES.yml` names every crate the applications now depend on,
the assistant's runtime included, with its licence — and a test says the file
is not older than the manifests it describes, so that the next dependency
cannot be forgotten.

    verify: python3 .claude/hooks/proved.py -p ui-kit the_notices_name_every_crate_the_suite_ships

### F4 — it works outside a test

The rig tours the real binaries: Scriva's Assist against the fake helper, and
Calx's; and the helper on this computer answers a request in Scriva with no
network at all. The shots are kept in the story's evidence, and what they show
is written down beside them.

    verify: python3 -c "import pathlib,sys; p=pathlib.Path.home()/'dev/stories/st29-officina/bugs/evidence/assist'; sys.exit(0 if p.is_dir() and any(p.glob('*.png')) and (p/'README.md').exists() else 1)"

## The end

### Z1 — the page did not move, and the files did not change

    verify: cargo xtask compare --check
    verify: cargo xtask fidelity
