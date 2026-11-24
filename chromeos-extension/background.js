// Service worker: schedules and applies wallpapers on ChromeOS.
import { ImmichClient, activeAlbumUrls, linkOrder, parseServer, parseShareLink, pick, tryLinks, withServer } from "./immich.js";
import { getSettings, hasAlbums } from "./settings.js";
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
  if (!s.paused && hasAlbums(s)) {
    chrome.alarms.create(ALARM, { periodInMinutes: Math.max(1, s.intervalMinutes) });
  }
}

/**
 * Run `fn(client)` for one album: on its own server first, then (if turned
 * on) the backup address — same /s/... path, different server. Whichever
 * server worked last is tried first.
 */
async function withLinks(s, albumUrl, fn) {
  const link = parseShareLink(albumUrl);
  const { lastGoodServer } = await chrome.storage.local.get("lastGoodServer");
  const backup = s.useBackupDomain && s.backupDomain ? parseServer(s.backupDomain) : "";
  const servers = linkOrder(link.server, backup, lastGoodServer);
  return tryLinks(servers, link.server, fn, (server) => chrome.storage.local.set({ lastGoodServer: server }),
    (server) => new ImmichClient(withServer(link, server)));
}

/**
 * Photos from every active album in one pool (every photo equally likely).
 * Each asset remembers its album, which is needed to download it. Albums
 * that can't be reached are skipped as long as another one works.
 */
async function getAssets(s) {
  const { listCache = {} } = await chrome.storage.local.get("listCache");
  const pool = [];
  const seen = new Set();
  const errors = [];
  for (const url of activeAlbumUrls(s)) {
    let assets = listCache[url]?.fetched > Date.now() - LIST_CACHE_MS ? listCache[url].assets : null;
    if (!assets) {
      try {
        assets = await withLinks(s, url, (c) => c.listAssets());
        listCache[url] = { fetched: Date.now(), assets };
      } catch (e) {
        const name = s.albums.find((a) => a.url === url)?.name || url;
        errors.push(`${name}: ${e.message || e}`);
        continue;
      }
    }
    for (const a of assets) {
      if (!seen.has(a.id)) {
        seen.add(a.id);
        pool.push({ ...a, album: url });
      }
    }
  }
  await chrome.storage.local.set({ listCache });
  if (!pool.length) throw new Error(errors.join(" · ") || "The albums have no photos.");
  return pool;
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
    if (!hasAlbums(s)) throw new Error("Add an Immich album in the extension options.");

    let asset;
    if (redim) {
      ({ currentAsset: asset } = await chrome.storage.local.get("currentAsset"));
      if (!asset) return; // nothing applied yet; the next photo will use the new setting
    } else {
      const assets = await getAssets(s);
      const { history = [] } = await chrome.storage.local.get("history");
      asset = pick(assets, s.orientation, history);
      if (!asset) throw new Error("The shared album has no photos.");
      await chrome.storage.local.set({ history });
      await setStatus({ photoCount: assets.filter((a) => a.isImage).length });
    }

    // Older saved photos (before multiple albums) don't record their album.
    const albumUrl = asset.album || activeAlbumUrls(s)[0];
    const image = await withLinks(s, albumUrl, (c) => c.download(asset, s.preferOriginal));
    const data = await prepareImage(image, s.dimPercent, s.blurLevel);
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
  if (reason === "install" || !hasAlbums(s)) chrome.runtime.openOptionsPage();
});

chrome.runtime.onStartup.addListener(async () => {
  await schedule();
  const s = await getSettings();
  if (hasAlbums(s) && !s.paused && s.changeOnStartup) rotateNow().catch(() => {});
});

// "Change when I unlock": ChromeOS reports locked -> active when you sign back in.
// The service worker can be stopped while locked, so remember the state in
// session storage rather than a variable.
chrome.idle.onStateChanged.addListener(async (state) => {
  if (state === "locked") {
    await chrome.storage.session.set({ wasLocked: true });
    return;
  }
  if (state !== "active") return;
  const { wasLocked, lastUnlockChange = 0 } = await chrome.storage.session.get(["wasLocked", "lastUnlockChange"]);
  if (!wasLocked) return;
  await chrome.storage.session.set({ wasLocked: false });
  const s = await getSettings();
  if (!hasAlbums(s) || s.paused || !s.changeOnUnlock) return;
  if (Date.now() - lastUnlockChange < 60_000) return;
  await chrome.storage.session.set({ lastUnlockChange: Date.now() });
  rotateNow().catch(() => {});
});

chrome.storage.onChanged.addListener(async (changes, area) => {
  if (area !== "sync") return;
  const links = ["albums", "albumMode", "selectedAlbum", "useBackupDomain", "backupDomain"];
  if (links.some((k) => k in changes)) await chrome.storage.local.remove("listCache");
  if ([...links, "intervalMinutes", "paused"].some((k) => k in changes)) await schedule();
});

chrome.runtime.onMessage.addListener((msg, _sender, reply) => {
  if (msg?.type === "next" || msg?.type === "redim") {
    rotateNow({ redim: msg.type === "redim" }).then(
      () => reply({ ok: true }),
      (e) => reply({ ok: false, error: String(e.message || e) }),
    );
    return true; // async reply
  }
  if (msg?.type === "inspect") {
    // Check an album link (own server, then the backup address): name + photos.
    (async () => {
      const link = parseShareLink(msg.url);
      const servers = [link.server];
      if (msg.backupDomain) servers.push(parseServer(msg.backupDomain));
      return tryLinks(servers, link.server, async (c) => {
        const assets = await c.listAssets();
        return { name: await c.albumName().catch(() => null), count: assets.filter((a) => a.isImage).length };
      }, undefined, (server) => new ImmichClient(withServer(link, server)));
    })().then(
      (info) => reply({ ok: true, ...info }),
      (e) => reply({ ok: false, error: String(e.message || e) }),
    );
    return true;
  }
  if (msg?.type === "testBackup") {
    (async () => {
      const link = withServer(parseShareLink(msg.url), parseServer(msg.domain));
      const assets = await new ImmichClient(link).listAssets();
      return assets.filter((a) => a.isImage).length;
    })().then(
      (count) => reply({ ok: true, count }),
      (e) => reply({ ok: false, error: String(e.message || e) }),
    );
    return true;
  }
});
