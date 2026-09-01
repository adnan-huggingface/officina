"""Runs a cargo subcommand, finding cargo the way a hook has to.

A hook does not inherit the shell a person logs in with, and on this machine
cargo lives where only the profile puts it on the path.
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def main() -> int:
    binary = shutil.which("cargo")
    if binary is None:
        for candidate in (Path.home() / ".cargo" / "bin" / "cargo.exe",
                          Path.home() / ".cargo" / "bin" / "cargo"):
            if candidate.exists():
                binary = str(candidate)
                break
    if binary is None:
        print("cargo is not installed, or is not where this can find it")
        return 0
    env = dict(os.environ)
    env["PATH"] = str(Path(binary).parent) + os.pathsep + env.get("PATH", "")
    done = subprocess.run([binary, *sys.argv[1:]], cwd=ROOT, text=True,
                          capture_output=True, timeout=7200, env=env)
    sys.stdout.write(done.stdout[-4000:])
    sys.stderr.write(done.stderr[-4000:])
    return done.returncode


if __name__ == "__main__":
    raise SystemExit(main())
