"""Whether the work is finished, decided by running things rather than by reading.

**"Done" has to be a command.** Every time this project's agent has stopped
early it has been because "done" lived in prose it was free to reinterpret, and
prose loses to fatigue. So the definition lives here, it exits non-zero until it
is true, and nothing that reads it gets an opinion.

Two halves, and both must pass:

* the repository's own gates, which are the same ones a person runs;
* every item of `PLAN.md`, each of which carries a `verify:` command and is
  finished exactly when that command exits zero.

**There are no boxes to tick and no ledger to keep.** An earlier version of this
read checkboxes out of the plan, which meant the thing doing the work could
finish it by editing a file — a definition of done that the worker may edit is
not one. Now the plan is immutable and the repository is asked directly. The
worker cannot claim an item; it can only make a command pass.

The weakness this leaves, stated rather than hidden: a `verify:` command is only
as good as what it checks. Some below run a test, which is strong. A few can
only grep a file, which is weak, and `PLAN.md` marks those as weak.

Run it by hand at any time:

    python .claude/hooks/gate.py            # the whole gate
    python .claude/hooks/gate.py --quick    # skip the slow half
    python .claude/hooks/gate.py --items    # just the plan, itemised

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


def toolchain_env() -> dict:
    """The environment with cargo's own directory on the path.

    A hook does not inherit the shell a person logs in with, and a `verify:`
    command is as entitled to find `cargo` as the gates are.
    """
    env = dict(os.environ)
    binary = tool("cargo")
    if binary:
        env["PATH"] = str(Path(binary).parent) + os.pathsep + env.get("PATH", "")
    return env


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


PLAN_ITEM = re.compile(r"^###\s+(\S+)\s+—\s*(.*)$")
PLAN_VERIFY = re.compile(r"^\s{4,}verify:\s*(.+)$")


def items():
    """The plan, as (id, what it is, the command that proves it).

    An item without a `verify:` line is a plan item nothing can check, which is
    a plan item this cannot hold anybody to — so it is reported as an error in
    the plan rather than silently passed.
    """
    if not PLAN.exists():
        return []
    out, current = [], None
    for line in PLAN.read_text(encoding="utf-8").splitlines():
        heading = PLAN_ITEM.match(line)
        if heading:
            current = [heading.group(1), heading.group(2).strip(), None]
            out.append(current)
            continue
        verify = PLAN_VERIFY.match(line)
        if verify and current is not None and current[2] is None:
            current[2] = verify.group(1).strip()
    return [tuple(item) for item in out]


def unproven():
    """The plan items whose own command does not pass, and what it said."""
    out = []
    for ident, what, command in items():
        if command is None:
            out.append((ident, what, "no `verify:` line — nothing can prove this"))
            continue
        try:
            done = subprocess.run(
                command,
                cwd=ROOT,
                shell=True,
                capture_output=True,
                text=True,
                timeout=3600,
                env=toolchain_env(),
            )
        except subprocess.TimeoutExpired:
            out.append((ident, what, "its verify command timed out"))
            continue
        if done.returncode != 0:
            said = (done.stdout + done.stderr).strip().splitlines()
            tail = chr(10).join(said[-8:]) or f"exit {done.returncode}"
            out.append((ident, what, tail))
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
    for ident, what, why in unproven():
        lines.append(f"{ident} is not done — {what}")
        lines.extend(f"    {line}" for line in why.splitlines()[:8])
    for name, said in failing(quick):
        lines.append(f"`{name}` failed:")
        lines.extend(f"  {line}" for line in said.splitlines())
    return chr(10).join(lines)


def main():
    quick = "--quick" in sys.argv
    if "--items" in sys.argv:
        outstanding = {ident for ident, _, _ in unproven()}
        for ident, what, command in items():
            mark = "no " if ident in outstanding else "yes"
            print(f"  {mark}  {ident}  {what}")
            if command is None:
                print("        (no verify: line)")
        return 1 if outstanding else 0
    outstanding = report(quick)
    if not outstanding:
        print("finished: every gate passes and every plan item proves itself")
        return 0
    print(outstanding)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
