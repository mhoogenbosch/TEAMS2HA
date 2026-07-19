# Changelog

All notable changes to this fork ([mhoogenbosch/TEAMS2HA](https://github.com/mhoogenbosch/TEAMS2HA)) are documented here.
The format is loosely based on [Keep a Changelog](https://keepachangelog.com/). Original app by [jimmyeao](https://github.com/jimmyeao/TEAMS2HA).

## [v1.4.0] — 2026-07-19
### Breaking
- **`ismuted` and `isvideoon` are now binary sensors instead of switches.** Their command path died when Microsoft retired the Teams local API — in HA the toggles silently bounced back without doing anything. The old retained switch discovery configs and states are cleaned up automatically on the first connect, which removes the old `switch.…` entities from Home Assistant. **Action required:** update automations/dashboards that referenced `switch.…_is_muted` / `switch.…_is_video_on` to the new `binary_sensor` entities. The system-mic switch (`micsystemmuted`) is unaffected and remains genuinely controllable.
### Fixed
- **First-run migration could delete the app's own uninstall entry.** The ClickOnce cleanup matched any HKCU uninstall entry whose display name contains "teams2ha" — on a machine without an old ClickOnce install, the first match was the NSIS entry this very installer had just created, removing Teams2HA from Add/Remove Programs. The cleanup now only matches genuine ClickOnce entries (dfshim.dll uninstall command).
- **The app could freeze entirely while the broker was unreachable on the home network.** State publishes went into a bounded MQTT request queue that is not drained during the 5-second reconnect-retry sleep; once full, the awaited publish blocked the central event loop and, cascading, every monitor thread. All state/discovery publishes now use the non-blocking `try_publish`/`try_subscribe` (with a larger queue), so events keep flowing and a failed delivery is simply retried.
- **A failed state publish was cached as delivered.** `publish_state` logged individual publish errors but always reported success, so the "only cache after a successful publish" retry mechanism never actually retried — a mute/meeting flank could be lost for good if the broker hiccuped at the wrong moment. Publish failures now propagate and the state is re-sent on the next event or reconnect.
- **Presence transition lines could report the old status.** Log lines that name two statuses ("from Busy to Available") matched the first keyword in a fixed list instead of the most recent one. The parser now takes the status closest to the end of the line, with word-boundary matching (e.g. "Available" no longer matches inside "Unavailable").
### Security
- **The MQTT password is now encrypted at rest with Windows DPAPI** (user scope, `dpapi:<hex>` in settings.json) instead of stored in plain text. An existing plaintext password keeps working and is migrated automatically on the next settings save. The WPF app had this protection; it was lost in the Tauri rewrite.
- **The Run-at-boot registry value now quotes the executable path.** An unquoted path with spaces is the classic unquoted-path problem: broken autostart at best, binary planting at worst.
- **The "Ignore Cert Errors" toggle is removed from the UI.** The backend has ignored it since v1.3.8 (when it stopped silently downgrading TLS to plain TCP), so the toggle only suggested a capability that does not exist. TLS connections always verify the certificate.
- **A Content Security Policy is now set** for the app window (previously `null`). Defence in depth: broker-supplied strings end up in the UI, and a strict CSP limits what any future injection could do.
- **Toast payloads from MQTT are capped at 1 KB** before being handed to the Windows notification pipeline.

## [v1.3.14] — 2026-07-19
### Fixed
- **v1.3.13 rendered a blank window.** The auto-save code used `useRef` without importing it — a runtime `ReferenceError` that the vite build does not catch, crashing the entire UI on load (the MQTT bridge kept running). v1.3.13 installs must update to this version manually: the in-app updater lives in the UI that fails to render. ESLint (`no-undef`) now runs in CI so this class of error fails the build instead of shipping.
### Added
- The window title shows the running version ("Teams2HA v1.3.14").

## [v1.3.13] — 2026-07-19
### Changed
- **Settings auto-save.** The Save button is gone; changes persist automatically 1.5 s after the last edit, with a small "✓ Saved" indicator. The MQTT connection is only rebuilt when a connection-relevant field actually changed (address, port, credentials, prefix, TLS/WebSocket flags) — switching the theme no longer drops the broker session.

## [v1.3.12] — 2026-07-19
### Added
- **Controllable system-mic mute switch** (`Mic Muted (System)`): mutes the default communications microphone at the Windows level via the audio endpoint API, so it genuinely works from Home Assistant — unlike the Teams-session switches, which lost their command path when Microsoft retired the Teams local API. State is polled and stays in sync when you mute via Windows itself.
- **Session Locked binary sensor**: Windows lock/unlock events (WTS session notifications) as an at-desk signal for automations.
- **Toast notifications from Home Assistant**: a `notify` entity (MQTT discovery) per machine; `notify.send_message` shows the message as a Windows toast. Payload can be plain text or JSON with `title`/`message`.
- **Tray icon status dot**: red while in a meeting, orange while muted (system-wide, or Teams-muted during a call), with a matching tooltip.
- The sensor strip shows the system microphone state as a fourth pill.
### Changed
- Release notes are now composed automatically: the release body starts with the tag's section from this changelog, followed by the install instructions (moved to `.github/RELEASE_TEMPLATE.md`).

## [v1.3.11] — 2026-07-19
### Added
- **"System" theme option.** The light/dark toggle is now a three-way choice (System / Light / Dark). "System" follows the Windows app theme live — switching Windows between light and dark mode restyles the app immediately.

## [v1.3.10] — 2026-07-19
### Added
- **Sensor strip** below the status bar showing the individual signals the app publishes: microphone (on/muted), camera (on/off) and whether Teams is running — with the same colour coding as the status bar.
- **Release Notes button** in the Updates card. Opens the GitHub release page for the offered update (or, when up to date, the installed version) in the default browser, where the notes render properly — instead of the raw markdown the old `window.confirm()` prompt used to show.
### Fixed
- A scrollbar could still appear around the window itself (e.g. when the Updates card grows or wraps); window-level scrolling is now disabled entirely — the content area remains the only (invisible) scroller, and the updater row wraps cleanly now that it holds two buttons.

## [v1.3.9] — 2026-07-19
### Added
- **In-app Updates card** at the top of the window: shows the installed version and update status, with an explicit *Check for Updates* / *Install & Restart* button and a download progress bar. Replaces the `window.confirm()` prompt entirely.
- The update check now also runs **every hour**, not only at startup. When a background check finds an update, the window is brought up so it gets noticed.
### Fixed
- Applied review feedback from upstream PRs [#97](https://github.com/jimmyeao/TEAMS2HA/pull/97)/[#99](https://github.com/jimmyeao/TEAMS2HA/pull/99): single read-lock in `publish()`, and the mute-state cache is cleared when no Teams capture session exists so a new call's first reading always produces an event.
### Changed
- The main window no longer shows a scrollbar (content still scrolls); default window height increased to 780 px so everything fits.
- The legacy .NET/WPF app was removed from the repository (dormant, relied on the deprecated Teams local API; still available in git history and upstream).

## [v1.3.8] — 2026-07-19
### Fixed
- **Repaired the v1.3.7 regression.** v1.3.7 was accidentally built from a base without the fork features (home-network gating, availability/LWT, Modern Standby resume, close-to-tray, panic logging, `sw_version`). `master` is now synced with upstream (which merged those features via [PR #93](https://github.com/jimmyeao/TEAMS2HA/pull/93)) and carries the remaining fork commits, so this is the first release with **both** the auto-updater and all features. v1.3.7 is marked as a pre-release with a do-not-install warning.
- TLS transport selection: "Use TLS" combined with "ignore certificate errors" silently connected over **plain TCP**, and TLS + websockets produced `ws://` instead of `wss://`. TLS now always yields an encrypted transport; the unsupported cert-skip flag logs a warning.
- Updater prompt was fired in a hidden window (the app starts minimized to the tray), so an available update could never be confirmed. The window is now shown and focused first.
- Home detection: a crashed gateway-MAC lookup counted as "home" while a timed-out one counted as "not home"; both now resolve to "not home" (debounced as before).
- Mute detection: audio-API (COM) failures and "no Teams capture session" were both reported as *muted*, so a transient hiccup mid-call could produce a false mute flank. No reading now keeps the last known state.
### Changed
- State is published to MQTT only when it actually changed (a fresh connection still pushes the full state); the per-publish log line moved to debug level. Keeps `teams2ha.log` and the broker a lot quieter.
### Removed
- The `hasunreadmessages` binary sensor. Its heuristic matched practically every Teams log line, making the sensor meaningless; there is no reliable unread signal in the Teams logs. The retained discovery config is cleaned up automatically, removing the stale entity from Home Assistant.

## [v1.3.7] — 2026-07-19
### Added
- **Signed auto-updater.** The app now checks GitHub on startup for a newer signed release and can download, install and relaunch itself. Implemented with `tauri-plugin-updater` + `tauri-plugin-process`, a `plugins.updater` endpoint pointing at the release `latest.json`, an embedded minisign public key, and `createUpdaterArtifacts` in the build. Releases are signed in CI and now ship `latest.json` + `.sig` alongside the installers.
- Bilingual (EN + NL) release notes as the default release body.
### Fixed
- Correct updater capability `process:allow-restart` (an earlier attempt used the non-existent `process:allow-relaunch`, which failed the build).
### Changed
- Removed the Checkmarx workflow (it ran on a Windows runner but is a Linux-only container action, so it failed on every PR).
### Note
- Existing installs (which predate the updater) must install v1.3.7 manually once; every release after this updates automatically.

## [v1.3.6] — 2026-07-14
### Fixed
- Clear `is_in_meeting` on the log "call ended" signal, no longer gated on presence.

## [v1.3.5] — 2026-07-14
### Added
- Log panics to a file (the windowless app otherwise loses stderr), next to `settings.json`.

## [v1.3.4] — 2026-07-13
### Added
- Report `sw_version` in the MQTT discovery device block (previously showed a stale version from the old .NET install).

## [v1.3.3] — 2026-07-13
### Changed
- The window close button hides to the tray instead of quitting, keeping the MQTT bridge alive.

## [v1.3.2] — 2026-07-13
### Fixed
- Reconnect and refresh state after Modern Standby / sleep (resume detection via a clock gap in the gating poller); ARP lookup timeout hardened; presence defaults to "Unknown" to override stale retained state.

## [v1.3.1] — 2026-07-10
### Fixed
- Review follow-ups: run the ARP lookup off the async runtime (`spawn_blocking`); use a watch channel for settings instead of reloading per poll.

## [v1.3.0] — 2026-07-10
### Added
- First Rust/Tauri release of the fork.
- **Home-network gating** — connect to MQTT only when the default-gateway MAC matches the configured home network.
- **Availability / LWT** — every entity gets a retained Last Will availability topic, so entities go `unavailable` when the app is away/asleep/closed. This is the structural fix for a "sticky" `is_in_meeting` that would otherwise stay `on` forever.

---

### Earlier versions (1.0.x – 1.2.x)
These were the legacy **.NET / WPF** builds of Teams2HA (upstream). They relied on the Microsoft Teams local API, which Microsoft has since deprecated — the reason for the Rust/Tauri rewrite from v1.3.0 onward. The .NET source was removed from this fork after v1.3.7 (still available in the git history and upstream).

[v1.3.14]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.14
[v1.3.13]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.13
[v1.3.12]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.12
[v1.3.11]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.11
[v1.3.10]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.10
[v1.3.9]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.9
[v1.3.8]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.8
[v1.3.7]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.7
[v1.3.6]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.6
[v1.3.5]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.5
[v1.3.4]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.4
[v1.3.3]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.3
[v1.3.2]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.2
[v1.3.1]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.1
[v1.3.0]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.0
