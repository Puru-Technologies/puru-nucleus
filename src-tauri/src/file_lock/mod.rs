//! Reliable file-lock resolution via the Windows Restart Manager.
//!
//! `taskkill /F` on our *tracked* PID isn't enough when a duplicate or zombie
//! JVM holds a service JAR memory-mapped: the file stays locked and the update
//! swap fails with SHARING_VIOLATION (os error 32). The Restart Manager answers
//! "which processes have THIS file open" authoritatively — the same mechanism
//! Windows installers use for the "the following applications are using this
//! file" prompt — so we can kill exactly those and confirm the file is free
//! before renaming.

use std::path::Path;

/// PIDs of every process that currently has `path` open or memory-mapped.
/// Returns an empty vec when nothing holds it (or on any RM error — callers
/// treat "unknown" as "not held" and fall back to their own retry).
#[cfg(windows)]
pub fn processes_locking(path: &Path) -> Vec<u32> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows_sys::Win32::System::RestartManager::{
        RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY,
        RM_PROCESS_INFO,
    };

    if !path.exists() {
        return Vec::new();
    }
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut session: u32 = 0;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        if RmStartSession(&mut session, 0, key.as_mut_ptr()) != ERROR_SUCCESS {
            return Vec::new();
        }

        let files: [*const u16; 1] = [wide.as_ptr()];
        let rc = RmRegisterResources(
            session,
            1,
            files.as_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
        );
        if rc != ERROR_SUCCESS {
            RmEndSession(session);
            return Vec::new();
        }

        let mut pids = Vec::new();
        let mut needed: u32 = 0;
        let mut count: u32 = 0;
        let mut reason: u32 = 0;
        // First call sizes the buffer (needed = number of affected apps).
        let rc = RmGetList(
            session,
            &mut needed,
            &mut count,
            std::ptr::null_mut(),
            &mut reason,
        );
        if (rc == ERROR_SUCCESS || rc == ERROR_MORE_DATA) && needed > 0 {
            let mut infos: Vec<RM_PROCESS_INFO> = vec![std::mem::zeroed(); needed as usize];
            count = needed;
            let rc = RmGetList(session, &mut needed, &mut count, infos.as_mut_ptr(), &mut reason);
            if rc == ERROR_SUCCESS {
                for info in infos.iter().take(count as usize) {
                    pids.push(info.Process.dwProcessId);
                }
            }
        }
        RmEndSession(session);
        pids
    }
}

#[cfg(not(windows))]
pub fn processes_locking(_path: &Path) -> Vec<u32> {
    Vec::new()
}

/// Force-free `path`: kill every process holding it (Restart Manager →
/// `taskkill /F`), re-checking until nothing holds it or `max_wait_secs`
/// elapses. This is the fail-safe used before an update swap and before a
/// (re)start, so a duplicate/zombie JVM can never keep a service JAR locked.
/// Returns the number of processes killed, or an error naming the stubborn PIDs
/// if the file is still held at the deadline.
pub async fn free_file(path: &Path, max_wait_secs: u64) -> Result<usize, String> {
    let self_pid = std::process::id();
    let mut killed = 0usize;
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(max_wait_secs);

    loop {
        let holders: Vec<u32> = processes_locking(path)
            .into_iter()
            .filter(|p| *p != self_pid && *p != 0)
            .collect();

        if holders.is_empty() {
            return Ok(killed);
        }

        for pid in &holders {
            tracing::warn!(
                "free_file: killing PID {} still holding {}",
                pid,
                path.display()
            );
            let _ = crate::process_explorer::kill_pid(*pid).await;
            killed += 1;
        }

        if tokio::time::Instant::now() >= deadline {
            let still: Vec<u32> = processes_locking(path)
                .into_iter()
                .filter(|p| *p != self_pid && *p != 0)
                .collect();
            if still.is_empty() {
                return Ok(killed);
            }
            return Err(format!(
                "{} is still locked after {}s (held by PID(s) {:?})",
                path.display(),
                max_wait_secs,
                still
            ));
        }

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}
