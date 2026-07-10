use std::time::Duration;
use tokio::sync::mpsc;

/// Home-network detection based on the MAC address of the default IPv4 gateway.
///
/// The gateway MAC is used instead of SSID or broker reachability because:
/// - it works both on Wi-Fi and wired/docked connections,
/// - reading the SSID on Windows 11 requires location permission,
/// - a VPN tunnel can make the broker reachable away from home, but it can never make a
///   foreign gateway answer ARP with the home router's MAC.
///
/// An empty configured MAC disables the feature (always considered home).

pub fn is_home(configured: &str) -> bool {
    let macs = parse_macs(configured);
    if macs.is_empty() {
        return true; // Feature disabled — behave as before.
    }
    match current_gateway_mac_bytes() {
        Some(mac) => macs.contains(&mac),
        None => false,
    }
}

/// Formatted gateway MAC for the "Use Current Network" button, e.g. "AA:BB:CC:DD:EE:FF".
pub fn current_gateway_mac() -> Option<String> {
    current_gateway_mac_bytes().map(format_mac)
}

/// Spawns the poller. Sends the initial state immediately, then emits on every
/// home/away transition. Flipping to away requires two consecutive misses so a
/// transient ARP failure doesn't drop the connection.
pub fn start(tx: mpsc::Sender<bool>) {
    tauri::async_runtime::spawn(async move {
        let mut last: Option<bool> = None;
        let mut away_strikes: u8 = 0;
        loop {
            let configured = crate::settings::Settings::load().home_gateway_mac;
            // SendARP can block for up to a second — keep it off the async runtime.
            let home = tokio::task::spawn_blocking(move || is_home(&configured))
                .await
                .unwrap_or(true);

            let effective = if home {
                away_strikes = 0;
                true
            } else {
                away_strikes = away_strikes.saturating_add(1);
                // Until confirmed away, keep reporting the previous state (initially away).
                if away_strikes >= 2 { false } else { last.unwrap_or(false) }
            };

            if last != Some(effective) {
                last = Some(effective);
                log::info!(
                    "Home network state: {}",
                    if effective { "home" } else { "away" }
                );
                if tx.send(effective).await.is_err() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_secs(20)).await;
        }
    });
}

fn parse_macs(configured: &str) -> Vec<[u8; 6]> {
    configured
        .split([',', ';', ' '])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .filter_map(|p| {
            let hex: String = p.chars().filter(char::is_ascii_hexdigit).collect();
            if hex.len() != 12 {
                log::warn!("Ignoring invalid home gateway MAC entry: {p}");
                return None;
            }
            let mut mac = [0u8; 6];
            for (i, byte) in mac.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
            }
            Some(mac)
        })
        .collect()
}

fn format_mac(mac: [u8; 6]) -> String {
    mac.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(windows)]
fn current_gateway_mac_bytes() -> Option<[u8; 6]> {
    use windows::Win32::NetworkManagement::IpHelper::{GetBestRoute, SendARP, MIB_IPFORWARDROW};

    unsafe {
        let mut row: MIB_IPFORWARDROW = std::mem::zeroed();
        // Any public IP works — we only need the route's next hop (the LAN gateway).
        let dest = u32::from_ne_bytes([8, 8, 8, 8]);
        if GetBestRoute(dest, None, &mut row) != 0 {
            return None;
        }
        let gateway = row.dwForwardNextHop;
        if gateway == 0 {
            // On-link route (e.g. a VPN tunnel) — nothing to ARP.
            return None;
        }
        let mut mac = [0u8; 8];
        let mut len: u32 = 6;
        if SendARP(gateway, 0, mac.as_mut_ptr() as *mut core::ffi::c_void, &mut len) != 0 || len != 6 {
            return None;
        }
        Some([mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]])
    }
}

#[cfg(not(windows))]
fn current_gateway_mac_bytes() -> Option<[u8; 6]> {
    None
}
