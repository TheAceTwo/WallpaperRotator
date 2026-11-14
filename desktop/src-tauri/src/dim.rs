//! Darken a wallpaper image before it is applied.

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageDecoder, ImageReader};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

/// Write a copy of `src` darkened by `percent` (0–100) to `dst` as JPEG.
///
/// Returns the path to use as the wallpaper: `src` itself when `percent` is
/// 0 (no re-encode, no quality loss), otherwise `dst`.
///
/// EXIF orientation is applied before saving, because the re-encoded file
/// has no EXIF and phone photos would otherwise appear sideways.
pub fn dim_file(src: &Path, dst: &Path, percent: u8) -> Result<PathBuf, String> {
    if percent == 0 {
        return Ok(src.to_path_buf());
    }
    let err = |e: &dyn std::fmt::Display| format!("couldn't dim the photo: {e}");

    let mut decoder = ImageReader::open(src)
        .map_err(|e| err(&e))?
        .with_guessed_format()
        .map_err(|e| err(&e))?
        .into_decoder()
        .map_err(|e| err(&e))?;
    let orientation = decoder.orientation().map_err(|e| err(&e))?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| err(&e))?;
    img.apply_orientation(orientation);

    let mut rgb = img.into_rgb8();
    let factor = 1.0 - f32::from(percent.min(100)) / 100.0;
    for px in rgb.pixels_mut() {
        for c in px.0.iter_mut() {
            *c = (f32::from(*c) * factor).round() as u8;
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
        assert_eq!(dim_file(&src, &tmp("zero-out.jpg"), 0).unwrap(), src);
    }

    #[test]
    fn fifty_percent_halves_brightness() {
        let src = tmp("half.png");
        solid_png(&src, 16, 16, [200, 100, 40]);
        let out = dim_file(&src, &tmp("half-out.jpg"), 50).unwrap();
        let px = image::open(out).unwrap().into_rgb8().get_pixel(8, 8).0;
        // JPEG is lossy: allow a small tolerance.
        for (got, want) in px.iter().zip([100u8, 50, 20]) {
            assert!((i16::from(*got) - i16::from(want)).abs() <= 3, "{px:?}");
        }
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

        let out = dim_file(&src, &tmp("rotated-out.jpg"), 10).unwrap();
        let img = image::open(out).unwrap();
        assert_eq!((img.width(), img.height()), (4, 8));
    }
}
