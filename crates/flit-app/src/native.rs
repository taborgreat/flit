//! Showing a hidden window from another thread. A hidden window gets no
//! redraw events, so nothing egui-side can run to un-hide it; the OS call
//! makes it visible again, after which the normal repaint path works.

use std::sync::atomic::{AtomicIsize, Ordering};

/// The native handle of the app window (0 until known).
pub static HWND: AtomicIsize = AtomicIsize::new(0);

pub fn remember(handle: Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError>) {
    #[allow(unused_variables)]
    if let Ok(h) = handle {
        #[cfg(windows)]
        if let raw_window_handle::RawWindowHandle::Win32(w) = h.as_raw() {
            HWND.store(w.hwnd.get(), Ordering::SeqCst);
        }
    }
}

/// Bring the window back (restore if minimized, show if hidden, foreground).
pub fn show() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW};
        let hwnd = HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
            // SAFETY: plain Win32 calls on a window handle this process owns.
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            }
        }
    }
}
