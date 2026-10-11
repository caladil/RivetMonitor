//! **Opening a URL or a file the way the operator's own desktop would.**
//!
//! `ShellExecuteW` with the `open` verb, which is what a double-click does: the
//! fleet portal and a machine's index go to the default browser — the one
//! already enrolled with the operator certificate `rivet-proxyd` asks for —
//! and `View Config` goes to whatever handles `.toml`.

use std::path::Path;

/// Open a URL in the default browser.
pub fn open_url(url: &str) {
    open(url);
}

/// Open a file with its registered handler.
pub fn open_path(path: &Path) {
    open(&path.display().to_string());
}

#[cfg(windows)]
fn open(what: &str) {
    use std::os::windows::ffi::OsStrExt as _;

    let wide = |text: &str| -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let verb = wide("open");
    let target = wide(what);
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call, and
    // every other argument is a documented null/zero.
    unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}

#[cfg(not(windows))]
fn open(what: &str) {
    // This program is Windows-only by decision (`ADR-001` §2) — a tray icon is
    // a platform's own idiom. The non-Windows arm exists so the parts that are
    // not the tray can still be compiled and tested elsewhere.
    eprintln!("open: {what}");
}
