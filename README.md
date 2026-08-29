# Officina

Officina is a native office suite, written in Rust, for the two file formats
an engineer cannot avoid. It is two desktop applications:

- **Calx** — spreadsheets. Opens and saves `.xlsx`, reads `.xls`, reads and
  writes `.csv` and `.tsv`.
- **Scriva** — documents. Opens and saves `.docx`, reads `.doc`, reads and
  writes Markdown and plain text.

Each is a single executable with no runtime to install, no bundled browser, and
no telemetry. Between them they are about 100,000 lines of Rust and 1,350 tests.

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

## Install

Requires a Rust toolchain (1.80 or newer).

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

The code is written for both platforms — the config directory, the font search
and the path comparison all have Linux branches, and the only Windows-specific
paths in the repository are in tests that skip themselves when Office is not
installed. But this has been **built and run on Windows 11 only**. A Linux build
has not been verified, and until someone runs `cargo xtask check` on Ubuntu that
sentence should be read exactly as written.

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
