//! Image helpers: PNG encode/decode, list thumbnails ("Show thumbnails"), combining images
//! ("Multiple Images Horizontally/Vertically") and Windows DIB conversion for imports.

use std::io::Cursor;

use image::{DynamicImage, GenericImageView, ImageFormat, RgbaImage, imageops};

use crate::{Error, Result};

pub fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    Ok(image::load_from_memory(bytes)?)
}

pub fn encode_png(img: &DynamicImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)?;
    Ok(out)
}

/// A PNG no larger than `max_w` × `max_h`, preserving the aspect ratio.
pub fn thumbnail_png(png: &[u8], max_w: u32, max_h: u32) -> Result<Vec<u8>> {
    if let Some((w, h)) = crate::model::png_dimensions(png) {
        if w <= max_w && h <= max_h {
            return Ok(png.to_vec());
        }
    }
    let img = decode(png)?;
    let (w, h) = img.dimensions();
    if w <= max_w && h <= max_h {
        return encode_png(&img);
    }
    encode_png(&img.thumbnail(max_w, max_h))
}

/// Grayscale pixels prepared for a text recognition engine (see [`gray_for_ocr`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrGray {
    pub width: u32,
    pub height: u32,
    /// One byte per pixel, row by row.
    pub pixels: Vec<u8>,
}

impl OcrGray {
    /// The share of pixels darker than mid-gray: near 1 for a dark-theme screenshot.
    pub fn dark_fraction(&self) -> f64 {
        if self.pixels.is_empty() {
            return 0.0;
        }
        self.pixels.iter().filter(|&&p| p < 128).count() as f64 / self.pixels.len() as f64
    }

    /// A binary PGM (`P5`) file, optionally with light and dark swapped.
    pub fn to_pgm(&self, inverted: bool) -> Vec<u8> {
        let mut out = format!("P5\n{} {}\n255\n", self.width, self.height).into_bytes();
        if inverted {
            out.extend(self.pixels.iter().map(|p| 255 - p));
        } else {
            out.extend_from_slice(&self.pixels);
        }
        out
    }
}

/// A grayscale copy of an image, enlarged `scale` times, with transparent areas turned white.
/// For engines that read small text poorly: Tesseract works best on text about 30 px tall,
/// and a 100 % screenshot has 11–13 px text.
pub fn gray_for_ocr(image: &[u8], scale: u32) -> Result<OcrGray> {
    let rgba = decode(image)?.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut gray = image::GrayImage::new(w, h);
    for (src, dst) in rgba.pixels().zip(gray.pixels_mut()) {
        let [r, g, b, a] = src.0.map(u32::from);
        let luma = (r * 299 + g * 587 + b * 114) / 1000;
        dst.0 = [((luma * a + 255 * (255 - a)) / 255) as u8];
    }
    let scale = scale.max(1);
    if scale > 1 {
        let (sw, sh) = (w.saturating_mul(scale), h.saturating_mul(scale));
        if u64::from(sw) * u64::from(sh) > 400_000_000 {
            return Err(Error::Invalid("enlarged image would be too large".into()));
        }
        gray = imageops::resize(&gray, sw, sh, imageops::FilterType::CatmullRom);
    }
    Ok(OcrGray { width: gray.width(), height: gray.height(), pixels: gray.into_raw() })
}

/// Combine several images side by side (`horizontal`) or stacked, on a transparent canvas.
pub fn combine_png(images: &[&[u8]], horizontal: bool) -> Result<Vec<u8>> {
    if images.is_empty() {
        return Err(Error::Invalid("no images to combine".into()));
    }
    let decoded: Vec<RgbaImage> = images.iter().map(|b| decode(b).map(|i| i.to_rgba8())).collect::<Result<_>>()?;
    let (w, h) = if horizontal {
        (decoded.iter().map(|i| i.width()).sum(), decoded.iter().map(|i| i.height()).max().unwrap_or(0))
    } else {
        (decoded.iter().map(|i| i.width()).max().unwrap_or(0), decoded.iter().map(|i| i.height()).sum())
    };
    if u64::from(w) * u64::from(h) > 400_000_000 {
        return Err(Error::Invalid("combined image would be too large".into()));
    }
    let mut canvas = RgbaImage::new(w, h);
    let mut offset = 0i64;
    for img in &decoded {
        if horizontal {
            imageops::overlay(&mut canvas, img, offset, 0);
            offset += i64::from(img.width());
        } else {
            imageops::overlay(&mut canvas, img, 0, offset);
            offset += i64::from(img.height());
        }
    }
    encode_png(&DynamicImage::ImageRgba8(canvas))
}

