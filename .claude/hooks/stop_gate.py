"""Refuses to let the session end while the gate says the work is not done.

**This is the piece that removes the agent's discretion about "finished".** A
`Stop` hook runs when the assistant tries to end its turn; exiting 2 refuses,
and whatever it wrote to stderr is handed back as the reason. So the definition
of done in `gate.py` decides, and the model does not.

Exit 2 rather than the JSON `decision` shape, deliberately: the exit code and
stderr are the oldest and least ambiguous half of the hook contract, and this
has to keep working across versions without anybody noticing that it quietly
stopped.

**The guard matters as much as the block.** `stop_hook_active` is true when a
previous refusal is already in force, and honouring it is what stops a gate that
can never pass from becoming an endless loop that spends the budget on nothing.
A wall this hook cannot get past has to reach a person.

Wire it up in `.claude/settings.json`:

    {
      "hooks": {
        "Stop": [
          {
            "matcher": "*",
            "hooks": [
              {
                "type": "command",
                "command": "python .claude/hooks/stop_gate.py",
                "timeout": 3600
              }
            ]
          }
        ]
      }
    }

Off by default, and it should be: a hook that refuses to let a session end
changes every session in the project, including the ones that were only ever
going to be a question.
"""

import json
import sys

sys.path.insert(0, str(__import__("pathlib").Path(__file__).resolve().parent))
import gate  # noqa: E402


def main():
    raw = sys.stdin.read()
    try:
        event = json.loads(raw) if raw.strip() else {}
    except json.JSONDecodeError:
        # An unreadable event is not grounds for holding somebody's session
        # open. Let it end and say nothing.
        return 0

    # Already refused once. Whatever is outstanding is outstanding for a reason
    # this hook cannot fix by asking again.
    if event.get("stop_hook_active"):
        return 0

    try:
        outstanding = gate.report()
    except gate.Unrunnable as why:
        # Say it once, and let the session end. A gate that cannot be asked is
        # not a gate that says no.
        print(f"the completion gate could not run: {why}", file=sys.stderr)
        return 0
    if not outstanding:
        return 0

    print(
        "The work is not finished. `python .claude/hooks/gate.py` says:\n\n"
        f"{outstanding}\n\n"
        "Carry on until it exits zero. If something in it cannot be made to "
        "pass, say which and why rather than working around it.",
        file=sys.stderr,
    )
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
