const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const boolFields = ["setDesktop", "setLockScreen", "separateLockImage", "preferOriginal", "allowPolicyLockScreen"];

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
  return [parts.join(" · "), s.lockScreenNote?.startsWith("Lock screen not") ? "err" : ""];
}

function showStatus(s) {
  const [text, kind] = describeStatus(s);
  setMsg($("status"), text, kind);
}

function readForm() {
  const c = { ...config };
  c.shareUrl = $("shareUrl").value.trim();
  c.intervalMinutes = Number($("interval").value);
  c.orientation = $("orientation").value;
  for (const f of boolFields) c[f] = $(f).checked;
  return c;
}

function updateDependents() {
  $("separateLockImage").disabled = !$("setLockScreen").checked;
}

async function init() {
  const platform = await invoke("platform");
  if (!platform.separateLockScreen) {
    $("lockOptions").hidden = true;
    $("policyOption").hidden = true;
    $("macLockNote").hidden = platform.os !== "macos";
  }

  config = await invoke("get_config");
  $("shareUrl").value = config.shareUrl;
  const interval = $("interval");
  if (![...interval.options].some((o) => Number(o.value) === config.intervalMinutes)) {
    interval.add(new Option(`${config.intervalMinutes} minutes`, config.intervalMinutes));
  }
  interval.value = String(config.intervalMinutes);
  $("orientation").value = config.orientation;
  for (const f of boolFields) $(f).checked = !!config[f];
  $("autostart").checked = await invoke("get_autostart");
  updateDependents();
  showStatus(await invoke("get_status"));
  listen("status", (e) => showStatus(e.payload));
}

$("setLockScreen").addEventListener("change", updateDependents);

$("test").addEventListener("click", async () => {
  const btn = $("test");
  btn.disabled = true;
  setMsg($("testResult"), "Checking…");
  try {
    const n = await invoke("test_link", { shareUrl: $("shareUrl").value.trim() });
    setMsg($("testResult"), `Connected: ${n} photos found.`, "ok");
  } catch (e) {
    setMsg($("testResult"), String(e), "err");
  } finally {
    btn.disabled = false;
  }
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

$("next").addEventListener("click", async () => {
  const btn = $("next");
  btn.disabled = true;
  setMsg($("status"), "Fetching a new photo…");
  try {
    showStatus(await invoke("next_wallpaper"));
  } catch (e) {
    setMsg($("status"), String(e), "err");
  } finally {
    btn.disabled = false;
  }
});

init().catch((e) => setMsg($("status"), String(e), "err"));
