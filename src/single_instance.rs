//! Only one Scout window at a time. A second start doesn't open another window: it finds the running copy's main window,
//! brings it back from minimized and in front, and exits. A named mutex tells the copies apart.
//!
//! The updater restarts Scout into the new build while the old one is still closing, so that start waits for the old
//! copy to let go of the mutex instead of handing off to it (see `update::Updater::restart`).

/// Set on the process the updater starts, so it waits for the old copy to exit instead of handing off to it.
pub const RESTART_ENV: &str = "SCOUT_UPDATE_RESTART";

/// True when this process should carry on and start the app. False when another copy is already running: its window
/// has been brought forward, and this process should exit.
#[cfg(windows)]
pub fn claim() -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
    use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
    use windows::core::w;

    let restarting = std::env::var_os(RESTART_ENV).is_some();
    let handle: HANDLE = match unsafe { CreateMutexW(None, false, w!("Local\\ScoutDesktop.SingleInstance")) } {
        Ok(h) => h,
        // Without the mutex there's nothing to compare against, so start as before.
        Err(_) => return true,
    };
    let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already && restarting {
        // The old copy closes its window right after starting this one. Wait for it to let go, for up to 15 seconds.
        // Owned now (or the old copy is stuck). The handle is never closed, so the mutex lasts as long as this process.
        unsafe { WaitForSingleObject(handle, 15_000) };
        return true;
    }
    if !already {
        // Never closed on purpose: the mutex lasts as long as this process, and Windows releases it when it exits.
        return true;
    }
    focus_existing();
    false
}

#[cfg(not(windows))]
pub fn claim() -> bool {
    true
}

/// Restores the running copy's main window and brings it to the front.
#[cfg(windows)]
fn focus_existing() {
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow};
    use windows::core::{BOOL, PWSTR};

    // The copy to look for is the one running this same exe.
    let mine = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().to_lowercase(),
        Err(_) => return,
    };

    /// The main window: titled "Scout", owned by a process running this exe. Setup's own windows have other titles.
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let want = unsafe { &*(lparam.0 as *const (String, std::cell::Cell<Option<isize>>)) };
        let mut title = [0u16; 64];
        let n = unsafe { GetWindowTextW(hwnd, &mut title) };
        if String::from_utf16_lossy(&title[..n.max(0) as usize]) != "Scout" {
            return BOOL(1);
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == std::process::id() {
            return BOOL(1);
        }
        let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return BOOL(1) };
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }.is_ok();
        let _ = unsafe { CloseHandle(process) };
        if ok && String::from_utf16_lossy(&buf[..len as usize]).to_lowercase() == want.0 {
            want.1.set(Some(hwnd.0 as isize));
            return BOOL(0); // Found it: stop looking.
        }
        BOOL(1)
    }

    let state: (String, std::cell::Cell<Option<isize>>) = (mine, std::cell::Cell::new(None));
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&state as *const _ as isize));
    }
    if let Some(raw) = state.1.get() {
        let hwnd = HWND(raw as *mut _);
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }
}
