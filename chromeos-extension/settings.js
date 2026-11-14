// Settings live in chrome.storage.sync (follows the user's Google account).
// The share link is entered by each user; nothing is baked into the extension.
export const DEFAULTS = {
  shareUrl: "",
  intervalMinutes: 60,
  orientation: "landscape",
  preferOriginal: true,
  paused: false,
  dimPercent: 0,
};

export async function getSettings() {
  return { ...DEFAULTS, ...(await chrome.storage.sync.get(Object.keys(DEFAULTS))) };
}
