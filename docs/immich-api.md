# Immich API contract

Every client (desktop app, ChromeOS extension, iPhone Shortcut) uses the same
read-only calls, authenticated **only** by the album's public share link. No
Immich API key or user login is ever needed.

| Implementation | File |
|---|---|
| Rust (desktop) | `crates/immich-core/src/client.rs` |
| JavaScript (ChromeOS) | `chromeos-extension/immich.js` |
| iPhone Shortcut | `ios-shortcut/BUILD_GUIDE.md` |

## Auth

A share link is `https://SERVER/share/KEY` (or `https://SERVER/s/SLUG` for a
custom URL). Every request appends `?key=KEY` (or `?slug=SLUG`).

Password-protected links aren't supported. Create the link without a password.

## Listing the album

Different Immich releases return assets differently, so clients try these in order:

1. `GET /api/shared-links/me` → `SharedLinkResponseDto`
   - `assets[]` non-empty → individually shared photos; done.
   - `album.assets[]` non-empty → done.
   - otherwise take `album.id`.
2. `GET /api/albums/{albumId}` → `assets[]` (older/mid releases).
3. Timeline (what Immich's own share page uses, so it always works):
   - `GET /api/timeline/buckets?albumId=…` → `[{ timeBucket, count }]`
     (older releases need `&size=MONTH`; clients retry with it on HTTP 400)
   - `GET /api/timeline/bucket?albumId=…&timeBucket=…` → either
     - newer: columnar `{ id: [...], isImage: [...], ratio: [...], isTrashed: [...] }`
     - older: `AssetResponseDto[]`

Fields used from `AssetResponseDto`: `id`, `type` (`IMAGE`/`VIDEO`),
`originalMimeType`, `isTrashed`, `width`/`height` or
`exifInfo.exifImageWidth`/`exifImageHeight`, plus `exifInfo.orientation`
(5–8 swaps width and height).

## Downloading

- Original: `GET /api/assets/{id}/original`. Used only for JPEG/PNG, and only
  if the link has **Allow download** on (otherwise 401/403).
- Fallback: `GET /api/assets/{id}/thumbnail?size=preview`, a JPEG of about
  1440px by default (WebP if the server is configured that way; the ChromeOS
  extension converts it).

## Checking your server

```bash
cargo run -p immich-core --example smoke -- "https://SERVER/share/KEY"
```

This prints which route worked, the photo counts by shape, and saves one
random photo.
