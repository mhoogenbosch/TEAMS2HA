//! Teams' in-app mute and camera state, read via UI Automation.
//!
//! # Why UIA and not the audio stack
//!
//! Verified on 2026-07-26 against Teams 26183.1903.4892.4448: pressing Teams' own mute
//! button changes **nothing** observable in the audio stack.
//!
//! * `ISimpleAudioVolume` on Teams' capture session — unchanged (that flag is the
//!   volume-mixer per-app mute; muting from the Windows control panel *does* move it,
//!   which is why `wasapi_monitor` is still worth keeping).
//! * `IAudioEndpointVolume` on the capture device — unchanged.
//! * Releasing the capture session — Teams used to drop its session when muted, and
//!   `wasapi_monitor` inferred mute from that. This build keeps the session open, so
//!   that signal is gone.
//! * Teams' own logs — no mute state logged at all.
//! * Teams' local API port (8124) — not listening.
//!
//! The one place the state *is* visible is the meeting window's mute button, whose
//! accessible name flips between `Mute mic` (live) and `Unmute mic` (muted).
//!
//! # Why UIA for the camera too
//!
//! The camera signal has an analogous gap. `registry_monitor` reads the Privacy Consent
//! Store, which reflects *physical*-camera use: a virtual-camera passthrough (OBS,
//! NVIDIA Broadcast) sitting between the webcam and Teams does not reliably route
//! through the Frame Server capability check that feeds the store, so `LastUsedTimeStop`
//! can stay stuck while video is genuinely on. The meeting window's camera button
//! (`Turn camera off` while on, `Turn camera on` while off) reads the same way as mute
//! and takes precedence over the registry whenever it is available — see
//! `recompute_video` in `lib.rs`.
//!
//! # Known limitations — please read before relying on this
//!
//! * **Needs a realised Teams window.** With Teams closed to the tray its processes have
//!   no window, nothing appears in the UIA tree, and there is no reading at all.
//! * **Name-based.** A Teams UI rename or a non-English UI breaks it. There is no
//!   `TogglePattern` on the buttons, so the accessible name is the only available signal.
//!   The failure mode is benign: no reading rather than a wrong one, so mute falls back
//!   to `wasapi_monitor` and video to `registry_monitor`, exactly as before.
//! * Reports `Unknown`/`VideoUnknown` rather than a guess whenever a button cannot be
//!   found, so a missing reading is never mistaken for "unmuted" or "camera off".

use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiaEvent {
    MuteChanged(bool),
    /// No Teams mute button visible (typically: not in a meeting, or no Teams window).
    Unknown,
    /// Teams' meeting-toolbar camera button flipped. Takes precedence over
    /// `registry_monitor`'s reading whenever available — see `recompute_video` in
    /// `lib.rs` — because it reflects Teams' own belief about the video state regardless
    /// of which device is actually feeding the camera.
    VideoChanged(bool),
    /// No Teams camera button visible. Deliberately distinct from `VideoChanged(false)`
    /// so `recompute_video` falls back to the registry reading instead of concluding the
    /// camera is off.
    VideoUnknown,
}

pub fn start(tx: mpsc::Sender<UiaEvent>) {
    // UIA is COM; it needs its own thread with an apartment, like wasapi_monitor.
    std::thread::spawn(move || poll_blocking(tx));
}

/// `Mute mic` → not muted, `Unmute mic` → muted.
///
/// Order matters: "Unmute mic" also contains "mute", so the unmute test must come first.
fn classify(name: &str) -> Option<bool> {
    let lower = name.to_lowercase();
    if lower.contains("unmute") {
        Some(true)
    } else if lower.contains("mute") {
        Some(false)
    } else {
        None
    }
}

/// `Turn camera off` → video on, `Turn camera on` → video off.
///
/// Action-based naming, same convention as `classify`: the label says what clicking the
/// button would do, not what the current state is. Match on the whole phrase rather than
/// just "camera" — Teams' toolbar also carries "Camera settings" and device-picker
/// buttons, and those must not be read as a state.
fn classify_camera(name: &str) -> Option<bool> {
    let lower = name.to_lowercase();
    if lower.contains("turn camera off") {
        Some(true)
    } else if lower.contains("turn camera on") {
        Some(false)
    } else {
        None
    }
}

