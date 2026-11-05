//! Polls WASAPI capture sessions every 250 ms, for two separate facts.
//!
//! * **Mute.** This is **not** the primary mute source — Teams' own mute button is
//!   invisible here; `uia_monitor` reads that. What this catches is the *OS-level* mute:
//!   muting Teams from the Windows volume mixer or Sound settings, which UI Automation
//!   cannot see.
//! * **Microphone in use.** Whether one of Teams' capture sessions is in
//!   `AudioSessionStateActive`, i.e. Teams is actually pulling audio from a microphone.
//!   `update_mic_in_use` in `lib.rs` uses it as a meeting start/end signal, next to the
//!   registry's mic-in-use flag.
//!
//! # Why the session state, measured 2026-11-05 (Teams 26260.1701.5139.3736)
//!
//! The registry flag `registry_monitor` reads had gone dead on the laptop this was
//! measured on: Windows 11 26H2 (build 26300) wrote no `LastUsedTimeStart`/`Stop` for
//! *any* app after 2026-08-07, so the only start signal left was Teams' own log, which
//! Teams sometimes fails to write (see `log_watcher`). The session state was watched
//! through a working day of real calls:
//!
//! * three call starts: active within 3 s of the log's start line, once 39 s before it
//!   (Teams was already capturing before the call was joined, most likely for the
//!   pre-join screen's level meter);
//! * three hang-ups: inactive within 3 s of the log's end line;
//! * still active while muted in Teams (a 22-second mute during a call);
//! * inactive for the whole hour between two calls, although Teams keeps the session
//!   itself open all day.
//!
//! It is a live reading rather than a timestamp left behind, so unlike the registry it
//! needs no trust-after-inactivity gate, and it also catches a call that is already
//! running when the app (re)starts.

use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasapiEvent {
    /// A Teams capture session exists and reported this mute state.
    MuteChanged(bool),
    /// Teams has no capture session at all.
    ///
    /// Reported as its own event rather than as "muted". On older Teams builds this was
    /// *the* mute signal — Teams released its capture session when muted — but it is an
    /// inference, not a reading, so `recompute_muted` in lib.rs uses it only as a
    /// fallback when UI Automation has no reading. Treating it as authoritative would
    /// let a stale "no session" override a perfectly good UIA reading of "not muted".
    NoSession,
    /// Whether any Teams capture session is active (Teams is using a microphone). No
    /// session at all counts as `false`; a failed audio-API read is not reported.
    CaptureActiveChanged(bool),
}

pub fn start(tx: mpsc::Sender<WasapiEvent>) {
    // WASAPI COM calls must run on a dedicated thread.
    std::thread::spawn(move || {
        poll_wasapi_blocking(tx);
    });
}

#[cfg(windows)]
fn poll_wasapi_blocking(tx: mpsc::Sender<WasapiEvent>) {
    use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{
        CoInitializeEx, CoCreateInstance, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let enumerator: IMMDeviceEnumerator =
            match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
                Ok(e) => e,
                Err(e) => {
                    log::error!("WASAPI: CoCreateInstance failed: {e}");
                    return;
                }
            };

        // Some(Some(flag)) = a session exists and reported that flag
        // Some(None)       = no Teams capture session
        // None             = nothing reported yet
        let mut last_sent: Option<Option<bool>> = None;
        // Same idea for "is Teams capturing": None = nothing reported yet.
        let mut last_active: Option<bool> = None;
        // Error state is logged on transition only — at 4 polls/sec, logging every
        // failure would flood the file, but silence made "unchanged" and "could not
        // read" indistinguishable when diagnosing mute detection.
        let mut in_error = false;

        loop {
            std::thread::sleep(Duration::from_millis(250));

            let reading = read_teams_capture(&enumerator);
            match (&reading, in_error) {
                (Err(()), false) => {
                    log::warn!("WasapiMonitor: audio API read failing - holding last state");
                    in_error = true;
                }
                (Ok(_), true) => {
                    log::info!("WasapiMonitor: audio API readable again");
                    in_error = false;
                }
                _ => {}
            }

            // ┌───────────────────┬──────────────────────────────────────────────────┐
            // │ Err(())           │ the audio API failed: no measurement at all, so  │
            // │                   │ hold the last state rather than inventing a mute │
            // │                   │ flank mid-call.                                  │
            // │ Ok(None)          │ no Teams capture session → report NoSession.     │
            // │ Ok(Some(reading)) │ a session exists → report its mute flags.         │
            // └───────────────────┴──────────────────────────────────────────────────┘
            //
            // History, because this arm has been got wrong twice:
            //
            // Teams' own mute button is invisible here — verified 2026-07-26 across
            // several real calls that the per-session mute (ISimpleAudioVolume) and
            // the capture endpoint mute (IAudioEndpointVolume) both stay false through
            // mute toggles. On older builds Teams *released* its capture session when
            // muted, so `Ok(None)` was the mute signal, and commit e5c28ed broke mute
            // entirely by reclassifying it as "no reading, hold last state".
            //
            // Current Teams keeps the session open while muted, so that inference no
            // longer fires anyway, and uia_monitor is the real source. `Ok(None)` is
            // therefore reported as its own event and left for lib.rs to weigh: still
            // usable as a fallback when UIA has no reading, but never allowed to
            // override one. Reporting it as "muted" here would mean a Teams that is
            // merely idle could mark you muted the moment a meeting is detected.
            let current: Option<Option<bool>> = match &reading {
                Err(()) => None,
                Ok(None) => Some(None),
                Ok(Some(r)) => Some(Some(r.muted())),
            };
            // A failed read holds the last state here too: inventing an "inactive"
            // could end a meeting that is still running.
            let active: Option<bool> = match &reading {
                Err(()) => None,
                Ok(None) => Some(false),
                Ok(Some(r)) => Some(r.active),
            };
            if let Some(a) = active {
                if last_active != Some(a) {
                    last_active = Some(a);
                    log::info!("WasapiMonitor: Teams capturing → {a}");
                    let _ = tx.blocking_send(WasapiEvent::CaptureActiveChanged(a));
                }
            }

            if let Some(state) = current {
                if last_sent != Some(state) {
                    last_sent = Some(state);
                    match (state, &reading) {
                        // Both flag components are logged: if Teams ever does start
                        // reflecting mute in one of them, this is where it will show.
                        (Some(m), Ok(Some(r))) => {
                            log::info!("WasapiMonitor: session mute → {m} ({r})");
                            let _ = tx.blocking_send(WasapiEvent::MuteChanged(m));
                        }
                        (Some(m), _) => {
                            log::info!("WasapiMonitor: session mute → {m}");
                            let _ = tx.blocking_send(WasapiEvent::MuteChanged(m));
                        }
                        (None, _) => {
                            log::info!("WasapiMonitor: no Teams capture session");
                            let _ = tx.blocking_send(WasapiEvent::NoSession);
                        }
                    }
                }
            }
        }
    }
}

