"""Runs the keystroke driver, and fails unless it actually drove the application.

**A gate made of shell commands quietly drops the checks that are not shell
commands.** That is not a hypothesis: the plan that built the OpenDocument
writer was converted from an approved plan whose sixth verification step asked
for the running application to be driven New through Save As, by menu and
keystroke. The step had no exit code, so it did not survive the conversion, and
the whole application half of that plan came down to two unit tests. ADR 0002
exists because one afternoon of this exercise found a crash and two silent data
losses that a suite of 1,350 tests never touched.

So the exercise gets an exit code. This hook runs the driver and then asks the
driver's own leavings whether it did anything:

    python .claude/hooks/drove_it.py

The contract the driver is written to, and the whole of it:

    python tools/drive/scriva_odt.py --out <dir>

exits zero, and leaves in `<dir>` at least three `.png` screenshots read back
during the run and one `recreated.odt` saved through the application's own Save
As dialog, whose text holds the sentence below. ADR 0002's driver rules are not
optional and are not restated here: find the window by process name, check the
foreground before every input, abort rather than type into someone else's
window.

**What this cannot do**, said plainly rather than discovered later: a driver
that wrote the `.odt` itself and screenshotted nothing in particular would pass.
The check is against a driver that is broken or absent, not against one that is
dishonest — and the answer to a dishonest one is that a person looks at the
screenshots, once, which is cheap because they are sitting in a directory. What
it does buy is that the exercise cannot be skipped in silence, which is exactly
how it was skipped last time.
"""

import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DRIVER = ROOT / "tools" / "drive" / "scriva_odt.py"
OUT = ROOT / "target" / "drive"

# What the driver types, so that what lands on disk can be tied to the run that
# produced it rather than to any `.odt` that happened to be lying about.
SENTINEL = "SCRIVA ODT DRIVE"


def fail(why: str) -> None:
    print(f"the keystroke exercise did not happen: {why}")
    sys.exit(1)


def main() -> None:
    if not DRIVER.exists():
        fail(f"{DRIVER.relative_to(ROOT)} is not there")

    OUT.mkdir(parents=True, exist_ok=True)
    for stale in OUT.glob("*"):
        if stale.is_file():
            stale.unlink()

    try:
        done = subprocess.run(
            [sys.executable, str(DRIVER), "--out", str(OUT)],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=1800,
        )
    except subprocess.TimeoutExpired:
        fail("the driver did not finish within half an hour")

    said = (done.stdout + done.stderr).strip()
    if done.returncode != 0:
        fail(f"the driver exited {done.returncode}\n{said[-4000:]}")

    shots = sorted(OUT.glob("*.png"))
    if len(shots) < 3:
        fail(f"{len(shots)} screenshots were read back, which is not a run")

    saved = OUT / "recreated.odt"
    if not saved.exists():
        fail("nothing was saved through Save As")

    try:
        with zipfile.ZipFile(saved) as package:
            names = package.namelist()
            if names[0] != "mimetype":
                fail("what was saved is not an OpenDocument package: mimetype is not first")
            body = package.read("content.xml").decode("utf-8", "replace")
    except zipfile.BadZipFile:
        fail("what was saved is not a package at all")
    except KeyError:
        fail("what was saved has no content.xml")

    if SENTINEL not in body:
        fail(f"what was saved does not hold {SENTINEL!r}, so it is not this run's document")

    print(f"drove it: {len(shots)} screenshots, and {saved.name} came back holding what was typed")
    if said:
        print(said[-2000:])


if __name__ == "__main__":
    main()
