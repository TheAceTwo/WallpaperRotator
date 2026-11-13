// Service worker: schedules and applies wallpapers on ChromeOS.
import { ImmichClient, parseShareLink, pick } from "./immich.js";
import { getSettings } from "./settings.js";

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

/** chrome.wallpaper only accepts JPEG/PNG; convert anything else (e.g. WebP previews). */
async function toJpegIfNeeded({ bytes, type }) {
  if (/^image\/(jpeg|jpg|png)\b/.test(type)) return bytes;
  const bitmap = await createImageBitmap(new Blob([bytes], { type }));
  const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
  canvas.getContext("2d").drawImage(bitmap, 0, 0);
  const blob = await canvas.convertToBlob({ type: "image/jpeg", quality: 0.92 });
  return blob.arrayBuffer();
}

let running = null;

async function rotateNow() {
  // Collapse overlapping requests (alarm + button press).
  if (running) return running;
  running = (async () => {
    try {
      if (!chrome.wallpaper) {
        throw new Error("This device isn't ChromeOS, so Chrome can't change the wallpaper.");
      }
      const s = await getSettings();
      if (!s.shareUrl) throw new Error("Add your Immich share link in the extension options.");
      const client = new ImmichClient(parseShareLink(s.shareUrl));
      const assets = await getAssets(client, s.shareUrl);

      const { history = [] } = await chrome.storage.local.get("history");
      const asset = pick(assets, s.orientation, history);
      if (!asset) throw new Error("The shared album has no photos.");

      const data = await toJpegIfNeeded(await client.download(asset, s.preferOriginal));
      await chrome.wallpaper.setWallpaper({ data, layout: "CENTER_CROPPED", filename: "immich-wallpaper" });

      await chrome.storage.local.set({ history });
      await setStatus({
        lastChanged: Date.now(),
        lastError: null,
        photoCount: assets.filter((a) => a.isImage).length,
      });
    } catch (e) {
      await setStatus({ lastError: String(e.message || e) });
      throw e;
    } finally {
      running = null;
    }
  })();
  return running;
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
  if (msg?.type === "next") {
    rotateNow().then(
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
