//! Microsoft Defender interop — exclusion registration and quarantine diagnosis.
//!
//! ## Why this exists
//!
//! puru-dc registers a SYSTEM boot task (`PuruDC`) and a logon task (`PuruDCGui`)
//! via `schtasks`, then spawns a fleet of JVM children. To Defender's behaviour
//! engine that is indistinguishable from malware establishing persistence, and an
//! unsigned binary has no file reputation to offset it. On 2026-08-21 this fired
//! for real on a hospital box: `Behavior:Win32/Persistence.A!ml` quarantined
//! `puru-dc.exe`, **both** shortcuts and `System32\Tasks\PuruDC` — the tray icon
//! vanished a few minutes after boot, the daemon died, and PACS silently lost its
//! token broker. The already-spawned JVMs kept running, so the box looked healthy.
//!
//! The detection is *behavioural*, not hash-based: rebuilding or rolling back the
//! binary does not avoid it. Two things do, and we want both:
//!
//! 1. **Authenticode signing** — the real fix, since reputation is what the ML
//!    model weighs most. Not something this module can do; see `RELEASE.md`.
//! 2. **A process exclusion registered before the persistence write** — what this
//!    module does. `-ExclusionProcess` is the one that matters: it suppresses
//!    *behaviour monitoring* for actions performed by that image. `-ExclusionPath`
//!    only stops on-access file scanning, so a path exclusion alone would not have
//!    prevented the incident.
//!
//! Ordering is the whole point. [`ensure_exclusions`] must run *before* the task
//! is registered, otherwise Defender is already watching when the persistence
//! write happens.
//!
//! ## Never assume a write succeeded
//!
//! With Tamper Protection on (the default on Windows 11), `Add-MpPreference` can
//! be refused while still exiting 0. Every write here is verified by reading the
//! preference back, and a refusal is reported as [`ExclusionOutcome::Blocked`] so
//! the operator is told to add it via the Windows Security UI rather than being
//! left with a silently unprotected install.

use serde::{Deserialize, Serialize};

/// Result of trying to register Defender exclusions for puru-dc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExclusionOutcome {
    /// Both exclusions are present (either we just added them, or they already were).
    Registered,
    /// Defender refused the write — almost always Tamper Protection, or not elevated.
    /// Carries an operator-facing explanation.
    Blocked(String),
    /// Defender is not the active AV (third-party product, Server core, non-Windows).
    NotApplicable,
}

impl ExclusionOutcome {
    /// One-line summary suitable for appending to a `service install` message.
    pub fn summary(&self) -> String {
        match self {
            ExclusionOutcome::Registered => {
                "Defender exclusions registered for puru-dc.".to_string()
            }
            ExclusionOutcome::Blocked(why) => format!("WARNING: {}", why),
            ExclusionOutcome::NotApplicable => {
                "Microsoft Defender not active — no exclusions needed.".to_string()
            }
        }
    }

    /// True when puru-dc is at risk of being quarantined and the operator must act.
    pub fn needs_operator_action(&self) -> bool {
        matches!(self, ExclusionOutcome::Blocked(_))
    }
}

/// Escape a string for embedding in a PowerShell **single-quoted** literal.
///
/// Only `'` is special inside `'...'`; it is escaped by doubling. This keeps
/// install paths containing spaces, `$`, or backticks inert.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn ps_single_quote(s: &str) -> String {
    s.replace('\'', "''")
}

/// Marker Defender returns from `Get-MpPreference` when the caller is not elevated.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const NOT_ADMIN_MARKER: &str = "Must be an administrator";

