//! Running the `waydroid` command.

use std::fs::{self, File};
use std::io::{Read, Seek};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use tokio::process::Command;
use zbus::Connection;

use crate::notify;

/// `session start` only returns when the session ends, so it only counts as
/// failed if it exits within this long.
const START_GRACE: Duration = Duration::from_secs(5);
/// How much of a failed command's stderr to show.
const ERROR_LINES: usize = 5;

/// Runs `waydroid` in the background and shows a notification if it fails.
pub fn spawn_waydroid(args: &[&str], session: Connection) {
    let command = format!("waydroid {}", args.join(" "));
    let starts_session = args == ["session", "start"];
    // A file rather than a pipe: the session outlives `session start`'s grace
    // period, and would get EPIPE writing to a pipe after the tray quits.
    let stderr = scratch_file();
    let child = Command::new("waydroid")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr.as_ref().and_then(|file| file.try_clone().ok()).map_or_else(Stdio::null, Stdio::from))
        // Own process group, so quitting the tray doesn't take a session with it.
        .process_group(0)
        .spawn();
    tokio::spawn(async move {
        let summary = format!("{command} failed");
        let status = match child {
            Err(err) => return notify::failure(&session, &summary, &err.to_string()).await,
            Ok(mut child) if starts_session => {
                match tokio::time::timeout(START_GRACE, child.wait()).await {
                    // Still going, so the session started. Reap it when it ends.
                    Err(_) => return drop(child.wait().await),
                    // A started session doesn't return, so exiting this soon is
                    // a failure even with status 0, which is what Waydroid exits
                    // with when the container isn't listening.
                    Ok(status) => status,
                }
            }
            Ok(mut child) => match child.wait().await {
                Ok(status) if status.success() => return,
                status => status,
            },
        };
        let body = stderr.and_then(last_lines).unwrap_or_else(|| match status {
            Ok(status) => status.to_string(),
            Err(err) => err.to_string(),
        });
        notify::failure(&session, &summary, &body).await;
    });
}

/// An already-deleted temp file, to capture a child's stderr in.
fn scratch_file() -> Option<File> {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let name = format!("waydroid-tray-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed));
    let path = std::env::temp_dir().join(name);
    let file = File::options().read(true).write(true).create_new(true).open(&path).ok()?;
    fs::remove_file(&path).ok()?;
    Some(file)
}

fn last_lines(mut file: File) -> Option<String> {
    let mut text = String::new();
    file.rewind().ok()?;
    file.read_to_string(&mut text).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(ERROR_LINES)..].join("\n");
    (!tail.is_empty()).then_some(tail)
}
