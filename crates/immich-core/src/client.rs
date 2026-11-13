//! Minimal, read-only Immich client that authenticates with a shared-link key.

use crate::model::Asset;
use crate::share::ShareLink;
use crate::Error;
use reqwest::{Response, StatusCode};
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;

/// Which API route produced the asset list (useful for debugging and for
/// choosing the right iPhone Shortcut recipe).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListSource {
    SharedLink,
    Album,
    Timeline,
}

pub struct ImmichClient {
    http: reqwest::Client,
    link: ShareLink,
}

/// A downloaded image ready to be written to disk.
pub struct Image {
    pub bytes: Vec<u8>,
    /// File extension without the dot (`jpg`, `png`, `webp`).
    pub ext: &'static str,
}

/// Formats every target OS can use directly as a wallpaper.
fn wallpaper_safe(content_type: &str) -> Option<&'static str> {
    match content_type.split(';').next().unwrap_or("").trim() {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        _ => None,
    }
}

impl ImmichClient {
    pub fn new(link: ShareLink) -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("immich-wallpaper/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .build()?;
        Ok(Self { http, link })
    }

    pub fn link(&self) -> &ShareLink {
        &self.link
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Response, Error> {
        let url = format!("{}{}", self.link.server, path);
        let resp = self
            .http
            .get(url)
            .query(&[self.link.auth_param()])
            .query(query)
            .send()
            .await?;
        match resp.status() {
            s if s.is_success() => Ok(resp),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(Error::Unauthorized),
            s => Err(Error::Status(s.as_u16(), path.to_string())),
        }
    }

    async fn get_json(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, Error> {
        Ok(self.get(path, query).await?.json().await?)
    }

    /// List every asset in the shared album (or individually shared assets).
    ///
    /// Tries, in order, the strategies that work across Immich releases:
    ///  1. `assets` embedded in `/api/shared-links/me`
    ///  2. `/api/albums/{id}` → `assets`
    ///  3. `/api/timeline/buckets` + `/api/timeline/bucket` (what Immich's own
    ///     share page uses, so it always works with a share key)
    pub async fn list_assets(&self) -> Result<Vec<Asset>, Error> {
        Ok(self.list_assets_with_source().await?.0)
    }

    pub async fn list_assets_with_source(&self) -> Result<(Vec<Asset>, ListSource), Error> {
        let me = self.get_json("/api/shared-links/me", &[]).await?;

        let direct = me.get("assets").map(Asset::parse_list).unwrap_or_default();
        if !direct.is_empty() {
            return Ok((direct, ListSource::SharedLink));
        }
        let album = me.get("album").ok_or(Error::Empty)?;
        let embedded = album.get("assets").map(Asset::parse_list).unwrap_or_default();
        if !embedded.is_empty() {
            return Ok((embedded, ListSource::Album));
        }
        let album_id = album.get("id").and_then(Value::as_str).ok_or(Error::Empty)?.to_string();

        match self.get_json(&format!("/api/albums/{album_id}"), &[]).await {
            Ok(full) => {
                let list = full.get("assets").map(Asset::parse_list).unwrap_or_default();
                if !list.is_empty() {
                    return Ok((list, ListSource::Album));
                }
            }
            Err(Error::Unauthorized) => return Err(Error::Unauthorized),
            Err(_) => {} // fall through to the timeline API
        }

        let list = self.list_via_timeline(&album_id).await?;
        if list.is_empty() {
            Err(Error::Empty)
        } else {
            Ok((list, ListSource::Timeline))
        }
    }

    async fn list_via_timeline(&self, album_id: &str) -> Result<Vec<Asset>, Error> {
        // Newer servers take no `size`; older ones require `size=MONTH`.
        let mut extra: Vec<(&str, &str)> = vec![("albumId", album_id)];
        let buckets = match self.get_json("/api/timeline/buckets", &extra).await {
            Ok(b) => b,
            Err(Error::Status(400, _)) => {
                extra.push(("size", "MONTH"));
                self.get_json("/api/timeline/buckets", &extra).await?
            }
            Err(e) => return Err(e),
        };

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for b in buckets.as_array().into_iter().flatten() {
            let Some(tb) = b.get("timeBucket").and_then(Value::as_str) else { continue };
            let mut q = extra.clone();
            q.push(("timeBucket", tb));
            let page = self.get_json("/api/timeline/bucket", &q).await?;
            for a in Asset::parse_list(&page) {
                if seen.insert(a.id.clone()) {
                    out.push(a);
                }
            }
        }
        Ok(out)
    }

    /// Download an asset as a wallpaper-ready image.
    ///
    /// With `prefer_original`, JPEG/PNG originals are used at full resolution.
    /// Everything else (HEIC, RAW, video, or downloads disabled on the link)
    /// falls back to Immich's `preview` rendition, which is a JPEG by default.
    pub async fn download(&self, asset: &Asset, prefer_original: bool) -> Result<Image, Error> {
        let mime_ok = asset.mime.as_deref().map_or(true, |m| wallpaper_safe(m).is_some());
        if prefer_original && asset.is_image && mime_ok {
            if let Ok(resp) = self.get(&format!("/api/assets/{}/original", asset.id), &[]).await {
                let ct = content_type(&resp);
                if let Some(ext) = wallpaper_safe(&ct) {
                    return Ok(Image { bytes: resp.bytes().await?.to_vec(), ext });
                }
            }
        }

        let resp = self
            .get(&format!("/api/assets/{}/thumbnail", asset.id), &[("size", "preview")])
            .await?;
        let ct = content_type(&resp);
        let ext = wallpaper_safe(&ct).unwrap_or(if ct.contains("webp") { "webp" } else { "jpg" });
        Ok(Image { bytes: resp.bytes().await?.to_vec(), ext })
    }
}

fn content_type(resp: &Response) -> String {
    resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase()
}
