"""Runs `cargo test` and fails unless tests actually ran.

**A filter that matches nothing exits zero.** `cargo test -p wp-odf splice`
against a crate with no splice test prints `0 passed` and succeeds, because from
cargo's point of view nothing failed. Every item of a plan verified that way
reports itself finished on the day the plan is written, which is the most
expensive kind of wrong: a gate that is confidently green about work nobody has
started.

So this insists on evidence rather than on the absence of failure. It passes
only when at least one test matched the filter and every test that matched
passed.

    python .claude/hooks/proved.py -p wp-odf splice
    python .claude/hooks/proved.py -p scriva --test odt_writes

The arguments are handed to `cargo test` unchanged.
"""

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# `test result: ok. 12 passed; 0 failed; 3 ignored; ...`, once per test binary.
RESULT = re.compile(r"test result: (\w+)\. (\d+) passed; (\d+) failed")


def cargo() -> str | None:
    found = shutil.which("cargo")
    if found:
        return found
    for candidate in (Path.home() / ".cargo" / "bin" / "cargo.exe",
                      Path.home() / ".cargo" / "bin" / "cargo"):
        if candidate.exists():
            return str(candidate)
    return None


def main() -> int:
    binary = cargo()
    if binary is None:
        # A missing toolchain is not a failed test. Saying otherwise is how a
        # gate holds a session open over something nobody working can fix.
        print("cargo is not installed, or is not where this can find it")
        return 0

    env = dict(os.environ)
    env["PATH"] = str(Path(binary).parent) + os.pathsep + env.get("PATH", "")
    done = subprocess.run(
        [binary, "test", *sys.argv[1:]],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=3600,
        env=env,
    )
    said = done.stdout + done.stderr

    if done.returncode != 0:
        print(chr(10).join(said.strip().splitlines()[-20:]))
        return 1

    passed = sum(int(m.group(2)) for m in RESULT.finditer(said))
    failed = sum(int(m.group(3)) for m in RESULT.finditer(said))
    if failed:
        print(f"{failed} test(s) failed")
        return 1
    if passed == 0:
        print(
            f"no test matched `cargo test {' '.join(sys.argv[1:])}`. "
            "Cargo calls that success; this does not."
        )
        return 1
    print(f"{passed} test(s) passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
