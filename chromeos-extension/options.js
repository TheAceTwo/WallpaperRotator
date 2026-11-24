import { parseServer, parseShareLink } from "./immich.js";
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
function requestHostAccess(albumUrls, backupDomain) {
  const servers = albumUrls.filter(Boolean).map((u) => parseShareLink(u).server);
  if (backupDomain) servers.push(parseServer(backupDomain));
  const origins = [...new Set(servers)].map((s) => `${s}/*`);
  return chrome.permissions.request({ origins }).then((granted) => {
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
  albums = s.albums.map((a) => ({ ...a }));
  $("backupDomain").value = s.backupDomain;
  $("useBackupDomain").checked = s.useBackupDomain;
  renderAlbums(s.albumMode, s.selectedAlbum);
  showBackupRow();
  const interval = $("interval");
  if (![...interval.options].some((o) => Number(o.value) === s.intervalMinutes)) {
    interval.add(new Option(`${s.intervalMinutes} minutes`, s.intervalMinutes));
  }
  interval.value = String(s.intervalMinutes);
  $("orientation").value = s.orientation;
  $("preferOriginal").checked = s.preferOriginal;
  $("paused").checked = s.paused;
  $("changeOnStartup").checked = s.changeOnStartup;
  $("changeOnUnlock").checked = s.changeOnUnlock;
  $("dim").value = String(s.dimPercent);
  $("blur").value = String(s.blurLevel);
  showDimValue();
  await showStatus();
  chrome.storage.onChanged.addListener((c, area) => area === "local" && c.status && showStatus());
}

// ---- Albums ----
let albums = [];
const albumTitle = (a) => a.name?.trim() || a.url;
const currentMode = () => document.querySelector('input[name="albumMode"]:checked')?.value ?? "all";

function renderAlbums(mode, selected) {
  const list = $("albumList");
  list.replaceChildren();
  albums.forEach((a, i) => {
    const li = document.createElement("li");
    const text = document.createElement("div");
    text.className = "album-text";
    const name = document.createElement("div");
    name.className = "album-name";
    name.textContent = albumTitle(a);
    text.append(name);
    if (a.name?.trim()) {
      const url = document.createElement("div");
      url.className = "album-url";
      url.textContent = a.url;
      text.append(url);
    }
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "icon-button";
    remove.title = "Remove album";
    remove.setAttribute("aria-label", `Remove ${albumTitle(a)}`);
    remove.textContent = "\u2715";
    remove.addEventListener("click", () => {
      albums.splice(i, 1);
      renderAlbums(currentMode(), $("selectedAlbum").value);
      msg($("testResult"), "Album removed. Click Save to keep this change.");
    });
    li.append(text, remove);
    list.append(li);
  });
  const picker = $("selectedAlbum");
  picker.replaceChildren(...albums.map((a) => new Option(albumTitle(a), a.url)));
  if (albums.some((a) => a.url === selected)) picker.value = selected;
  const radio = document.querySelector(`input[name="albumMode"][value="${mode === "one" ? "one" : "all"}"]`);
  if (radio) radio.checked = true;
  $("albumModeRow").hidden = albums.length < 2;
  picker.disabled = currentMode() !== "one";
}
document.querySelectorAll('input[name="albumMode"]').forEach((r) =>
  r.addEventListener("change", () => ($("selectedAlbum").disabled = currentMode() !== "one")),
);

const backupIfOn = () => ($("useBackupDomain").checked ? $("backupDomain").value.trim() : "");

$("addAlbum").addEventListener("click", () => {
  const url = $("newAlbumUrl").value.trim();
  if (!url) return;
  if (albums.some((a) => a.url === url)) return msg($("testResult"), "That album is already in the list.", "err");
  let access;
  try {
    access = requestHostAccess([url], backupIfOn()); // must start before any await
  } catch (e) {
    return msg($("testResult"), e.message, "err");
  }
  msg($("testResult"), "Checking…");
  access
    .then(() => send({ type: "inspect", url, backupDomain: backupIfOn() }))
    .then((r) => {
      albums.push({ name: r.name ?? "", url });
      $("newAlbumUrl").value = "";
      renderAlbums(currentMode(), $("selectedAlbum").value);
      msg($("testResult"), `Added${r.name ? ` “${r.name}”` : ""}: ${r.count} photos. Click Save to start using it.`, "ok");
    })
    .catch((e) => msg($("testResult"), e.message, "err"));
});
$("newAlbumUrl").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("addAlbum").click();
});

