use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, MissedTickBehavior};

#[derive(Debug, Clone)]
// The shared "Changed" postfix is deliberate: it matches the other monitors'
// event enums and lib.rs match arms; renaming buys nothing but churn.
#[allow(clippy::enum_variant_names)]
pub enum LogEvent {
    MuteChanged(bool),
    MeetingChanged(bool),
    PresenceChanged(String),
}

/// Call-lifecycle bookkeeping keyed on Teams' own call ids.
///
/// Teams runs calls concurrently: an incoming call that rings during a meeting
/// is a second call, and declining (or missing) it logs `reportIncomingCall` +
/// `NotifyCallEnded` for that id — without ever logging `NotifyCallActive`.
/// The old boolean treated *any* end-line as "the meeting is over", so a
/// declined call ended the running meeting in HA (incident 2026-07-30).
/// Only ids that were seen active may close the call state, and only when the
/// last one is gone.
#[derive(Default)]
struct CallState {
    /// Ids seen in a `NotifyCallActive` line, with the moment they were seen —
    /// the timestamp exists so `expire` can drop a call whose end-line never
    /// arrived (see `STALE_CALL`).
    active: HashMap<String, Instant>,
    /// Set when a `NotifyCallActive` line carried no parseable id (format
    /// drift): fall back to the pre-id semantics where any end-line closes the
    /// call. Carries the same timestamp, for the same reason.
    legacy: Option<Instant>,
}

impl CallState {
    fn in_call(&self) -> bool {
        self.legacy.is_some() || !self.active.is_empty()
    }

    fn clear(&mut self) {
        self.active.clear();
        self.legacy = None;
    }

    /// Feed one log line through the call state machine. Returns the new
    /// in-call value when the line *transitions* it, None otherwise (either
    /// not a call line, or a call line that doesn't change the outcome).
    fn apply(&mut self, line: &str) -> Option<bool> {
        self.apply_at(line, Instant::now())
    }

    fn apply_at(&mut self, line: &str, now: Instant) -> Option<bool> {
        let was = self.in_call();
        if line.contains("NotifyCallActive") || line.contains("reportCallActive") {
            match extract_call_id(line) {
                Some(id) => {
                    log::info!("LogWatcher: call active ({id})");
                    self.active.insert(id, now);
                    // An id-bearing line supersedes any id-less line from the
                    // same activation batch (Teams writes both, order varies).
                    self.legacy = None;
                }
                None => {
                    if self.active.is_empty() {
                        log::warn!(
                            "LogWatcher: call active without parseable id — legacy mode"
                        );
                        self.legacy = Some(now);
                    } else {
                        // Teams logs several NotifyCallActive lines per
                        // activation and only the Hfp one carries the call id
                        // ("CallInfo: NotifyCallActive causeId: …", "CallInfo:
                        // CallTracker: Calling NotifyCallActive without
                        // deviceId…"). While id'd calls are tracked these are
                        // duplicates, not a format change.
                        log::debug!("LogWatcher: ignoring id-less active-line while tracking ids");
                    }
                }
            }
        } else if line.contains("CallEnded") || line.contains("NotifyCallEnded") {
            match extract_call_id(line) {
                Some(id) => {
                    if self.active.remove(&id).is_some() {
                        log::info!("LogWatcher: call ended ({id})");
                    } else if self.active.is_empty() && self.legacy.is_some() {
                        // An id-less active call is closed by whichever
                        // end-line arrives first.
                        log::info!("LogWatcher: call ended (legacy, {id})");
                        self.legacy = None;
                    } else {
                        // End of a call that never went active here: a
                        // declined/missed incoming call, or one of the
                        // duplicate end-lines Teams writes per call. Must
                        // not touch the running meeting.
                        log::debug!("LogWatcher: ignoring end of inactive call {id}");
                    }
                }
                None => {
                    if self.active.is_empty() {
                        if self.legacy.is_some() {
                            log::info!("LogWatcher: call ended (no id)");
                        }
                        self.legacy = None;
                    } else {
                        // We are tracking id'd calls; an end-line without an
                        // id is log noise, not one of ours.
                        log::debug!("LogWatcher: ignoring id-less end-line");
                    }
                }
            }
        } else {
            return None;
        }
        let in_call = self.in_call();
        (in_call != was).then_some(in_call)
    }

