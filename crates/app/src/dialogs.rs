//! Keep the system file dialogs centered over the app window.
//!
//! On Windows the common file dialog opens near the owner window's top-left
//! corner (or wherever it was last), so the UI calls [`center_next_dialog`]
//! right before opening one: a short-lived thread waits for the dialog to
//! appear and moves it to the middle of its owner. macOS shows the dialogs as
//! sheets attached to the window, which needs nothing.

#[tauri::command]
pub fn center_next_dialog() {
    #[cfg(windows)]
    win::center_next();
}

#[cfg(windows)]
mod win {
    use std::time::{Duration, Instant};

    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindow, GetWindowRect, GetWindowThreadProcessId,
        IsWindowVisible, SetWindowPos, GW_OWNER, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    };

    /// Give up when no dialog shows up within this time.
    const WAIT: Duration = Duration::from_secs(5);
    const POLL: Duration = Duration::from_millis(8);

    struct Search {
        pid: u32,
        found: HWND,
    }

    /// A visible dialog box (`#32770`) of this process that has an owner.
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam as *mut Search);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != search.pid || IsWindowVisible(hwnd) == 0 || GetWindow(hwnd, GW_OWNER).is_null() {
            return 1;
        }
        let mut class = [0u16; 16];
        let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        if String::from_utf16_lossy(&class[..len]) != "#32770" {
            return 1;
        }
        search.found = hwnd;
        0
    }

    pub(super) fn center_next() {
        std::thread::spawn(|| {
            let deadline = Instant::now() + WAIT;
            while Instant::now() < deadline {
                let mut search = Search {
                    pid: std::process::id(),
                    found: std::ptr::null_mut(),
                };
                // SAFETY: `search` outlives the call; the callback only
                // touches it through `lparam`.
                unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
                if !search.found.is_null() {
                    // SAFETY: plain Win32 calls on a window handle; a handle
                    // that died in between makes them fail harmlessly.
                    unsafe { center(search.found) };
                    return;
                }
                std::thread::sleep(POLL);
            }
        });
    }

    unsafe fn center(dialog: HWND) {
        let owner = GetWindow(dialog, GW_OWNER);
        let mut d: RECT = std::mem::zeroed();
        let mut o: RECT = std::mem::zeroed();
        if GetWindowRect(dialog, &mut d) == 0 || GetWindowRect(owner, &mut o) == 0 {
            return;
        }
        let x = o.left + ((o.right - o.left) - (d.right - d.left)) / 2;
        // Never above the owner's title bar.
        let y = (o.top + ((o.bottom - o.top) - (d.bottom - d.top)) / 2).max(o.top);
        SetWindowPos(
            dialog,
            std::ptr::null_mut(),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}