#[cfg(windows)]
fn poll_blocking(tx: mpsc::Sender<UiaEvent>) {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let ctx = match Uia::new() {
            Some(c) => c,
            None => {
                log::error!("UiaMonitor: could not initialise UI Automation; Teams mute/camera will not be detected");
                return;
            }
        };

        let mut last_mute: Option<bool> = None;
        let mut last_video: Option<bool> = None;
        // Holding the buttons found last time avoids re-walking the whole WebView2 tree
        // every poll; each is only re-searched once its own cached element goes stale.
        let mut cached_mute = None;
        let mut cached_video = None;

        loop {
            std::thread::sleep(Duration::from_millis(750));

            let (mute, video) = ctx.read(&mut cached_mute, &mut cached_video);

            if mute != last_mute {
                match mute {
                    Some(m) => {
                        log::info!("UiaMonitor: Teams mute → {m}");
                        let _ = tx.blocking_send(UiaEvent::MuteChanged(m));
                    }
                    None => {
                        log::info!("UiaMonitor: no Teams mute button visible");
                        let _ = tx.blocking_send(UiaEvent::Unknown);
                    }
                }
                last_mute = mute;
            }

            if video != last_video {
                match video {
                    Some(v) => {
                        log::info!("UiaMonitor: Teams camera → {v}");
                        let _ = tx.blocking_send(UiaEvent::VideoChanged(v));
                    }
                    None => {
                        log::info!("UiaMonitor: no Teams camera button visible");
                        let _ = tx.blocking_send(UiaEvent::VideoUnknown);
                    }
                }
                last_video = video;
            }
        }
    }
}

#[cfg(windows)]
struct Uia {
    /// The factory that produced `root` and the conditions. COM refcounts each interface
    /// independently so they would outlive it, but it is held for the lifetime of the
    /// monitor rather than dropped immediately after construction — there is no reason to
    /// let the automation object tear down while we are still querying its elements.
    _automation: windows::Win32::UI::Accessibility::IUIAutomation,
    root: windows::Win32::UI::Accessibility::IUIAutomationElement,
    any: windows::Win32::UI::Accessibility::IUIAutomationCondition,
    buttons: windows::Win32::UI::Accessibility::IUIAutomationCondition,
}

#[cfg(windows)]
impl Uia {
    unsafe fn new() -> Option<Self> {
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
        use windows::Win32::System::Variant::VARIANT;
        use windows::Win32::UI::Accessibility::{
            CUIAutomation, IUIAutomation, UIA_ButtonControlTypeId, UIA_ControlTypePropertyId,
        };

        let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL).ok()?;
        let root = automation.GetRootElement().ok()?;
        let any = automation.CreateTrueCondition().ok()?;
        let buttons = automation
            .CreatePropertyCondition(
                UIA_ControlTypePropertyId,
                &VARIANT::from(UIA_ButtonControlTypeId.0),
            )
            .ok()?;

