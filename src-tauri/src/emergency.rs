//! Emergency stop-all scripts.
//!
//! Writes a self-contained OS script into the config directory that operators
//! can double-click when puru-dc itself is broken (won't launch, GUI crash,
//! CLI won't parse). The script kills every Puru JVM by PID file and, as a
//! fallback, by listening port — nothing in it touches or depends on puru-dc.
//!
//! Regenerated on every daemon/GUI startup so the port list and paths stay
//! current with the config. Landing location:
//!
//!   Windows: C:\PuruNucleus\emergency-stop.bat
//!   Unix:    <config_dir>/emergency-stop.sh
//!
//! Docker mode: operators can just run `docker compose stop` — no script needed.

use crate::config::NucleusConfig;

/// Every listening port a Puru JAR could hold. Matches the service registry;
/// updated here rather than derived at runtime because this file needs to be
/// runnable when puru-dc can't start (i.e. can't tell us the ports).
const PURU_JVM_PORTS: &[u16] = &[
    8080, // auth
    8081, // xenon
    8082, // has
    8083, // pacs (also 104, 106 for DICOM but those are dcm4che listeners)
    8084, // argon
    8085, // comm
    8086, // realtime
    8087, // neon
    8088, // integration
    8089, // mercury
    8094, // bridge
    8095, // counter
];

/// (Re)write the emergency-stop script into the config directory. Idempotent —
/// safe to call on every startup; the file is overwritten each time so its
/// port list and PID directory stay in sync with the current config.
pub fn ensure_emergency_stop_script(config: &NucleusConfig) {
    let dir = crate::config::config_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("emergency: could not create {}: {}", dir.display(), e);
        return;
    }

    #[cfg(target_os = "windows")]
    let (path, content) = (dir.join("emergency-stop.bat"), windows_script(config));

    #[cfg(not(target_os = "windows"))]
    let (path, content) = (dir.join("emergency-stop.sh"), unix_script(config));

    if let Err(e) = std::fs::write(&path, &content) {
        tracing::warn!("emergency: could not write {}: {}", path.display(), e);
        return;
    }

    // On Unix, mark it executable so a double-click / ./emergency-stop.sh works.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&path, perms);
        }
    }

    tracing::info!("emergency: stop-all script written to {}", path.display());
}

#[cfg(target_os = "windows")]
fn windows_script(config: &NucleusConfig) -> String {
    // Native-mode PID directory (falls back to the default when config is
    // silent). Docker deployments will find nothing here — and that's fine;
    // docker compose stop is the right lever there.
    let pid_dir = config.native_logs_dir().display().to_string();
    let ports = PURU_JVM_PORTS
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        r#"@echo off
setlocal EnableDelayedExpansion
title Puru DC — Emergency Stop
echo ============================================================
echo  Puru DC — Emergency Stop (native JAR services)
echo  Runs independently of puru-dc.exe. Docker mode: use
echo  "docker compose stop" instead — this script won't touch
echo  containers.
echo ============================================================
echo.

set "PID_DIR={pid_dir}"
set "KILLED=0"

REM ── 1. Kill by PID file. Each JVM writes <service>.pid on start. ─────────
if exist "%PID_DIR%\*.pid" (
    echo [1/2] Stopping JVMs by PID file in %PID_DIR%
    for %%f in ("%PID_DIR%\*.pid") do (
        set /p PID=<"%%f"
        if defined PID (
            echo   - %%~nf  (PID !PID!)
            taskkill /F /PID !PID! >nul 2>&1
            if !errorlevel! equ 0 (
                set /a KILLED+=1
                del /q "%%f" >nul 2>&1
            )
            set PID=
        )
    )
) else (
    echo [1/2] No PID files under %PID_DIR% — nothing to stop by PID.
)
echo.

REM ── 2. Fallback: kill whatever is listening on Puru JVM ports. ───────────
echo [2/2] Sweeping listening ports (fallback)
for %%p in ({ports}) do (
    for /f "tokens=5" %%a in ('netstat -ano -p tcp ^| findstr /r /c:":%%p .*LISTENING"') do (
        echo   - port %%p  (PID %%a)
        taskkill /F /PID %%a >nul 2>&1
        if !errorlevel! equ 0 set /a KILLED+=1
    )
)
echo.

echo ============================================================
echo  Done. Processes killed: !KILLED!
echo  MySQL and RabbitMQ are Windows services — they were NOT
echo  touched. Manage those with "net stop MySQL" / "net stop
echo  RabbitMQ" if you need to.
echo ============================================================
echo.
pause
endlocal
"#,
        pid_dir = pid_dir,
        ports = ports,
    )
}

#[cfg(not(target_os = "windows"))]
fn unix_script(config: &NucleusConfig) -> String {
    let pid_dir = config.native_logs_dir().display().to_string();
    let ports = PURU_JVM_PORTS
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        r#"#!/usr/bin/env bash
# Puru DC — emergency stop-all for native JAR services.
# Runs independently of puru-dc. Docker: use `docker compose stop` instead.

set -u
PID_DIR="{pid_dir}"
PORTS=({ports})
KILLED=0

echo "============================================================"
echo " Puru DC — Emergency Stop (native JAR services)"
echo "============================================================"
echo

# 1. Kill by PID file.
if compgen -G "$PID_DIR/*.pid" >/dev/null; then
    echo "[1/2] Stopping JVMs by PID file in $PID_DIR"
    for f in "$PID_DIR"/*.pid; do
        pid="$(cat "$f" 2>/dev/null || true)"
        name="$(basename "$f" .pid)"
        if [ -n "${{pid:-}}" ] && kill -0 "$pid" 2>/dev/null; then
            echo "  - $name  (PID $pid)"
            kill -TERM "$pid" 2>/dev/null || true
            sleep 1
            if kill -0 "$pid" 2>/dev/null; then
                kill -KILL "$pid" 2>/dev/null || true
            fi
            KILLED=$((KILLED+1))
        fi
        rm -f "$f"
    done
else
    echo "[1/2] No PID files under $PID_DIR — nothing to stop by PID."
fi
echo

# 2. Fallback: kill whatever is listening on Puru JVM ports.
echo "[2/2] Sweeping listening ports (fallback)"
for p in "${{PORTS[@]}}"; do
    if command -v lsof >/dev/null 2>&1; then
        pids="$(lsof -t -iTCP:"$p" -sTCP:LISTEN 2>/dev/null || true)"
    else
        pids="$(ss -Hltnp "sport = :$p" 2>/dev/null | grep -oE 'pid=[0-9]+' | cut -d= -f2 | sort -u || true)"
    fi
    for pid in $pids; do
        echo "  - port $p  (PID $pid)"
        kill -TERM "$pid" 2>/dev/null || true
        KILLED=$((KILLED+1))
    done
done
echo

echo "============================================================"
echo " Done. Processes killed: $KILLED"
echo " MySQL and RabbitMQ are managed by systemd/launchd — they"
echo " were NOT touched. Use 'systemctl stop mysql' etc. if needed."
echo "============================================================"
"#,
        pid_dir = pid_dir,
        ports = ports,
    )
}
