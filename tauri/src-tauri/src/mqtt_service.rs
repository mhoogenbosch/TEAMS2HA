use crate::settings::Settings;
use anyhow::Result;
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS, TlsConfiguration, Transport};
use serde_json::json;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, watch};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingState {
    pub is_muted: bool,
    pub is_video_on: bool,
    pub is_in_meeting: bool,
    pub teams_running: bool,
    pub presence: String,
    /// Master mute of the default communications microphone (system-wide,
    /// genuinely settable from HA — unlike the Teams session mute above).
    pub mic_system_muted: bool,
    pub session_locked: bool,
}

impl Default for MeetingState {
    fn default() -> Self {
        Self {
            is_muted: false,
            is_video_on: false,
            is_in_meeting: false,
            teams_running: false,
            // "Unknown" instead of empty: the first post-connect state publish then
            // overwrites a stale retained presence on the broker (e.g. 'Busy' from
            // before a crash) instead of skipping the topic and leaving it stand.
            presence: "Unknown".into(),
            mic_system_muted: false,
            session_locked: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum MqttCommand {
    SetSystemMicMute(bool),
    /// Show a Windows toast; payload is the raw notify message (plain text or
    /// JSON with title/message).
    Notify(String),
}

pub struct MqttService {
    client: AsyncClient,
    prefix: String,
    // Dropping this signals the eventloop to stop.
    _stop_tx: watch::Sender<bool>,
}

impl MqttService {
    pub async fn connect(
        settings: &Settings,
        cmd_tx: mpsc::Sender<MqttCommand>,
        reconnect_tx: mpsc::Sender<()>,
        app: AppHandle,
    ) -> Result<Self> {
        let prefix = settings.sensor_prefix.to_lowercase();
        let port = settings.mqtt_port;

        let mut opts = MqttOptions::new(
            format!("teams2ha-{}", hostname::get()?.to_string_lossy()),
            &settings.mqtt_address,
            port,
        );
        opts.set_keep_alive(Duration::from_secs(30));
        opts.set_clean_session(true);

        // Last Will: whenever the connection dies without a clean DISCONNECT (crash, sleep,
        // leaving the network, or the app dropping the service on purpose), the broker marks
        // all entities unavailable in HA — instead of leaving stale retained states behind
        // (e.g. is_in_meeting stuck 'on' after closing the laptop mid-call).
        opts.set_last_will(LastWill::new(
            availability_topic(&prefix),
            "offline",
            QoS::AtLeastOnce,
            true,
        ));

        if !settings.mqtt_username.is_empty() {
            opts.set_credentials(&settings.mqtt_username, &settings.mqtt_password);
        }

        // "Use TLS" must always yield an encrypted transport. Previously the
        // ignore_cert_errors flag silently downgraded TLS to plain TCP, and the
        // TLS+websockets combination came out as unencrypted ws://.
        if settings.use_tls {
            if settings.ignore_cert_errors {
                log::warn!(
                    "MQTT: 'ignore certificate errors' is not supported; \
                     connecting with TLS and full certificate verification"
                );
            }
            if settings.use_websockets {
                opts.set_transport(Transport::Wss(TlsConfiguration::Native));
            } else {
                opts.set_transport(Transport::Tls(TlsConfiguration::Native));
            }
        } else if settings.use_websockets {
            opts.set_transport(Transport::Ws);
        }

        // Roomy request queue, and everything that publishes into it uses the
        // non-blocking try_* variants: while the broker is unreachable the
        // eventloop sits in its retry sleep and drains nothing, so an awaited
        // publish on a full queue would block the caller — in publish_state's
        // case the central event loop, freezing every monitor in the app.
        let (client, mut eventloop) = AsyncClient::new(opts, 256);
        let (stop_tx, mut stop_rx) = watch::channel(false);

        let client_clone = client.clone();
        let prefix_clone = prefix.clone();

        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    // Sender dropped (MqttService replaced/dropped) → stop.
                    _ = stop_rx.changed() => {
                        log::info!("MQTT: eventloop stopping");
                        break;
                    }
                    event = eventloop.poll() => match event {
                        Ok(Event::Incoming(Packet::ConnAck(_))) => {
                            log::info!("MQTT: connected to broker");
                            app.emit("mqtt-status", "Connected").ok();
                            publish_availability(&client_clone, &prefix_clone, true);
                            subscribe(&client_clone, &prefix_clone);
                            publish_discovery_inner(&client_clone, &prefix_clone);
                            let _ = reconnect_tx.send(()).await;
                        }
                        Ok(Event::Incoming(Packet::Publish(msg))) => {
                            handle_incoming(&prefix_clone, &msg.topic, &msg.payload, &cmd_tx).await;
                        }
                        Ok(Event::Outgoing(rumqttc::Outgoing::Disconnect)) => {
                            log::info!("MQTT: disconnect sent");
                        }
                        Err(e) => {
                            log::warn!("MQTT error: {e}");
                            app.emit("mqtt-status", "Disconnected").ok();
                            // Wait before retry, but honour stop signal.
                            tokio::select! {
                                _ = stop_rx.changed() => break,
                                _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
            log::info!("MQTT: eventloop exited");
        });

        Ok(Self {
            client,
            prefix,
            _stop_tx: stop_tx,
        })
    }

    /// Publish the full state. Returns Err when any topic could not be handed to
    /// the client (e.g. request queue full while the broker is unreachable) — the
    /// caller uses that to NOT cache the state as delivered, so the next event or
    /// the ConnAck re-publish retries it. Non-blocking by design: an awaited
    /// publish would stall the central event loop while the eventloop is in its
    /// reconnect-retry sleep.
    pub fn publish_state(&self, state: &MeetingState) -> Result<()> {
        let prefix = &self.prefix;
        let mut all_ok = true;

        let bool_pairs: &[(&str, &str, bool)] = &[
            ("switch", "micsystemmuted", state.mic_system_muted),
            ("binary_sensor", "ismuted", state.is_muted),
            ("binary_sensor", "isvideoon", state.is_video_on),
            ("binary_sensor", "isinmeeting", state.is_in_meeting),
            ("binary_sensor", "teamsrunning", state.teams_running),
            ("binary_sensor", "sessionlocked", state.session_locked),
        ];
        for (component, id, value) in bool_pairs {
            if let Err(e) = self.client.try_publish(
                format!("homeassistant/{component}/{prefix}/{id}/state"),
                QoS::AtLeastOnce,
                true,
                if *value { "ON" } else { "OFF" },
            ) {
                all_ok = false;
                log::warn!("MQTT publish failed [{id}]: {e}");
            }
        }

        if !state.presence.is_empty() {
            log::debug!(
                "MQTT publishing teamsstatus: '{}' → homeassistant/sensor/{prefix}/teamsstatus/state",
                state.presence
            );
            if let Err(e) = self.client.try_publish(
                format!("homeassistant/sensor/{prefix}/teamsstatus/state"),
                QoS::AtLeastOnce,
                true,
                state.presence.as_bytes().to_vec(),
            ) {
                all_ok = false;
                log::warn!("MQTT publish failed [teamsstatus]: {e}");
            }
        }

        anyhow::ensure!(all_ok, "one or more MQTT state publishes failed");
        Ok(())
    }
}

fn availability_topic(prefix: &str) -> String {
    format!("teams2ha/{prefix}/availability")
}

fn notify_topic(prefix: &str) -> String {
    format!("teams2ha/{prefix}/notify")
}

// The ConnAck-path helpers below use try_publish/try_subscribe: they run inside
// the eventloop task itself, and awaiting the request queue from there while it
// is full would deadlock the loop that is supposed to drain it.
fn publish_availability(client: &AsyncClient, prefix: &str, online: bool) {
    if let Err(e) = client.try_publish(
        availability_topic(prefix),
        QoS::AtLeastOnce,
        true,
        if online { "online" } else { "offline" },
    ) {
        log::warn!("MQTT availability publish failed: {e}");
    }
}

fn subscribe(client: &AsyncClient, prefix: &str) {
    for topic in [
        format!("homeassistant/switch/{prefix}/+/set"),
        notify_topic(prefix),
    ] {
        if let Err(e) = client.try_subscribe(topic, QoS::AtLeastOnce) {
            log::warn!("MQTT subscribe error: {e}");
        }
    }
}

fn publish_discovery_inner(client: &AsyncClient, prefix: &str) {
    let device = json!({
        "identifiers": [format!("teams2ha_{prefix}")],
        "name": format!("Teams2HA ({})", prefix),
        "model": "Teams2HA",
        "manufacturer": "jimmyeao",
        // Real app version (release builds get it stamped from the git tag). Without this
        // the device registry keeps showing whatever an older install once published.
        "sw_version": env!("CARGO_PKG_VERSION")
    });

    let switches = [("micsystemmuted", "Mic Muted (System)")];
    let binary_sensors = [
        ("ismuted", "Is Muted"),
        ("isvideoon", "Is Video On"),
        ("isinmeeting", "Is In Meeting"),
        ("teamsrunning", "Teams Running"),
        ("sessionlocked", "Session Locked"),
    ];

    // One-time cleanups. Empty retained payloads remove the old discovery
    // config and state from the broker (and thereby the entity from HA):
    // - 'hasunreadmessages': retired, its detection heuristic was meaningless.
    // - switch 'ismuted'/'isvideoon': re-published as binary_sensor (see above)
    //   because their command path died with the retired Teams local API — a
    //   toggle that does nothing is worse than a sensor.
    for topic in [
        format!("homeassistant/binary_sensor/{prefix}/hasunreadmessages/config"),
        format!("homeassistant/binary_sensor/{prefix}/hasunreadmessages/state"),
        format!("homeassistant/switch/{prefix}/ismuted/config"),
        format!("homeassistant/switch/{prefix}/ismuted/state"),
        format!("homeassistant/switch/{prefix}/isvideoon/config"),
        format!("homeassistant/switch/{prefix}/isvideoon/state"),
    ] {
        let _ = client.try_publish(topic, QoS::AtLeastOnce, true, "");
    }

    for (id, name) in &switches {
        let payload = json!({
            "name": name,
            "unique_id": format!("{prefix}_{id}"),
            "state_topic": format!("homeassistant/switch/{prefix}/{id}/state"),
            "command_topic": format!("homeassistant/switch/{prefix}/{id}/set"),
            "payload_on": "ON",
            "payload_off": "OFF",
            "availability_topic": availability_topic(prefix),
            "payload_available": "online",
            "payload_not_available": "offline",
            "device": device
        });
        if let Err(e) = client.try_publish(
            format!("homeassistant/switch/{prefix}/{id}/config"),
            QoS::AtLeastOnce,
            true,
            serde_json::to_vec(&payload).unwrap_or_default(),
        ) {
            log::warn!("Discovery publish failed for {id}: {e}");
        }
    }

    for (id, name) in &binary_sensors {
        let payload = json!({
            "name": name,
            "unique_id": format!("{prefix}_{id}"),
            "state_topic": format!("homeassistant/binary_sensor/{prefix}/{id}/state"),
            "payload_on": "ON",
            "payload_off": "OFF",
            "availability_topic": availability_topic(prefix),
            "payload_available": "online",
            "payload_not_available": "offline",
            "device": device
        });
        if let Err(e) = client.try_publish(
            format!("homeassistant/binary_sensor/{prefix}/{id}/config"),
            QoS::AtLeastOnce,
            true,
            serde_json::to_vec(&payload).unwrap_or_default(),
        ) {
            log::warn!("Discovery publish failed for {id}: {e}");
        }
    }

    // Notify entity: HA's notify.send_message publishes the message text to the
    // command topic; the app shows it as a Windows toast. Lets automations reach
    // this machine ("doorbell", "laundry done") even during headphone meetings.
    let notify_payload = json!({
        "name": "Toast",
        "unique_id": format!("{prefix}_toast"),
        "command_topic": notify_topic(prefix),
        "icon": "mdi:message-badge",
        "availability_topic": availability_topic(prefix),
        "payload_available": "online",
        "payload_not_available": "offline",
        "device": device
    });
    if let Err(e) = client.try_publish(
        format!("homeassistant/notify/{prefix}/toast/config"),
        QoS::AtLeastOnce,
        true,
        serde_json::to_vec(&notify_payload).unwrap_or_default(),
    ) {
        log::warn!("Discovery publish failed for toast: {e}");
    }

    let teamsstatus_payload = json!({
        "name": "Teams Status",
        "unique_id": format!("{prefix}_teamsstatus"),
        "state_topic": format!("homeassistant/sensor/{prefix}/teamsstatus/state"),
        "icon": "mdi:account-circle",
        "availability_topic": availability_topic(prefix),
        "payload_available": "online",
        "payload_not_available": "offline",
        "device": device
    });
    if let Err(e) = client.try_publish(
        format!("homeassistant/sensor/{prefix}/teamsstatus/config"),
        QoS::AtLeastOnce,
        true,
        serde_json::to_vec(&teamsstatus_payload).unwrap_or_default(),
    ) {
        log::warn!("Discovery publish failed for teamsstatus: {e}");
    }

    log::info!("MQTT: discovery published for prefix '{prefix}'");
}

async fn handle_incoming(
    prefix: &str,
    topic: &str,
    payload: &[u8],
    cmd_tx: &mpsc::Sender<MqttCommand>,
) {
    let payload_str = std::str::from_utf8(payload).unwrap_or("").trim();
    log::debug!("MQTT incoming: {topic} = {payload_str}");

    if topic == notify_topic(prefix) {
        if !payload_str.is_empty() {
            let _ = cmd_tx
                .send(MqttCommand::Notify(payload_str.to_string()))
                .await;
        }
        return;
    }

    // The only remaining commandable switch. The old ismuted/isvideoon set
    // topics are gone with their command path (retired Teams local API).
    if topic == format!("homeassistant/switch/{prefix}/micsystemmuted/set") {
        let _ = cmd_tx
            .send(MqttCommand::SetSystemMicMute(payload_str == "ON"))
            .await;
    }
}