#[cfg(target_os = "windows")]
fn exclusion_script(exe: &str, dir: &str) -> String {
    format!(
        r#"$ErrorActionPreference = 'SilentlyContinue'
$exe = '{exe}'
$dir = '{dir}'
if (-not (Get-Command Get-MpComputerStatus -ErrorAction SilentlyContinue)) {{
    Write-Output 'DEFENDER=ABSENT'
    exit
}}
$status = Get-MpComputerStatus
if ($null -eq $status) {{ Write-Output 'DEFENDER=ABSENT'; exit }}
if (-not $status.AntivirusEnabled) {{ Write-Output 'DEFENDER=DISABLED'; exit }}
Write-Output "TAMPER=$($status.IsTamperProtected)"

try {{ Add-MpPreference -ExclusionProcess $exe -ErrorAction Stop }} catch {{ }}
try {{ Add-MpPreference -ExclusionPath    $dir -ErrorAction Stop }} catch {{ }}

$pref = Get-MpPreference
if ($null -eq $pref) {{ Write-Output 'READBACK=DENIED'; exit }}
$procList = @($pref.ExclusionProcess)
$pathList = @($pref.ExclusionPath)
if (($procList -join ' ') -like '*{marker}*') {{ Write-Output 'READBACK=DENIED'; exit }}
if ($procList -contains $exe) {{ Write-Output 'PROC=OK' }} else {{ Write-Output 'PROC=FAIL' }}
if ($pathList -contains $dir) {{ Write-Output 'PATH=OK' }} else {{ Write-Output 'PATH=FAIL' }}
"#,
        exe = ps_single_quote(exe),
        dir = ps_single_quote(dir),
        marker = NOT_ADMIN_MARKER,
    )
}

/// Interpret the marker lines emitted by [`exclusion_script`].
///
/// Split out from the process invocation so the decision logic is unit-testable
/// without a live Defender.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_exclusion_output(stdout: &str, exe: &str) -> ExclusionOutcome {
    if stdout.contains("DEFENDER=ABSENT") || stdout.contains("DEFENDER=DISABLED") {
        return ExclusionOutcome::NotApplicable;
    }
    if stdout.contains("READBACK=DENIED") {
        return ExclusionOutcome::Blocked(
            "Could not read Defender exclusions — puru-dc must be installed from an \
             elevated prompt. Without a process exclusion Defender may quarantine \
             puru-dc.exe as Behavior:Win32/Persistence.A!ml."
                .to_string(),
        );
    }

    let proc_ok = stdout.contains("PROC=OK");
    let path_ok = stdout.contains("PATH=OK");

    if proc_ok && path_ok {
        return ExclusionOutcome::Registered;
    }

    // The process exclusion is the one that suppresses behaviour monitoring, so a
    // missing PROC is the dangerous case; call it out specifically.
    let tamper_on = stdout.contains("TAMPER=True");
    let hint = if tamper_on {
        "Tamper Protection is ON, which refuses exclusion changes from the command line. \
         Add them by hand: Windows Security > Virus & threat protection > Manage settings \
         > Exclusions > Add or remove exclusions."
    } else {
        "Add them by hand: Windows Security > Virus & threat protection > Manage settings \
         > Exclusions > Add or remove exclusions."
    };

    let missing = match (proc_ok, path_ok) {
        (false, false) => "process and folder exclusions",
        (false, true) => "the process exclusion",
        (true, false) => "the folder exclusion",
        (true, true) => unreachable!("handled above"),
    };

    ExclusionOutcome::Blocked(format!(
        "Defender did not accept {missing} for {exe}. {hint} \
         Until then Defender may quarantine puru-dc.exe and its boot task, which \
         silently stops the daemon."
    ))
}

/// Register Defender exclusions for the running puru-dc binary and its install
/// directory, verifying the change actually took.
///
/// Idempotent — `Add-MpPreference` on an already-excluded path is a no-op, so this
/// is safe to call on every install. Requires elevation; call it from a code path
/// that already has it (`service install`).
///
/// **Call this before registering the scheduled task.**
#[cfg(target_os = "windows")]
pub async fn ensure_exclusions() -> ExclusionOutcome {
    let exe = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => {
            return ExclusionOutcome::Blocked(format!(
                "Cannot determine the puru-dc executable path ({e}), so no Defender \
                 exclusion could be registered."
            ));
        }
    };
    let dir = std::path::Path::new(&exe)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| exe.clone());

    let output = crate::process::silent_cmd("powershell")
        .args(["-NoProfile", "-Command", &exclusion_script(&exe, &dir)])
        .output()
        .await;

    match output {
        Ok(o) => parse_exclusion_output(&String::from_utf8_lossy(&o.stdout), &exe),
        Err(e) => ExclusionOutcome::Blocked(format!(
            "Could not run PowerShell to register Defender exclusions: {e}"
        )),
    }
}

#[cfg(not(target_os = "windows"))]
pub async fn ensure_exclusions() -> ExclusionOutcome {
    ExclusionOutcome::NotApplicable
}

