# Changelog

All notable changes to this fork ([mhoogenbosch/TEAMS2HA](https://github.com/mhoogenbosch/TEAMS2HA)) are documented here.
The format is loosely based on [Keep a Changelog](https://keepachangelog.com/). Original app by [jimmyeao](https://github.com/jimmyeao/TEAMS2HA).

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
