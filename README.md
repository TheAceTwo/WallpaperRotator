# Wallpaper Rotator for Immich

Sets your desktop and lock screen backgrounds to random photos from a shared
[Immich](https://immich.app) album, on a schedule. It's built to hand out to
friends and family: everyone connects with a read-only album **share link**,
so nobody needs an Immich account or API key.

| Platform | What changes | How it's delivered |
|---|---|---|
| **Windows 10/11** | Desktop (all monitors) + lock screen (same or different photo) | Tray app, `.exe` / `.msi` installer |
| **macOS 11+** | Desktop on every screen (the lock screen follows it) | Menu-bar app, `.dmg` |
| **ChromeOS** | Wallpaper (the lock screen uses it too) | Chrome extension |
| **iPhone (iOS 16+)** | Lock screen and/or home screen | Apple Shortcuts automation ([why](ios-shortcut/BUILD_GUIDE.md)) |

---

## For the server owner

1. **Make the album public-by-link.** In Immich: open the album → **Share** →
   **Create link**. Leave the password **empty**, turn **Allow download** on
   (for full-resolution wallpapers), and set no expiry (or a long one).
2. Make sure that link opens on your **public domain over HTTPS** from
   outside your network (try it on your phone with Wi-Fi off).
3. Optional check from a computer with Rust installed:
   ```bash
   cargo run -p immich-core --example smoke -- "https://photos.yourdomain.com/share/XXXX"
   ```
4. Send people the share link plus the installer for their device (below).

To cut someone off, delete or regenerate the share link in Immich.

> The share link is **never** stored in this repo or compiled into the apps.
> Each person pastes it into their own settings. Don't commit it: this repo
> is public.

---

## Installing

Download from the [Releases](../../releases) page.

### Windows
1. Run `Wallpaper Rotator_x.y.z_x64-setup.exe`. Windows may show
   **"Windows protected your PC"** because the app isn't code-signed: click
   **More info → Run anyway**.
2. The settings window opens. Paste the album link, click **Test**, then **Save**.
3. The app lives in the system tray (^ by the clock). Left-click it for
   settings; right-click it for **Next wallpaper** / **Pause** / **Quit**.

If the lock screen doesn't change: Settings → Personalization → Lock screen →
set **Personalize your lock screen** to **Picture** (not Windows spotlight).

### macOS
1. Open the `.dmg` and drag **Wallpaper Rotator** to Applications.
2. The first time you open it, macOS says it can't verify the developer. Go
   to **System Settings → Privacy & Security**, scroll down, and click
   **Open Anyway**.
3. Paste the album link and click **Save**. The app lives in the menu bar
   (no Dock icon).

### ChromeOS
Until it's on the Chrome Web Store:
1. Download and unzip `wallpaper-rotator-chromeos.zip`.
2. Open `chrome://extensions`, turn on **Developer mode**, click
   **Load unpacked**, and pick the unzipped folder.
3. The options page opens. Paste the album link, click **Save**, and allow
   access to the photo server when Chrome asks.

Click the toolbar icon for the next wallpaper. Right-click it → **Options**
for settings.

### iPhone
See [ios-shortcut/README.md](ios-shortcut/README.md).

---

## Repo layout

```
crates/immich-core/    Rust: share-link parsing, album listing, random picking, download (+ tests)
crates/wallsetter/     Rust: Windows desktop/lock screen + macOS desktop setters
desktop/               Tauri v2 tray app (Windows + macOS); settings UI in desktop/src
chromeos-extension/    Manifest V3 extension (JS port of immich-core + tests)
ios-shortcut/          Build guide for the iPhone Shortcut
docs/immich-api.md     The Immich endpoints all clients use
```

## Development

```bash
cargo test -p immich-core -p wallsetter
```

```bash
node --test chromeos-extension/test/immich.test.mjs
```

```bash
cd desktop && npm ci && npm run dev
```

Linux needs `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev`
to build the desktop app. It runs on Linux for development, but the wallpaper
setters only target Windows and macOS.

### Releasing
1. Bump `version` in `Cargo.toml` (workspace), `desktop/src-tauri/tauri.conf.json`,
   and `chromeos-extension/manifest.json`.
2. `git tag v0.2.0 && git push --tags`
3. GitHub Actions builds the Windows and macOS installers plus the ChromeOS
   zip into a **draft release**. Review it and click Publish.

macOS builds are ad-hoc signed. To remove the "unverified developer" step,
join the Apple Developer Program and add the `APPLE_*` secrets listed in
`.github/workflows/release.yml`.

## License

MIT