/// What Teams' capture sessions showed in one poll.
///
/// The two independent mute flags that can hide a microphone are read together because
/// they mean different things:
///
/// * `session` — `ISimpleAudioVolume` on Teams' own capture session. This is the
///   per-app slider in the Windows volume mixer. Teams' in-app mute button does **not**
///   touch it; verified during a real call on 2026-07-26 where it stayed `false`
///   across several mute toggles.
/// * `endpoint` — `IAudioEndpointVolume` on the capture *device*. This is what the
///   Windows 11 mic-mute control drives, and Teams keeps it in sync via the VoIP call
///   coordinator (see `HfpVoipCallCoordinatorProvider` in the Teams logs).
///
/// Either being set means the far end hears nothing, so the reported state is the OR.
///
/// `active` is the separate "microphone in use" reading: at least one Teams session is
/// in `AudioSessionStateActive`.
#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
pub struct CaptureReading {
    pub session: bool,
    pub endpoint: bool,
    pub active: bool,
}

#[cfg(windows)]
impl CaptureReading {
    fn muted(&self) -> bool {
        self.session || self.endpoint
    }
}

#[cfg(windows)]
impl std::fmt::Display for CaptureReading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session={} endpoint={}", self.session, self.endpoint)
    }
}

/// Ok(Some(reading)) when a Teams capture session was measured, Ok(None) when no
/// Teams session exists, Err(()) when the audio API failed.
///
/// Note `Ok(None)` is a reading, not missing data: the caller reports it as
/// `NoSession` and as "not capturing". See the table in the poll loop before changing
/// this contract.
#[cfg(windows)]
unsafe fn read_teams_capture(
    enumerator: &windows::Win32::Media::Audio::IMMDeviceEnumerator,
) -> Result<Option<CaptureReading>, ()> {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eCapture, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2,
        ISimpleAudioVolume, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::CLSCTX_ALL;

    let collection = enumerator
        .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
        .map_err(|_| ())?;
    let count = collection.GetCount().map_err(|_| ())?;

    let mut teams_found = false;
    let mut session_muted = false;
    let mut endpoint_muted = false;
    let mut active = false;

    for i in 0..count {
        let device = match collection.Item(i) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let mgr: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let session_enum = match mgr.GetSessionEnumerator() {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Deliberately not `count` — that would shadow the device count above.
        let session_count = match session_enum.GetCount() {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut device_hosts_teams = false;

        for j in 0..session_count {
            let ctrl = match session_enum.GetSession(j) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let ctrl2: IAudioSessionControl2 = match ctrl.cast() {
                Ok(c) => c,
                Err(_) => continue,
            };

            let pid = match ctrl2.GetProcessId() {
                Ok(p) => p,
                Err(_) => continue,
            };

            if !crate::teams_proc::is_teams_pid(pid) {
                continue;
            }

            teams_found = true;
            device_hosts_teams = true;

            // Teams keeps a session open on every microphone all day; only a session
            // that is actually capturing is "active".
            if ctrl.GetState().is_ok_and(|st| st == AudioSessionStateActive) {
                active = true;
            }

            let vol: ISimpleAudioVolume = match ctrl.cast() {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Ok(mute) = vol.GetMute() {
                if mute.as_bool() {
                    session_muted = true;
                }
            }
        }

        // Only consult the endpoint of a device Teams is actually capturing from —
        // an unrelated muted mic (a webcam's, say) must not read as "Teams is muted".
        if device_hosts_teams {
            if let Ok(endpoint) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                if let Ok(mute) = endpoint.GetMute() {
                    if mute.as_bool() {
                        endpoint_muted = true;
                    }
                }
            }
        }
    }

    if teams_found {
        Ok(Some(CaptureReading {
            session: session_muted,
            endpoint: endpoint_muted,
            active,
        }))
    } else {
        Ok(None)
    }
}

// is_teams_pid moved to crate::teams_proc — uia_monitor needs it too.

#[cfg(not(windows))]
fn poll_wasapi_blocking(_tx: mpsc::Sender<WasapiEvent>) {
    log::warn!("WasapiMonitor: not supported on this platform");
}
