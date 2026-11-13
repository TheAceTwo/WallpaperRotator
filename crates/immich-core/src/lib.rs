//! Shared logic for Immich Wallpaper: parse a share link, list the album,
//! pick a random photo, download it.

pub mod client;
pub mod model;
pub mod picker;
pub mod share;

pub use client::{Image, ImmichClient, ListSource};
pub use model::{Asset, Orientation};
pub use picker::{pick, History};
pub use share::{ShareAuth, ShareLink};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid share link: {0}")]
    InvalidShareLink(String),
    #[error("the share link was rejected (expired, revoked, or password-protected)")]
    Unauthorized,
    #[error("server returned HTTP {0} for {1}")]
    Status(u16, String),
    #[error("the shared album has no photos")]
    Empty,
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
}
