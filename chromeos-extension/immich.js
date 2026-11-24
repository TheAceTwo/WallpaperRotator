// Shared Immich logic for the ChromeOS extension.
// Mirrors crates/immich-core (Rust). Pure JS: no chrome.* APIs, so it can be
// unit-tested in Node (see test/immich.test.mjs).

/** Parse https://host/share/<key> or https://host/s/<slug>. */
export function parseShareLink(input) {
  let url;
  try {
    url = new URL(String(input).trim());
  } catch {
    throw new Error("Not a valid link.");
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") {
    throw new Error("The link must start with https://");
  }
  const segs = url.pathname.split("/").filter(Boolean);
  let auth;
  if (segs[0] === "share" && segs[1]) auth = ["key", segs[1]];
  else if (segs[0] === "s" && segs[1]) auth = ["slug", segs[1]];
  else throw new Error("Expected a link like https://your-server/share/XXXX");
  return { server: url.origin, authParam: auth[0], authValue: auth[1] };
}

/**
 * Normalise a server address like "https://192.168.1.123:2283" to its origin.
 * Anything after the address is ignored; no scheme means https://.
 */
export function parseServer(input) {
  const raw = String(input || "").trim();
  let url;
  try {
    url = new URL(raw.includes("://") ? raw : `https://${raw}`);
  } catch {
    throw new Error("Not a valid address.");
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") {
    throw new Error("The address must start with https:// or http://");
  }
  return url.origin;
}

/** The same album link on another server (only the address changes). */
export function withServer(link, server) {
  return { ...link, server: server.replace(/\/+$/, "") };
}

/** Album links to use: all of them, or just the chosen one. */
export function activeAlbumUrls(s) {
  const all = (s.albums || []).map((a) => String(a.url || "").trim()).filter(Boolean);
  if (s.albumMode !== "one") return all;
  const chosen = all.find((u) => u === String(s.selectedAlbum || "").trim()) || all[0];
  return chosen ? [chosen] : [];
}

/** One AssetResponseDto → {id, isImage, ratio, mime} */
export function assetFromDto(v) {
  if (!v || typeof v.id !== "string" || v.isTrashed === true) return null;
  const isImage = typeof v.type === "string" ? v.type.toUpperCase() === "IMAGE" : v.isImage !== false;
  const exif = v.exifInfo || {};
  const w = v.width ?? exif.exifImageWidth;
  const h = v.height ?? exif.exifImageHeight;
  const rotated = ["5", "6", "7", "8"].includes(String(exif.orientation ?? ""));
  const ratio = w > 0 && h > 0 ? (rotated ? h / w : w / h) : null;
  return { id: v.id, isImage, ratio, mime: v.originalMimeType ?? null };
}

/** Array of DTOs, or the columnar time-bucket shape {id:[], isImage:[], ratio:[]} */
export function parseAssetList(v) {
  if (Array.isArray(v)) return v.map(assetFromDto).filter(Boolean);
  if (!v || !Array.isArray(v.id)) return [];
  const out = [];
  v.id.forEach((id, i) => {
    if (typeof id !== "string" || v.isTrashed?.[i] === true) return;
    const r = v.ratio?.[i];
    out.push({ id, isImage: v.isImage?.[i] !== false, ratio: r > 0 ? r : null, mime: null });
  });
  return out;
}

export class UnauthorizedError extends Error {
  constructor() {
    super("The share link was rejected (expired, revoked, or password-protected).");
  }
}

export class ImmichClient {
  constructor(link, fetchImpl = globalThis.fetch.bind(globalThis)) {
    this.link = link;
    this.fetch = fetchImpl;
  }

  url(path, query = {}) {
    const u = new URL(this.link.server + path);
    u.searchParams.set(this.link.authParam, this.link.authValue);
    for (const [k, v] of Object.entries(query)) u.searchParams.set(k, v);
    return u.toString();
  }

  async get(path, query) {
    const res = await this.fetch(this.url(path, query), { credentials: "omit" });
    if (res.status === 401 || res.status === 403) throw new UnauthorizedError();
    if (!res.ok) {
      const err = new Error(`Server returned HTTP ${res.status} for ${path}`);
      err.status = res.status;
      throw err;
    }
    return res;
  }

  async json(path, query) {
    return (await this.get(path, query)).json();
  }

  /** The shared album's name, if the link is for an album. */
  async albumName() {
    const me = await this.json("/api/shared-links/me");
    const name = me?.album?.albumName;
    return typeof name === "string" && name.trim() ? name : null;
  }

