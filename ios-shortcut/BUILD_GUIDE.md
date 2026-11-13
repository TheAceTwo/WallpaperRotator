# Building the iPhone Shortcut (one time)

iOS doesn't let apps change the wallpaper, so the iPhone version is a **Shortcut**
built in Apple's free Shortcuts app. You build it once on any iPhone, then
share an iCloud link. Friends tap the link, paste their album link, and they're
done. No Mac, Xcode, developer account, or App Store needed.

**Requires** iOS 16 or later (iOS 17+ recommended).

---

## 0. Before you start

1. In Immich, open the album → **Share** → **Create link**. Leave **password
   empty** and turn **Allow download** on. Copy the link; it looks like
   `https://photos.yourdomain.com/share/AbCd…`.
   - Tip: phones are portrait. A separate album of tall photos (with its own
     share link) looks much better than cropped landscape shots.
2. Work out which recipe your server needs. On a computer, run:
   ```bash
   cargo run -p immich-core --example smoke -- "https://photos.yourdomain.com/share/AbCd…"
   ```
   It prints `listed via: Album` → use **Recipe A**, or `listed via: Timeline`
   → use **Recipe B**.

   No computer with Rust? Use a browser instead. Open
   `https://SERVER/api/shared-links/me?key=KEY` and copy the `"id"` inside
   `"album"`. Then open `https://SERVER/api/albums/ALBUM_ID?key=KEY`. If that
   page shows `"assets":[{…` with items, use Recipe A. If it shows
   `"assets":[]` or has no `assets`, use Recipe B.
3. Make sure the iPhone already has a **Photos-type wallpaper** (Settings →
   Wallpaper → Add New Wallpaper → Photos). The Shortcut swaps the photo
   inside the current wallpaper.

---

## 1. Create the Shortcut

Open **Shortcuts** → **+**. Name it **Immich Wallpaper**. Add these actions
in order (search the action name in the bottom bar). Text in `monospace` is
typed exactly. **Blue variable names** are inserted by tapping the field and
picking the variable or "Select Magic Variable".

### Read the share link

| # | Action | Settings |
|---|--------|----------|
| 1 | **Text** | Paste your share link. (Friends replace it on import, see §3.) |
| 2 | **Match Text** | Pattern: `^(https?://[^/]+)/share/([^/?#]+)` in **Text** (from #1) |
| 3 | **Get Group from Matched Text** | Group at index `1` |
| 4 | **Set Variable** | Name `Server`, value = output of #3 |
| 5 | **Get Group from Matched Text** | Group at index `2` from **Matches** (#2) |
| 6 | **Set Variable** | Name `Key` |

### Find the album

| # | Action | Settings |
|---|--------|----------|
| 7 | **Get Contents of URL** | URL: **Server**`/api/shared-links/me?key=`**Key** (method GET) |
| 8 | **Get Dictionary Value** | Get **Value** for `album` in **Contents of URL** |
| 9 | **Get Dictionary Value** | Get **Value** for `id` in **Dictionary Value** (#8) |
| 10 | **Set Variable** | Name `AlbumID` |

### Pick a random photo: Recipe A (album returns assets)

| # | Action | Settings |
|---|--------|----------|
| 11 | **Get Contents of URL** | **Server**`/api/albums/`**AlbumID**`?key=`**Key** |
| 12 | **Get Dictionary Value** | **Value** for `assets` |
| 13 | **Get Item from List** | **Random Item** |
| 14 | **Get Dictionary Value** | **Value** for `id` |
| 15 | **Set Variable** | Name `AssetID` |

### Pick a random photo: Recipe B (newer Immich, timeline)

Use these instead of 11–15:

| # | Action | Settings |
|---|--------|----------|
| 11 | **Get Contents of URL** | **Server**`/api/timeline/buckets?albumId=`**AlbumID**`&key=`**Key** |
| 12 | **Get Item from List** | **Random Item** |
| 13 | **Get Dictionary Value** | **Value** for `timeBucket` |
| 14 | **URL Encode** | Encode **Dictionary Value** (#13) |
| 15 | **Get Contents of URL** | **Server**`/api/timeline/bucket?albumId=`**AlbumID**`&timeBucket=`**URL Encoded Text**`&key=`**Key** |
| 16 | **Get Dictionary Value** | **Value** for `id` (this is a list of ids) |
| 17 | **Get Item from List** | **Random Item** |
| 18 | **Set Variable** | Name `AssetID` |

### Download and apply

| # | Action | Settings |
|---|--------|----------|
| next | **Get Contents of URL** | **Server**`/api/assets/`**AssetID**`/thumbnail?size=preview&key=`**Key** |
| next | **Set Wallpaper Photo** (iOS 17: "Set Wallpaper Photo", iOS 16: "Set Wallpaper") | Image = **Contents of URL**. Tap the arrow (›): pick **Lock Screen and Home Screen** (or one of them), turn **Show Preview off**, and turn **Crop to Subject** off (the depth effect can hide clocks on busy photos). |

The `preview` size is a phone-sized JPEG that loads quickly over cellular.
If you want full resolution, use `/api/assets/`**AssetID**`/original?key=`**Key**
instead. That only works if "Allow download" is on, and some originals are HEIC
or RAW.

Tap ▶︎ to test. The first run asks permission to connect to your server.
Choose **Always Allow**.

---

## 2. Run it automatically

Shortcuts → **Automation** tab → **+** → **Time of Day** → choose a time and
**Daily** → **Run Immediately** (and turn off "Notify When Run") → pick
**Immich Wallpaper**.

Add more automations for more changes per day. **When charger is connected**
and **When I open an app** are fun triggers too.

---

## 3. Share it with friends and family

1. Long-press the shortcut → **Share** → **Options**: turn off "Include
   private data" if shown.
2. Open the shortcut → ⓘ (**Details**) → **Setup** → **Add Question** on
   the **Text** action (#1). Prompt: `Paste the Immich album link you were sent`.
   Now everyone is asked for their link when they add it. Your link never
   goes into the shared copy.
3. Share → **Copy iCloud Link**, and send that link (it can also go in this
   repo's README; it contains no secrets).

Each person still sets up their own automation (§2). Automations can't be
shared, so send them the README in this folder.

---

## Troubleshooting

| Symptom | Fix |
|--------|-----|
| "Could not connect to the server" | The link must use your **public** domain with HTTPS, not a LAN IP. |
| `Unauthorized` / 401 in step 7 | The share link was deleted, expired, or has a password. Make a new one with no password. |
| Step 12 (Recipe A) says no value / empty list | Your Immich is newer; switch to Recipe B. |
| Wallpaper doesn't change | Make sure a **Photos** wallpaper is active (step 0.3) and Show Preview is off. |
| Same photos keep coming up | It's random per run. Small albums repeat more often. |
