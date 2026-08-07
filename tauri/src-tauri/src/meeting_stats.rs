//! Per-day meeting totals, accumulated locally and kept on disk.
//!
//! `home_network` pauses MQTT whenever the machine is not on the home network, so
//! a day spent at the office publishes nothing at all: Home Assistant sees the
//! entities as unavailable and a helper that measures how long `isinmeeting` was
//! `on` reads that day as zero. The OS monitors keep running away from home
//! though — the app knows exactly when a meeting starts and ends wherever it is.
//!
//! So the totals are accumulated here instead of in Home Assistant. They survive
//! restarts and away-from-home stretches on disk, and are published as plain
//! counters on the next connection from home, which backfills the office time.

use crate::settings::data_dir;
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// How often the accumulator is ticked while the app runs. Also the worst-case
/// loss when the app is killed mid-meeting: totals are written on every change.
pub const TICK: Duration = Duration::from_secs(60);

/// Ceiling on what a single observation may add. Sleep, hibernate and a process
/// that was not scheduled for a while all look the same from here — a long gap
/// between two observations — and time spent in Modern Standby is not meeting
/// time. Anything longer than two ticks is treated as a gap and contributes only
/// those two ticks.
const MAX_STEP_MS: i64 = 2 * TICK.as_millis() as i64;

/// Milliseconds in a hundredth of an hour (36 s), the resolution published.
const MS_PER_CENTIHOUR: i64 = 36_000;

/// Days kept on disk. "This week" never needs more than 7; the rest is slack so
/// a laptop that was off for a while still reports a sane week total.
const RETAIN_DAYS: i64 = 21;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DayTotals {
    /// Milliseconds, not seconds: an observation arrives on every monitor event,
    /// far more often than once a second, and truncating each one to whole
    /// seconds would quietly lose most of a meeting.
    pub ms: i64,
    pub count: u32,
}

/// What gets published. Hundredths of an hour rather than milliseconds, so that
/// "changed since the last publish" means "changed in the value HA will see" —
/// at millisecond resolution every single observation would look like a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub today_centihours: i64,
    pub today_count: u32,
    pub week_centihours: i64,
}

impl Snapshot {
    pub fn today_hours(&self) -> f64 {
        self.today_centihours as f64 / 100.0
    }

    pub fn week_hours(&self) -> f64 {
        self.week_centihours as f64 / 100.0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MeetingStats {
    days: BTreeMap<NaiveDate, DayTotals>,
    /// Wall clock of the previous observation. Deliberately not persisted: the
    /// time between two runs of the app is not meeting time, so a fresh start
    /// must not credit the gap since the last write.
    #[serde(skip)]
    last_observed: Option<DateTime<Local>>,
    /// Meeting state at the previous observation, for edge detection. Also not
    /// persisted — a call cannot survive a restart of the app.
    #[serde(skip)]
    in_meeting: bool,
}

impl MeetingStats {
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        let Ok(json) = std::fs::read_to_string(&path) else {
            // Absent on first run, which is not worth a warning.
            return Self::default();
        };
        match serde_json::from_str(&json) {
            Ok(stats) => stats,
            Err(e) => {
                log::warn!("Could not read meeting stats ({e}); starting from zero");
                Self::default()
            }
        }
    }

    /// Write the totals to disk. A couple of KB, and only called when the
    /// published value changed — at most once per tick.
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        let json = match serde_json::to_string_pretty(self) {
            Ok(json) => json,
            Err(e) => {
                log::warn!("Could not serialise meeting stats: {e}");
                return;
            }
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&path, json) {
            log::warn!("Could not write meeting stats: {e}");
        }
    }

    fn path() -> Option<PathBuf> {
        data_dir().map(|dir| dir.join("meeting_stats.json"))
    }

    /// Record the meeting state as observed at `now`. Returns the current
    /// snapshot and whether the stored totals grew, i.e. whether they are worth
    /// writing to disk. Deciding what to *publish* is the caller's job: it knows
    /// whether the last publish actually reached the broker.
    ///
    /// Pruning alone does not count as a change — dropping a day that is out of
    /// the window has no effect on any published value, and the next real change
    /// takes it to disk anyway.
    pub fn observe(&mut self, now: DateTime<Local>, in_meeting: bool) -> (Snapshot, bool) {
        let mut grew = false;
        // Credit the interval that just passed to the state it was spent in,
        // which is the state recorded at the *previous* observation.
        if self.in_meeting {
            if let Some(previous) = self.last_observed {
                grew = self.credit(previous, now) > 0;
            }
        }
        if in_meeting && !self.in_meeting {
            self.days.entry(now.date_naive()).or_default().count += 1;
            grew = true;
        }
        self.in_meeting = in_meeting;
        self.last_observed = Some(now);
        self.prune(now.date_naive());

        (self.snapshot(now), grew)
    }

