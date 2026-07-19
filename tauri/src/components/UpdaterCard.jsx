import { useState, useEffect, useRef, useCallback } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";

const RELEASES_URL = "https://github.com/mhoogenbosch/TEAMS2HA/releases";

// Checks the GitHub releases feed (endpoint in tauri.conf.json -> plugins.updater)
// on startup and then every hour. Signatures are verified against the embedded
// public key. Installing is an explicit in-app button — no window.confirm(),
// which is invisible when the app sits hidden in the tray.
const CHECK_INTERVAL_MS = 60 * 60 * 1000;

export default function UpdaterCard() {
  const [current, setCurrent] = useState("");
  // idle | checking | uptodate | available | downloading | installing | error
  const [status, setStatus] = useState("idle");
  const [update, setUpdate] = useState(null);
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState("");
  const busyRef = useRef(false);

  const doCheck = useCallback(async (background) => {
    if (busyRef.current) return; // never interrupt a running install
    setStatus("checking");
    setError("");
    try {
      const u = await check();
      if (u) {
        setUpdate(u);
        setStatus("available");
        if (background) {
          // Found by a scheduled check: surface the window so it gets noticed.
          try {
            const w = getCurrentWindow();
            await w.show();
            await w.setFocus();
          } catch (e) {
            console.warn("Could not show window for update notice:", e);
          }
        }
      } else {
        setUpdate(null);
        setStatus("uptodate");
      }
    } catch (e) {
      setStatus("error");
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    getVersion().then(setCurrent).catch(() => {});
    doCheck(true);
    const timer = setInterval(() => doCheck(true), CHECK_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [doCheck]);

  const install = async () => {
    if (!update || busyRef.current) return;
    busyRef.current = true;
    setStatus("downloading");
    setProgress(0);
    let downloaded = 0;
    let total = 0;
    try {
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? 0;
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            if (total > 0) setProgress(Math.round((downloaded / total) * 100));
            break;
          case "Finished":
            setStatus("installing");
            break;
        }
      });
      await relaunch();
    } catch (e) {
      busyRef.current = false;
      setStatus("error");
      setError(String(e));
    }
  };

  const busy = status === "downloading" || status === "installing";

  // GitHub renders the release notes properly (markdown, tables); with an
  // update pending show the notes of the offered version, otherwise the
  // installed one.
  const openReleaseNotes = () => {
    const version = update?.version ?? current;
    const url = version ? `${RELEASES_URL}/tag/v${version}` : RELEASES_URL;
    openUrl(url).catch((e) => console.error("Could not open release notes:", e));
  };

  const statusText = {
    idle: "",
    checking: "Checking for updates…",
    uptodate: "Up to date",
    available: update ? `Update ${update.version} available` : "Update available",
    downloading: `Downloading… ${progress}%`,
    installing: "Installing — the app will restart",
    error: `Update check failed: ${error}`,
  }[status];

  const statusClass =
    status === "uptodate" ? "ok" : status === "error" ? "err" : status === "available" ? "avail" : "";

  return (
    <section className="card">
      <div className="updater-row">
        <div className="updater-info">
          <span className="updater-version">Teams2HA {current ? `v${current}` : ""}</span>
          <span className={`updater-status ${statusClass}`}>{statusText}</span>
        </div>
        <div className="updater-actions">
          <button type="button" className="btn-secondary updater-check" onClick={openReleaseNotes}>
            Release Notes
          </button>
          {status === "available" || busy ? (
            <button type="button" className="btn-primary" onClick={install} disabled={busy}>
              {busy ? "Installing…" : `Install v${update.version} & Restart`}
            </button>
          ) : (
            <button
              type="button"
              className="btn-secondary updater-check"
              onClick={() => doCheck(false)}
              disabled={status === "checking"}
            >
              {status === "checking" ? "Checking…" : "Check for Updates"}
            </button>
          )}
        </div>
      </div>
      {busy && (
        <div className="progress-track">
          <div className="progress-fill" style={{ width: `${progress}%` }} />
        </div>
      )}
    </section>
  );
}
