// Service worker: schedules and applies wallpapers on ChromeOS.
import { ImmichClient, parseShareLink, pick } from "./immich.js";
import { getSettings } from "./settings.js";
import { prepareImage } from "./image.js";

const ALARM = "rotate";
const LIST_CACHE_MS = 3 * 60 * 60 * 1000;

async function setStatus(patch) {
  const { status = {} } = await chrome.storage.local.get("status");
  await chrome.storage.local.set({ status: { ...status, ...patch } });
}

async function schedule() {
  const s = await getSettings();
  await chrome.alarms.clear(ALARM);
  if (!s.paused && s.shareUrl) {
    chrome.alarms.create(ALARM, { periodInMinutes: Math.max(1, s.intervalMinutes) });
  }
}

async function getAssets(client, shareUrl) {
  const { listCache } = await chrome.storage.local.get("listCache");
  if (listCache && listCache.shareUrl === shareUrl && Date.now() - listCache.fetched < LIST_CACHE_MS) {
    return listCache.assets;
  }
  const assets = await client.listAssets();
  await chrome.storage.local.set({ listCache: { shareUrl, fetched: Date.now(), assets } });
  return assets;
}

let queue = Promise.resolve();

/**
 * Change the wallpaper. With `redim`, re-applies the current photo with the
 * current dim setting instead of picking a new one.
 */
function rotateNow({ redim = false } = {}) {
  // Run one change at a time (alarm, button press and slider can overlap).
  const job = queue.then(() => doRotate(redim));
  queue = job.catch(() => {});
  return job;
}

async function doRotate(redim) {
  try {
    if (!chrome.wallpaper) {
      throw new Error("This device isn't ChromeOS, so Chrome can't change the wallpaper.");
    }
    const s = await getSettings();
    if (!s.shareUrl) throw new Error("Add your Immich share link in the extension options.");
    const client = new ImmichClient(parseShareLink(s.shareUrl));

    let asset;
    if (redim) {
      ({ currentAsset: asset } = await chrome.storage.local.get("currentAsset"));
      if (!asset) return; // nothing applied yet; the next photo will use the new setting
    } else {
      const assets = await getAssets(client, s.shareUrl);
      const { history = [] } = await chrome.storage.local.get("history");
      asset = pick(assets, s.orientation, history);
      if (!asset) throw new Error("The shared album has no photos.");
      await chrome.storage.local.set({ history });
      await setStatus({ photoCount: assets.filter((a) => a.isImage).length });
    }

    const data = await prepareImage(await client.download(asset, s.preferOriginal), s.dimPercent);
    await chrome.wallpaper.setWallpaper({ data, layout: "CENTER_CROPPED", filename: "immich-wallpaper" });

    await chrome.storage.local.set({ currentAsset: asset });
    await setStatus({ lastChanged: Date.now(), lastError: null });
  } catch (e) {
    await setStatus({ lastError: String(e.message || e) });
    throw e;
  }
}

chrome.alarms.onAlarm.addListener((a) => {
  if (a.name === ALARM) rotateNow().catch(() => {});
});

chrome.action.onClicked.addListener(() => rotateNow().catch(() => {}));

chrome.runtime.onInstalled.addListener(async ({ reason }) => {
  await schedule();
  const s = await getSettings();
  if (reason === "install" || !s.shareUrl) chrome.runtime.openOptionsPage();
});

chrome.runtime.onStartup.addListener(async () => {
  await schedule();
  const s = await getSettings();
  if (s.shareUrl && !s.paused) rotateNow().catch(() => {});
});

chrome.storage.onChanged.addListener(async (changes, area) => {
  if (area !== "sync") return;
  if ("shareUrl" in changes) await chrome.storage.local.remove("listCache");
  if ("shareUrl" in changes || "intervalMinutes" in changes || "paused" in changes) await schedule();
});

chrome.runtime.onMessage.addListener((msg, _sender, reply) => {
  if (msg?.type === "next" || msg?.type === "redim") {
    rotateNow({ redim: msg.type === "redim" }).then(
      () => reply({ ok: true }),
      (e) => reply({ ok: false, error: String(e.message || e) }),
    );
    return true; // async reply
  }
  if (msg?.type === "test") {
    (async () => {
      const client = new ImmichClient(parseShareLink(msg.shareUrl));
      const assets = await client.listAssets();
      return assets.filter((a) => a.isImage).length;
    })().then(
      (count) => reply({ ok: true, count }),
      (e) => reply({ ok: false, error: String(e.message || e) }),
    );
    return true;
  }
});
