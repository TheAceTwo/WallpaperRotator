const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const boolFields = ["setDesktop", "setLockScreen", "separateLockImage", "preferOriginal", "allowPolicyLockScreen", "autoUpdate", "changeOnStartup", "changeOnUnlock", "pauseDuringFullscreen", "useBackupLink", "smoothTransitions", "perVirtualDesktop"];

let config = null;

function setMsg(el, text, kind = "") {
  el.textContent = text;
  el.className = el.id === "status" ? `status ${kind}` : `hint ${kind}`;
}

function describeStatus(s) {
  if (s.lastError) return [`Problem: ${s.lastError}`, "err"];
  const parts = [];
  if (s.lastChanged) parts.push(`Last changed ${new Date(s.lastChanged * 1000).toLocaleTimeString()}`);
  if (s.photoCount != null) parts.push(`${s.photoCount} photos in album`);
  if (s.lockScreenNote) parts.push(s.lockScreenNote);
  if (s.transitionNote) parts.push(s.transitionNote);
  const warn = s.lockScreenNote?.startsWith("Lock screen not") || !!s.transitionNote;
  return [parts.join(" · "), warn ? "err" : ""];
}

function showStatus(s) {
  const [text, kind] = describeStatus(s);
  setMsg($("status"), text, kind);
}

function readForm() {
  const c = { ...config };
  c.shareUrl = $("shareUrl").value.trim();
  c.backupShareUrl = $("backupShareUrl").value.trim();
  c.intervalMinutes = Number($("interval").value);
  c.orientation = $("orientation").value;
  for (const f of boolFields) c[f] = $(f).checked;
  Object.assign(c, readEffects());
  return c;
}

function updateDependents() {
  $("separateLockImage").disabled = !$("setLockScreen").checked;
}

function showUpdate(info) {
  $("updateText").textContent = `Version ${info.version} is available.`;
  $("updateBanner").hidden = false;
  $("updateNow").hidden = false;
  $("updateStatus").textContent = "";
}

// An update may have been found in the background while this window was
// closed; show it whenever the window is opened or focused.
async function refreshPendingUpdate() {
  try {
    const version = await invoke("pending_update");
    if (version) showUpdate({ version });
  } catch {
    // older backend or no updater: ignore
  }
}
window.addEventListener("focus", refreshPendingUpdate);