        Some(Self {
            _automation: automation,
            root,
            any,
            buttons,
        })
    }

    /// Current `(mute, video)` state; either is `None` when its button cannot be found.
    unsafe fn read(
        &self,
        cached_mute: &mut Option<windows::Win32::UI::Accessibility::IUIAutomationElement>,
        cached_video: &mut Option<windows::Win32::UI::Accessibility::IUIAutomationElement>,
    ) -> (Option<bool>, Option<bool>) {
        // Fast path: the buttons found last time are usually still there, with only their
        // accessible name changed.
        let mut mute = cached_mute
            .as_ref()
            .and_then(|el| el.CurrentName().ok())
            .and_then(|name| classify(&name.to_string()));
        if mute.is_none() {
            // Stale (window closed, or the node was replaced) — drop it and re-search.
            *cached_mute = None;
        }

        let mut video = cached_video
            .as_ref()
            .and_then(|el| el.CurrentName().ok())
            .and_then(|name| classify_camera(&name.to_string()));
        if video.is_none() {
            *cached_video = None;
        }

        if mute.is_some() && video.is_some() {
            return (mute, video);
        }

        self.search(cached_mute, cached_video, &mut mute, &mut video);
        (mute, video)
    }

    /// Walks Teams' windows for whichever of the mute/camera buttons `read`'s fast path did
    /// not already resolve, filling in `mute`/`video` in place and caching whatever is found
    /// (leaving an already-resolved value and its cache entry untouched).
    unsafe fn search(
        &self,
        cached_mute: &mut Option<windows::Win32::UI::Accessibility::IUIAutomationElement>,
        cached_video: &mut Option<windows::Win32::UI::Accessibility::IUIAutomationElement>,
        mute: &mut Option<bool>,
        video: &mut Option<bool>,
    ) {
        use windows::Win32::UI::Accessibility::{TreeScope_Children, TreeScope_Descendants};

        let top = match self.root.FindAll(TreeScope_Children, &self.any) {
            Ok(t) => t,
            Err(_) => return,
        };
        let n = top.Length().unwrap_or(0);

        for i in 0..n {
            if mute.is_some() && video.is_some() {
                return;
            }

            let win = match top.GetElement(i) {
                Ok(w) => w,
                Err(_) => continue,
            };

            let pid = win.CurrentProcessId().unwrap_or(0) as u32;
            if pid == 0 || !crate::teams_proc::is_teams_pid(pid) {
                continue;
            }

            // Teams has several windows (main, meeting, notifications); only the meeting
            // window carries these buttons, so check them all and take the first hit.
            let found = match win.FindAll(TreeScope_Descendants, &self.buttons) {
                Ok(b) => b,
                Err(_) => continue,
            };

            let bn = found.Length().unwrap_or(0);
            for j in 0..bn {
                if mute.is_some() && video.is_some() {
                    break;
                }
                let btn = match found.GetElement(j) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                let name = match btn.CurrentName() {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                };

                // A button matches at most one of the two, so the clone below is only
                // there to keep `btn` available for the second check — in practice one of
                // the two arms is always skipped.
                if mute.is_none() {
                    if let Some(m) = classify(&name) {
                        *mute = Some(m);
                        *cached_mute = Some(btn.clone());
                    }
                }
                if video.is_none() {
                    if let Some(v) = classify_camera(&name) {
                        *video = Some(v);
                        *cached_video = Some(btn);
                    }
                }
            }
        }
    }
}

#[cfg(not(windows))]
fn poll_blocking(_tx: mpsc::Sender<UiaEvent>) {
    log::warn!("UiaMonitor: not supported on this platform");
}

#[cfg(test)]
mod tests {
    use super::{classify, classify_camera};

    #[test]
    fn names_observed_from_teams() {
        // Verbatim accessible names captured from Teams 26183.1903.4892.4448.
        assert_eq!(classify("Mute mic"), Some(false));
        assert_eq!(classify("Unmute mic"), Some(true));
    }

    #[test]
    fn unmute_wins_over_the_substring_mute() {
        // "Unmute" contains "mute"; getting this order wrong inverts the sensor.
        assert_eq!(classify("Unmute"), Some(true));
        assert_eq!(classify("UNMUTE MIC"), Some(true));
    }

    #[test]
    fn unrelated_buttons_are_ignored() {
        assert_eq!(classify("Leave"), None);
        assert_eq!(classify("Share content"), None);
        assert_eq!(classify(""), None);
    }

    #[test]
    fn camera_button_names() {
        // Action-based, same as mute: the label says what the click would do.
        assert_eq!(classify_camera("Turn camera off"), Some(true));
        assert_eq!(classify_camera("Turn camera on"), Some(false));
        assert_eq!(classify_camera("TURN CAMERA OFF"), Some(true));
    }

    #[test]
    fn camera_off_wins_over_the_substring_on() {
        // "Turn camera off" does not contain "turn camera on", but keep this pinned:
        // a looser match on "on" would read every camera-on label as camera-off.
        assert_eq!(classify_camera("Turn camera off"), Some(true));
    }

    #[test]
    fn other_camera_buttons_are_not_a_state() {
        // The toolbar carries these next to the toggle; reading them as a state would
        // pin is_video_on to whatever they happened to match.
        assert_eq!(classify_camera("Camera settings"), None);
        assert_eq!(classify_camera("Video effects"), None);
        assert_eq!(classify_camera("Mute mic"), None);
        assert_eq!(classify_camera(""), None);
    }

    #[test]
    fn the_two_classifiers_never_claim_the_same_button() {
        // search() checks both against one name; overlap would cache the wrong element.
        for name in [
            "Mute mic",
            "Unmute mic",
            "Turn camera off",
            "Turn camera on",
        ] {
            assert!(
                classify(name).is_none() || classify_camera(name).is_none(),
                "{name} classified as both mute and camera"
            );
        }
    }
}
