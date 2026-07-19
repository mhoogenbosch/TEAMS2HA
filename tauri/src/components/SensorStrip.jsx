// Individual sensor readouts, one pill per sensor, shown directly below the
// header bar. The header keeps the aggregates (MQTT / in-meeting / presence);
// this strip shows the underlying signals the app publishes to MQTT.
export default function SensorStrip({ meetingState }) {
  if (!meetingState) return null;

  const sensors = [
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
      label: "Teams",
      value: meetingState.teamsRunning ? "Running" : "Not running",
      state: meetingState.teamsRunning ? "ok" : "idle",
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