    /// Drop calls that have been active longer than `max_age`, and report the
    /// new in-call value when that ends the meeting.
    ///
    /// The floor under the rotation fix below: a `NotifyCallEnded` that never
    /// reaches us pins `is_in_meeting` on until Teams exits, and Teams survives
    /// hibernation, so "until Teams exits" was 10 days in the 2026-08-17
    /// incident. A Teams call does not run for half a day; a pinned state does.
    fn expire(&mut self, max_age: Duration) -> Option<bool> {
        self.expire_at(Instant::now(), max_age)
    }

    fn expire_at(&mut self, now: Instant, max_age: Duration) -> Option<bool> {
        let was = self.in_call();
        self.active.retain(|id, seen| {
            let keep = now.duration_since(*seen) <= max_age;
            if !keep {
                log::warn!("LogWatcher: dropping call {id} — active for over {max_age:?} with no end-line");
            }
            keep
        });
        if self
            .legacy
            .is_some_and(|seen| now.duration_since(seen) > max_age)
        {
            log::warn!("LogWatcher: dropping id-less call — active for over {max_age:?}");
            self.legacy = None;
        }
        let in_call = self.in_call();
        (in_call != was).then_some(in_call)
    }
}

/// Pull the 36-char GUID following a `callId: ` (HfpVoipCallCoordinatorImpl
/// lines) or `fired: ` (TeamsCallTracker lines) marker. The GUID is often
/// glued straight onto the next field (`…fa77causeId: …`), so take exactly
/// the GUID shape rather than splitting on whitespace. None on format drift —
/// callers then fall back to the legacy any-end-closes-the-call semantics.
fn extract_call_id(line: &str) -> Option<String> {
    let start = ["callId: ", "fired: "]
        .iter()
        .find_map(|m| line.find(m).map(|i| i + m.len()))?;
    let id: String = line.get(start..)?.chars().take(36).collect();
    is_guid(&id).then_some(id)
}

fn is_guid(id: &str) -> bool {
    id.len() == 36
        && id.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

pub fn start(tx: mpsc::Sender<LogEvent>, teams_running: watch::Receiver<bool>) {
    tauri::async_runtime::spawn(poll_loop(tx, teams_running));
}

/// How often to rescan the log directory for a rotated/newer file. The 250 ms
/// tick below only tails the already-open handle; a full directory scan
/// (read_dir + metadata per file) 4×/s was the most expensive idle work in the
/// app, and rotation being noticed a few seconds late is harmless.
const LOG_RESCAN: Duration = Duration::from_secs(5);

/// A tick arriving this much late means the process was frozen in between: the
/// machine slept. Same signal (and the same reasoning) as `registry_monitor`.
const RESUME_GAP: Duration = Duration::from_secs(60);

/// How long a call may stay active without an end-line before it is dropped as
/// stale. See `CallState::expire`.
const STALE_CALL: Duration = Duration::from_secs(12 * 60 * 60);

async fn poll_loop(tx: mpsc::Sender<LogEvent>, mut teams_running: watch::Receiver<bool>) {
    let mut current_file: Option<PathBuf> = None;
    let mut file_handle: Option<BufReader<File>> = None;
    let mut calls = CallState::default();

    let mut tick = interval(Duration::from_millis(250));
    // No catch-up burst of ticks after a suspend.
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut latest_cached: Option<PathBuf> = None;
    let mut next_scan = tokio::time::Instant::now();
    let mut previous_tick = Instant::now();

    loop {
        tick.tick().await;

        // A Teams exit (crash or quit) never writes end-lines for calls that
        // were still running — drop them, or a stale id would keep the call
        // set non-empty forever and pin is_in_meeting on.
        if teams_running.has_changed().unwrap_or(false)
            && !*teams_running.borrow_and_update()
        {
            let was = calls.in_call();
            calls.clear();
            if was {
                log::info!("LogWatcher: Teams stopped — clearing active call state");
                let _ = tx.send(LogEvent::MeetingChanged(false)).await;
            }
        }

        // A call cannot survive a suspend: the network drops and Teams tears it
        // down, usually without us ever reading the end-line. Whatever is still
        // "active" here is therefore stale by definition.
        if previous_tick.elapsed() > RESUME_GAP {
            let was = calls.in_call();
            calls.clear();
            if was {
                log::info!("LogWatcher: resume detected — clearing active call state");
                let _ = tx.send(LogEvent::MeetingChanged(false)).await;
            }
        }
        previous_tick = Instant::now();

        if tokio::time::Instant::now() >= next_scan {
            next_scan = tokio::time::Instant::now() + LOG_RESCAN;
            latest_cached = find_latest_log();
            if let Some(in_call) = calls.expire(STALE_CALL) {
                let _ = tx.send(LogEvent::MeetingChanged(in_call)).await;
            }
        }
        let latest = match latest_cached.clone() {
            Some(p) => p,
            None => continue,
        };

        // Switched to a new log file
        if current_file.as_deref() != Some(&latest) {
            // Teams rotates at 2 MB, which under call load is every few
            // minutes, and rotation is noticed up to LOG_RESCAN late. Drain
            // what is left in the old handle, then read the rotated file from
            // byte 0: everything in it was written after we opened its
            // predecessor. Seeking to the end here instead is how a
            // NotifyCallEnded went missing and pinned is_in_meeting on for ten
            // days (2026-08-17) — ~40 KB of log per rotation was skipped.
            let rotated = current_file.is_some();
            if let Some(reader) = &mut file_handle {
                drain(reader, &tx, &mut calls).await;
            }
            match open_log(&latest, rotated, &tx).await {
                Some(reader) => {
                    file_handle = Some(reader);
                    current_file = Some(latest);
                }
                None => continue,
            }
        }

        if let Some(reader) = &mut file_handle {
            drain(reader, &tx, &mut calls).await;
        }
    }
}

/// Open a log file for tailing, positioned according to why we are opening it.
///
/// `rotated` = we were already tailing a predecessor, so this file was created
/// moments ago and every line in it is news: start at byte 0. Otherwise this is
/// the first file of the run, which can be hours of history that must not be
/// replayed as if it were happening now: take the last known presence from it
/// and tail from the end.
async fn open_log(
    path: &Path,
    rotated: bool,
    tx: &mpsc::Sender<LogEvent>,
) -> Option<BufReader<File>> {
    let mut reader = match File::open(path) {
        Ok(f) => BufReader::new(f),
        Err(e) => {
            log::warn!("LogWatcher: cannot open log: {e}");
            return None;
        }
    };
    if rotated {
        log::info!("LogWatcher: rotation → {}", path.display());
    } else {
        log::info!("LogWatcher: opening {}", path.display());
        if let Some(presence) = scan_last_presence(&mut reader) {
            log::info!("LogWatcher: initial presence → {presence}");
            let _ = tx.send(LogEvent::PresenceChanged(presence)).await;
        }
        if let Err(e) = reader.seek(SeekFrom::End(0)) {
            log::warn!("LogWatcher: cannot seek to end: {e}");
        }
    }
    Some(reader)
}

/// Feed every line available on `reader` through the state machine, leaving the
/// handle at EOF so the next call resumes exactly where this one stopped.
async fn drain(
    reader: &mut BufReader<File>,
    tx: &mpsc::Sender<LogEvent>,
    calls: &mut CallState,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => process_line(line.trim(), tx, calls).await,
            Err(e) => {
                log::warn!("LogWatcher: read error: {e}");
                break;
            }
        }
    }
}

