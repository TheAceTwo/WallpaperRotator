//! Tolerant parsing of Immich asset JSON.
//!
//! Different Immich releases return assets in different shapes, so we parse
//! from `serde_json::Value` and keep only what we need:
//!   * `AssetResponseDto` objects (album `assets[]`, shared-link `assets[]`,
//!     and pre-1.133 timeline buckets)
//!   * the columnar `TimeBucketAssetResponseDto` used by newer timeline
//!     endpoints: `{ "id": [...], "isImage": [...], "ratio": [...] }`

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub is_image: bool,
    /// width / height after EXIF rotation, if known.
    pub ratio: Option<f64>,
    pub mime: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    #[default]
    Any,
    Landscape,
    Portrait,
}

impl Asset {
    pub fn matches(&self, o: Orientation) -> bool {
        match (o, self.ratio) {
            (Orientation::Any, _) => true,
            // Unknown dimensions: let it through rather than hide the photo.
            (_, None) => true,
            (Orientation::Landscape, Some(r)) => r >= 1.0,
            (Orientation::Portrait, Some(r)) => r <= 1.0,
        }
    }

    /// Parse a single `AssetResponseDto`.
    pub fn from_dto(v: &Value) -> Option<Self> {
        let id = v.get("id")?.as_str()?.to_string();
        let is_image = match v.get("type").and_then(Value::as_str) {
            Some(t) => t.eq_ignore_ascii_case("IMAGE"),
            None => v.get("isImage").and_then(Value::as_bool).unwrap_or(true),
        };
        if v.get("isTrashed").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        let mime = v
            .get("originalMimeType")
            .and_then(Value::as_str)
            .map(str::to_string);

        let exif = v.get("exifInfo");
        let num = |obj: Option<&Value>, k: &str| obj.and_then(|o| o.get(k)).and_then(Value::as_f64);
        let w = num(Some(v), "width").or_else(|| num(exif, "exifImageWidth"));
        let h = num(Some(v), "height").or_else(|| num(exif, "exifImageHeight"));
        // EXIF orientations 5-8 mean the image is rotated 90°.
        let rotated = exif
            .and_then(|e| e.get("orientation"))
            .and_then(|o| o.as_str().map(str::to_string).or_else(|| o.as_i64().map(|n| n.to_string())))
            .map(|o| matches!(o.as_str(), "5" | "6" | "7" | "8"))
            .unwrap_or(false);
        let ratio = match (w, h) {
            (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some(if rotated { h / w } else { w / h }),
            _ => None,
        };
        Some(Self { id, is_image, ratio, mime })
    }

    /// Parse either an array of DTOs or a columnar time-bucket object.
    pub fn parse_list(v: &Value) -> Vec<Self> {
        if let Some(arr) = v.as_array() {
            return arr.iter().filter_map(Self::from_dto).collect();
        }
        let Some(ids) = v.get("id").and_then(Value::as_array) else {
            return Vec::new();
        };
        let col = |k: &str| v.get(k).and_then(Value::as_array);
        let is_image = col("isImage");
        let ratio = col("ratio");
        let trashed = col("isTrashed");
        ids.iter()
            .enumerate()
            .filter_map(|(i, id)| {
                let id = id.as_str()?.to_string();
                if trashed.and_then(|t| t.get(i)).and_then(Value::as_bool) == Some(true) {
                    return None;
                }
                Some(Self {
                    id,
                    is_image: is_image.and_then(|c| c.get(i)).and_then(Value::as_bool).unwrap_or(true),
                    ratio: ratio.and_then(|c| c.get(i)).and_then(Value::as_f64).filter(|r| *r > 0.0),
                    mime: None,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_dto_with_rotated_exif() {
        let a = Asset::from_dto(&json!({
            "id": "a1", "type": "IMAGE", "originalMimeType": "image/jpeg",
            "exifInfo": { "exifImageWidth": 4000, "exifImageHeight": 3000, "orientation": "6" }
        }))
        .unwrap();
        assert!(a.is_image);
        assert!((a.ratio.unwrap() - 0.75).abs() < 1e-9);
        assert!(a.matches(Orientation::Portrait));
        assert!(!a.matches(Orientation::Landscape));
    }

    #[test]
    fn parses_columnar_bucket() {
        let list = Asset::parse_list(&json!({
            "id": ["x", "y", "z"], "isImage": [true, false, true],
            "ratio": [1.5, 1.0, 0.5], "isTrashed": [false, false, true]
        }));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "x");
        assert!(!list[1].is_image);
    }

    #[test]
    fn unknown_ratio_matches_everything() {
        let a = Asset { id: "q".into(), is_image: true, ratio: None, mime: None };
        assert!(a.matches(Orientation::Landscape) && a.matches(Orientation::Portrait));
    }
}
