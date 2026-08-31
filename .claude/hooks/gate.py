"""Whether the work is finished, decided by running things rather than by reading.

**"Done" has to be a command.** Every time this project's agent has stopped
early it has been because "done" lived in prose it was free to reinterpret, and
prose loses to fatigue. So the definition lives here, it exits non-zero until it
is true, and nothing that reads it gets an opinion.

Two halves, and both must pass:

* the repository's own gates, which are the same ones a person runs;
* the plan, if there is one — a checklist file whose unticked boxes are work
  that has not been done, whatever else is green.

Run it by hand at any time:

    python .claude/hooks/gate.py            # the whole gate
    python .claude/hooks/gate.py --quick    # skip the slow half

Exit 0 means finished. Exit 1 means not, and says what is outstanding on stdout.
"""

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# The plan, as a file of boxes. Absent is not an error: a session with no plan
# is held to the gates alone.
PLAN = ROOT / "PLAN.md"

# Each entry is (what it is called, the command, whether it is slow).
GATES = [
    ("cargo xtask check", ["cargo", "xtask", "check"], True),
    ("cargo xtask fidelity", ["cargo", "xtask", "fidelity"], True),
]


def tool(name):
    """Where a command actually is.

    A hook does not inherit the shell a person logs in with, and on this machine
    `cargo` lives in a directory that the profile puts on the path and nothing
    else does. The first version of this gate reported "cargo is not on PATH" as
    work outstanding, which would have refused to let any session end at all.
    """
    found = shutil.which(name)
    if found:
        return found
    beside = Path.home() / ".cargo" / "bin" / name
    for candidate in (beside, beside.with_suffix(".exe")):
        if candidate.exists():
            return str(candidate)
    return None


class Unrunnable(Exception):
    """The gate could not be *asked*, which is not the same as failing it.

    A missing toolchain is a person's problem and cannot be fixed by working
    longer, so it must not be reported as unfinished work: a hook that refused
    to end a session over it would hold every session in the project open.
    """


def unticked():
    """The plan's outstanding boxes, in order."""
    if not PLAN.exists():
        return []
    out = []
    for line in PLAN.read_text(encoding="utf-8").splitlines():
        if re.match(r"\s*[-*]\s+\[ \]\s+", line):
            out.append(line.strip())
    return out


def failing(quick):
    """The gates that did not pass, each with the tail of what it said."""
    out = []
    for name, command, slow in GATES:
        if quick and slow:
            continue
        binary = tool(command[0])
        if binary is None:
            raise Unrunnable(
                f"{command[0]} is not installed, or is not where this can find it"
            )
        # The toolchain's own directory on the path as well, since one tool here
        # shells out to another.
        env = dict(os.environ)
        env["PATH"] = str(Path(binary).parent) + os.pathsep + env.get("PATH", "")
        try:
            done = subprocess.run(
                [binary, *command[1:]],
                cwd=ROOT,
                capture_output=True,
                text=True,
                timeout=3600,
                env=env,
            )
        except subprocess.TimeoutExpired:
            out.append((name, "timed out"))
            continue
        if done.returncode != 0:
            said = (done.stdout + done.stderr).strip().splitlines()
            out.append((name, chr(10).join(said[-25:])))
    return out


def report(quick=False):
    """What is outstanding, as text. Empty means finished."""
    lines = []
    boxes = unticked()
    if boxes:
        lines.append(f"{len(boxes)} item(s) of {PLAN.name} are not done:")
        lines.extend(f"  {box}" for box in boxes[:10])
        if len(boxes) > 10:
            lines.append(f"  … and {len(boxes) - 10} more")
    for name, said in failing(quick):
        lines.append(f"`{name}` failed:")
        lines.extend(f"  {line}" for line in said.splitlines())
    return "\n".join(lines)


def main():
    quick = "--quick" in sys.argv
    outstanding = report(quick)
    if not outstanding:
        print("finished: every gate passes and the plan has no unticked boxes")
        return 0
    print(outstanding)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
