import { parseShareLink } from "./immich.js";
import { DEFAULTS, getSettings } from "./settings.js";

const $ = (id) => document.getElementById(id);

function msg(el, text, kind = "") {
  el.textContent = text;
  el.className = `${el.id === "status" ? "status" : "hint"} ${kind}`;
}

async function showStatus() {
  const { status } = await chrome.storage.local.get("status");
  if (!status) return;
  if (status.lastError) return msg($("status"), `Problem: ${status.lastError}`, "err");
  const parts = [];
  if (status.lastChanged) parts.push(`Last changed ${new Date(status.lastChanged).toLocaleTimeString()}`);
  if (status.photoCount != null) parts.push(`${status.photoCount} photos in album`);
  msg($("status"), parts.join(" · "));
}

/**
 * Ask for access to the user's server. Must be called synchronously inside a
 * click handler (before any await) so Chrome treats it as a user gesture.
 */
function requestHostAccess(shareUrl) {
  const { server } = parseShareLink(shareUrl);
  return chrome.permissions.request({ origins: [`${server}/*`] }).then((granted) => {
    if (!granted) throw new Error("Chrome needs permission to reach your photo server.");
  });
}

function send(message) {
  return chrome.runtime.sendMessage(message).then((r) => {
    if (!r?.ok) throw new Error(r?.error || "Unknown error");
    return r;
  });
}

async function init() {
  $("notChromeOS").hidden = !!chrome.wallpaper;
  $("osSettings").hidden = !chrome.wallpaper;
  const s = await getSettings();
  $("shareUrl").value = s.shareUrl;
  const interval = $("interval");
  if (![...interval.options].some((o) => Number(o.value) === s.intervalMinutes)) {
    interval.add(new Option(`${s.intervalMinutes} minutes`, s.intervalMinutes));
  }
  interval.value = String(s.intervalMinutes);
  $("orientation").value = s.orientation;
  $("preferOriginal").checked = s.preferOriginal;
  $("paused").checked = s.paused;
  await showStatus();
  chrome.storage.onChanged.addListener((c, area) => area === "local" && c.status && showStatus());
}

$("test").addEventListener("click", () => {
  const url = $("shareUrl").value.trim();
  let access;
  try {
    access = requestHostAccess(url);
  } catch (e) {
    return msg($("testResult"), e.message, "err");
  }
  msg($("testResult"), "Checking…");
  access
    .then(() => send({ type: "test", shareUrl: url }))
    .then((r) => msg($("testResult"), `Connected: ${r.count} photos found.`, "ok"))
    .catch((e) => msg($("testResult"), e.message, "err"));
});

$("save").addEventListener("click", () => {
  const settings = {
    ...DEFAULTS,
    shareUrl: $("shareUrl").value.trim(),
    intervalMinutes: Number($("interval").value),
    orientation: $("orientation").value,
    preferOriginal: $("preferOriginal").checked,
    paused: $("paused").checked,
  };
  let access = Promise.resolve();
  if (settings.shareUrl) {
    try {
      access = requestHostAccess(settings.shareUrl);
    } catch (e) {
      return msg($("status"), e.message, "err");
    }
  }
  access
    .then(() => chrome.storage.sync.set(settings))
    .then(() => {
      msg($("status"), "Saved.", "ok");
      if (settings.shareUrl && !settings.paused && chrome.wallpaper) {
        msg($("status"), "Saved. Changing wallpaper…", "ok");
        return send({ type: "next" });
      }
    })
    .catch((e) => msg($("status"), e.message, "err"));
});

$("next").addEventListener("click", () => {
  msg($("status"), "Fetching a new photo…");
  send({ type: "next" }).catch((e) => msg($("status"), e.message, "err"));
});

$("openWallpaper").addEventListener("click", () => {
  chrome.tabs
    .create({ url: "chrome://personalization/wallpaper" })
    .catch(() => msg($("status"), "Open the ChromeOS launcher and search for “Wallpaper”.", "err"));
});

init();