// ---- OS accent color ----
function hexToRgb(hex) {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function luminance([r, g, b]) {
  const lin = (c) => {
    c /= 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");
let accentHex = null;

const mix = (a, b, t) => a.map((v, i) => Math.round(v * (1 - t) + b[i] * t));
const contrast = (l1, l2) => (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
const toHex = (rgb) => "#" + rgb.map((v) => v.toString(16).padStart(2, "0")).join("");

function applyAccent() {
  const root = document.documentElement.style;
  if (!accentHex) {
    ["--accent", "--accent-text"].forEach((v) => root.removeProperty(v));
    return;
  }
  let rgb = hexToRgb(accentHex);
  // Keep the accent visible against the page background (dark ≈ #17181c, light ≈ #f4f5f8).
  if (darkQuery.matches) {
    while (contrast(luminance(rgb), luminance([23, 24, 28])) < 4.5 && luminance(rgb) < 0.9) rgb = mix(rgb, [255, 255, 255], 0.1);
  } else {
    while (contrast(luminance(rgb), luminance([244, 245, 248])) < 4.5 && luminance(rgb) > 0.01) rgb = mix(rgb, [0, 0, 0], 0.1);
  }
  const lum = luminance(rgb);
  root.setProperty("--accent", toHex(rgb));
  // Text on accent buttons: white if it's readable (4.5:1), otherwise whichever contrasts more.
  const white = contrast(lum, 1);
  const black = contrast(lum, 0);
  root.setProperty("--accent-text", white >= 4.5 || white >= black ? "#ffffff" : "#111111");
}

async function refreshAccent() {
  try {
    accentHex = await invoke("accent_color");
  } catch {
    accentHex = null;
  }
  applyAccent();
}

window.addEventListener("focus", refreshAccent);
darkQuery.addEventListener("change", applyAccent);

async function init() {
  refreshPendingUpdate();
  refreshAccent();
  const platform = await invoke("platform");
  if (platform.os === "windows" || platform.os === "macos") {
    $("osSettings").hidden = false;
    if (platform.os === "windows") {
      $("openBackground").textContent = "Background settings";
      $("openLockScreen").hidden = false;
    }
  }
  if (!platform.separateLockScreen) {
    $("lockOptions").hidden = true;
    $("policyOption").hidden = true;
    $("macLockNote").hidden = platform.os !== "macos";
  }

  config = await invoke("get_config");
  $("shareUrl").value = config.shareUrl;
  $("backupShareUrl").value = config.backupShareUrl ?? "";
  const interval = $("interval");
  if (![...interval.options].some((o) => Number(o.value) === config.intervalMinutes)) {
    interval.add(new Option(`${config.intervalMinutes} minutes`, config.intervalMinutes));
  }
  interval.value = String(config.intervalMinutes);
  $("orientation").value = config.orientation;
  for (const f of boolFields) $(f).checked = !!config[f];
  showBackupRow();
  $("dim").value = String(config.dimPercent ?? 0);
  $("blur").value = String(config.blurLevel ?? 0);
  $("separateOtherMonitors").checked = !!config.separateOtherMonitors;
  $("otherDim").value = String(config.otherDimPercent ?? 0);
  $("otherBlur").value = String(config.otherBlurLevel ?? 0);
  $("dimLockScreen").checked = config.dimLockScreen ?? true;
  $("dimLockRow").hidden = !platform.separateLockScreen;
  $("fullscreenRow").hidden = platform.os !== "windows";
  $("smoothRow").hidden = platform.os !== "windows";
  setupPerDesktop(platform.os);
  $("trayHint").hidden = platform.os !== "windows" || !!config.trayHintDismissed;
  await refreshMonitors();
  $("autostart").checked = await invoke("get_autostart");
  updateDependents();
  showStatus(await invoke("get_status"));
  listen("status", (e) => showStatus(e.payload));

  $("version").textContent = `Version ${await invoke("app_version")}`;
  listen("update-available", (e) => showUpdate(e.payload));
  listen("update-none", () => ($("updateStatus").textContent = "You're up to date."));
  listen("update-installing", (e) => {
    $("updateText").textContent = `Installing version ${e.payload}… the app will restart.`;
    $("updateBanner").hidden = false;
    $("installUpdate").disabled = true;
  });
}

for (const [id, page] of [["openBackground", "background"], ["openLockScreen", "lockscreen"]]) {
  $(id).addEventListener("click", () =>
    invoke("open_wallpaper_settings", { page }).catch((e) => setMsg($("status"), String(e), "err")),
  );
}

// ---- Dim & blur: labels update while dragging, applied on release ----
const fxSliders = {
  dim: ["dimValue", (v) => `${v}%`],
  blur: ["blurValue", (v) => `${v}`],
  otherDim: ["otherDimValue", (v) => `${v}%`],
  otherBlur: ["otherBlurValue", (v) => `${v}`],
};

function showEffectValues() {
  for (const [id, [labelId, fmt]] of Object.entries(fxSliders)) {
    const v = Number($(id).value);
    $(labelId).textContent = v === 0 ? "Off" : fmt(v);
  }
  const separate = $("separateOtherMonitors").checked && !$("separateRow").hidden;
  $("otherFx").hidden = !separate;
  $("mainFxTitle").hidden = !separate;
}

function readEffects() {
  return {
    dimPercent: Number($("dim").value),
    blurLevel: Number($("blur").value),
    dimLockScreen: $("dimLockScreen").checked,
    separateOtherMonitors: $("separateOtherMonitors").checked,
    otherDimPercent: Number($("otherDim").value),
    otherBlurLevel: Number($("otherBlur").value),
  };
}

async function applyEffects() {
  const effects = readEffects();
  showEffectValues();
  setMsg($("dimResult"), "Updating the wallpaper…");
  // The setting is saved even if there's no wallpaper to re-apply yet.
  config = { ...config, ...effects };
  try {
    await invoke("set_effects", { effects });
    setMsg($("dimResult"), "Wallpaper updated.", "ok");
  } catch (e) {
    setMsg($("dimResult"), String(e), "err");
  }
}

// Per-monitor options only make sense with more than one monitor.
async function refreshMonitors() {
  try {
    const n = await invoke("monitor_count");
    $("separateRow").hidden = !(n > 1);
  } catch {
    $("separateRow").hidden = true;
  }
  showEffectValues();
}
window.addEventListener("focus", refreshMonitors);

for (const id of Object.keys(fxSliders)) {
  $(id).addEventListener("input", showEffectValues);
  $(id).addEventListener("change", applyEffects);
}
$("separateOtherMonitors").addEventListener("change", applyEffects);
$("dimLockScreen").addEventListener("change", applyEffects);

$("openLog").addEventListener("click", () =>
  invoke("open_log").catch((e) => setMsg($("status"), String(e), "err")),
);

$("closeFlyout").addEventListener("click", () => invoke("hide_settings").catch(() => {}));
$("dismissTrayHint").addEventListener("click", () => {
  $("trayHint").hidden = true;
  config = { ...config, trayHintDismissed: true };
  invoke("dismiss_tray_hint").catch(() => {});
});
// Escape closes the flyout, like system flyouts.
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") invoke("hide_settings").catch(() => {});
});

$("checkUpdate").addEventListener("click", async () => {
  const btn = $("checkUpdate");
  btn.disabled = true;
  $("updateStatus").textContent = "Checking…";
  try {
    const info = await invoke("check_update");
    if (info) showUpdate(info);
    else $("updateStatus").textContent = "You're up to date.";
  } catch (e) {
    $("updateStatus").textContent = String(e);
  } finally {
    btn.disabled = false;
  }
});

let installing = false;

async function installNow() {
  installing = true;
  $("installUpdate").disabled = $("updateNow").disabled = true;
  $("updateBanner").hidden = false;
  $("updateText").textContent = "Downloading update… the app will restart.";
  try {
    await invoke("install_update");
  } catch (e) {
    $("updateText").textContent = String(e);
    $("installUpdate").disabled = $("updateNow").disabled = false;
    installing = false;
  }
}

// ---- Close back to the tray after 20 minutes without being used ----
const IDLE_HIDE_MS = 20 * 60 * 1000;
let lastActivity = Date.now();
const markActive = () => (lastActivity = Date.now());
for (const ev of ["pointermove", "pointerdown", "keydown", "wheel"]) {
  window.addEventListener(ev, markActive, { passive: true });
}
window.addEventListener("focus", markActive);

setInterval(() => {
  if (installing || Date.now() - lastActivity < IDLE_HIDE_MS) return;
  markActive(); // the timer starts over when the window is shown again
  invoke("hide_settings").catch(() => {});
}, 60 * 1000);
$("installUpdate").addEventListener("click", installNow);
$("updateNow").addEventListener("click", installNow);

$("setLockScreen").addEventListener("change", updateDependents);

// "Different photo on each virtual desktop" (Windows 11) / "each Space" (macOS).
async function setupPerDesktop(os) {
  if (os !== "windows" && os !== "macos") return;
  $("perDesktopRow").hidden = false;
  if (os === "macos") {
    $("perDesktopLabel").textContent = "Use a different photo on each Space";
    $("perDesktopHint").textContent = "macOS only lets apps change the Space you're on, so other Spaces get their photo when you switch to them.";
    return;
  }
  let count = null;
  try {
    count = await invoke("virtual_desktops");
  } catch {}
  if (count == null) {
    $("perVirtualDesktop").disabled = true;
    $("perDesktopHint").textContent = "Needs Windows 11 version 24H2 or newer.";
  } else {
    $("perDesktopHint").textContent =
      `You have ${count} desktop${count === 1 ? "" : "s"}. Smooth transitions and per-monitor dim/blur don't apply while this is on.`;
  }
}

// The backup link field only shows while the Advanced option is on.
function showBackupRow() {
  $("backupLinkRow").hidden = !$("useBackupLink").checked;
}
$("useBackupLink").addEventListener("change", showBackupRow);

async function testLink(inputId, buttonId, resultId) {
  const btn = $(buttonId);
  btn.disabled = true;
  setMsg($(resultId), "Checking…");
  try {
    const n = await invoke("test_link", { shareUrl: $(inputId).value.trim() });
    setMsg($(resultId), `Connected: ${n} photos found.`, "ok");
  } catch (e) {
    setMsg($(resultId), String(e), "err");
  } finally {
    btn.disabled = false;
  }
}
$("testBackup").addEventListener("click", () => testLink("backupShareUrl", "testBackup", "testBackupResult"));

$("test").addEventListener("click", async () => {
  await testLink("shareUrl", "test", "testResult");
});

$("save").addEventListener("click", async () => {
  try {
    const next = readForm();
    await invoke("save_config", { config: next });
    await invoke("set_autostart", { enabled: $("autostart").checked }).catch(() => {});
    config = next;
    setMsg($("status"), next.shareUrl ? "Saved. Changing wallpaper…" : "Saved.", "ok");
  } catch (e) {
    setMsg($("status"), String(e), "err");
  }
});

// Both "Next wallpaper" buttons (top and bottom) do the same thing.
async function nextWallpaper() {
  const buttons = [$("next"), $("nextTop")];
  buttons.forEach((b) => (b.disabled = true));
  $("nextTop").textContent = "Changing…";
  $("nextTopResult").hidden = true;
  setMsg($("status"), "Fetching a new photo…");
  try {
    showStatus(await invoke("next_wallpaper"));
  } catch (e) {
    setMsg($("status"), String(e), "err");
    // Also show problems by the top button, where the click happened.
    setMsg($("nextTopResult"), String(e), "err");
    $("nextTopResult").hidden = false;
  } finally {
    buttons.forEach((b) => (b.disabled = false));
    $("nextTop").textContent = "Next wallpaper";
  }
}
$("next").addEventListener("click", nextWallpaper);
$("nextTop").addEventListener("click", nextWallpaper);

init().catch((e) => setMsg($("status"), String(e), "err"));
