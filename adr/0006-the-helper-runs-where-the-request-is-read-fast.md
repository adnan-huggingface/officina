# ADR 0006 — The helper runs where the request is read fast

**Status:** accepted, 2026-09-20. Follows [ADR 0005](0005-an-assistant-below-the-bar-is-not-offered.md).

## The decision

The helper on this computer runs on the graphics processor when this build
can use one and the driver answers for it, and on the processor otherwise;
which, it says. A model is offered on a graphics processor when the card holds
it with room over and the model's measured wait there is within the bar, and
on a processor when the processor's memory, instructions and measured wait
allow — which, as measured, no model of the catalogue does.

Officina is therefore **two builds**: the portable one, for every computer,
and the graphics one, for a computer with an NVIDIA graphics processor, its
driver and its CUDA 12 runtime libraries. The archive says which it is (`-nvidia`), the guide says which to
take, and the portable build names the card it could have used.

## Why

Phase 7 measured the bar (`bugs/assist-bar.md`): on a fast desktop processor
the 4B took 39 seconds and the 8B 48 before the first word, at the median;
the same 8B on this workstation's RTX 3090, through Ollama, took 4.2 and
passed 17 of 20. Reading a thousand-token request is arithmetic over all of
it, and a processor does that at ten to twenty tokens a second while a
graphics processor does it at thousands. No smaller model closes the gap, and
the bar does not bend for it (ADR 0005). So the helper runs where the request
is read fast, or it is not offered — and a normal user has no Ollama, so the
runtime that reaches the card has to be Officina's own.

## What follows from it

**One feature, off by default.** candle's CUDA kernels come in behind `cuda`
on `assist`, passed through `ui-kit`, both applications and `xtask`. The gate
builds and tests without it; the deck and the spike take `--cpu` to hear the
processor on a build that has the card.

**The device is chosen once, without loading a model** (`assist::local::runs_on`):
the first CUDA device the driver answers for, else the processor. The pane's
header says which — "Helper on this computer — Qwen3 8B, on the graphics
processor" — and so does the card's row, with the wait measured there.

**The computer is judged with the card in view.** `Hardware::graphics` says
whether the card is usable by this build; `tier` judges a usable card by its
own memory (the model's, with a gigabyte over for the card's own work) and the
model's measured graphics wait, and a processor as phase 7 left it. A card
that is there but not usable — the portable build, or a driver that did not
answer — is named where the processor is judged, and the graphics build
pointed at.

**Two builds, because the libraries are linked, not loaded.** candle links
`cudarc` dynamically at load time — against the driver's `libcuda` *and*
NVIDIA's CUDA 12 runtime libraries, `libcublas` and `libcurand`, which come
with the toolkit or its runtime packages, not with the driver. A binary built
with the feature does not start without all three; a gaming computer with
only the GeForce driver gets no window. The guide says what to install. One
binary that probed at startup would have been simpler for the person and was
not available without patching candle (cudarc's `dynamic-loading` cannot sit
beside the `dynamic-linking` candle asks for, and it panics on a missing
library rather than failing); that is written here so that the day it is, the
two archives can become one — or NVIDIA's redistributable runtime libraries
travel in the archive, which is a licensing decision not taken here.

**Built for the oldest card it can run on.** candle compiles its kernels
for one compute capability — the build machine's card unless told — and a
card of a lower one refuses them (`CUDA_ERROR_INVALID_PTX`, found on this
workstation's Tesla P40, a Pascal card, given kernels built beside an RTX
3090). Told to build for Pascal, candle's kernels do not compile at all
(`atomicAdd` on half floats); told to build for Turing, nor do they
(`__hmax_nan`, an Ampere intrinsic). So the floor is not ours to choose: the
graphics build sets `CUDA_COMPUTE_CAP=80`, Ampere — the GeForce RTX 30 series
of 2020 — and every NVIDIA card from there on runs it; the P40 could not be
measured, and the deck's graphics numbers are one card's, the floor's own
generation.

## What was rejected

- **Ollama as the answer.** It is the fast path on this workstation and the
  card points at it — but a normal user does not have it, and an assistant
  that needs another program installed is not "on this computer".
- **A smaller model for the processor.** Measured: the 1.7B is the smallest
  that calls tools at all and fails five items of six; speed on a processor
  comes from a smaller request, not a smaller model, and the request is what
  the person asked.
- **Metal, here.** The same shape for Apple silicon, and the same feature
  gate; not built or measured on this Linux workstation, and said so rather
  than pretended.

## What it costs

A second archive to build, name and explain; a build machine with the CUDA
toolkit; and a person with an AMD or Intel graphics processor gets the
portable build's honest sentence and no local helper. Those are real costs,
and the measured alternative — a helper that takes a minute — was judged not
an assistant.
