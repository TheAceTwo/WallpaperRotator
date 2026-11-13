//! Parsing of Immich share links.
//!
//! Immich produces two styles of public link:
//!   https://photos.example.com/share/<KEY>   -> API auth via `?key=<KEY>`
//!   https://photos.example.com/s/<SLUG>      -> API auth via `?slug=<SLUG>` (custom URL)

use crate::Error;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareAuth {
    Key(String),
    Slug(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareLink {
    /// Server origin, e.g. `https://photos.example.com` (no trailing slash).
    pub server: String,
    pub auth: ShareAuth,
}

impl ShareLink {
    pub fn parse(input: &str) -> Result<Self, Error> {
        let input = input.trim();
        let url = Url::parse(input).map_err(|_| Error::InvalidShareLink("not a valid URL".into()))?;
        if url.scheme() != "https" && url.scheme() != "http" {
            return Err(Error::InvalidShareLink("link must start with https://".into()));
        }
        let host = url
            .host_str()
            .ok_or_else(|| Error::InvalidShareLink("link has no host".into()))?;
        let server = match url.port() {
            Some(p) => format!("{}://{}:{}", url.scheme(), host, p),
            None => format!("{}://{}", url.scheme(), host),
        };

        let segs: Vec<&str> = url
            .path_segments()
            .map(|s| s.filter(|p| !p.is_empty()).collect())
            .unwrap_or_default();

        // Allow either /share/<key> or /s/<slug>, possibly followed by more path
        // (e.g. /share/<key>/photos/<id> when copied from an opened photo).
        let auth = match segs.as_slice() {
            ["share", key, ..] => ShareAuth::Key((*key).to_string()),
            ["s", slug, ..] => ShareAuth::Slug((*slug).to_string()),
            _ => {
                return Err(Error::InvalidShareLink(
                    "expected a link like https://your-server/share/XXXX".into(),
                ))
            }
        };
        Ok(Self { server, auth })
    }

    /// Query parameter used to authenticate API calls with this link.
    pub fn auth_param(&self) -> (&'static str, &str) {
        match &self.auth {
            ShareAuth::Key(k) => ("key", k.as_str()),
            ShareAuth::Slug(s) => ("slug", s.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_key_links() {
        let l = ShareLink::parse("https://photos.example.com/share/AbC-123_xyz").unwrap();
        assert_eq!(l.server, "https://photos.example.com");
        assert_eq!(l.auth, ShareAuth::Key("AbC-123_xyz".into()));
        assert_eq!(l.auth_param(), ("key", "AbC-123_xyz"));
    }

    #[test]
    fn parses_slug_links_with_port_and_trailing_path() {
        let l = ShareLink::parse("  http://10.0.0.5:2283/s/family/photos/abc  ").unwrap();
        assert_eq!(l.server, "http://10.0.0.5:2283");
        assert_eq!(l.auth, ShareAuth::Slug("family".into()));
    }

    #[test]
    fn rejects_non_share_links() {
        assert!(ShareLink::parse("https://photos.example.com/albums/123").is_err());
        assert!(ShareLink::parse("photos.example.com/share/abc").is_err());
        assert!(ShareLink::parse("ftp://x/share/abc").is_err());
    }
}
