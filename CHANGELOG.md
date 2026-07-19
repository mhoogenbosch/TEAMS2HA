# Changelog

All notable changes to this fork ([mhoogenbosch/TEAMS2HA](https://github.com/mhoogenbosch/TEAMS2HA)) are documented here.
The format is loosely based on [Keep a Changelog](https://keepachangelog.com/). Original app by [jimmyeao](https://github.com/jimmyeao/TEAMS2HA).

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
These were the legacy **.NET / WPF** builds of Teams2HA (upstream). They relied on the Microsoft Teams local API, which Microsoft has since deprecated — the reason for the Rust/Tauri rewrite from v1.3.0 onward. The .NET source remains in the repository root, dormant.

[v1.3.7]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.7
[v1.3.6]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.6
[v1.3.5]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.5
[v1.3.4]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.4
[v1.3.3]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.3
[v1.3.2]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.2
[v1.3.1]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.1
[v1.3.0]: https://github.com/mhoogenbosch/TEAMS2HA/releases/tag/v1.3.0
