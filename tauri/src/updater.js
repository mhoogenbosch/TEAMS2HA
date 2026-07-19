import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getCurrentWindow } from "@tauri-apps/api/window";

// Checks the GitHub releases feed (via the endpoint configured in
// tauri.conf.json -> plugins.updater) for a newer, signed version. When one is
// found the user is asked; on confirmation it is downloaded, installed and the
// app relaunches. Signature is verified against the embedded public key, so a
// tampered latest.json / installer is rejected automatically.
export async function checkForUpdates({ silent = true } = {}) {
  try {
    const update = await check();

    if (!update) {
      if (!silent) window.alert("Je gebruikt de nieuwste versie van Teams2HA.");
      return;
    }

    // The app usually starts hidden in the tray ("run minimized"); a confirm()
    // fired in a hidden window is invisible, so the update would never proceed.
    // Bring the window up before asking.
    try {
      const win = getCurrentWindow();
      await win.show();
      await win.setFocus();
    } catch (e) {
      console.warn("Kon venster niet tonen voor update-prompt:", e);
    }

    const ok = window.confirm(
      `Teams2HA ${update.version} is beschikbaar (huidig: ${update.currentVersion}).\n\n` +
        `${update.body ?? ""}\n\nNu downloaden, installeren en de app herstarten?`,
    );
    if (!ok) return;

    let downloaded = 0;
    let total = 0;
    await update.downloadAndInstall((event) => {
      switch (event.event) {
        case "Started":
          total = event.data.contentLength ?? 0;
          console.log(`Update-download gestart (${total} bytes)`);
          break;
        case "Progress":
          downloaded += event.data.chunkLength;
          console.log(`Gedownload ${downloaded}/${total}`);
          break;
        case "Finished":
          console.log("Download klaar — installeren…");
          break;
      }
    });

    await relaunch();
  } catch (err) {
    // Never let an update failure crash the app; just log it.
    console.error("Update-check mislukt:", err);
    if (!silent) window.alert("Update-check mislukt: " + err);
  }
}
