//! Process utilities — cross-platform command spawning without console windows,
//! and lifetime coupling so children never outlive the supervisor.

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
    #[cfg_attr(not(windows), allow(unused_mut))]
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
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut cmd = std::process::Command::new(program);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd
}

// ── Child lifetime coupling ─────────────────────────────────────────────────

/// Tie a spawned child to this process's lifetime, so it cannot outlive us.
///
/// Every service JVM must die with its supervisor. Without this, killing
/// puru-dc — by `taskkill /F`, by a crash, or by Defender quarantining the exe
/// — leaves the JVMs running: still bound to their ports, still answering
/// health checks, and no longer managed by anything. The box reads as healthy
/// while nothing is actually supervising it, and the next puru-dc cannot start
/// its own copies because the ports are taken.
///
/// # Windows
///
/// Uses a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. The job is
/// anonymous and created once per process, so this process holds the only
/// handle to it; when the process object is destroyed the kernel closes that
/// handle, and closing the last handle terminates every process in the job.
/// That is what makes it survive `taskkill /F` — there is no cleanup code to
/// run and no signal to miss, the guarantee is enforced by the kernel.
///
/// Note the assignment happens just after spawn rather than while suspended,
/// so a child that forks within those few microseconds would escape the job.
/// `java.exe` does not, and spawning suspended would mean dropping down to raw
/// `CreateProcess` to get the thread handle back.
///
/// # Other platforms
///
/// A no-op for now. The equivalent is a process group plus a `SIGKILL` to
/// `-pgid` from a supervisor, which cannot be made to survive `SIGKILL` of the
/// parent the way a job object does; a PR_SET_PDEATHSIG-style approach has to
/// be set by the child itself.
#[cfg_attr(not(windows), allow(unused_variables))]
pub fn tie_to_supervisor_lifetime(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        if let Err(e) = windows_job::assign(pid) {
            // Never fail a service start over this — a child outliving us is
            // recoverable, a service that would not start is not.
            tracing::warn!(
                "Could not tie PID {} to the supervisor job object: {} — it may \
                 outlive puru-dc if we are force-killed",
                pid,
                e
            );
        } else {
            tracing::debug!("PID {} assigned to the kill-on-close job object", pid);
        }
    }
}

#[cfg(target_os = "windows")]
mod windows_job {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
        JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    /// Raw handle wrapper so the `OnceLock` can be `Send + Sync`. The handle is
    /// deliberately never closed: it must stay open for the whole process
    /// lifetime, because closing it is exactly what kills the children.
    #[derive(Clone, Copy)]
    struct JobHandle(isize);
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    static JOB: OnceLock<Option<JobHandle>> = OnceLock::new();

