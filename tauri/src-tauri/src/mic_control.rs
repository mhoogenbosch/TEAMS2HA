/// System-level microphone mute: reads and sets the master mute flag of the
/// default communications capture endpoint (the device Teams records from).
/// Unlike the Teams session mute (wasapi_monitor), this works without Teams —
/// setting it actually cuts the microphone for every application, so the HA
/// switch backed by this is genuinely controllable.
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum MicEvent {
    SystemMuteChanged(bool),
}

pub fn start(tx: mpsc::Sender<MicEvent>) {
    // COM calls run on a dedicated thread, same pattern as wasapi_monitor.
    std::thread::spawn(move || {
        poll_mic_blocking(tx);
    });
}

#[cfg(windows)]
fn poll_mic_blocking(tx: mpsc::Sender<MicEvent>) {
    use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let enumerator: IMMDeviceEnumerator =
            match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
                Ok(e) => e,
                Err(e) => {
                    log::error!("MicControl: CoCreateInstance failed: {e}");
                    return;
                }
            };

        let mut last: Option<bool> = None;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            // Query the default endpoint every cycle so switching the default
            // microphone in Windows is picked up automatically.
            if let Ok(muted) = read_default_mic_mute(&enumerator) {
                if Some(muted) != last {
                    last = Some(muted);
                    log::info!("MicControl: system mic mute → {muted}");
                    let _ = tx.blocking_send(MicEvent::SystemMuteChanged(muted));
                }
            }
        }
    }
}

#[cfg(windows)]
unsafe fn read_default_mic_mute(
    enumerator: &windows::Win32::Media::Audio::IMMDeviceEnumerator,
) -> Result<bool, ()> {
    use windows::Win32::Media::Audio::{eCapture, eCommunications, Endpoints::IAudioEndpointVolume};
    use windows::Win32::System::Com::CLSCTX_ALL;

    let device = enumerator
        .GetDefaultAudioEndpoint(eCapture, eCommunications)
        .map_err(|_| ())?;
    let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).map_err(|_| ())?;
    volume.GetMute().map(|b| b.as_bool()).map_err(|_| ())
}

/// Set the master mute of the default communications microphone. Runs its own
/// COM init so it can be called from spawn_blocking.
#[cfg(windows)]
pub fn set_system_mic_mute(muted: bool) -> Result<(), String> {
    use windows::Win32::Media::Audio::{
        eCapture, eCommunications, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator,
        MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eCapture, eCommunications)
            .map_err(|e| e.to_string())?;
        let volume: IAudioEndpointVolume =
            device.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
        volume
            .SetMute(muted, std::ptr::null())
            .map_err(|e| e.to_string())
    }
}

#[cfg(not(windows))]
fn poll_mic_blocking(_tx: mpsc::Sender<MicEvent>) {
    log::warn!("MicControl: not supported on this platform");
}

#[cfg(not(windows))]
pub fn set_system_mic_mute(_muted: bool) -> Result<(), String> {
    Err("not supported on this platform".into())
}
