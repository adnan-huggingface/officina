# Officina

Officina is a vibe-first office suite. You add a feature by describing it to a
coding agent, and the agent builds it by the practice this repository writes
down: a plan whose every item proves itself, tests that use the applications
the way a person does, a gate that has to pass, and a record of what was done
and what it taught. The applications are the product, and so is the way they
grow.

Under the practice sits the promise that makes it safe to point an agent at
real files: saving never rewrites what you did not edit.

**The documents are yours to vibe as well.** `Ctrl+Alt+A` opens Assist in either
application: ask for what you want in your own words — improve this wording, add
a column that totals the others — and a model does it through the editor's own
functions. In Scriva what it changes arrives as tracked changes by "Assistant",
to accept or reject; in Calx it lands at once as one labelled entry with Undo on
its card. Where your computer has no helper of its own, the one it
offers you downloads once and runs on your own processor, so nothing you write
leaves the machine; Claude with your own key, or an Ollama you already run, is a
row away. The decision behind it is
[ADR 0004](adr/0004-the-model-proposes-the-editor-disposes.md), and
[GUIDE.md](GUIDE.md) says how to use it.

Officina is two native desktop applications, written in Rust:

- **Calx** — spreadsheets. Opens and saves `.xlsx`, reads `.xls`, reads and
  writes `.csv` and `.tsv`.
- **Scriva** — documents. Opens and saves `.docx` and `.odt`, reads `.doc`,
  reads and writes Markdown and plain text.

Each is a single executable with no runtime to install, no bundled browser, and
no telemetry. Between them they are about 177,000 lines of Rust and 2,100 tests.

## The one thing worth knowing

**These applications do not rewrite your files.** When you save, the parts of the
package you did not change are copied back byte for byte, and only the parts you
did change are edited in place. A `.docx` that you open and save without editing
comes out identical to the byte — the same rsids, the same content controls, the
same equations, the same things this project has never heard of.

That is not an optimisation. It is the design, because the alternative — reading
a file into a model and printing the model back — silently drops everything the
model does not know about, and no test suite can enumerate what a real document
contains. `cargo xtask fidelity` checks it on every corpus file, twice: once for
an untouched save, once for a save after an edit.

See [DESIGN.md](DESIGN.md) §3 for how it works and [FORMATS.md](FORMATS.md) for
exactly what is and is not understood.

## Adding a feature by vibe

Describe the feature to an agent working in this repository. Claude Code reads
[CLAUDE.md](CLAUDE.md), which brings in [AGENTS.md](AGENTS.md); other agents
read AGENTS.md directly. From there the agent works the way every change here
has been made:

1. **It writes a plan.** [PLAN.md](PLAN.md) lists the work, and each item
   carries a `verify:` command. An item is done when its command exits zero,
   and the plan is not edited while the work runs, so nothing doing the work
   can declare itself finished.
2. **It writes the tests.** They drive the application through the window's
   own frame, with the keys, clicks and menus a person would use, and read
   back what the window painted. [adr/0002](adr/0002-test-by-using-it-as-a-human.md)
   records why: one afternoon of using the application as a person found a
   crash and two silent data losses that a green suite had never touched.
3. **It writes the code** at the layer that owns the behaviour. The interface
   never touches a file format, so a feature cannot quietly change what is
   saved.
4. **It passes the gate.** `python .claude/hooks/gate.py` exits zero only when
   formatting, clippy, every test, the layout check and every plan item pass.
5. **It records the change.** [PROGRESS.md](PROGRESS.md) is the work log, and
   [LEARNINGS.md](LEARNINGS.md) keeps what a format taught, so the next agent
   starts from it.

[MAP.md](MAP.md), regenerated on every check, tells an agent where each thing
lives and what Word or Excel was measured to do.

The Scriva redesign of September 2026 is the worked example: ten phases built
from a written specification, one commit each, every claim proved by a test.
PROGRESS.md tells it, from "The chrome is one palette" to "What driving the
redesign found".

## Install

Requires a Rust toolchain (1.95 or newer).

```bash
cargo xtask install
```

That builds both applications in release mode and copies them to
`~/.local/bin`. Configuration and the recent-files list live in
`~/.config/calx/` and `~/.config/scriva/`; nothing is written anywhere else.

To make a double-clicked file open in the right application:

```bash
cargo xtask associate
```

On Linux that writes `.desktop` entries under `~/.local/share/applications`. On
Windows it prints the commands to run rather than editing your registry for you.

To build an archive to copy to another machine:

```bash
cargo xtask package
```

That produces `target/dist/officina-<version>-<arch>-<os>.zip`, holding both
executables and this documentation. It is a convenience rather than a layout:
each binary is self-contained, so unzipping it anywhere and running it from
there works.

### Linux

Officina was built on Windows 11 and has since been built and tested on Ubuntu
24.04, where `cargo xtask check` passes whole. The tests that drive Word or
Excel skip themselves where Office is not installed, and the layout check reads
the committed renderings in `corpus/rendered/`, so it needs neither. Renewing
one of those renderings needs a machine with the application that owns the
format.

## Other commands

```bash
cargo xtask check
```

Formatting, clippy with warnings denied, and the whole test suite.

```bash
cargo xtask fidelity
```

The round-trip harness: every file in `corpus/` is opened and saved, then opened,
edited and saved, and both results are compared with the original part by part.

```bash
cargo xtask perf
```

A stopwatch over the corpus, then over documents and workbooks larger than any
of it.

## Documentation

- [GUIDE.md](GUIDE.md) — how to use them.
- [FORMATS.md](FORMATS.md) — what is read, what is written, what is preserved
  untouched, and what is not understood.
- [DESIGN.md](DESIGN.md) — the architecture, and the rules it holds to.
- [LEARNINGS.md](LEARNINGS.md) — what building this taught, written down so the
  next format does not repeat it.
- [PROGRESS.md](PROGRESS.md) — the work log, chunk by chunk.
- [LAYOUT.md](LAYOUT.md) — how far each corpus document stands from Word’s
  own rendering of it, mark by mark: every word, and every rule, shading and
  picture around them.
- [adr/](adr/) — the decisions that shaped the code, and the evidence that
  settled each one.
- [retrospectives/](retrospectives/) — how the work was done, dated, and
  written to be useful on a different project.

## Trademarks

Microsoft, Word, and Excel are trademarks of the Microsoft group of companies.
Officina is an independent project built from the published file-format
specifications (ECMA-376/ISO 29500 and Microsoft's Open Specifications); it is
not affiliated with, endorsed by, or sponsored by Microsoft. Product names
appear in this repository only to describe the file formats these applications
read and write.

## Licence

Officina is dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option — the usual arrangement for Rust projects. Use it, modify it,
redistribute it, sell things built on it; both licences allow all of that.
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 licence, shall
be dual-licensed as above, without any additional terms or conditions.
