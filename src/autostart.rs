//! **"Start with Windows"** — the menu's checkmark, and nothing more.
//!
//! One value under `HKEY_CURRENT_USER\…\CurrentVersion\Run`, holding this
//! executable's own path. It is the current USER's key: nothing here touches a
//! machine-wide setting, a service, a scheduled task or the certificate store.
//!
//! **It is only ever written by the operator clicking the item.** Nothing in a
//! pass, a first run or an update turns it on — a program that enrolled itself
//! at startup because it had been run once would be making a decision that is
//! not its own.

#[cfg(windows)]
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
/// The value's name under that key.
#[cfg(windows)]
const VALUE: &str = "RivetMonitor";

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Is Rivet Monitor set to start with Windows?
#[cfg(windows)]
#[must_use]
pub fn enabled() -> bool {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegGetValueW, RegOpenKeyExW,
    };

    let path = wide(RUN_KEY);
    let name = wide(VALUE);
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: `path` is NUL-terminated and `key` is a valid out-pointer.
    let opened = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_READ, &mut key) };
    if opened != ERROR_SUCCESS {
        return false;
    }
    let mut size: u32 = 0;
    // SAFETY: a size-query call — every pointer is either valid or the
    // documented null.
    let found = unsafe {
        RegGetValueW(
            key,
            std::ptr::null(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    // SAFETY: `key` came from a successful open and is not used again.
    unsafe { RegCloseKey(key) };
    found == ERROR_SUCCESS
}

/// Turn it on or off.
///
/// # Errors
///
/// The registry's own refusal, as a number, with what was being attempted.
#[cfg(windows)]
pub fn set(on: bool) -> Result<(), String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW,
        RegOpenKeyExW, RegSetValueExW,
    };

    let exe = std::env::current_exe().map_err(|e| format!("this program has no path: {e}"))?;
    let command = format!("\"{}\"", exe.display());
    let path = wide(RUN_KEY);
    let name = wide(VALUE);
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: `path` is NUL-terminated and `key` is a valid out-pointer.
    let opened =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_SET_VALUE, &mut key) };
    if opened != ERROR_SUCCESS {
        return Err(format!("cannot open HKCU\\{RUN_KEY} for writing: {opened}"));
    }
    let result = if on {
        let value = wide(&command);
        let bytes = std::mem::size_of_val(&value[..]);
        // SAFETY: `value` is a NUL-terminated UTF-16 buffer and `bytes` is its
        // exact length, which is what REG_SZ requires.
        unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast::<u8>(),
                u32::try_from(bytes).unwrap_or(0),
            )
        }
    } else {
        // SAFETY: `name` is NUL-terminated; a missing value is reported, not
        // fatal.
        unsafe { RegDeleteValueW(key, name.as_ptr()) }
    };
    // SAFETY: `key` came from a successful open and is not used again.
    unsafe { RegCloseKey(key) };
    if result == ERROR_SUCCESS || (!on && !enabled()) {
        Ok(())
    } else {
        Err(format!(
            "the registry refused to {} HKCU\\{RUN_KEY}\\{VALUE}: {result}",
            if on { "write" } else { "remove" }
        ))
    }
}

#[cfg(not(windows))]
#[must_use]
pub fn enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set(_on: bool) -> Result<(), String> {
    Err("start-with-Windows is a Windows setting (ADR-001 §2)".to_string())
}
