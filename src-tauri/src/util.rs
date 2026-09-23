use tauri::{AppHandle, Emitter};
use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader};
use std::sync::mpsc;

/// Drain both pipes of a running child concurrently, invoking `on_line` for
/// each line in arrival order, so a verbose tool cannot deadlock a full
/// stdout pipe against a blocked stderr read. Blocking; runs the child to
/// completion and returns its exit status.
pub fn pump_child(
    tag: &str,
    mut cmd: Command,
    on_line: impl Fn(&str),
) -> Result<std::process::ExitStatus, String> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn {}: {}", tag, e))?;

    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel::<String>();

    let h_out = std::thread::spawn({
        let tx = tx.clone();
        move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(format!("[out] {}", line)).is_err() {
                    break;
                }
            }
        }
    });
    let h_err = std::thread::spawn(move || {
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            if tx.send(format!("[err] {}", line)).is_err() {
                break;
            }
        }
    });

    for line in rx {
        on_line(&line);
    }
    let _ = h_out.join();
    let _ = h_err.join();
    child.wait().map_err(|e| e.to_string())
}
/// run_logged: pump_child plus UI emission. Blocking; callers must wrap in
/// tauri::async_runtime::spawn_blocking.
pub fn run_logged(
    app_handle: &AppHandle,
    tag: &str,
    cmd: Command,
) -> Result<(), String> {
    let tag_owned = tag.to_string();
    let app_handle2 = app_handle.clone();
    let status = pump_child(tag, cmd, move |line| {
        let _ = app_handle2.emit("log-event", format!("[{}] {}", tag_owned, line));
    })
    .map_err(|e| {
        let _ = app_handle.emit("log-event", format!("[Error] {}", e));
        e
    })?;
    if status.success() {
        let _ = app_handle.emit("log-event", format!("[System] {} completed successfully.", tag));
        Ok(())
    } else {
        let _ = app_handle.emit("log-event", format!("[Error] {} failed with status: {}", tag, status));
        Err(format!("{} failed with status: {}", tag, status))
    }
}

/// Build a Command with cwd set; helper for uniform call sites.
pub fn cmd(program: &str) -> Command {
    Command::new(program)
}
/// Look an external tool up on PATH.
pub fn find_tool(name: &str) -> Option<std::path::PathBuf> {
    crate::tools::find_in_path(name)
}
/// Run a command capturing output instead of streaming; for programmatic
/// queries (tools without useful progress logs).
pub fn capture(program: &str, args: &[&str], cwd: Option<&std::path::Path>) -> std::io::Result<std::process::Output> {
    let mut c = Command::new(program);
    c.args(args);
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    c.output()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for the stderr-deadlock class: huge stderr + tiny stdout.
    /// The pre-fix runner read stdout to EOF before touching stderr, which
    /// hung forever here (64 KiB pipe fills at ~4k lines).
    #[test]
    fn drains_stderr_without_deadlock() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo tiny; i=0; while [ $i -lt 20000 ]; do echo boom >&2; i=$((i+1)); done");
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c2 = count.clone();
        let status = pump_child("t", cmd, move |_| {
            c2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        })
        .unwrap();
        assert!(status.success());
        assert!(count.load(std::sync::atomic::Ordering::Relaxed) >= 20001);
    }

    #[test]
    fn reports_failure_status() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo out; echo err >&2; exit 3");
        let status = pump_child("sh", cmd, |_| {}).unwrap();
        assert_eq!(status.code(), Some(3));
    }
}

