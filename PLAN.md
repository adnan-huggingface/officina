# Assist: the card is chosen for what it can run

The review of `c6f48d7` found that Officina picks its graphics card by **where
it sits**. `assist::local::graphics_device` sets `CUDA_DEVICE_ORDER=PCI_BUS_ID`
and opens device 0, so the card in the lowest slot wins. The ordering it
overrides is CUDA's own `FASTEST_FIRST`, which ranks by the compute capability
that decides whether the kernels load at all.

`Device::new_cuda(0)` **succeeds** on a card whose kernels cannot run: only the
first launch fails, with `CUDA_ERROR_INVALID_PTX`. So on a computer whose older
card sits in the lower slot, Officina reports a graphics processor, reads the
wrong card's memory, offers the 8B, and dies on the first request. This
workstation — an RTX 3090 on bus `01` and a Tesla P40 on bus `10` — escapes by
slot order alone, and the P40 is exactly the card that cannot run the kernels
(ADR 0006: candle 0.9.2 compiles for Ampere and up).

Two more findings from the same review fall out of the same change. The card's
memory is read by running `nvidia-smi`, so a working card is refused where the
driver's command-line tool is not installed — and the tool is asked from the
look's background thread, where `std::env::set_var` is unsound and is `unsafe`
from edition 2024.

**This file is immutable while the work runs.** Nothing that does the work may
edit it: not to reword an item, not to remove one, and above all not to mark one
done. Phase 8's plan is in the history, and every item of it proved itself
before it was replaced.

Each item carries a `verify:` command and is finished exactly when that command
exits zero. `python3 .claude/hooks/gate.py` runs them all. A `cargo test`
filter that matches no test exits zero, so every test item goes through
`.claude/hooks/proved.py`, which insists a test ran.

## Why

A card is not a slot. What decides whether Officina's helper can use a card is
its compute capability against the kernels in this binary, and how much memory
it has — both of which the CUDA driver will say, about every card, before any
context is made. Asking the driver directly is shorter than asking a separate
program to say it, is right where that program is not installed, and needs no
environment variable written behind other threads' backs.

## Items

### K1 — the card is chosen for what it can run, not where it sits

`assist::local` asks the CUDA driver about **every** card — its name, its
memory and its compute capability — before opening anything, and chooses the
one with the most memory among those whose capability meets the kernels'
floor. A card below the floor is never opened, however large it is and
whatever slot it is in. The choosing is a pure function over a list of cards,
so it is tested without a graphics processor: a bigger, older card must not
beat a smaller, newer one.

    verify: python3 .claude/hooks/proved.py -p assist a_bigger_older_card_does_not_win_over_one_the_kernels_can_run
    verify: python3 -c "import pathlib,sys; t=pathlib.Path('crates/assist/src/local.rs').read_text(encoding='utf-8'); sys.exit(0 if 'KERNEL_FLOOR' in t else 1)"

### K2 — the card says its own memory, and the tool is asked only when there is no driver to ask

Where the feature is in and the driver answers, `Hardware::graphics` takes the
card's name and memory from the driver. `nvidia-smi` is asked only by a build
that has no CUDA in it at all — the portable one, which must still name the
card it cannot use so that the sentence can point at the other archive. A card
the driver names is never of unknown memory.

    verify: python3 .claude/hooks/proved.py -p assist a_card_the_driver_named_is_not_of_unknown_memory
    verify: python3 .claude/hooks/proved.py -p assist a_graphics_processor_this_build_cannot_use_is_named_and_the_other_build_pointed_at

### K3 — nothing writes the environment behind another thread

`CUDA_DEVICE_ORDER` is not set by Officina. The ordinal Officina opens is the
one it chose from the driver's own list, which needs no ordering imposed on it.

    verify: python3 -c "import pathlib,sys; t=pathlib.Path('crates/assist/src/local.rs').read_text(encoding='utf-8'); sys.exit(0 if 'set_var' not in t else 1)"
    verify: python3 .claude/hooks/proved.py -p assist without_a_graphics_processor_the_helper_reads_onto_the_processor_and_says_so

### K4 — the record

ADR 0006 gains what this settles: a card is chosen by capability and memory,
not by slot; the driver is the source for both; the floor is the kernels'.
The bug note in the story, PROGRESS.md, and GUIDE.md where it tells a person
which card will do.

    verify: python3 -c "import pathlib,sys; t=pathlib.Path('adr/0006-the-helper-runs-where-the-request-is-read-fast.md').read_text(encoding='utf-8'); sys.exit(0 if 'not by slot' in t else 1)"
    verify: python3 -c "import pathlib,sys; p=pathlib.Path.home()/'dev/stories/st29-officina/bugs/the-card-was-chosen-by-its-slot.md'; sys.exit(0 if p.exists() and 'FASTEST_FIRST' in p.read_text(encoding='utf-8') else 1)"

## The end

### Z1 — the page did not move, and the files did not change

    verify: cargo xtask compare --check
    verify: cargo xtask fidelity
