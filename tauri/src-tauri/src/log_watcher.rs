use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::interval;

#[derive(Debug, Clone)]
// The shared "Changed" postfix is deliberate: it matches the other monitors'
// event enums and lib.rs match arms; renaming buys nothing but churn.
#[allow(clippy::enum_variant_names)]
pub enum LogEvent {
    MuteChanged(bool),
    MeetingChanged(bool),
    PresenceChanged(String),
}

pub fn start(tx: mpsc::Sender<LogEvent>) {
    tauri::async_runtime::spawn(poll_loop(tx));
}

/// How often to rescan the log directory for a rotated/newer file. The 250 ms
/// tick below only tails the already-open handle; a full directory scan
/// (read_dir + metadata per file) 4×/s was the most expensive idle work in the
/// app, and rotation being noticed a few seconds late is harmless.
const LOG_RESCAN: Duration = Duration::from_secs(5);

async fn poll_loop(tx: mpsc::Sender<LogEvent>) {
    let mut current_file: Option<PathBuf> = None;
    let mut file_handle: Option<(BufReader<File>, u64)> = None;
    let mut in_call = false;

    let mut tick = interval(Duration::from_millis(250));
    let mut latest_cached: Option<PathBuf> = None;
    let mut next_scan = tokio::time::Instant::now();

    loop {
        tick.tick().await;

        if tokio::time::Instant::now() >= next_scan {
            next_scan = tokio::time::Instant::now() + LOG_RESCAN;
            latest_cached = find_latest_log();
        }
        let latest = match latest_cached.clone() {
            Some(p) => p,
            None => continue,
        };

        // Switched to a new log file
        if current_file.as_deref() != Some(&latest) {
            log::info!("LogWatcher: opening {}", latest.display());
            match File::open(&latest) {
                Ok(f) => {
                    let mut reader = BufReader::new(f);
                    // Scan the last 256 KB for the most recent presence entry
                    // before tailing, so we report current status immediately.
                    if let Some(presence) = scan_last_presence(&mut reader) {
                        log::info!("LogWatcher: initial presence → {presence}");
                        let _ = tx.send(LogEvent::PresenceChanged(presence)).await;
                    }
                    let end = reader.seek(SeekFrom::End(0)).unwrap_or(0);
                    file_handle = Some((reader, end));
                    current_file = Some(latest);
                }
                Err(e) => {
                    log::warn!("LogWatcher: cannot open log: {e}");
                    continue;
                }
            }
        }

        if let Some((reader, _pos)) = &mut file_handle {
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        process_line(line.trim(), &tx, &mut in_call).await;
                    }
                    Err(e) => {
                        log::warn!("LogWatcher: read error: {e}");
                        break;
                    }
                }
            }
        }
    }
}

async fn process_line(line: &str, tx: &mpsc::Sender<LogEvent>, in_call: &mut bool) {
    if line.contains("NotifyCallMuteStateChanged") {
        let muted = line.contains("muteState: true");
        log::debug!("LogWatcher: mute → {muted}");
        let _ = tx.send(LogEvent::MuteChanged(muted)).await;
    } else if line.contains("NotifyCallActive") {
        log::info!("LogWatcher: call active");
        *in_call = true;
        let _ = tx.send(LogEvent::MeetingChanged(true)).await;
    } else if line.contains("CallEnded") || line.contains("NotifyCallEnded") {
        log::info!("LogWatcher: call ended");
        *in_call = false;
        let _ = tx.send(LogEvent::MeetingChanged(false)).await;
    } else if line.contains("UserPresenceAction") {
        if let Some(status) = extract_presence(line) {
            log::debug!("LogWatcher: presence → {status}");
            let _ = tx.send(LogEvent::PresenceChanged(status)).await;
        }
    }
}

/// Read the last 256 KB of the log file and return the most recent presence value.
fn scan_last_presence(reader: &mut BufReader<File>) -> Option<String> {
    const SCAN_BYTES: u64 = 256 * 1024;
    let file_len = reader.seek(SeekFrom::End(0)).ok()?;
    let start = file_len.saturating_sub(SCAN_BYTES);
    reader.seek(SeekFrom::Start(start)).ok()?;

    let mut last = None;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if line.contains("UserPresenceAction") {
                    if let Some(s) = extract_presence(line.trim()) {
                        last = Some(s);
                    }
                }
            }
            Err(_) => break,
        }
    }
    last
}

/// Return the presence keyword closest to the end of the line. Transition lines
/// ("from Busy to Available") name two statuses and the new one comes last —
/// first-match-in-list returned the OLD status for those. Matches are word-bounded
/// so e.g. "Available" never matches inside "Unavailable".
fn extract_presence(line: &str) -> Option<String> {
    const STATUSES: [&str; 6] = [
        "Busy", "Available", "Away", "DoNotDisturb", "BeRightBack", "Offline",
    ];
    let bytes = line.as_bytes();
    let mut best: Option<(usize, &str)> = None;
    for status in STATUSES {
        let mut from = 0;
        while let Some(rel) = line[from..].find(status) {
            let idx = from + rel;
            let end = idx + status.len();
            let bounded = (idx == 0 || !bytes[idx - 1].is_ascii_alphabetic())
                && (end >= bytes.len() || !bytes[end].is_ascii_alphabetic());
            if bounded && best.is_none_or(|(b, _)| idx >= b) {
                best = Some((idx, status));
            }
            from = end;
        }
    }
    best.map(|(_, s)| s.to_string())
}

fn find_latest_log() -> Option<PathBuf> {
    // Classic-Teams fallback (…\Microsoft\Teams\logs.txt) removed: classic Teams
    // was retired by Microsoft in 2024; the packaged new-Teams dir is the only
    // log source left. read_dir on a missing dir simply yields None.
    let teams_appdata = std::env::var("LOCALAPPDATA").ok()?;
    let log_dir = PathBuf::from(&teams_appdata).join("Packages")
        .join("MSTeams_8wekyb3d8bbwe")
        .join("LocalCache")
        .join("Microsoft")
        .join("MSTeams")
        .join("Logs");

    std::fs::read_dir(&log_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("MSTeams_")
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}