async fn process_line(line: &str, tx: &mpsc::Sender<LogEvent>, calls: &mut CallState) {
    if line.contains("NotifyCallMuteStateChanged") {
        let muted = line.contains("muteState: true");
        log::debug!("LogWatcher: mute → {muted}");
        let _ = tx.send(LogEvent::MuteChanged(muted)).await;
    } else if let Some(in_call) = calls.apply(line) {
        let _ = tx.send(LogEvent::MeetingChanged(in_call)).await;
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

#[cfg(test)]
mod tests {
    use super::{drain, extract_call_id, open_log, CallState, LogEvent, STALE_CALL};
    use std::io::Write;
    use std::time::{Duration, Instant};

    // All lines are verbatim from the Teams logs of 2026-07-30: the end/ring
    // lines from the incident that motivated id-tracking, the active lines
    // from the v1.4.3 field test the same day (ids swapped for consistency).
    // Teams writes THREE NotifyCallActive lines per activation; only the Hfp
    // one carries the call id.
    const ACTIVE_MEETING: &str =
        "HfpVoipCallCoordinatorImpl: NotifyCallActive callId: d84becb7-4285-4d44-9d4d-e61364d07d11causeId: 5478474f-3fc5-444f-b895-4c3d96476fa8";
    const ACTIVE_DUP_CAUSE: &str =
        "CallInfo: NotifyCallActive causeId: 5478474f-3fc5-444f-b895-4c3d96476fa8";
    const ACTIVE_DUP_NO_DEVICE: &str =
        "CallInfo: CallTracker: Calling NotifyCallActive without deviceId, deviceId is empty";
    const INCOMING_RING: &str =
        "HfpVoipCallCoordinatorImpl: reportIncomingCall for callId: c9158e4a-9792-4685-8671-30226038fa77";
    const INCOMING_ENDED: &str =
        "HfpVoipCallCoordinatorImpl: NotifyCallEnded callId: c9158e4a-9792-4685-8671-30226038fa77causeId: bae9fd1b-aece-4163-a999-0db507f8de2c";
    const MEETING_ENDED: &str =
        "HfpVoipCallCoordinatorImpl: NotifyCallEnded callId: d84becb7-4285-4d44-9d4d-e61364d07d11causeId: a879e043-6006-4daf-add5-d816bc102653";
    const MEETING_ENDED_TRACKER: &str =
        "TeamsCallTracker: CallEnded fired: d84becb7-4285-4d44-9d4d-e61364d07d11";

    #[test]
    fn extracts_guid_glued_to_cause_id() {
        assert_eq!(
            extract_call_id(INCOMING_ENDED).as_deref(),
            Some("c9158e4a-9792-4685-8671-30226038fa77")
        );
    }

    #[test]
    fn extracts_guid_from_tracker_line() {
        assert_eq!(
            extract_call_id(MEETING_ENDED_TRACKER).as_deref(),
            Some("d84becb7-4285-4d44-9d4d-e61364d07d11")
        );
    }

    #[test]
    fn rejects_lines_without_guid() {
        assert_eq!(extract_call_id("NotifyCallEnded callId: not-a-guid"), None);
        assert_eq!(extract_call_id("CallEnded without any marker"), None);
    }

    #[test]
    fn declined_incoming_call_does_not_end_running_meeting() {
        // The 2026-07-30 incident, replayed line for line.
        let mut calls = CallState::default();
        assert_eq!(calls.apply(ACTIVE_MEETING), Some(true));
        assert_eq!(calls.apply(INCOMING_RING), None); // ring is not a call line
        assert_eq!(calls.apply(INCOMING_ENDED), None); // must NOT end the meeting
        assert!(calls.in_call());
        assert_eq!(calls.apply(MEETING_ENDED), Some(false));
        // Teams' duplicate end-line for the same call stays silent.
        assert_eq!(calls.apply(MEETING_ENDED_TRACKER), None);
    }

    #[test]
    fn duplicate_active_and_end_lines_are_idempotent() {
        let mut calls = CallState::default();
        assert_eq!(calls.apply(ACTIVE_MEETING), Some(true));
        assert_eq!(calls.apply(ACTIVE_MEETING), None);
        assert_eq!(calls.apply(MEETING_ENDED), Some(false));
        assert_eq!(calls.apply(MEETING_ENDED), None);
    }

    #[test]
    fn end_of_never_active_call_alone_stays_silent() {
        // Declined incoming call while NOT in a meeting: nothing to end.
        let mut calls = CallState::default();
        assert_eq!(calls.apply(INCOMING_ENDED), None);
        assert!(!calls.in_call());
    }

    #[test]
    fn legacy_mode_without_ids_keeps_old_semantics() {
        let mut calls = CallState::default();
        assert_eq!(calls.apply("NotifyCallActive (new format?)"), Some(true));
        assert!(calls.in_call());
        // Any end-line closes a legacy call — id'd or not.
        assert_eq!(calls.apply("CallEnded (new format?)"), Some(false));

        assert_eq!(calls.apply("NotifyCallActive (new format?)"), Some(true));
        assert_eq!(calls.apply(INCOMING_ENDED), Some(false));
    }

    #[test]
    fn duplicate_idless_active_lines_do_not_arm_legacy_mode() {
        // Field test 2026-07-30 on v1.4.3: the two CallInfo lines armed legacy
        // mode, which would have let a declined incoming call end the meeting
        // again — the exact bug this module exists to fix.
        let mut calls = CallState::default();
        assert_eq!(calls.apply(ACTIVE_MEETING), Some(true));
        assert_eq!(calls.apply(ACTIVE_DUP_CAUSE), None);
        assert_eq!(calls.apply(ACTIVE_DUP_NO_DEVICE), None);
        assert_eq!(calls.apply(INCOMING_ENDED), None);
        assert!(calls.in_call());
        assert_eq!(calls.apply(MEETING_ENDED), Some(false));
    }

    #[test]
    fn id_bearing_active_line_supersedes_legacy_from_same_batch() {
        // Same activation batch, order flipped: an id-less line arms legacy,
        // the id-bearing line for the same call takes over cleanly.
        let mut calls = CallState::default();
        assert_eq!(calls.apply(ACTIVE_DUP_NO_DEVICE), Some(true));
        assert_eq!(calls.apply(ACTIVE_MEETING), None);
        assert_eq!(calls.apply(MEETING_ENDED), Some(false));
    }

    #[test]
    fn idless_end_line_is_noise_while_tracking_ids() {
        let mut calls = CallState::default();
        assert_eq!(calls.apply(ACTIVE_MEETING), Some(true));
        assert_eq!(calls.apply("SomeTelemetry: CallEndedReason summary"), None);
        assert!(calls.in_call());
    }

    #[test]
    fn clear_resets_everything() {
        let mut calls = CallState::default();
        calls.apply(ACTIVE_MEETING);
        calls.clear();
        assert!(!calls.in_call());
        // A late end-line for the cleared call is ignored.
        assert_eq!(calls.apply(MEETING_ENDED), None);
    }

    // Instants are only ever moved forward here: `Instant::now() - 13h` panics
    // on a machine that booted less than 13 hours ago, which is every CI runner.
    #[test]
    fn a_call_without_an_end_line_expires() {
        let t0 = Instant::now();
        let mut calls = CallState::default();
        assert_eq!(calls.apply_at(ACTIVE_MEETING, t0), Some(true));

        // Still inside the ceiling: a long meeting is a real meeting.
        assert_eq!(calls.expire_at(t0 + Duration::from_secs(11 * 3600), STALE_CALL), None);
        assert!(calls.in_call());

        // Past it: this is the 2026-08-17 pin, and it must end by itself.
        assert_eq!(calls.expire_at(t0 + Duration::from_secs(13 * 3600), STALE_CALL), Some(false));
        assert!(!calls.in_call());
        // Idempotent: no second event once it is gone.
        assert_eq!(calls.expire_at(t0 + Duration::from_secs(14 * 3600), STALE_CALL), None);
    }

    #[test]
    fn an_idless_call_expires_too() {
        let t0 = Instant::now();
        let mut calls = CallState::default();
        assert_eq!(calls.apply_at("NotifyCallActive (new format?)", t0), Some(true));
        assert_eq!(calls.expire_at(t0 + Duration::from_secs(13 * 3600), STALE_CALL), Some(false));
    }

    #[test]
    fn expiring_one_of_two_calls_keeps_the_meeting_running() {
        let t0 = Instant::now();
        let mut calls = CallState::default();
        assert_eq!(calls.apply_at(ACTIVE_MEETING, t0), Some(true));
        let later = t0 + Duration::from_secs(11 * 3600);
        assert_eq!(
            calls.apply_at(
                "HfpVoipCallCoordinatorImpl: NotifyCallActive callId: c9158e4a-9792-4685-8671-30226038fa77causeId: x",
                later
            ),
            None
        );
        // The first call ages out, the second is young: still in a meeting.
        assert_eq!(calls.expire_at(t0 + Duration::from_secs(13 * 3600), STALE_CALL), None);
        assert!(calls.in_call());
    }

    /// Write `lines` to a uniquely named file in the temp dir.
    fn temp_log(name: &str, lines: &[&str]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("teams2ha-test-{name}.log"));
        let mut f = std::fs::File::create(&path).expect("create temp log");
        for line in lines {
            writeln!(f, "{line}").expect("write temp log");
        }
        path
    }

    // The 2026-08-17 incident: Teams rotates its log every few minutes while a
    // call runs, and the watcher used to seek to the end of every file it opened
    // — including a rotated one, discarding the ~40 KB written before it noticed.
    // A NotifyCallEnded in that window left is_in_meeting pinned on for ten days.
    #[tokio::test]
    async fn a_rotated_file_is_read_from_the_start() {
        let path = temp_log("rotated", &[ACTIVE_MEETING, MEETING_ENDED]);
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mut calls = CallState::default();

        let mut reader = open_log(&path, true, &tx).await.expect("open");
        drain(&mut reader, &tx, &mut calls).await;

        assert!(matches!(rx.try_recv(), Ok(LogEvent::MeetingChanged(true))));
        assert!(matches!(rx.try_recv(), Ok(LogEvent::MeetingChanged(false))));
        assert!(!calls.in_call());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn the_first_file_of_the_run_is_not_replayed() {
        // Same content, but this file predates the app: replaying it would
        // announce a meeting that ended before the app was even started.
        let path = temp_log("first-open", &[ACTIVE_MEETING, MEETING_ENDED]);
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mut calls = CallState::default();

        let mut reader = open_log(&path, false, &tx).await.expect("open");
        drain(&mut reader, &tx, &mut calls).await;

        assert!(rx.try_recv().is_err(), "history must not be replayed");
        assert!(!calls.in_call());
        let _ = std::fs::remove_file(path);
    }
}