    pub fn snapshot(&self, now: DateTime<Local>) -> Snapshot {
        let today = now.date_naive();
        let totals = self.days.get(&today).copied().unwrap_or_default();
        // Week starts on Monday, matching the history_stats helpers this replaces.
        let monday =
            today - ChronoDuration::days(i64::from(today.weekday().num_days_from_monday()));
        let week_ms: i64 = self.days.range(monday..=today).map(|(_, t)| t.ms).sum();
        Snapshot {
            today_centihours: totals.ms / MS_PER_CENTIHOUR,
            today_count: totals.count,
            week_centihours: week_ms / MS_PER_CENTIHOUR,
        }
    }

    /// Add the interval between two observations to the day(s) it belongs to.
    /// Returns the milliseconds credited.
    fn credit(&mut self, previous: DateTime<Local>, now: DateTime<Local>) -> i64 {
        let elapsed = (now - previous).num_milliseconds();
        if elapsed <= 0 {
            // The clock stepped backwards (NTP correction, timezone change).
            return 0;
        }
        let credited = elapsed.min(MAX_STEP_MS);
        let mut cursor = now - ChronoDuration::milliseconds(credited);
        while cursor < now {
            let day = cursor.date_naive();
            // A meeting can run across midnight, and the part after it belongs
            // to the new day — otherwise "today" keeps counting last night's call.
            let chunk_end = match next_midnight(day) {
                Some(midnight) if midnight > cursor && midnight < now => midnight,
                _ => now,
            };
            self.days.entry(day).or_default().ms += (chunk_end - cursor).num_milliseconds();
            cursor = chunk_end;
        }
        credited
    }

    fn prune(&mut self, today: NaiveDate) {
        let cutoff = today - ChronoDuration::days(RETAIN_DAYS);
        self.days.retain(|day, _| *day >= cutoff);
    }

    #[cfg(test)]
    fn ms_on(&self, day: NaiveDate) -> i64 {
        self.days.get(&day).map(|t| t.ms).unwrap_or(0)
    }
}

