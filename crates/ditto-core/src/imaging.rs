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
