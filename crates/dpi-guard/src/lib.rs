//! Keeping Windows from fighting the user over a dragged window.
//!
//! On a desktop with two monitors at different scales, Windows tells a window
//! it has crossed over by sending `WM_DPICHANGED` — *during* the drag, from
//! inside the modal move loop. winit answers by resizing the window to keep
//! its logical size, and the resize can shove the window's majority back onto
//! the monitor it came from, which sends the message again, which resizes
//! again. The window shudders between two sizes at the border and the drag
//! cannot get through it. Worse, each round trip converts physical to logical
//! and back with a scale that has already moved on, so the window comes out
//! the far side *smaller than it went in* — measured here: 900×600 becomes
//! 625×417 in one crossing.
//!
//! The tactic is the one MSDN itself implies: do not resize in the middle of
//! a drag. A subclass installed ahead of winit's window procedure swallows
//! `WM_DPICHANGED` while the window is being dragged, and when the drag ends
//! it sends one fresh `WM_DPICHANGED` for the monitor the window actually
//! landed on. One message, one resize, after the user's hand is off the
//! window.
//!
//! This crate exists so `ui-kit` can keep its `forbid(unsafe_code)`: talking
//! to a window procedure is unsafe by nature, and all of it is fenced in
//! here. Everything that can be a decision rather than a syscall is in
//! [`Guard`], which is plain arithmetic and has tests.

use raw_window_handle::HasWindowHandle;

/// What to do with a message, decided by [`Guard`] so it can be tested
/// without a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Hand the message on to winit.
    Forward,
    /// The message is dealt with; winit never sees it.
    Swallow,
    /// The drag ended with a swallowed change pending: forward this message,
    /// then tell the window about the monitor it is on now.
    Settle,
}

/// The decision state, pure so the tests can drive it.
///
/// `dragging` mirrors `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE`. `known` is the
/// last DPI winit was actually told about — not what Windows believes, which
/// runs ahead of it while changes are being swallowed. The tick ring is a
/// storm damper for the one case deferral cannot reach: a window *dropped*
/// straddling the border, where a lone resize can flip the majority monitor
/// and ring the bell again with nobody touching the window.
#[derive(Debug)]
pub struct Guard {
    dragging: bool,
    deferred: bool,
    known: u32,
    recent: [u32; 4],
    at: usize,
}

/// Pass-throughs closer together than this, four in a row, are a feedback
/// loop rather than a user rearranging their monitors.
const STORM_MS: u32 = 500;

impl Guard {
    pub fn new(dpi: u32) -> Guard {
        Guard {
            dragging: false,
            deferred: false,
            known: dpi,
            recent: [u32::MAX - STORM_MS; 4],
            at: 0,
        }
    }

    pub fn drag_started(&mut self) {
        self.dragging = true;
    }

    /// Whether a drag is in progress, for the messages whose answer depends
    /// on it.
    pub fn dragging(&self) -> bool {
        self.dragging
    }

    /// The drag ended. Says whether a deferred change needs settling.
    pub fn drag_ended(&mut self) -> Action {
        self.dragging = false;
        if std::mem::take(&mut self.deferred) {
            Action::Settle
        } else {
            Action::Forward
        }
    }

    /// A `WM_DPICHANGED` arrived, `now` being a millisecond tick.
    pub fn dpi_changed(&mut self, dpi: u32, now: u32) -> Action {
        if self.dragging {
            self.deferred = true;
            return Action::Swallow;
        }
        // Four pass-throughs inside the window is the border flip-flop:
        // letting the fifth through keeps the loop alive forever.
        if now.wrapping_sub(self.recent[self.at]) < STORM_MS {
            return Action::Swallow;
        }
        self.recent[self.at] = now;
        self.at = (self.at + 1) % self.recent.len();
        self.known = dpi;
        Action::Forward
    }

    /// The DPI winit currently believes, for deciding whether settling is
    /// even needed.
    pub fn known(&self) -> u32 {
        self.known
    }
}

/// Installs the guard on `window`. A no-op anywhere but Windows.
pub fn install(window: &impl HasWindowHandle) {
    #[cfg(windows)]
    win::install(window);
    #[cfg(not(windows))]
    let _ = window;
}

