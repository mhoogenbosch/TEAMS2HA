//! Shared helper for identifying Microsoft Teams processes.
//!
//! Lives in its own module because both `wasapi_monitor` (audio sessions) and
//! `uia_monitor` (windows) need it. It was previously private to wasapi_monitor;
//! copying it would have been the third place in this codebase where duplicated
//! logic drifted apart.

/// True when `pid` belongs to a Microsoft Teams executable.
///
/// PID→is-Teams lookups (OpenProcess + QueryFullProcessImageNameW) are the expensive
/// part of the 4 Hz polling loops, and a live PID's image name never changes.
/// Cache per PID; flush periodically to cope with PID reuse. (Fork perf fix,
/// re-applied here after the upstream refactor moved the lookup to this module —
/// caching centrally now benefits both wasapi_monitor and uia_monitor.)
#[cfg(windows)]
pub fn is_teams_pid(pid: u32) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    static CACHE: Mutex<Option<(Instant, HashMap<u32, bool>)>> = Mutex::new(None);

    let mut guard = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = Instant::now();
    let entry = guard.get_or_insert_with(|| (now, HashMap::new()));
    if now.duration_since(entry.0) > Duration::from_secs(60) {
        *entry = (now, HashMap::new());
    }
    if let Some(&cached) = entry.1.get(&pid) {
        return cached;
    }
    let value = is_teams_pid_uncached(pid);
    entry.1.insert(pid, value);
    value
}

/// Uncached lookup: matches on the image name rather than a fixed path — Teams is an
/// MSIX package whose install directory carries the version
/// (…\MSTeams_26183.1903.4892.4448_x64__…), so it changes on every auto-update.
#[cfg(windows)]
fn is_teams_pid_uncached(pid: u32) -> bool {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => h,
            Err(_) => return false,
        };

        let mut buf = vec![0u16; 260];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);

        if ok.is_err() {
            return false;
        }

        let name = OsString::from_wide(&buf[..size as usize])
            .to_string_lossy()
            .to_lowercase();
        name.contains("ms-teams") || name.contains("msteams")
    }
}

#[cfg(not(windows))]
pub fn is_teams_pid(_pid: u32) -> bool {
    false
}
