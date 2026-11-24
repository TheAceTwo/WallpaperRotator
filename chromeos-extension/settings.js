// Settings live in chrome.storage.sync (follows the user's Google account).
// Album links are entered by each user; nothing is baked into the extension.
import { parseServer } from "./immich.js";

export const DEFAULTS = {
  /** [{ name, url }] — the Immich shared albums photos come from. */
  albums: [],
  /** "all" = shuffle photos from every album, "one" = only `selectedAlbum`. */
  albumMode: "all",
  selectedAlbum: "",
  /** Same album links on another server (home network / Tailscale). */
  useBackupDomain: false,
  backupDomain: "",
  intervalMinutes: 60,
  orientation: "landscape",
  preferOriginal: true,
  paused: false,
  dimPercent: 0,
  blurLevel: 0,
  changeOnStartup: true,
  changeOnUnlock: false,
};

/** Keys from before multiple albums (0.1.8 and earlier). */
const LEGACY = ["shareUrl", "useBackupLink", "backupShareUrl"];

export async function getSettings() {
  const stored = await chrome.storage.sync.get([...Object.keys(DEFAULTS), ...LEGACY]);
  const s = { ...DEFAULTS, ...stored };
  // One-time move of the single link / backup link into the new settings.
  if (LEGACY.some((k) => k in stored)) {
    if (!s.albums.length && s.shareUrl) s.albums = [{ name: "", url: String(s.shareUrl).trim() }];
    if (!s.backupDomain && s.backupShareUrl) {
      try {
        s.backupDomain = parseServer(s.backupShareUrl);
        s.useBackupDomain = !!s.useBackupLink;
      } catch {}
    }
    await chrome.storage.sync.set({ albums: s.albums, backupDomain: s.backupDomain, useBackupDomain: s.useBackupDomain });
    await chrome.storage.sync.remove(LEGACY);
  }
  for (const k of LEGACY) delete s[k];
  return s;
}

/** Is at least one album set up? */
export function hasAlbums(s) {
  return (s.albums || []).some((a) => String(a.url || "").trim());
}
