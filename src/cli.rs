//! Running the `waydroid` command.

use std::fs::{self, File};
use std::io::{self, Read, Seek};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use tokio::process::{Child, Command};
use zbus::Connection;

use crate::notify;

/// `session start` only returns when the session ends, so it only counts as
/// failed if it exits within this long (pinned by
/// `harness::command_errors::session_start_exiting_zero_early_is_notified`,
/// `harness::command_errors::lasting_session_start_is_not_notified`).
const START_GRACE: Duration = Duration::from_secs(5);
const ERROR_LINES_SHOWN: usize = 5;

/// Runs `waydroid` in the background and shows a notification if it fails.
pub fn spawn_waydroid(args: &[&str], session: Connection) {
    let summary = format!("waydroid {} failed", args.join(" "));
    let starts_session = args == ["session", "start"];
    // A file, not a pipe: a session outliving the tray would get EPIPE writing to one.
    let stderr = scratch_file();
    let child = Command::new("waydroid")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(
            stderr
                .as_ref()
                .and_then(|file| file.try_clone().ok())
                .map_or_else(Stdio::null, Stdio::from),
        )
        // Own process group, so quitting the tray doesn't take a session with it.
        .process_group(0)
        .spawn();
    tokio::spawn(async move {
        let mut child = match child {
            Ok(child) => child,
            Err(err) => return notify::failure(&session, &summary, &err.to_string()).await,
        };
        let Some(status) = failed_exit(&mut child, starts_session).await else {
            return;
        };
        let body = stderr.and_then(last_lines).unwrap_or_else(|| match status {
            Ok(status) => status.to_string(),
            Err(err) => err.to_string(),
        });
        notify::failure(&session, &summary, &body).await;
    });
}

/// Waits for `child` and returns its exit if that was a failure. A session
/// start that outlasts `START_GRACE` is waited out and never fails.
async fn failed_exit(child: &mut Child, starts_session: bool) -> Option<io::Result<ExitStatus>> {
    if starts_session {
        let early = tokio::time::timeout(START_GRACE, child.wait()).await;
        if early.is_err() {
            let _ = child.wait().await;
        }
        return early.ok();
    }
    match child.wait().await {
        Ok(status) if status.success() => None,
        status => Some(status),
    }
}

const MAX_STDERR_BYTES: u64 = 64 * 1024;

/// An already-deleted temp file, to capture a child's stderr in.
fn scratch_file() -> Option<File> {
    use std::os::unix::fs::OpenOptionsExt;

    static COUNT: AtomicU32 = AtomicU32::new(0);
    let name = format!(
        "waydroid-tray-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    );
    let path = std::env::temp_dir().join(name);
    let file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .ok()?;
    fs::remove_file(&path).ok()?;
    Some(file)
}

fn last_lines(mut file: File) -> Option<String> {
    let len = file.metadata().ok().map_or(0, |m| m.len());
    if len > MAX_STDERR_BYTES {
        file.seek(io::SeekFrom::End(-(MAX_STDERR_BYTES as i64)))
            .ok()?;
    } else {
        file.rewind().ok()?;
    }
    let mut bytes = Vec::new();
    file.take(MAX_STDERR_BYTES).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(ERROR_LINES_SHOWN)..].join("\n");
    (!tail.is_empty()).then_some(tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn scratch_file_is_private_and_unlinked() {
        let mut f = scratch_file().expect("scratch file created");
        let meta = f.metadata().unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert_eq!(std::os::unix::fs::MetadataExt::nlink(&meta), 0);
        writeln!(f, "line1\nline2").unwrap();
        assert_eq!(last_lines(f).as_deref(), Some("line1\nline2"));
    }

    #[test]
    fn last_lines_caps_oversized_stderr_to_tail() {
        let mut f = scratch_file().expect("scratch file created");
        let filler = "x".repeat(1024);
        for _ in 0..100 {
            writeln!(f, "{filler}").unwrap();
        }
        writeln!(f, "tail-a\ntail-b").unwrap();
        let got = last_lines(f).expect("last_lines returned Some");
        assert!(got.ends_with("tail-a\ntail-b"), "{got}");
    }
}
