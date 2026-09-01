"""Fidelity's second check covers `.odt`, and passes.

**`cargo xtask fidelity` passing is not evidence that it looked.** Check 2 —
open, edit one paragraph, save, reopen, account for every byte that moved —
currently steps over an OpenDocument package, because there is no writer for one
yet. So the harness is green *because* it is not asking, and an item verified by
"fidelity passes" reported itself finished on the day it was written.

This asks both halves of the question: that the skip is gone, and that the
harness still passes without it.
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# The skip itself, matched as code rather than as the comment above it: a
# comment is reflowed by `cargo fmt` and a marker that keys on one is a marker
# that quietly stops matching. While this stands, check 2 has never seen an
# `.odt`.
SKIP = re.compile(r"is_open_document\(&path\)\s*\{\s*continue;")


def main() -> int:
    harness = (ROOT / "xtask" / "src" / "fidelity.rs").read_text(encoding="utf-8")
    if SKIP.search(harness):
        print("xtask/src/fidelity.rs still steps over .odt in check 2")
        return 1
    done = subprocess.run(
        [sys.executable, str(ROOT / ".claude" / "hooks" / "gate_cargo.py"), "xtask", "fidelity"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=7200,
    )
    if done.returncode != 0:
        print((done.stdout + done.stderr).strip()[-1500:])
    return done.returncode


if __name__ == "__main__":
    raise SystemExit(main())
