//! Check a share link against a real server without touching your wallpaper.
//!
//!   cargo run -p immich-core --example smoke -- "https://photos.example.com/share/XXXX" [out-dir]
//!
//! Prints what the album contains and downloads one randomly picked photo.

use immich_core::{pick, History, ImmichClient, Orientation, ShareLink};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(url) = args.next() else {
        eprintln!("usage: smoke <share-url> [out-dir]");
        std::process::exit(2);
    };
    let out = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));

    let link = ShareLink::parse(&url).unwrap_or_else(|e| fail(e));
    println!("server: {}", link.server);
    let client = ImmichClient::new(link).unwrap_or_else(|e| fail(e));

    let (assets, source) = client.list_assets_with_source().await.unwrap_or_else(|e| fail(e));
    println!("listed via: {source:?} (iPhone Shortcut: use recipe {})",
        if source == immich_core::ListSource::Timeline { "B" } else { "A" });
    let images = assets.iter().filter(|a| a.is_image).count();
    let landscape = assets.iter().filter(|a| a.is_image && a.ratio.is_some_and(|r| r >= 1.0)).count();
    let portrait = assets.iter().filter(|a| a.is_image && a.ratio.is_some_and(|r| r < 1.0)).count();
    println!(
        "assets: {} ({} images: {} landscape, {} portrait, {} unknown shape)",
        assets.len(),
        images,
        landscape,
        portrait,
        images - landscape - portrait
    );

    let asset = pick(&assets, Orientation::Landscape, &mut History::default(), &mut rand::thread_rng())
        .unwrap_or_else(|| fail("album is empty"));
    println!("picked: {} ({:?})", asset.id, asset.mime);
    let img = client.download(asset, true).await.unwrap_or_else(|e| fail(e));
    let path = out.join(format!("smoke-{}.{}", asset.id, img.ext));
    std::fs::write(&path, &img.bytes).unwrap_or_else(|e| fail(e));
    println!("saved {} bytes to {}", img.bytes.len(), path.display());
}

fn fail(e: impl std::fmt::Display) -> ! {
    eprintln!("error: {e}");
    std::process::exit(1);
}
