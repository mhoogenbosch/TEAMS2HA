[![CodeQL](https://github.com/jimmyeao/TEAMS2HA/actions/workflows/codeql.yml/badge.svg)](https://github.com/jimmyeao/TEAMS2HA/actions/workflows/codeql.yml)[![GitHub tag](https://img.shields.io/github/tag/jimmyeao/TEAMS2HA?include_prereleases=&sort=semver&color=blue)](https://github.com/jimmyeao/TEAMS2HA/releases/)
[![License](https://img.shields.io/badge/License-MIT-blue)](#license)
[![issues - Teams2HA](https://img.shields.io/github/issues/jimmyeao/TEAMS2HA)](https://github.com/jimmyeao/TEAMS2HA/issues)

<H1>Teams2HA</H1>

<H1>IMPORTANT</H1>
  
Microsoft are deprecating the Teams local API, which has sadly broken our application.
I have written a new lightweight version in Rust/Tauri that uses teams logs and hardware signals to see if you are in a meeting, get your status, mute state and video state. You will need to remove the old version, and install this version - admin rights are NOT required.

Download the latest version from https://github.com/jimmyeao/TEAMS2HA/releases (app will auto update once installed)
<img width="822" height="712" alt="image" src="https://github.com/user-attachments/assets/5595f5ff-e4f3-44e6-8054-1cc381370fab" />



<h2>Fork changes (mhoogenbosch/TEAMS2HA)</h2>

This fork adds two things to the Tauri app (and equivalent fixes to the legacy WPF app):

<b>Home detection</b> — optionally only connect to MQTT while on your home network. Detection is based on the MAC address of the default IPv4 gateway (ARP), so it works on Wi-Fi and wired/docked connections, needs no location permission, and cannot be fooled by a VPN tunnel or a foreign network using the same subnet. Configure it in Settings &rarr; Home Detection ("Use Current Network" fills in the current gateway's MAC). Leave empty to always connect (upstream behaviour).

<b>Availability (Last Will)</b> — all entities now carry an MQTT availability topic (<code>teams2ha/&lt;prefix&gt;/availability</code>) with a retained Last Will. When the app exits, the laptop sleeps or you leave the home network, Home Assistant marks every Teams2HA entity <i>unavailable</i> instead of keeping the last retained state forever (no more <code>is_in_meeting</code> stuck 'on' after closing the laptop mid-call).

<b>Suspend/resume resilience (v1.3.2)</b> — after Modern Standby (S0ix) Windows delivers no reliable resume event to a suspended tray app, and the pre-sleep MQTT session can be a silently dead TCP connection. The app now detects a resume by clock gap and rebuilds the MQTT connection from scratch (fresh ConnAck &rarr; availability, discovery and current state re-published). The gateway-MAC lookup also got a timeout so one hung ARP call can no longer freeze home detection.

<b>No more phantom calls from a stale Consent Store (v1.3.2)</b> — Windows' privacy registry keeps <code>LastUsedTimeStop = 0</code> ("camera/mic in use") when Teams dies or the machine suspends mid-call. An 'active' reading now only counts after the device has been seen inactive at least once since app start or system resume, so a leftover marker can no longer publish a phantom in-meeting/video-on state. The app also logs to <code>teams2ha.log</code> next to <code>settings.json</code> (default level info, <code>RUST_LOG</code> overrides).

<h3>Code signing policy</h3>

Free code signing for this fork's releases is provided by <a href="https://about.signpath.io/">SignPath.io</a>, with a certificate by the <a href="https://signpath.org/">SignPath Foundation</a>.

Releases are built from this repository by GitHub Actions (see <code>.github/workflows/release.yml</code>) and signed through SignPath after manual approval of each release. Team roles: committer, reviewer and approver is Martijn Hoogenbosch (<a href="https://github.com/mhoogenbosch">@mhoogenbosch</a>).

Privacy: this program does not transfer any user data to any third party. It communicates exclusively with the MQTT broker you configure yourself.

<h2>MQTT</h2>

Provide your MQTT instance details (IP, username and password) The password is encrypted before being saved to the settings file and is not stored in clear text.
We support plain MQTT, MQTT over TLS, MQTT over Websockets and MQTT over Websockets with TLS and the ability to ignore certificate errors if you are using self-signed certs (I would strongly advise you to use Lets Encrypt as a minimum)

<h2>Entities</h2>

This is how it should look in MQTT in Homeassistant

The topic will be 
- homeassistant/switch/YOURNAME/ismuted
- homeassistant/switch/YOURNAME/isvideoon
- homeassistant/sensor/YOURNAME/teamsstatus/state
- homeassistant/sensor/YOURNAME/presence/state
- homeassistant/binary_sensor/YOURNAME/isinmeeting/state
- homeassistant/binary_sensor/YOURNAME/teamsrunning/state

<img width="1037" height="584" alt="image" src="https://github.com/user-attachments/assets/476b0107-d738-4f37-96a4-a50b9ed3ed6a" />

(note, 2 way control is not possible at the moment, investigating the reliability of addign this in)

Footnote: I have left the old .net source code intact, in case Microsoft reverse their decidion, the new code is in the Tauri folder, if you need to make changes. PRs always welcome :)




