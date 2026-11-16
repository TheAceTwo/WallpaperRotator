//! Dim and/or blur a wallpaper image before it is applied.

use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageDecoder, ImageReader};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

/// Blurred images don't need full resolution; working at this size keeps
/// blurring fast on 24-megapixel photos.
const BLUR_MAX_EDGE: u32 = 2560;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effects {
    /// Darken by this many percent (0–80).
    pub dim: u8,
    /// Blur strength 0–10 (0 = off).
    pub blur: u8,
}

impl Effects {
    pub fn is_none(self) -> bool {
        self.dim == 0 && self.blur == 0
    }
}

/// Write `src` with `fx` applied to `dst` (JPEG) and return the path to use.
///
/// With no effects, returns `src` untouched (no re-encode, no quality loss).
/// EXIF orientation is applied first, because the re-encoded file has no
/// EXIF and phone photos would otherwise appear sideways.
pub fn apply_effects(src: &Path, dst: &Path, fx: Effects) -> Result<PathBuf, String> {
    if fx.is_none() {
        return Ok(src.to_path_buf());
    }
    let err = |e: &dyn std::fmt::Display| format!("couldn't process the photo: {e}");

    let mut decoder = ImageReader::open(src)
        .map_err(|e| err(&e))?
        .with_guessed_format()
        .map_err(|e| err(&e))?
        .into_decoder()
        .map_err(|e| err(&e))?;
    let orientation = decoder.orientation().map_err(|e| err(&e))?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| err(&e))?;
    img.apply_orientation(orientation);

    if fx.blur > 0 {
        if img.width().max(img.height()) > BLUR_MAX_EDGE {
            img = img.resize(BLUR_MAX_EDGE, BLUR_MAX_EDGE, FilterType::Triangle);
        }
    }
    let mut rgb = img.into_rgb8();
    if fx.blur > 0 {
        // Scale with image size so a level looks the same on any photo:
        // level 10 ≈ a 30 px blur on a 2560 px wide image.
        let edge = rgb.width().max(rgb.height()) as f32;
        let sigma = f32::from(fx.blur.min(10)) * 3.0 * edge / BLUR_MAX_EDGE as f32;
        rgb = imageops::fast_blur(&rgb, sigma.max(0.5));
    }
    if fx.dim > 0 {
        let factor = 1.0 - f32::from(fx.dim.min(100)) / 100.0;
        for px in rgb.pixels_mut() {
            for c in px.0.iter_mut() {
                *c = (f32::from(*c) * factor).round() as u8;
            }
        }
    }

    let out = BufWriter::new(File::create(dst).map_err(|e| err(&e))?);
    JpegEncoder::new_with_quality(out, 92)
        .encode_image(&rgb)
        .map_err(|e| err(&e))?;
    Ok(dst.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dim-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn solid_png(path: &Path, w: u32, h: u32, color: [u8; 3]) {
        RgbImage::from_pixel(w, h, Rgb(color)).save(path).unwrap();
    }

    #[test]
    fn zero_percent_returns_original() {
        let src = tmp("zero.png");
        solid_png(&src, 4, 4, [200, 100, 50]);
        assert_eq!(apply_effects(&src, &tmp("zero-out.jpg"), Effects::default()).unwrap(), src);
    }

    #[test]
    fn fifty_percent_halves_brightness() {
        let src = tmp("half.png");
        solid_png(&src, 16, 16, [200, 100, 40]);
        let out = apply_effects(&src, &tmp("half-out.jpg"), Effects { dim: 50, blur: 0 }).unwrap();
        let px = image::open(out).unwrap().into_rgb8().get_pixel(8, 8).0;
        // JPEG is lossy: allow a small tolerance.
        for (got, want) in px.iter().zip([100u8, 50, 20]) {
            assert!((i16::from(*got) - i16::from(want)).abs() <= 3, "{px:?}");
        }
    }

    #[test]
    fn blur_softens_edges_but_keeps_size() {
        // Left half black, right half white: blurring makes the edge grey.
        let src = tmp("edge.png");
        // 1280 px wide: level 5 is a ~7.5 px blur (it scales with image size).
        RgbImage::from_fn(1280, 40, |x, _| if x < 640 { Rgb([0, 0, 0]) } else { Rgb([255, 255, 255]) })
            .save(&src)
            .unwrap();
        let out = apply_effects(&src, &tmp("edge-out.jpg"), Effects { dim: 0, blur: 5 }).unwrap();
        let img = image::open(out).unwrap().into_rgb8();
        assert_eq!(img.dimensions(), (1280, 40));
        let near_edge = img.get_pixel(636, 20).0[0];
        assert!(near_edge > 40, "edge should be blurred, got {near_edge}");
        assert!(img.get_pixel(10, 20).0[0] < 20, "far from the edge stays dark");
    }

    #[test]
    fn large_images_are_downscaled_when_blurring() {
        let src = tmp("big.png");
        solid_png(&src, 4000, 3000, [10, 200, 30]);
        let out = apply_effects(&src, &tmp("big-out.jpg"), Effects { dim: 0, blur: 3 }).unwrap();
        let img = image::open(out).unwrap();
        assert_eq!((img.width(), img.height()), (2560, 1920));
    }

    /// A 8x4 (landscape) JPEG tagged EXIF orientation 6 (rotate 90°) must come
    /// out 4x8 (portrait), like it looks in a photo viewer.
    #[test]
    fn applies_exif_orientation() {
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, 90)
            .encode_image(&RgbImage::from_pixel(8, 4, Rgb([120, 120, 120])))
            .unwrap();
        // Minimal big-endian EXIF APP1 segment with Orientation (0x0112) = 6.
        let tiff: &[u8] = &[
            b'M', b'M', 0, 42, 0, 0, 0, 8, // header, IFD at offset 8
            0, 1, // one entry
            0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, // Orientation SHORT = 6
            0, 0, 0, 0, // no next IFD
        ];
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(tiff);
        let len = (app1.len() + 2) as u16;
        let mut tagged = jpeg[..2].to_vec(); // SOI
        tagged.extend_from_slice(&[0xFF, 0xE1]);
        tagged.extend_from_slice(&len.to_be_bytes());
        tagged.extend_from_slice(&app1);
        tagged.extend_from_slice(&jpeg[2..]);
        let src = tmp("rotated.jpg");
        std::fs::write(&src, tagged).unwrap();

        let out = apply_effects(&src, &tmp("rotated-out.jpg"), Effects { dim: 10, blur: 0 }).unwrap();
        let img = image::open(out).unwrap();
        assert_eq!((img.width(), img.height()), (4, 8));
    }
}