/// Convert a Windows `CF_DIB`/`CF_DIBV5` payload (a BMP without its file header) to PNG.
pub fn dib_to_png(dib: &[u8]) -> Result<Vec<u8>> {
    let u32_at = |o: usize| dib.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let u16_at = |o: usize| dib.get(o..o + 2).map(|b| u16::from_le_bytes(b.try_into().unwrap()));
    let invalid = || Error::Invalid("malformed DIB".into());
    let header_size = u32_at(0).ok_or_else(invalid)? as usize;
    if !(12..=124).contains(&header_size) || dib.len() < header_size {
        return Err(invalid());
    }
    let (bit_count, compression, colors_used) = if header_size == 12 {
        (u16_at(10).ok_or_else(invalid)?, 0, 0)
    } else {
        (u16_at(14).ok_or_else(invalid)?, u32_at(16).ok_or_else(invalid)?, u32_at(32).unwrap_or(0))
    };
    let palette_entries = if colors_used != 0 {
        colors_used as usize
    } else if bit_count <= 8 {
        1usize << bit_count
    } else {
        0
    };
    let entry_size = if header_size == 12 { 3 } else { 4 };
    let masks = match (compression, header_size) {
        (3, 40) => 12, // BI_BITFIELDS
        (6, 40) => 16, // BI_ALPHABITFIELDS
        _ => 0,
    };
    let pixel_offset = 14 + header_size + masks + palette_entries * entry_size;
    let file_size = 14 + dib.len();
    let mut bmp = Vec::with_capacity(file_size);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
    bmp.extend_from_slice(&[0, 0, 0, 0]);
    bmp.extend_from_slice(&(pixel_offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    let img = image::load_from_memory_with_format(&bmp, ImageFormat::Bmp)?;
    encode_png(&img)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        encode_png(&DynamicImage::ImageRgba8(RgbaImage::from_pixel(w, h, image::Rgba(rgba)))).unwrap()
    }

    #[test]
    fn thumbnails_only_shrink() {
        let small = png(10, 10, [1, 2, 3, 255]);
        assert_eq!(thumbnail_png(&small, 100, 100).unwrap(), small);
        let big = png(400, 100, [1, 2, 3, 255]);
        let t = thumbnail_png(&big, 200, 200).unwrap();
        assert_eq!(crate::model::png_dimensions(&t), Some((200, 50)));
    }

    #[test]
    fn gray_for_ocr_flattens_transparency_and_enlarges() {
        let mut img = RgbaImage::from_pixel(2, 1, image::Rgba([0, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 0, 0, 0]));
        let two_pixels = encode_png(&DynamicImage::ImageRgba8(img)).unwrap();
        // Opaque black stays black; transparent black becomes white.
        let gray = gray_for_ocr(&two_pixels, 1).unwrap();
        assert_eq!(gray, OcrGray { width: 2, height: 1, pixels: vec![0, 255] });
        assert_eq!(gray.dark_fraction(), 0.5);
        assert_eq!(gray.to_pgm(false), b"P5\n2 1\n255\n\x00\xff");
        assert_eq!(gray.to_pgm(true), b"P5\n2 1\n255\n\xff\x00");

        let big = gray_for_ocr(&png(3, 2, [255, 255, 255, 255]), 2).unwrap();
        assert_eq!((big.width, big.height, big.pixels.len()), (6, 4, 24));
        assert!(big.pixels.iter().all(|&p| p == 255));
        assert_eq!(big.dark_fraction(), 0.0);
        assert!(gray_for_ocr(b"not an image", 2).is_err());
    }

    #[test]
    fn combine_both_directions() {
        let a = png(10, 5, [255, 0, 0, 255]);
        let b = png(4, 8, [0, 255, 0, 255]);
        let h = combine_png(&[&a, &b], true).unwrap();
        assert_eq!(crate::model::png_dimensions(&h), Some((14, 8)));
        let v = combine_png(&[&a, &b], false).unwrap();
        assert_eq!(crate::model::png_dimensions(&v), Some((10, 13)));
        let img = decode(&h).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(12, 7).0, [0, 255, 0, 255]);
        assert_eq!(img.get_pixel(0, 7).0[3], 0, "padding is transparent");
    }

    #[test]
    fn dib_24bit_roundtrip() {
        // 2x2, 24-bit, bottom-up BI_RGB DIB: rows padded to 4 bytes.
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&24u16.to_le_bytes());
        dib.extend_from_slice(&[0u8; 24]); // compression .. clrImportant
        // bottom row: blue, green ; top row: red, white (BGR order)
        dib.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0]);
        dib.extend_from_slice(&[0, 0, 255, 255, 255, 255, 0, 0]);
        let png = dib_to_png(&dib).unwrap();
        let img = decode(&png).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (2, 2));
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0]);
        assert_eq!(img.get_pixel(1, 0).0, [255, 255, 255]);
        assert_eq!(img.get_pixel(0, 1).0, [0, 0, 255]);
        assert_eq!(img.get_pixel(1, 1).0, [0, 255, 0]);
        assert!(dib_to_png(&[1, 2, 3]).is_err());
    }
}