/// Start of the day after `day`, in local time. `None` only when there is no
/// such instant (a timezone shift that skips midnight), in which case the caller
/// keeps the whole interval on the current day.
fn next_midnight(day: NaiveDate) -> Option<DateTime<Local>> {
    let naive = day.succ_opt()?.and_hms_opt(0, 0, 0)?;
    Local.from_local_datetime(&naive).earliest()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// August 2026: the 2nd is a Sunday, the 3rd a Monday, the 5th a Wednesday.
    fn at(day: u32, hour: u32, min: u32, sec: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 8, day, hour, min, sec)
            .single()
            .expect("test timestamps avoid DST transitions")
    }

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, day).unwrap()
    }

    /// Sit in a meeting from 10:00 for `minutes`, observed once per tick — real
    /// time can only be accumulated in tick-sized steps, since a bigger step is
    /// treated as a sleep gap.
    fn run_meeting(stats: &mut MeetingStats, day: u32, minutes: i64) {
        let start = at(day, 10, 0, 0);
        stats.observe(start, true);
        for minute in 1..=minutes {
            stats.observe(start + ChronoDuration::minutes(minute), true);
        }
        stats.observe(start + ChronoDuration::minutes(minutes), false);
    }

    #[test]
    fn time_between_observations_is_credited_to_the_meeting() {
        let mut stats = MeetingStats::default();
        stats.observe(at(5, 10, 0, 0), true);
        stats.observe(at(5, 10, 1, 0), true);
        assert_eq!(stats.ms_on(date(5)), 60_000);
    }

    #[test]
    fn a_sleep_gap_is_not_meeting_time() {
        // The laptop was in Modern Standby for three hours with a call still
        // "running": crediting the whole gap would invent three hours of meeting.
        let mut stats = MeetingStats::default();
        stats.observe(at(5, 10, 0, 0), true);
        stats.observe(at(5, 13, 0, 0), true);
        assert_eq!(stats.ms_on(date(5)), MAX_STEP_MS);
    }

    #[test]
    fn a_meeting_across_midnight_is_split_over_both_days() {
        let mut stats = MeetingStats::default();
        stats.observe(at(5, 23, 59, 30), true);
        stats.observe(at(6, 0, 0, 30), true);
        assert_eq!(stats.ms_on(date(5)), 30_000);
        assert_eq!(stats.ms_on(date(6)), 30_000);
    }

    #[test]
    fn meetings_are_counted_on_the_rising_edge_only() {
        let mut stats = MeetingStats::default();
        stats.observe(at(5, 10, 0, 0), true);
        stats.observe(at(5, 10, 1, 0), true);
        stats.observe(at(5, 10, 2, 0), false);
        stats.observe(at(5, 11, 0, 0), true);
        assert_eq!(stats.snapshot(at(5, 12, 0, 0)).today_count, 2);
    }

    #[test]
    fn time_outside_a_meeting_is_not_credited() {
        let mut stats = MeetingStats::default();
        stats.observe(at(5, 10, 0, 0), false);
        stats.observe(at(5, 10, 1, 0), false);
        assert_eq!(stats.ms_on(date(5)), 0);
    }

    #[test]
    fn a_backwards_clock_step_credits_nothing() {
        let mut stats = MeetingStats::default();
        stats.observe(at(5, 10, 0, 0), true);
        stats.observe(at(5, 9, 0, 0), true);
        assert_eq!(stats.ms_on(date(5)), 0);
    }

    #[test]
    fn the_week_total_starts_on_monday() {
        let mut stats = MeetingStats::default();
        // Half an hour on each of Sunday the 2nd (previous week), Monday the 3rd
        // and Wednesday the 5th.
        for day in [2, 3, 5] {
            run_meeting(&mut stats, day, 30);
        }
        let snapshot = stats.snapshot(at(5, 12, 0, 0));
        assert_eq!(snapshot.today_hours(), 0.5);
        // Monday + Wednesday, not the Sunday before them.
        assert_eq!(snapshot.week_hours(), 1.0);
    }

    #[test]
    fn the_day_total_starts_over_after_midnight() {
        let mut stats = MeetingStats::default();
        run_meeting(&mut stats, 5, 30);
        assert_eq!(stats.snapshot(at(5, 23, 0, 0)).today_hours(), 0.5);
        assert_eq!(stats.snapshot(at(6, 0, 1, 0)).today_hours(), 0.0);
    }

    #[test]
    fn totals_survive_a_round_trip_through_json() {
        let mut stats = MeetingStats::default();
        run_meeting(&mut stats, 5, 30);
        let json = serde_json::to_string(&stats).expect("serialises");
        let restored: MeetingStats = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(restored.ms_on(date(5)), stats.ms_on(date(5)));
        assert_eq!(
            restored.snapshot(at(5, 12, 0, 0)),
            stats.snapshot(at(5, 12, 0, 0))
        );
        // The volatile fields are deliberately left out of the file: the gap
        // between two runs of the app is not meeting time.
        assert!(restored.last_observed.is_none());
        assert!(!restored.in_meeting);
    }

    #[test]
    fn days_beyond_the_retention_window_are_dropped() {
        let mut stats = MeetingStats::default();
        stats.observe(at(1, 10, 0, 0), true);
        stats.observe(at(1, 10, 1, 0), true);
        assert_eq!(stats.ms_on(date(1)), 60_000);
        // Observing a month later prunes it.
        stats.observe(
            at(1, 10, 1, 0) + ChronoDuration::days(RETAIN_DAYS + 1),
            false,
        );
        assert_eq!(stats.ms_on(date(1)), 0);
    }

    #[test]
    fn only_a_growing_total_is_worth_writing_to_disk() {
        let mut stats = MeetingStats::default();
        // Idle: nothing accumulates, so there is nothing to save.
        assert!(!stats.observe(at(5, 10, 0, 0), false).1);
        assert!(!stats.observe(at(5, 10, 1, 0), false).1);
        // Start of a meeting bumps the count...
        assert!(stats.observe(at(5, 10, 2, 0), true).1);
        // ...and each tick inside it adds time.
        assert!(stats.observe(at(5, 10, 3, 0), true).1);
        // The tick that ends it credits the last interval; the one after does not.
        assert!(stats.observe(at(5, 10, 4, 0), false).1);
        assert!(!stats.observe(at(5, 10, 5, 0), false).1);
    }
}