/// Ask Defender whether it has a threat detection naming puru-dc.
///
/// Used to turn "the daemon is mysteriously gone" into a specific, actionable
/// diagnosis. Returns the detection time and threat id when found.
///
/// Cheap enough for a status call but not for a hot loop — it shells out to
/// PowerShell and Defender's cmdlets are slow (~1s).
#[cfg(target_os = "windows")]
pub async fn recent_detection() -> Option<String> {
    let script = r#"$ErrorActionPreference='SilentlyContinue'
Get-MpThreatDetection |
  Where-Object { ($_.Resources -join ';') -like '*puru-dc*' -or ($_.Resources -join ';') -like '*PuruDC*' } |
  Sort-Object InitialDetectionTime -Descending |
  Select-Object -First 1 |
  ForEach-Object { "DETECTED=$($_.InitialDetectionTime)|$($_.ThreatID)" }"#;

    let output = crate::process::silent_cmd("powershell")
        .args(["-NoProfile", "-Command", script])
        .output()
        .await
        .ok()?;

    parse_detection_output(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(target_os = "windows"))]
pub async fn recent_detection() -> Option<String> {
    None
}

/// Turn the `DETECTED=<time>|<id>` marker into an operator-facing sentence.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_detection_output(stdout: &str) -> Option<String> {
    let line = stdout.lines().find(|l| l.starts_with("DETECTED="))?;
    let body = line.trim_start_matches("DETECTED=").trim();
    let (when, id) = body.split_once('|').unwrap_or((body, ""));
    Some(format!(
        "Microsoft Defender quarantined puru-dc at {when} (threat id {id}). \
         This removes puru-dc.exe, its shortcuts and the PuruDC boot task, which \
         stops the daemon without any error in the app's own logs. Restore the \
         binary and register a Defender process exclusion for it."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_quotes_are_doubled_for_powershell() {
        assert_eq!(ps_single_quote(r"C:\Program Files\puru-dc"), r"C:\Program Files\puru-dc");
        assert_eq!(ps_single_quote("it's"), "it''s");
    }

    #[test]
    fn both_exclusions_present_is_registered() {
        let out = "TAMPER=False\nPROC=OK\nPATH=OK\n";
        assert_eq!(parse_exclusion_output(out, "x.exe"), ExclusionOutcome::Registered);
    }

    #[test]
    fn absent_defender_is_not_applicable() {
        assert_eq!(
            parse_exclusion_output("DEFENDER=ABSENT\n", "x.exe"),
            ExclusionOutcome::NotApplicable
        );
        assert_eq!(
            parse_exclusion_output("DEFENDER=DISABLED\n", "x.exe"),
            ExclusionOutcome::NotApplicable
        );
    }

    #[test]
    fn unelevated_readback_is_blocked() {
        let outcome = parse_exclusion_output("READBACK=DENIED\n", "x.exe");
        assert!(outcome.needs_operator_action());
        match outcome {
            ExclusionOutcome::Blocked(m) => assert!(m.contains("elevated")),
            _ => panic!("expected Blocked"),
        }
    }

    /// The regression that matters: Add-MpPreference exits 0 under Tamper
    /// Protection but the exclusion is never stored. Exit status must not be
    /// trusted — only the readback.
    #[test]
    fn tamper_protection_refusal_is_blocked_and_names_tamper() {
        let out = "TAMPER=True\nPROC=FAIL\nPATH=FAIL\n";
        match parse_exclusion_output(out, "puru-dc.exe") {
            ExclusionOutcome::Blocked(m) => {
                assert!(m.contains("Tamper Protection"));
                assert!(m.contains("process and folder exclusions"));
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    /// A path exclusion alone does NOT stop the behavioural detection, so a
    /// missing process exclusion must still be reported as blocked.
    #[test]
    fn path_only_is_still_blocked() {
        let out = "TAMPER=False\nPROC=FAIL\nPATH=OK\n";
        match parse_exclusion_output(out, "puru-dc.exe") {
            ExclusionOutcome::Blocked(m) => assert!(m.contains("the process exclusion")),
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn detection_output_is_parsed_into_a_sentence() {
        let msg = parse_detection_output("DETECTED=21-08-2026 09:35:13|2147737394\n")
            .expect("should parse");
        assert!(msg.contains("21-08-2026 09:35:13"));
        assert!(msg.contains("2147737394"));
    }

    #[test]
    fn no_detection_returns_none() {
        assert!(parse_detection_output("").is_none());
        assert!(parse_detection_output("some unrelated output\n").is_none());
    }
}
