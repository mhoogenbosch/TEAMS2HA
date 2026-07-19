# Teams2HA

Bridge your Microsoft Teams status to Home Assistant over MQTT — meeting state, presence, mute and video, published as native HA entities.

> **This is a fork.** Teams2HA was created and is maintained by **[jimmyeao](https://github.com/jimmyeao/TEAMS2HA)** — all credit for the original app and the Rust/Tauri rewrite goes to them. This fork ([mhoogenbosch/TEAMS2HA](https://github.com/mhoogenbosch/TEAMS2HA)) adds a signed auto-updater, home-network gating, MQTT availability/LWT, a controllable system-mic switch, session-lock sensing, HA-to-Windows toasts and assorted hardening on top of that work — see the table below. Where possible, changes are offered back upstream as pull requests.

[![License](https://img.shields.io/badge/License-MIT-blue)](#license)

---

## Why a Rust/Tauri app?

Microsoft deprecated the Teams **local API**, which broke the classic integration. Instead of that API, Teams2HA now reads **Teams log files** and **hardware signals** (microphone/camera use, audio session state) to determine whether you are in a meeting and what your status, mute and video state are. It's a small, windowless tray app.

- **No admin rights required** — installs per-user to `%LOCALAPPDATA%`.
- **Windows x64 and ARM64** builds.

## What this fork adds

| Feature | What it does |
|---|---|
| 🔄 **Signed auto-updater** | In-app **Updates card** (version, status, progress bar, release-notes button); checks GitHub on startup and hourly, verifies the minisign signature and installs + relaunches on your confirmation. Live since **v1.3.7**. |
| 🏠 **Home-network gating** | Only connects to MQTT when you're on your home network, matched on the **default-gateway MAC** (Settings → Home Detection → *Use Current Network*). Leave empty to always connect. |
| 🟢 **Availability / LWT** | Every entity has an `availability_topic` with a retained **Last Will**. When the app is closed, asleep, crashed or away, its entities go `unavailable` in HA — this is the structural fix for a "sticky" `is_in_meeting`. |
| 🎙️ **System mic-mute switch** | `micsystemmuted` mutes the default communications microphone at the Windows level — the one switch that is **genuinely controllable** from HA (the Teams-session toggles died with the retired local API). |
| 🔒 **Session Locked sensor** | Windows lock/unlock as a binary sensor — an at-desk signal for automations. |
| 💬 **Toasts from HA** | A `notify` entity per machine: `notify.send_message` pops a Windows toast (plain text or JSON `{title, message}`) — reaches you even in headphone meetings. |
| 🔴 **Tray status dot** | Tray icon shows red while in a meeting, orange while muted, with a matching tooltip. |
| 🔐 **Hardened & honest** | MQTT password DPAPI-encrypted at rest; TLS never silently downgrades; CSP on the window; truthful entity types (state-only signals are binary sensors, not fake switches, since **v1.4.0**). |
| 🖥️ **UI niceties** | Sensor strip (mic/camera/system-mic/Teams pills), System/Light/Dark theme following Windows live, settings auto-save, version in the window title. |
| 🧾 **Device `sw_version`** | The installed version is reported in the MQTT discovery device block. |
| 🗕 **Close-to-tray** | The window close button hides to the tray instead of quitting (keeping the MQTT bridge alive). |
| 💤 **Modern Standby resume** | Reconnects and republishes state after sleep/standby (clock-gap detection — Windows gives suspended desktop apps no reliable resume event). |

## Installation

1. Download the installer for your architecture from the [**latest release**](https://github.com/mhoogenbosch/TEAMS2HA/releases/latest) and run it.

   | File | Architecture |
   |------|-------------|
   | `Teams2HA_*_x64-setup.exe` | Intel / AMD (most PCs) |
   | `Teams2HA_*_arm64-setup.exe` | ARM64 (Surface Pro X, Copilot+ PCs) |

2. If you previously had the old .NET version installed, it is automatically removed on first launch.

> **First install is manual, after that it's automatic.** Versions before v1.3.7 had no updater, so you install v1.3.7 by hand once; every release after that installs itself.
>
> **SmartScreen / Defender:** the installers are signed for *update authenticity* (minisign) but are **not** Authenticode-signed, so Windows Defender may occasionally flag a fresh build as a false positive (`Trojan:Win32/Bearfoos.B!ml`). Choose *Allow* / *Keep* if that happens.

## Configuration

### MQTT
Provide your MQTT broker details (host, username, password). The password is encrypted with Windows DPAPI (user scope) before being written to the settings file — never stored in clear text; an existing plaintext value is migrated on the next save. Supported: plain MQTT, MQTT over TLS, MQTT over WebSockets, and WebSockets over TLS. Certificate verification is always on — the old "ignore certificate errors" option is gone (it used to silently downgrade to an unencrypted connection).

### Home Detection (this fork)
Under **Settings → Home Detection**, click **Use Current Network** while on your home Wi-Fi/LAN to store your gateway's MAC address. The app then only connects to MQTT when that gateway is present — so it stays quiet on other networks even when the broker is reachable over a VPN.

## Entities

Published via MQTT discovery under your chosen name:

- `binary_sensor/<YOURNAME>/ismuted` — Teams-session mute (was a `switch` before v1.4.0)
- `binary_sensor/<YOURNAME>/isvideoon` — camera in use (was a `switch` before v1.4.0)
- `binary_sensor/<YOURNAME>/isinmeeting`
- `binary_sensor/<YOURNAME>/teamsrunning`
- `binary_sensor/<YOURNAME>/sessionlocked` — Windows session locked
- `switch/<YOURNAME>/micsystemmuted` — system-wide mute of the default communications microphone (genuinely controllable from HA)
- `sensor/<YOURNAME>/teamsstatus` — presence (Available/Busy/…)
- a `notify` entity ("Toast") — `notify.send_message` shows a Windows toast on the machine

Each carries an availability topic so HA shows them as `unavailable` when the app is away.

> **Why sensors, not switches?** Microsoft retired the Teams local API, so `ismuted`/`isvideoon` commands had nowhere to go — a toggle that silently bounces back. Since v1.4.0 they are binary sensors; the system-mic switch is the one control that genuinely works. **Upgrading from ≤ v1.3.x:** the old switch entities are removed automatically (retained discovery configs are cleared) — update any HA automations that referenced `switch.…_is_muted` / `switch.…_is_video_on` to the new `binary_sensor` entities.

## Repository layout

| Path | Contents |
|------|----------|
| `tauri/` | **The app** — Rust/Tauri backend (`src-tauri/`) + React frontend (`src/`). This is what the releases build. |

The legacy .NET/WPF app that used to live in the repository root was removed — it depended on the Teams local API that Microsoft deprecated, and it was no longer built or released. It remains available in the git history and upstream.

## Building (Tauri app)

```bash
cd tauri
npm install
npm run tauri dev      # run locally
npm run tauri build    # produce an installer
```

Releases are produced by GitHub Actions on a `vX.Y.Z` tag (`.github/workflows/release.yml`), which builds x64 + ARM64, signs the updater artifacts and publishes `latest.json` alongside the installers.

## Changelog

See [CHANGELOG.md](CHANGELOG.md).

## Credits

- **Original author & maintainer:** [jimmyeao](https://github.com/jimmyeao) — [jimmyeao/TEAMS2HA](https://github.com/jimmyeao/TEAMS2HA). The application, the Rust/Tauri rewrite and the core design are theirs.
- This fork is maintained by [mhoogenbosch](https://github.com/mhoogenbosch); PRs are contributed back upstream where they make sense.

## License

MIT — see the upstream project. PRs always welcome. 🙂
