"""Makes an OpenDocument document in Scriva the way a person does: by keystroke.

Everything below the menus has tests. The path *from* a menu — from a keystroke
to the model to the file — has none, and ADR 0002 is the record of what lives
there: one afternoon of driving the real binaries found a crash and two silent
data losses that 1,350 passing tests never came near. The `.odt` writer was
built and merged without anybody once saving one through the application, so
this is that save, made by a driver rather than by a person so that it has an
exit code and cannot be dropped in silence again.

    python tools/drive/scriva_odt.py --out target/drive

It launches the built Scriva, makes a document through File ▸ New, types it,
formats it through the Format menu, saves it through the real Save As dialog as
`recreated.odt`, reopens it through the real Open dialog, and reads the window
back at every step. `.claude/hooks/drove_it.py` runs it and then asks its
leavings whether it did anything.

**ADR 0002's rules for a driver, and how each is kept here.**

*Find the target by process name, never by a window-title substring.* This is
stricter than that: the process is launched here, so every window is checked
against that process's own id, and the title is read only for what it says
about the document.

*Check before every input that the foreground window belongs to the target, and
abort rather than type into another window.* `press` and `write` both go
through `guard`, which raises rather than sends. A driver that types half a
document into a terminal is not a failed test, it is an incident.

*Trust nothing about coordinates.* There are none: not one click is sent. Every
command is reached by Alt and the underlined letter, which is both what the menu
bar is built for and the only way of reaching a menu that survives the window
being any size on either of two monitors at different scales. The screenshots
are still taken per-monitor-aware, because a rectangle Windows reports for a
window on the 150% screen is virtualised down to the primary's scale otherwise,
and the shot would be a crop of the corner rather than the window.

**What the screenshots are for.** They are read back, not merely written: a
window that never painted, a menu that did not open and a page with no ink on it
are all detected here rather than left for a person to notice. What they cannot
do is judge whether the page is *right*, which is what the fidelity harness and
`cargo xtask compare` are for. A person looking at them once is cheap, and they
are left in the output directory for exactly that.
"""

from __future__ import annotations

import argparse
import ctypes
import os
import subprocess
import sys
import time
import zipfile
from ctypes import wintypes
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# The sentence the document is built around, so that what lands on disk can be
# tied to the run that made it rather than to any `.odt` lying about.
SENTINEL = "SCRIVA ODT DRIVE"

# What goes in the header, which is the part of the document that is written
# into `styles.xml` rather than into `content.xml`.
HEADING = "DRIVEN HEADER"

# What the document is saved as, and what `drove_it.py` looks for.
SAVED_AS = "recreated.odt"


class Wall(Exception):
    """Something the driver could not get past.

    Named for what ADR 0002 §5 calls it: a wall is either fixed in the same
    sitting or written down as a limitation, and either way it is reported
    rather than worked around.
    """


# --------------------------------------------------------------- Windows

user32 = ctypes.WinDLL("user32", use_last_error=True)
gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

# Per-monitor v2. Without it Windows virtualises every rectangle it reports for
# a window on the 150% screen down to the primary's 125%, and a capture of the
# window is silently a crop of its top-left corner.
PER_MONITOR_AWARE_V2 = ctypes.c_void_p(-4)

VK = {
    "return": 0x0D,
    "escape": 0x1B,
    "space": 0x20,
    "end": 0x23,
    "home": 0x24,
    "left": 0x25,
    "up": 0x26,
    "right": 0x27,
    "down": 0x28,
    "delete": 0x2E,
    "f4": 0x73,
    "menu": 0x12,  # Alt
    "control": 0x11,
    "shift": 0x10,
}

KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_UNICODE = 0x0004
INPUT_KEYBOARD = 1
SRCCOPY = 0x00CC0020


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [
        ("dx", wintypes.LONG),
        ("dy", wintypes.LONG),
        ("mouseData", wintypes.DWORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [
        ("wVk", wintypes.WORD),
        ("wScan", wintypes.WORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]


class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [
        ("uMsg", wintypes.DWORD),
        ("wParamL", wintypes.WORD),
        ("wParamH", wintypes.WORD),
    ]


class _INPUTUNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT), ("hi", HARDWAREINPUT)]


class INPUT(ctypes.Structure):
    _anonymous_ = ("u",)
    _fields_ = [("type", wintypes.DWORD), ("u", _INPUTUNION)]


def send(*events: INPUT) -> None:
    array = (INPUT * len(events))(*events)
    sent = user32.SendInput(len(events), array, ctypes.sizeof(INPUT))
    if sent != len(events):
        raise Wall(f"SendInput sent {sent} of {len(events)} events")


def key_event(vk: int, up: bool) -> INPUT:
    event = INPUT()
    event.type = INPUT_KEYBOARD
    event.ki = KEYBDINPUT(vk, 0, KEYEVENTF_KEYUP if up else 0, 0, None)
    return event


def char_event(ch: str, up: bool) -> INPUT:
    """A character as itself rather than as a key on some particular layout.

    `KEYEVENTF_UNICODE` is the only way to type a path that is sure of what it
    is typing: a virtual-key code means a different character on a keyboard laid
    out differently, and this has to be able to write a colon and a backslash.
    """
    event = INPUT()
    event.type = INPUT_KEYBOARD
    flags = KEYEVENTF_UNICODE | (KEYEVENTF_KEYUP if up else 0)
    event.ki = KEYBDINPUT(0, ord(ch), flags, 0, None)
    return event


def foreground_pid() -> int:
    hwnd = user32.GetForegroundWindow()
    if not hwnd:
        return 0
    pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return pid.value


def windows_of(pid: int) -> list[int]:
    """Every visible top-level window the process owns.

    A native dialog is a window of the same process rather than the same window,
    which is why this is a list and why the foreground check asks about the
    process rather than about one handle.
    """
    found: list[int] = []
    proc = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    def visit(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(hwnd):
            found.append(hwnd)
        return True

    user32.EnumWindows(proc(visit), 0)
    return found


def title_of(hwnd: int) -> str:
    length = user32.GetWindowTextLengthW(hwnd)
    buf = ctypes.create_unicode_buffer(length + 1)
    user32.GetWindowTextW(hwnd, buf, length + 1)
    return buf.value


def window_rect(hwnd: int) -> tuple[int, int, int, int]:
    rect = wintypes.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    return rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top


class Scriva:
    """The running application, and the only thing allowed to be typed into."""

    def __init__(self, exe: Path, out: Path):
        self.out = out
        self.shots = 0
        self.process = subprocess.Popen([str(exe)], cwd=str(ROOT))
        self.pid = self.process.pid
        self.hwnd = self.wait_for_window()
        self.raise_it()

    # ------------------------------------------------------------ the window

    def wait_for_window(self, seconds: float = 60.0) -> int:
        """The main window, waited for by process id and by nothing else."""
        deadline = time.time() + seconds
        while time.time() < deadline:
            if self.process.poll() is not None:
                raise Wall(f"Scriva exited with {self.process.returncode} before it drew anything")
            for hwnd in windows_of(self.pid):
                if title_of(hwnd):
                    return hwnd
            time.sleep(0.2)
        raise Wall("Scriva drew no window within a minute")

    def raise_it(self) -> None:
        """Puts the application in front, and refuses to go on if it will not.

        Windows will not let a process it does not consider foreground steal the
        focus outright, so this asks the way that works — restore, then set —
        and then checks rather than assumes.
        """
        ours = kernel32.GetCurrentThreadId()
        for attempt in range(40):
            user32.ShowWindow(self.hwnd, 9)  # SW_RESTORE
            # A process Windows does not consider foreground cannot simply take
            # the focus, and this one is a script started from a terminal. The
            # sanctioned way round it is to share an input queue with whoever
            # has the focus for the moment it takes to hand it over.
            theirs = user32.GetWindowThreadProcessId(user32.GetForegroundWindow(), None)
            attached = theirs and user32.AttachThreadInput(ours, theirs, True)
            user32.SetForegroundWindow(self.hwnd)
            user32.BringWindowToTop(self.hwnd)
            if attached:
                user32.AttachThreadInput(ours, theirs, False)
            time.sleep(0.15 if attempt < 10 else 0.5)
            if foreground_pid() == self.pid:
                return
        raise Wall("Scriva would not come to the foreground")

    def guard(self, what: str) -> None:
        """The check ADR 0002 asks for before *every* input.

        Not "is Scriva running" but "is what I am about to type going to
        Scriva": the afternoon that produced the rule ended with half a resume
        typed into a terminal whose title happened to match.
        """
        if self.process.poll() is not None:
            raise Wall(f"Scriva has exited ({self.process.returncode}); {what} was not sent")
        pid = foreground_pid()
        if pid != self.pid:
            raise Wall(
                f"the foreground window belongs to process {pid}, not to Scriva ({self.pid}); "
                f"{what} was not sent"
            )

    def title(self) -> str:
        """What the title bar says, which is where this reads the document from.

        `ui_kit::shell` writes `• name — Scriva` while there are unsaved changes
        and `name — Scriva` once there are not, so the title answers both "did
        the save happen" and "which document is open" without a coordinate.
        """
        for hwnd in windows_of(self.pid):
            text = title_of(hwnd)
            if text.endswith("Scriva"):
                self.hwnd = hwnd
                return text
        return ""

    def await_title(self, predicate, what: str, seconds: float = 20.0) -> str:
        deadline = time.time() + seconds
        while time.time() < deadline:
            title = self.title()
            if predicate(title):
                return title
            time.sleep(0.2)
        raise Wall(f"{what}; the title bar still says {self.title()!r}")

    # ------------------------------------------------------------- the input

    def press(self, *keys: str, hold: tuple[str, ...] = (), pause: float = 0.25) -> None:
        self.guard(f"{'+'.join([*hold, *keys])}")
        events = [key_event(VK[k], False) for k in hold]
        for key in keys:
            code = VK.get(key)
            if code is None:
                code = ord(key.upper())
            events.append(key_event(code, False))
            events.append(key_event(code, True))
        events.extend(key_event(VK[k], True) for k in reversed(hold))
        send(*events)
        time.sleep(pause)

    def write(self, text: str, pause: float = 0.4) -> None:
        self.guard(f"typing {text!r}")
        # In small batches, and checked between them: a long string sent as one
        # SendInput call is a long string typed into whatever the foreground
        # window becomes halfway through it.
        for start in range(0, len(text), 16):
            chunk = text[start : start + 16]
            self.guard(f"typing {chunk!r}")
            events = []
            for ch in chunk:
                events.append(char_event(ch, False))
                events.append(char_event(ch, True))
            send(*events)
            time.sleep(0.05)
        time.sleep(pause)

    def menu(self, top: str, *items: str) -> None:
        """A command reached the way the menu bar is meant to be reached.

        Alt and the underlined letter opens the menu; inside an open menu the
        bare letter runs the item. Both are `ui_kit::menu`'s own doing, and
        driving them is the only part of this that tests what a person's hands
        would touch.

        The menu is *seen* to open before its item is chosen, because the cost
        of assuming is not a failed step: a letter meant for a menu that never
        opened is a letter typed into the document, and the run would go on
        looking well until something read what it wrote.
        """
        for _ in range(3):
            before = self.look()
            self.press(top, hold=("menu",), pause=0.4)
            # Waited for rather than looked for once: the window paints when it
            # is asked to and a screenshot taken between the keystroke and the
            # frame shows the menu that has not opened yet.
            for _ in range(8):
                if differs(before, self.look()) >= 0.004:
                    for item in items:
                        self.press(item, pause=0.5)
                    return
                time.sleep(0.3)
            # Escape first, in case a menu did open and this could not see it:
            # a second Alt on an open menu would close it rather than open it.
            self.press("escape", pause=0.4)
        self.shot(f"no-{top}-menu")
        raise Wall(f"Alt+{top.upper()} opened no menu")

    # ------------------------------------------------------- what it looks like

    def look(self):
        """The window as it is on the screen, read back rather than filed away.

        From the screen rather than from the window itself: a `PrintWindow` of a
        surface a GPU composited comes back black, and every check below is
        about what was actually painted.
        """
        from PIL import Image

        self.guard("a look at the window")
        left, top, width, height = window_rect(self.hwnd)
        if width <= 0 or height <= 0:
            raise Wall(f"the window measures {width}x{height}")
        screen = user32.GetDC(0)
        memory = gdi32.CreateCompatibleDC(screen)
        bitmap = gdi32.CreateCompatibleBitmap(screen, width, height)
        gdi32.SelectObject(memory, bitmap)
        gdi32.BitBlt(memory, 0, 0, width, height, screen, left, top, SRCCOPY)

        class BITMAPINFOHEADER(ctypes.Structure):
            _fields_ = [
                ("biSize", wintypes.DWORD),
                ("biWidth", wintypes.LONG),
                ("biHeight", wintypes.LONG),
                ("biPlanes", wintypes.WORD),
                ("biBitCount", wintypes.WORD),
                ("biCompression", wintypes.DWORD),
                ("biSizeImage", wintypes.DWORD),
                ("biXPelsPerMeter", wintypes.LONG),
                ("biYPelsPerMeter", wintypes.LONG),
                ("biClrUsed", wintypes.DWORD),
                ("biClrImportant", wintypes.DWORD),
            ]

        header = BITMAPINFOHEADER()
        header.biSize = ctypes.sizeof(BITMAPINFOHEADER)
        header.biWidth = width
        # Negative, so the rows arrive the way an image is written rather than
        # bottom upwards, which is how a device-independent bitmap is stored.
        header.biHeight = -height
        header.biPlanes = 1
        header.biBitCount = 32
        buffer = ctypes.create_string_buffer(width * height * 4)
        gdi32.GetDIBits(memory, bitmap, 0, height, buffer, ctypes.byref(header), 0)
        gdi32.DeleteObject(bitmap)
        gdi32.DeleteDC(memory)
        user32.ReleaseDC(0, screen)

        return Image.frombuffer("RGB", (width, height), buffer, "raw", "BGRX", 0, 1)

    def shot(self, name: str):
        """A look that is kept, so that a person can spend a minute on the run.

        What is checked here is coarse — a window that painted, a page with ink
        on it — and deliberately so. Whether the page is *right* is a question
        for `cargo xtask compare`, and whether it reads right is a question for
        eyes on these files.
        """
        image = self.look()
        self.shots += 1
        image.save(self.out / f"{self.shots:02d}-{name}.png")
        return image

    def close(self) -> None:
        if self.process.poll() is None:
            try:
                self.press("f4", hold=("menu",), pause=1.5)
            except Wall:
                pass
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()


# ------------------------------------------------------- reading a screenshot


def ink(image, band: tuple[float, float] = (0.25, 0.95)) -> int:
    """How many dark pixels there are down the middle of the page.

    A crude question deliberately: it separates a window that has words on its
    page from one that has none, which is the failure this cannot afford to miss
    — a driver that typed into nothing would otherwise save an empty document
    and call it a run. Whether the words are the *right* words is what reading
    the saved file back is for.
    """
    width, height = image.size
    top = int(height * band[0])
    bottom = int(height * band[1])
    page = image.crop((int(width * 0.2), top, int(width * 0.8), bottom))
    small = page.convert("L")
    return sum(1 for value in small.tobytes() if value < 120)


def differs(before, after) -> float:
    """The fraction of the window that changed between two shots."""
    if before.size != after.size:
        return 1.0
    a = before.convert("L").tobytes()
    b = after.convert("L").tobytes()
    moved = sum(1 for x, y in zip(a, b) if abs(x - y) > 24)
    return moved / max(1, len(a))


# ------------------------------------------------------------------ the drive


def dialog_of(app: Scriva, seconds: float = 30.0) -> int:
    """Waits for a native dialog: the window of this process that has the focus.

    Found by enumeration rather than by faith, as ADR 0002 asks — one of these
    once opened entirely off-screen, and a driver that assumed where it was
    would have typed a file name into the document.

    *The focus* rather than merely another window, and that is not fastidiousness:
    winit keeps a window called `Winit Thread Event Target` which is a top-level
    window of the process, reports itself visible, has no title and never
    closes. The first version of this took it for the dialog, went on to type a
    file path before the dialog had opened — so it went into the document — and
    then waited a minute for a window that will outlive the application to shut.
    Waiting for the focus to move is waiting for the thing that actually
    happened.
    """
    deadline = time.time() + seconds
    while time.time() < deadline:
        hwnd = user32.GetForegroundWindow()
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        name = title_of(hwnd)
        if owner.value == app.pid and hwnd != app.hwnd and name and not name.endswith("Scriva"):
            return hwnd
        time.sleep(0.2)
    raise Wall("no dialog opened")


def gone(hwnd: int, seconds: float) -> bool:
    deadline = time.time() + seconds
    while time.time() < deadline:
        if not user32.IsWindow(hwnd) or not user32.IsWindowVisible(hwnd):
            return True
        time.sleep(0.2)
    return False


def name_the_file(app: Scriva, hwnd: int, path: Path) -> None:
    """Types a path into a file dialog and accepts it.

    Accepted more than once if it has to be, which is what a person does and
    what the dialog sometimes needs: a name that matches a file already in the
    folder drops an autocompletion list under the box, and the first Return
    goes to the list rather than to the dialog. Silence would be worse than a
    second Return — the first version of this waited a minute for a dialog that
    was waiting for it.
    """
    app.guard(f"the name {path}")
    app.write(str(path), pause=0.6)
    for _ in range(3):
        app.press("return", pause=1.0)
        if gone(hwnd, 8.0):
            return
    raise Wall("the dialog is still open after the name was typed into it")


def drive(exe: Path, out: Path) -> Path:
    app = Scriva(exe, out)
    saved = out / SAVED_AS
    try:
        opened = app.shot("launched")
        if len(set(opened.convert("L").tobytes())) < 8:
            raise Wall("the window is one flat colour, so it never painted")

        # A document of its own, made the way the first thing a person does is
        # made — through the menu rather than through the shortcut beside it.
        app.menu("f", "n")
        app.await_title(lambda t: t.startswith("Document"), "File ▸ New gave no new document")

        app.write(SENTINEL)
        app.press("return")
        app.write("This document was made through the menus and the keyboard, ")
        # Bold through the Format menu, then off again through the shortcut: two
        # ways to the same command, and the empty-paragraph case that ADR 0002
        # found silently doing nothing.
        app.menu("o", "b")
        app.write("including this")
        app.press("b", hold=("control",))
        app.write(", and saved as OpenDocument text.")

        # A header, because a header is a first-week thing to want and because
        # it is written into the other part of the package: a save that keeps
        # the body and loses the header would look entirely successful from the
        # page the caret is on.
        app.menu("i", "h", "e")
        app.write(HEADING)
        app.press("escape", pause=0.6)

        typed = app.shot("typed")
        if ink(typed) < 200:
            raise Wall("nothing was typed onto the page")
        app.await_title(lambda t: t.startswith("•"), "typing did not make the document dirty")

        # Save As, through the application's own dialog.
        app.menu("f", "a")
        name_the_file(app, dialog_of(app), saved)
        app.await_title(
            lambda t: t.startswith(SAVED_AS),
            "the document was not saved under the name it was given",
            seconds=60,
        )
        app.shot("saved")
        if not saved.exists():
            raise Wall(f"{saved} is not there after a save that reported success")

        # And opened again, which is the half of a save that proves it: a file
        # that cannot be read back is not a document.
        app.menu("f", "n")
        app.await_title(lambda t: t.startswith("Document"), "File ▸ New gave no new document")
        app.menu("f", "o")
        name_the_file(app, dialog_of(app), saved)
        app.await_title(
            lambda t: t.startswith(SAVED_AS),
            "the saved document did not open again",
            seconds=60,
        )
        reopened = app.shot("reopened")
        if ink(reopened) < 200:
            raise Wall("the document opened with nothing on its page")
    finally:
        app.close()
    return saved


def check(saved: Path) -> None:
    """What is on disk, read by something that is not Scriva."""
    with zipfile.ZipFile(saved) as package:
        names = package.namelist()
        if names[0] != "mimetype":
            raise Wall("mimetype is not the first entry, so this is not an ODF package")
        body = package.read("content.xml").decode("utf-8", "replace")
        bands = package.read("styles.xml").decode("utf-8", "replace")
    if HEADING not in bands:
        raise Wall(f"styles.xml does not hold the header, {HEADING!r}")
    if SENTINEL not in body:
        raise Wall(f"content.xml does not hold {SENTINEL!r}")
    if 'fo:font-weight="bold"' not in body:
        # The one thing here that was made through a menu rather than typed. A
        # save that keeps the words and loses the formatting is exactly the
        # silent loss ADR 0002 was written about, and it looks like a success
        # from every other angle.
        raise Wall("what was made bold through the Format menu is not bold in the file")


def build(exe: Path) -> Path:
    """Builds Scriva every time, rather than driving whatever is lying there.

    An hour was once spent on a feature that "completely failed" in the running
    application and worked in every test, and the answer was a six-day-old
    binary. A build that is already up to date costs a second; driving a stale
    one costs an afternoon and reports the wrong thing about the code.
    """
    cargo = "cargo"
    home = Path.home() / ".cargo" / "bin" / "cargo.exe"
    if home.exists():
        cargo = str(home)
    done = subprocess.run([cargo, "build", "--release", "-p", "scriva"], cwd=str(ROOT))
    if done.returncode != 0 or not exe.exists():
        raise Wall("Scriva could not be built")
    return exe


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, help="where the screenshots and the document go")
    parser.add_argument(
        "--exe",
        default=str(ROOT / "target" / "release" / "scriva.exe"),
        help="the Scriva to drive; built if it is not there",
    )
    args = parser.parse_args()

    if os.name != "nt":
        print("this drives Windows; there is nothing here to drive")
        return 1

    user32.SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)

    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    saved = out / SAVED_AS
    if saved.exists():
        saved.unlink()

    try:
        exe = build(Path(args.exe).resolve())
        saved = drive(exe, out)
        check(saved)
    except Wall as wall:
        print(f"the drive hit a wall: {wall}")
        return 1
    print(f"drove Scriva through New, typing, Save As and Open; {saved.name} holds what was typed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