    /// Create the process-wide job object, once.
    fn job() -> Option<JobHandle> {
        *JOB.get_or_init(|| unsafe {
            // Anonymous: no name means no other process can open a handle to
            // it, so ours is the only one keeping the children alive.
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                tracing::warn!("CreateJobObject failed: {}", std::io::Error::last_os_error());
                return None;
            }

            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

            let ok = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                tracing::warn!(
                    "SetInformationJobObject(KILL_ON_JOB_CLOSE) failed: {}",
                    std::io::Error::last_os_error()
                );
                CloseHandle(handle);
                return None;
            }

            tracing::info!("Supervisor job object created — children die with puru-dc");
            Some(JobHandle(handle as isize))
        })
    }

    /// Test hook: is this PID in *our* job object, and is the job set to kill
    /// its members when the last handle closes?
    #[cfg(test)]
    pub fn verify(pid: u32) -> Result<(bool, bool), std::io::Error> {
        use windows_sys::Win32::System::JobObjects::{
            IsProcessInJob, QueryInformationJobObject,
        };
        use windows_sys::Win32::System::Threading::PROCESS_QUERY_INFORMATION;

        let job = job().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::Other, "job object unavailable")
        })?;

        unsafe {
            let proc: HANDLE = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
            if proc.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let mut in_job: i32 = 0;
            let ok = IsProcessInJob(proc, job.0 as HANDLE, &mut in_job);
            CloseHandle(proc);
            if ok == 0 {
                return Err(std::io::Error::last_os_error());
            }

            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            let ok = QueryInformationJobObject(
                job.0 as HANDLE,
                JobObjectExtendedLimitInformation,
                &mut info as *mut _ as *mut std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                std::ptr::null_mut(),
            );
            if ok == 0 {
                return Err(std::io::Error::last_os_error());
            }

            let kills_on_close = info.BasicLimitInformation.LimitFlags
                & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                != 0;

            Ok((in_job != 0, kills_on_close))
        }
    }

    pub fn assign(pid: u32) -> Result<(), std::io::Error> {
        let job = job().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::Other, "job object unavailable")
        })?;

        unsafe {
            // PROCESS_SET_QUOTA | PROCESS_TERMINATE is the documented minimum
            // for AssignProcessToJobObject.
            let proc: HANDLE = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if proc.is_null() {
                return Err(std::io::Error::last_os_error());
            }

            let ok = AssignProcessToJobObject(job.0 as HANDLE, proc);
            let err = std::io::Error::last_os_error();
            CloseHandle(proc);

            if ok == 0 {
                return Err(err);
            }
        }

        Ok(())
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    /// A child handed to `tie_to_supervisor_lifetime` must land in a job that
    /// is configured to kill its members when the last handle closes. That flag
    /// is the whole guarantee: it is what makes an orphaned JVM impossible even
    /// when puru-dc is `taskkill /F`-ed or quarantined mid-run.
    #[test]
    fn child_is_assigned_to_a_kill_on_close_job() {
        // `ping` idles without a console, tolerates redirected stdio, and exits
        // on its own if this test somehow fails to clean it up.
        let mut child = silent_std_cmd("ping")
            .args(["-n", "30", "127.0.0.1"])
            .spawn()
            .expect("spawn test child");

        let pid = child.id();
        tie_to_supervisor_lifetime(pid);

        let (in_job, kills_on_close) =
            windows_job::verify(pid).expect("query job membership");

        let _ = child.kill();
        let _ = child.wait();

        assert!(in_job, "child was not assigned to the supervisor job");
        assert!(
            kills_on_close,
            "job is missing JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE — children would survive"
        );
    }

    fn is_alive(pid: u32) -> bool {
        let out = silent_std_cmd("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/NH"])
            .output()
            .expect("tasklist");
        String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    }

    /// Helper process for the end-to-end test below: ties a grandchild to its
    /// own lifetime, publishes the PID, then waits to be force-killed.
    /// `#[ignore]` so it only runs when invoked explicitly.
    #[test]
    #[ignore]
    fn job_e2e_helper() {
        let out = match std::env::var("PURU_JOB_HELPER_OUT") {
            Ok(p) => p,
            Err(_) => return, // not the helper invocation
        };

        // `ping` rather than `timeout`: timeout.exe refuses to run when stdio
        // is redirected ("Input redirection is not supported"), which is always
        // the case under the test harness.
        let child = silent_std_cmd("ping")
            .args(["-n", "300", "127.0.0.1"])
            .spawn()
            .expect("spawn grandchild");

        tie_to_supervisor_lifetime(child.id());
        std::fs::write(&out, child.id().to_string()).expect("publish pid");

        // The test that spawned us kills us long before this elapses.
        std::thread::sleep(std::time::Duration::from_secs(60));
    }

    /// The guarantee that matters: `taskkill /F` on the supervisor — no unwind,
    /// no destructors, no chance to clean up — must still take its children
    /// down. This is the scenario that left five orphaned JVMs holding ports
    /// after puru-dc was killed.
    #[test]
    fn force_killing_the_supervisor_kills_its_children() {
        let pid_file = std::env::temp_dir()
            .join(format!("puru-job-e2e-{}.pid", std::process::id()));
        let _ = std::fs::remove_file(&pid_file);

        let mut helper = silent_std_cmd(std::env::current_exe().expect("current exe"))
            // `--exact` matches the full test path, not the bare fn name.
            .args([
                "process::tests::job_e2e_helper",
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .env("PURU_JOB_HELPER_OUT", &pid_file)
            .spawn()
            .expect("spawn helper");

        // Wait for the helper to publish its grandchild's PID.
        let mut grandchild_pid = None;
        for _ in 0..100 {
            if let Ok(s) = std::fs::read_to_string(&pid_file) {
                if let Ok(pid) = s.trim().parse::<u32>() {
                    grandchild_pid = Some(pid);
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let grandchild_pid = grandchild_pid.expect("helper never published a PID");
        assert!(is_alive(grandchild_pid), "grandchild should be running");

        // Force-kill the supervisor. No graceful path, no cleanup code runs.
        let _ = silent_std_cmd("taskkill")
            .args(["/F", "/PID", &helper.id().to_string()])
            .output();
        let _ = helper.wait();

        // The kernel closes the job handle with the process; that must take the
        // grandchild with it.
        let mut died = false;
        for _ in 0..100 {
            if !is_alive(grandchild_pid) {
                died = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        if !died {
            let _ = silent_std_cmd("taskkill")
                .args(["/F", "/PID", &grandchild_pid.to_string()])
                .output();
        }
        let _ = std::fs::remove_file(&pid_file);

        assert!(
            died,
            "grandchild {} survived a force-kill of its supervisor — it would \
             have been an orphan holding its port",
            grandchild_pid
        );
    }
}
