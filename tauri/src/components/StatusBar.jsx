// Header bar: the technical preconditions only (is the pipeline able to work at all?).
// MQTT = can we publish; Teams running = is there anything to observe. The observed
// Teams signals themselves (meeting, presence, mic, camera) live in the SensorStrip.
export default function StatusBar({ mqttStatus, meetingState }) {
  const connected = mqttStatus === "Connected";

  return (
    <div className="status-bar">
      <div className={`status-indicator ${connected ? "connected" : "disconnected"}`}>
        <span className="status-dot" />
        <span className="status-label">MQTT: {mqttStatus}</span>
      </div>
      {meetingState && (
        <div className={`status-indicator ${meetingState.teamsRunning ? "connected" : "disconnected"}`}>
          <span className="status-dot" />
          <span className="status-label">
            Teams: {meetingState.teamsRunning ? "Running" : "Not running"}
          </span>
        </div>
      )}
    </div>
  );
}