  /** Same fallback chain as the Rust client. */
  async listAssets() {
    const me = await this.json("/api/shared-links/me");
    const direct = parseAssetList(me.assets);
    if (direct.length) return direct;
    if (!me.album) throw new Error("The share link has no album.");
    const embedded = parseAssetList(me.album.assets);
    if (embedded.length) return embedded;
    const albumId = me.album.id;

    try {
      const full = await this.json(`/api/albums/${albumId}`);
      const list = parseAssetList(full.assets);
      if (list.length) return list;
    } catch (e) {
      if (e instanceof UnauthorizedError) throw e;
    }

    const extra = { albumId };
    let buckets;
    try {
      buckets = await this.json("/api/timeline/buckets", extra);
    } catch (e) {
      if (e.status !== 400) throw e;
      extra.size = "MONTH";
      buckets = await this.json("/api/timeline/buckets", extra);
    }
    const seen = new Set();
    const out = [];
    for (const b of buckets || []) {
      if (!b?.timeBucket) continue;
      const page = await this.json("/api/timeline/bucket", { ...extra, timeBucket: b.timeBucket });
      for (const a of parseAssetList(page)) {
        if (!seen.has(a.id)) {
          seen.add(a.id);
          out.push(a);
        }
      }
    }
    if (!out.length) throw new Error("The shared album has no photos.");
    return out;
  }

  /** Returns {bytes: ArrayBuffer, type: string}. */
  async download(asset, preferOriginal) {
    const safe = (t) => /^image\/(jpeg|jpg|png)\b/.test(t || "");
    if (preferOriginal && asset.isImage && (asset.mime == null || safe(asset.mime))) {
      try {
        const res = await this.get(`/api/assets/${asset.id}/original`);
        const type = res.headers.get("content-type") || "";
        if (safe(type)) return { bytes: await res.arrayBuffer(), type };
      } catch {
        // downloads disabled on the link, etc. → use the preview
      }
    }
    const res = await this.get(`/api/assets/${asset.id}/thumbnail`, { size: "preview" });
    return { bytes: await res.arrayBuffer(), type: res.headers.get("content-type") || "image/jpeg" };
  }
}

/**
 * Servers (or links) to try, in order: main, then backup (if set and
 * different), except that whichever worked last time goes first.
 */
export function linkOrder(main, backup, lastGood) {
  const urls = [String(main || "").trim()];
  const b = String(backup || "").trim();
  if (b && b !== urls[0]) urls.push(b);
  const i = lastGood ? urls.indexOf(lastGood) : -1;
  if (i > 0) [urls[0], urls[i]] = [urls[i], urls[0]];
  return urls;
}

/**
 * Run `fn(client)` against each server until one succeeds. Calls
 * `onSuccess(server)` with the one that worked; if all fail, throws one
 * error naming each address's problem.
 */
export async function tryLinks(urls, main, fn, onSuccess = () => {}, makeClient = (u) => new ImmichClient(parseShareLink(u))) {
  const errors = [];
  for (const url of urls) {
    try {
      const result = await fn(makeClient(url));
      await onSuccess(url);
      return result;
    } catch (e) {
      errors.push([url, String(e.message || e)]);
    }
  }
  if (errors.length === 1) throw new Error(errors[0][1]);
  throw new Error(errors.map(([u, m]) => `${u === main ? "Main address" : "Backup address"}: ${m}`).join(" · "));
}

export function matchesOrientation(a, orientation) {
  if (orientation === "any" || a.ratio == null) return true;
  return orientation === "landscape" ? a.ratio >= 1 : a.ratio <= 1;
}

const MAX_HISTORY = 200;

/**
 * Pick a random asset, avoiding the most recent half of the pool.
 * `history` is an array of ids (oldest first) and is updated in place.
 */
export function pick(assets, orientation, history, random = Math.random) {
  const tiers = [
    (a) => a.isImage && matchesOrientation(a, orientation),
    (a) => a.isImage,
    () => true,
  ];
  let candidates = [];
  for (const t of tiers) {
    candidates = assets.filter(t);
    if (candidates.length) break;
  }
  if (!candidates.length) return null;

  const window = Math.min(Math.floor(candidates.length / 2), MAX_HISTORY);
  const blocked = new Set(window ? history.slice(-window) : []);
  const fresh = candidates.filter((a) => !blocked.has(a.id));
  const pool = fresh.length ? fresh : candidates;
  const chosen = pool[Math.floor(random() * pool.length)];

  const idx = history.indexOf(chosen.id);
  if (idx !== -1) history.splice(idx, 1);
  history.push(chosen.id);
  while (history.length > MAX_HISTORY) history.shift();
  return chosen;
}