#[cfg(windows)]
mod win {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SendMessageW, WM_DPICHANGED, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE,
        WM_GETDPISCALEDSIZE, WM_NCDESTROY,
    };

    use super::{Action, Guard};

    /// Any value distinguishes this subclass from others on the same window.
    const SUBCLASS_ID: usize = 0x5C71;

    pub fn install(window: &impl HasWindowHandle) {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return;
        };
        let hwnd = win32.hwnd.get() as HWND;
        // SAFETY: the hwnd came from the live window handle above; the guard
        // box is owned by the subclass from here and freed on WM_NCDESTROY.
        unsafe {
            let guard = Box::into_raw(Box::new(Guard::new(GetDpiForWindow(hwnd))));
            SetWindowSubclass(hwnd, Some(proc), SUBCLASS_ID, guard as usize);
        }
    }

    /// Runs ahead of winit's window procedure, including inside the modal
    /// move loop, which is the whole point: `WM_DPICHANGED` is *sent* there,
    /// not posted, so no message-pump hook ever sees it.
    unsafe extern "system" fn proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        data: usize,
    ) -> LRESULT {
        let guard = &mut *(data as *mut Guard);
        match msg {
            WM_ENTERSIZEMOVE => guard.drag_started(),
            WM_EXITSIZEMOVE => {
                if guard.drag_ended() == Action::Settle {
                    // Let winit take its in-size-move marker down first, so
                    // the resize below is handled as an ordinary one and not
                    // biased around a cursor that is no longer dragging.
                    let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                    settle(hwnd, guard);
                    return result;
                }
            }
            WM_DPICHANGED => {
                let dpi = (wparam & 0xFFFF) as u32;
                if guard.dpi_changed(dpi, GetTickCount()) == Action::Swallow {
                    return 0;
                }
            }
            WM_GETDPISCALEDSIZE if guard.dragging() => {
                // Windows asks what size the window wants at the new DPI and,
                // by default, linear-scales it *itself* as part of the drag.
                // With the `WM_DPICHANGED` after it swallowed, that default
                // resize would stand untold — winit would still think in the
                // old scale, and the settling resize at the end of the drag
                // would scale a window that had already been scaled. Answer:
                // this size, no change. The one resize happens at the drop.
                //
                // The size Windows applies here is the *window's*, not the
                // client area's — answering with the client size shrinks the
                // window by its own decorations at every crossing.
                let size = &mut *(lparam as *mut SIZE);
                let mut outer = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                if GetWindowRect(hwnd, &mut outer) != 0 {
                    size.cx = outer.right - outer.left;
                    size.cy = outer.bottom - outer.top;
                    return 1;
                }
            }
            WM_NCDESTROY => {
                let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                RemoveWindowSubclass(hwnd, Some(proc), SUBCLASS_ID);
                drop(Box::from_raw(data as *mut Guard));
                return result;
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }

    /// Tells the window about the monitor it landed on, once, after a drag
    /// during which changes were swallowed.
    unsafe fn settle(hwnd: HWND, guard: &Guard) {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let (mut x, mut y) = (0u32, 0u32);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) != 0 || x == 0 {
            return;
        }
        // Ended back where it started: there is nothing to tell.
        if x == guard.known() {
            return;
        }
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return;
        }
        // The suggested rect is the window where it is: winit takes the
        // position from it and computes the size itself. Sent, not posted,
        // so the rect on this stack stays alive for the duration.
        let wparam = ((y as usize) << 16) | x as usize;
        SendMessageW(hwnd, WM_DPICHANGED, wparam, &rect as *const RECT as LPARAM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_mid_drag_is_swallowed_and_settled_at_the_end() {
        let mut guard = Guard::new(120);
        guard.drag_started();
        // The border flip-flop: each crossing rings the bell, none get through.
        assert_eq!(guard.dpi_changed(144, 1000), Action::Swallow);
        assert_eq!(guard.dpi_changed(120, 1016), Action::Swallow);
        assert_eq!(guard.dpi_changed(144, 1031), Action::Swallow);
        assert_eq!(guard.drag_ended(), Action::Settle);
        // The settling message itself then passes through.
        assert_eq!(guard.dpi_changed(144, 1200), Action::Forward);
        assert_eq!(guard.known(), 144);
    }

    #[test]
    fn a_drag_that_never_crossed_has_nothing_to_settle() {
        let mut guard = Guard::new(120);
        guard.drag_started();
        assert_eq!(guard.drag_ended(), Action::Forward);
    }

    #[test]
    fn an_ordinary_scale_change_passes_straight_through() {
        // The user changed display scaling in Settings: no drag, no storm.
        let mut guard = Guard::new(96);
        assert_eq!(guard.dpi_changed(120, 5000), Action::Forward);
        assert_eq!(guard.known(), 120);
    }

    #[test]
    fn a_storm_with_nobody_dragging_is_cut_off() {
        // Dropped straddling the border: the lone resize flips the majority
        // monitor, which asks for another resize, forever. Four get through
        // and the fifth is refused, which breaks the cycle.
        let mut guard = Guard::new(120);
        let mut verdicts = Vec::new();
        for i in 0..6u32 {
            let dpi = if i % 2 == 0 { 144 } else { 120 };
            verdicts.push(guard.dpi_changed(dpi, 2000 + i * 30));
        }
        assert_eq!(
            verdicts,
            [
                Action::Forward,
                Action::Forward,
                Action::Forward,
                Action::Forward,
                Action::Swallow,
                Action::Swallow,
            ]
        );
        // Once the dust settles the guard listens again.
        assert_eq!(guard.dpi_changed(144, 9000), Action::Forward);
    }
}
