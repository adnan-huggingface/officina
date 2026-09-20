# ADR 0005 — An assistant below the bar is not offered

**Status:** accepted, 2026-09-20. Follows [ADR 0004](0004-the-model-proposes-the-editor-disposes.md).

## The decision

An assistant, in this suite, does six things. A helper that does not do all six
is not offered as one — not as a default, not as a fallback, not as a toy with
an honest label. Officina examines the computer it runs on and offers the
largest helper of its catalogue that the computer can hold and run within the
bar, or none; where none, it says in words what the computer lacks and what
would answer instead.

The six:

1. **Does what it is asked, through the tools, and never claims otherwise.** A
   chip pressed is a card made.
2. **A paragraph rewritten is better than the original and means the same.**
3. **Writes a coherent passage on request** — a few hundred words, not one.
4. **Reads a whole short document** for a summary or an answer, and says so
   when it cannot hold it.
5. **In Calx, a formula that computes**, and its own `#NAME?` fixed when the
   engine says so.
6. **First word within about five seconds, a paragraph proposal within about
   twenty**, on the computer it is offered on.

## Why

The programme's first premise was *no subscription, no GPU, and still a good
default*: a model of 1.7 billion parameters, quantized to four bits, on the
processor. It was chosen for its licence and its size and described, honestly,
as "good for rewording, grammar, summaries and simple sums". Two days of the
user using it from their own desk found what no review had:

- it said "I have improved the wording of paragraph 1" and changed nothing
  (`bugs/assistant-claims-a-change-it-did-not-make.md`);
- asked for a story, it wrote one sentence
  (`bugs/the-story-was-one-sentence.md`);
- a change took ninety seconds to appear and four minutes to finish.

The design then grew a sentence — "Nothing was changed" — to catch the model
lying. That was a symptom mistaken for a feature: a pane that has to police its
helper's claims has a helper below the bar. The user's judgement, in one line,
was *"My complaint is with the quality of the assistant's output"*, and the
decision that followed: the assistant is not a toy; it must do all six things;
and Officina should look at the computer and say whether a local helper is even
an option.

## What follows from it

**The computer is examined, and nothing is run to find out.** `assist::machine`
reads how much memory the computer has, asks the processor for its vector
instructions, and looks for a graphics processor by its driver's tool. `tier`
is the largest model of the catalogue whose memory, with room over for the
applications and the documents, fits. A computer below the floor gets a row on
the first-run card that cannot be chosen and says why — "This computer has
8.0 GB of memory; a helper worth having needs 8.5 GB to run beside your
documents" — and no local choice in Settings. A graphics processor Officina's
own helper cannot use yet is named, because Ollama on the same computer would.

**The catalogue is Qwen3 4B and Qwen3 8B**, Q4_K_M, from Qwen's own GGUF
repositories, Apache-2.0, pinned by size and hash. 4B is the floor, the smallest
that does items 1–5. 8B is the first size at which most people stop noticing
the model. The 1.7B is withdrawn and its weights removed with the others.

**The workload is reading, so the prefix is kept.** A request is about a
thousand tokens in and a hundred or two out, and seven hundred of the thousand
never change within a session. The local helper keeps the model's keys and
values for the shared beginning of consecutive prompts and reads only what
follows. Candle's Qwen3 could only clear its cache whole, so its four hundred
lines are vendored into `assist::local::qwen3` with two functions added —
`truncate_kv_cache` and `kv_len` — and its tracing spans taken out.

**The bar is a deck, run by hand.** `cargo xtask assist-eval` puts a model
through some twenty requests against documents and a workbook built in memory,
one mechanical check per item, and prints the times. What the gate proves is the
checks, against the scripted helper. What the deck measures against the real
weights is recorded in `bugs/assist-bar.md` and sets the thresholds.

## What was rejected

- **Keeping the 1.7B as a labelled toy.** The user's answer was direct: not a
  toy. A helper that fails item 1 is not a smaller assistant, it is an
  unreliable one, and the label does not make it reliable.
- **A speed bar in place of a size bar.** Timing a model on first run to decide
  whether to offer it would run a gigabyte to answer a question about the
  settings, and the answer is mostly memory and instruction set anyway.
- **llama.cpp for speed.** The user chose to ship candle regardless of speed
  (ADR 0004); the prefix cache is the software lever that stays within it.

## What it costs

A computer with less than 8.5 GB of memory, or a processor from before AVX2,
gets no helper of its own. That is a real loss for some people and this record
says so rather than hiding it behind a model that would disappoint them. They
are offered Ollama, on their computer or another, and Claude with a key.

## Postscript, the same day: what the measurements settled

The deck was run against the 4B and 8B on a fast desktop processor (Ryzen 7
9700X, AVX-512), in a release build tuned to it. **The 4B's median wait before
the first word of a paragraph request was 39 seconds; its median request 96
seconds; it passed 12 of 20.** Nothing but a request whose document was
already in the kept prefix came in under five seconds. The 8B is slower and
better. The numbers are in `bugs/assist-bar.md`.

So item 6 is not met by any model of the catalogue on a processor alone, on a
processor faster than most people have — and the rule above applies to
Officina's own helper today: **it is not offered.** The first-run card says
why, with the measured wait, and that Ollama on a computer with a graphics
processor, or Claude, answers within the bar; a person who already has a
helper's weights on the disk keeps it, since they chose it. The catalogue, the
tier, the prefix cache and the deck are built and tested against a catalogue
that would pass, for the day Officina's helper can use a graphics processor —
this workstation has one that would read the same request in well under a
second.

What this costs is said plainly: for now there is no helper that is free,
private and worth having. The alternative — offering the 4B with an honest
"about a minute before the first word" on its row — is one constant away
(`FIRST_WORD_BAR`), and it was the user's decision that a helper below the bar
is not an assistant.