function showBackupRow() {
  $("backupLinkRow").hidden = !$("useBackupDomain").checked;
}
$("useBackupDomain").addEventListener("change", showBackupRow);

$("testBackup").addEventListener("click", () => {
  if (!albums.length) return msg($("testBackupResult"), "Add an album first.", "err");
  const domain = $("backupDomain").value.trim();
  let access;
  try {
    access = requestHostAccess([], domain);
  } catch (e) {
    return msg($("testBackupResult"), e.message, "err");
  }
  msg($("testBackupResult"), "Checking…");
  access
    .then(() => send({ type: "testBackup", domain, url: albums[0].url }))
    .then((r) => msg($("testBackupResult"), `Connected through the backup address: ${r.count} photos in “${albumTitle(albums[0])}”.`, "ok"))
    .catch((e) => msg($("testBackupResult"), e.message, "err"));
});

$("save").addEventListener("click", () => {
  const settings = {
    ...DEFAULTS,
    albums: albums.map((a) => ({ ...a })),
    albumMode: currentMode(),
    selectedAlbum: $("selectedAlbum").value,
    useBackupDomain: $("useBackupDomain").checked,
    backupDomain: $("backupDomain").value.trim(),
    intervalMinutes: Number($("interval").value),
    orientation: $("orientation").value,
    preferOriginal: $("preferOriginal").checked,
    paused: $("paused").checked,
    changeOnStartup: $("changeOnStartup").checked,
    changeOnUnlock: $("changeOnUnlock").checked,
    dimPercent: Number($("dim").value),
    blurLevel: Number($("blur").value),
  };
  let access = Promise.resolve();
  if (settings.albums.length) {
    try {
      access = requestHostAccess(settings.albums.map((a) => a.url), settings.useBackupDomain && settings.backupDomain);
    } catch (e) {
      return msg($("status"), e.message, "err");
    }
  }
  access
    .then(() => chrome.storage.sync.set(settings))
    .then(() => {
      msg($("status"), "Saved.", "ok");
      if (settings.albums.length && !settings.paused && chrome.wallpaper) {
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

function showDimValue() {
  const d = Number($("dim").value);
  const b = Number($("blur").value);
  $("dimValue").textContent = d === 0 ? "Off" : `${d}%`;
  $("blurValue").textContent = b === 0 ? "Off" : `${b}`;
}

async function applyEffects() {
  const dimPercent = Number($("dim").value);
  const blurLevel = Number($("blur").value);
  await chrome.storage.sync.set({ dimPercent, blurLevel });
  if (!chrome.wallpaper) return msg($("dimResult"), "Saved.", "ok");
  msg($("dimResult"), "Updating the wallpaper…");
  send({ type: "redim" })
    .then(() => msg($("dimResult"), "Wallpaper updated.", "ok"))
    .catch((e) => msg($("dimResult"), e.message, "err"));
}

for (const id of ["dim", "blur"]) {
  $(id).addEventListener("input", showDimValue);
  $(id).addEventListener("change", applyEffects);
}

$("openWallpaper").addEventListener("click", () => {
  chrome.tabs
    .create({ url: "chrome://personalization/wallpaper" })
    .catch(() => msg($("status"), "Open the ChromeOS launcher and search for “Wallpaper”.", "err"));
});

init();
