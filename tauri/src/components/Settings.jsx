import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

const DEFAULT_SETTINGS = {
  mqttAddress: "",
  mqttPort: 1883,
  mqttUsername: "",
  mqttPassword: "",
  sensorPrefix: "",
  useTls: false,
  ignoreCertErrors: false,
  useWebsockets: false,
  runAtBoot: false,
  runMinimized: false,
  theme: "dark",
  colorScheme: "DeepPurple / Lime",
  homeGatewayMac: "",
};

export default function Settings() {
  const [settings, setSettings] = useState(DEFAULT_SETTINGS);
  const [saving, setSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useState(null);

  useEffect(() => {
    invoke("get_settings")
      .then(setSettings)
      .catch((e) => console.error("load settings:", e));
  }, []);

  // Apply theme immediately on change (before save). "system" resolves to the
  // Windows app theme via prefers-color-scheme (WebView2 tracks it live).
  useEffect(() => {
    const query = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const resolved =
        settings.theme === "system" ? (query.matches ? "dark" : "light") : settings.theme;
      document.documentElement.setAttribute("data-theme", resolved);
    };
    apply();
    if (settings.theme !== "system") return;
    query.addEventListener("change", apply);
    return () => query.removeEventListener("change", apply);
  }, [settings.theme]);

  const set = (key, value) => setSettings((s) => ({ ...s, [key]: value }));

  const useCurrentNetwork = async () => {
    try {
      const mac = await invoke("get_current_gateway_mac");
      if (mac) {
        set("homeGatewayMac", mac);
      } else {
        setSaveStatus("error: could not detect the current gateway MAC");
        setTimeout(() => setSaveStatus(null), 4000);
      }
    } catch (err) {
      setSaveStatus("error: " + err);
      setTimeout(() => setSaveStatus(null), 4000);
    }
  };

  const handleSave = async (e) => {
    e.preventDefault();
    setSaving(true);
    setSaveStatus(null);
    try {
      await invoke("save_settings", { settings });
      setSaveStatus("saved");
    } catch (err) {
      setSaveStatus("error: " + err);
    } finally {
      setSaving(false);
      setTimeout(() => setSaveStatus(null), 3000);
    }
  };

  return (
    <form className="settings-form" onSubmit={handleSave}>

      {/* MQTT Configuration */}
      <section className="card">
        <h2 className="card-title">MQTT Configuration</h2>

        <div className="field-row">
          <div className="field flex-grow">
            <label>Host Address</label>
            <input
              type="text"
              value={settings.mqttAddress}
              onChange={(e) => set("mqttAddress", e.target.value)}
              placeholder="e.g. 192.168.1.10"
            />
          </div>
          <div className="field field-narrow">
            <label>Port</label>
            <input
              type="number"
              value={settings.mqttPort}
              onChange={(e) => set("mqttPort", parseInt(e.target.value) || 1883)}
              min={1}
              max={65535}
            />
          </div>
        </div>

        <div className="field">
          <label>Username</label>
          <input
            type="text"
            value={settings.mqttUsername}
            onChange={(e) => set("mqttUsername", e.target.value)}
          />
        </div>

        <div className="field">
          <label>Password</label>
          <input
            type="password"
            value={settings.mqttPassword}
            onChange={(e) => set("mqttPassword", e.target.value)}
          />
        </div>

        <div className="chip-row">
          <Chip
            label="TLS"
            checked={settings.useTls}
            onChange={(v) => set("useTls", v)}
          />
          <Chip
            label="Ignore Cert Errors"
            checked={settings.ignoreCertErrors}
            onChange={(v) => set("ignoreCertErrors", v)}
          />
          <Chip
            label="WebSockets"
            checked={settings.useWebsockets}
            onChange={(v) => set("useWebsockets", v)}
          />
        </div>
      </section>

      {/* Options */}
      <section className="card">
        <h2 className="card-title">Options</h2>

        <div className="field">
          <label>Sensor Prefix</label>
          <input
            type="text"
            value={settings.sensorPrefix}
            onChange={(e) => set("sensorPrefix", e.target.value)}
            placeholder="Your machine name"
          />
        </div>

        <div className="chip-row">
          <Chip
            label="Run at Boot"
            checked={settings.runAtBoot}
            onChange={(v) => set("runAtBoot", v)}
          />
          <Chip
            label="Start Minimised"
            checked={settings.runMinimized}
            onChange={(v) => set("runMinimized", v)}
          />
        </div>

        <div className="field">
          <label>Theme</label>
          <div className="chip-row">
            {[
              ["system", "🖥 System"],
              ["light", "☀ Light"],
              ["dark", "🌙 Dark"],
            ].map(([value, label]) => (
              <button
                type="button"
                key={value}
                className={`chip ${settings.theme === value ? "chip-active" : ""}`}
                onClick={() => set("theme", value)}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
      </section>

      {/* Home Detection */}
      <section className="card">
        <h2 className="card-title">Home Detection</h2>
        <p className="card-hint">
          Only connect to MQTT while on your home network, matched by the default
          gateway&apos;s MAC address. Leave empty to always connect. Away from home,
          all entities show as unavailable in Home Assistant.
        </p>
        <div className="field-row">
          <div className="field flex-grow">
            <label>Home gateway MAC</label>
            <input
              type="text"
              value={settings.homeGatewayMac}
              onChange={(e) => set("homeGatewayMac", e.target.value)}
              placeholder="e.g. AA:BB:CC:DD:EE:FF"
            />
          </div>
          <button type="button" className="btn-secondary" onClick={useCurrentNetwork}>
            Use Current Network
          </button>
        </div>
      </section>

      {/* Actions */}
      <div className="action-row">
        {saveStatus && (
          <span className={`save-status ${saveStatus === "saved" ? "ok" : "err"}`}>
            {saveStatus === "saved" ? "✓ Saved" : saveStatus}
          </span>
        )}
        <button type="submit" className="btn-primary" disabled={saving}>
          {saving ? "Saving…" : "Save Settings"}
        </button>
      </div>

    </form>
  );
}

function Chip({ label, checked, onChange }) {
  return (
    <label className={`chip ${checked ? "chip-active" : ""}`}>
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        hidden
      />
      {label}
    </label>
  );
}
