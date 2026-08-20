//! Process utilities — cross-platform command spawning without console windows.

use std::ffi::OsStr;
use tokio::process::Command;

/// Create a `Command` that won't flash a console window on Windows.
///
/// On Windows, sets `CREATE_NO_WINDOW` (0x08000000) creation flag.
/// On other platforms, returns a plain `Command`.
///
/// Accepts anything `std::process::Command::new` does — `&str`, `String`,
/// `PathBuf`, `&Path`, `&OsStr`, … — so callers can pass a resolved executable
/// path without stringifying it.
pub fn silent_cmd<S: AsRef<OsStr>>(program: S) -> Command {
    let mut cmd = Command::new(program);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd
}

/// Synchronous sibling of [`silent_cmd`] for non-async code paths (quick
/// `tasklist`/`pgrep`/`reg` probes, silent-install runners). Without this,
/// those calls flash a console window on Windows — and the installer/watchdog
/// each fire dozens of these per run.
pub fn silent_std_cmd<S: AsRef<OsStr>>(program: S) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd
}
