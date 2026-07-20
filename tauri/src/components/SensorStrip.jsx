// Individual sensor readouts, one pill per sensor, shown directly below the
// header bar. The header keeps the technical preconditions (MQTT / Teams running);
// this strip shows the observed Teams signals the app publishes to MQTT.

// Presence keyword → pill colour, loosely matching the Teams presence bullets:
// green for Available, red for Busy/DoNotDisturb, orange for Away/BeRightBack.
const PRESENCE_STATE = {
  Available: "ok",
  Busy: "alert",
  DoNotDisturb: "alert",
  Away: "active",
  BeRightBack: "active",
  Offline: "idle",
};

export default function SensorStrip({ meetingState }) {
  if (!meetingState) return null;

  const presence = meetingState.presence || "";
  const sensors = [
    {
      label: "Meeting",
      value: meetingState.isInMeeting ? "In meeting" : "Not in meeting",
      state: meetingState.isInMeeting ? "active" : "idle",
    },
    ...(presence
      ? [{
          label: "Status",
          value: presence,
          state: PRESENCE_STATE[presence] || "idle",
        }]
      : []),
    {
      label: "Mic",
      value: meetingState.isMuted ? "Muted" : "On",
      state: meetingState.isMuted ? "alert" : "ok",
    },
    {
      label: "Camera",
      value: meetingState.isVideoOn ? "On" : "Off",
      state: meetingState.isVideoOn ? "active" : "idle",
    },
    {
      label: "Mic (system)",
      value: meetingState.micSystemMuted ? "Muted" : "On",
      state: meetingState.micSystemMuted ? "alert" : "ok",
    },
  ];

  return (
    <div className="sensor-strip">
      {sensors.map((s) => (
        <span key={s.label} className={`sensor-pill sensor-${s.state}`}>
          <span className="sensor-dot" />
          {s.label}: {s.value}
        </span>
      ))}
    </div>
  );
}
