/// Windows session lock/unlock detection via WTSRegisterSessionNotification.
/// A dedicated thread runs a message-only window whose wndproc receives
/// WM_WTSSESSION_CHANGE; lock state flows out as events. Feeds the
/// "Session Locked" binary_sensor (at-desk detection for HA automations).
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum SessionEvent {
    LockedChanged(bool),
}

#[cfg(windows)]
static EVENT_TX: std::sync::OnceLock<mpsc::Sender<SessionEvent>> = std::sync::OnceLock::new();

pub fn start(tx: mpsc::Sender<SessionEvent>) {
    #[cfg(windows)]
    {
        if EVENT_TX.set(tx).is_err() {
            log::warn!("SessionMonitor: already started");
            return;
        }
        std::thread::spawn(run_message_loop);
    }
    #[cfg(not(windows))]
    {
        let _ = tx;
        log::warn!("SessionMonitor: not supported on this platform");
    }
}

#[cfg(windows)]
fn run_message_loop() {
    use windows::core::w;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::RemoteDesktop::{
        WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassW, HWND_MESSAGE, MSG,
        WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
    };

    unsafe {
        let instance = match GetModuleHandleW(None) {
            Ok(h) => h,
            Err(e) => {
                log::error!("SessionMonitor: GetModuleHandleW failed: {e}");
                return;
            }
        };

        let class_name = w!("Teams2HASessionMonitor");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            log::error!("SessionMonitor: RegisterClassW failed");
            return;
        }

        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            class_name,
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE), // message-only window: no UI, just a wndproc
            None,
            Some(instance.into()),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                log::error!("SessionMonitor: CreateWindowExW failed: {e}");
                return;
            }
        };

        if let Err(e) = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) {
            log::error!("SessionMonitor: WTSRegisterSessionNotification failed: {e}");
            return;
        }
        log::info!("SessionMonitor: registered for session lock/unlock notifications");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::DefWindowProcW;

    // From WinUser.h — these constants are not exposed by the windows crate.
    const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
    const WTS_SESSION_LOCK: u32 = 0x7;
    const WTS_SESSION_UNLOCK: u32 = 0x8;

    if msg == WM_WTSSESSION_CHANGE {
        let locked = match wparam.0 as u32 {
            WTS_SESSION_LOCK => Some(true),
            WTS_SESSION_UNLOCK => Some(false),
            _ => None,
        };
        if let Some(locked) = locked {
            log::info!("SessionMonitor: session locked → {locked}");
            if let Some(tx) = EVENT_TX.get() {
                let _ = tx.blocking_send(SessionEvent::LockedChanged(locked));
            }
        }
        return windows::Win32::Foundation::LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
