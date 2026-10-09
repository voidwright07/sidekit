//! Images as Base64 / data URIs, and QR codes.

use base64::Engine as _;

pub struct ImageInfo {
    pub mime: &'static str,
    pub ext: &'static str,
    pub bytes: Vec<u8>,
    pub size: Option<(u32, u32)>,
}

/// The image type, going by the first bytes.
pub fn sniff(b: &[u8]) -> Option<(&'static str, &'static str)> {
    Some(match b {
        [0x89, b'P', b'N', b'G', ..] => ("image/png", "png"),
        [0xFF, 0xD8, 0xFF, ..] => ("image/jpeg", "jpg"),
        [b'G', b'I', b'F', b'8', ..] => ("image/gif", "gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => ("image/webp", "webp"),
        [b'B', b'M', ..] => ("image/bmp", "bmp"),
        [0, 0, 1, 0, ..] => ("image/x-icon", "ico"),
        [b'I', b'I', 0x2A, 0, ..] | [b'M', b'M', 0, 0x2A, ..] => ("image/tiff", "tiff"),
        _ => {
            let head = String::from_utf8_lossy(&b[..b.len().min(512)]).to_lowercase();
            if head.trim_start().starts_with("<svg") || (head.contains("<?xml") && head.contains("<svg")) {
                ("image/svg+xml", "svg")
            } else {
                return None;
            }
        }
    })
}

fn be16(b: &[u8], i: usize) -> Option<u32> {
    Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32)
}

fn le16(b: &[u8], i: usize) -> Option<u32> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32)
}

fn le32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// Pixel dimensions from the header, for the common formats.
pub fn dimensions(b: &[u8], ext: &str) -> Option<(u32, u32)> {
    match ext {
        "png" => Some((u32::from_be_bytes(b.get(16..20)?.try_into().ok()?), u32::from_be_bytes(b.get(20..24)?.try_into().ok()?))),
        "gif" => Some((le16(b, 6)?, le16(b, 8)?)),
        "bmp" => Some((le32(b, 18)?, (le32(b, 22)? as i32).unsigned_abs())),
        "jpg" => {
            let mut i = 2;
            while i + 9 < b.len() {
                if b[i] != 0xFF {
                    return None;
                }
                let marker = b[i + 1];
                let len = be16(b, i + 2)? as usize;
                // SOF0..SOF15, except DHT (C4), JPG (C8) and DAC (CC).
                if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
                    return Some((be16(b, i + 7)?, be16(b, i + 5)?));
                }
                i += 2 + len;
            }
            None
        }
        "webp" => match b.get(12..16)? {
            b"VP8X" => {
                let w = u32::from_le_bytes([b[24], b[25], b[26], 0]) + 1;
                let h = u32::from_le_bytes([b[27], b[28], b[29], 0]) + 1;
                Some((w, h))
            }
            b"VP8 " => Some((le16(b, 26)? & 0x3FFF, le16(b, 28)? & 0x3FFF)),
            b"VP8L" => {
                let bits = le32(b, 21)?;
                Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
            }
            _ => None,
        },
        _ => None,
    }
}

/// A data URI or bare Base64 → image bytes and what they are.
pub fn decode_image(input: &str) -> Result<ImageInfo, String> {
    let t = input.trim();
    let b64 = match t.strip_prefix("data:") {
        Some(rest) => rest.split_once(',').map(|(_, d)| d).ok_or("The data URI has no comma before its data")?,
        None => t,
    };
    let clean: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&clean)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(clean.trim_end_matches('=')))
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(clean.trim_end_matches('=')))
        .map_err(|_| "This is not valid Base64".to_string())?;
    let (mime, ext) = sniff(&bytes).ok_or("The decoded bytes are not a PNG, JPEG, GIF, WebP, BMP, ICO, TIFF or SVG image")?;
    let size = dimensions(&bytes, ext);
    Ok(ImageInfo { mime, ext, bytes, size })
}

pub fn data_uri(bytes: &[u8]) -> Result<String, String> {
    let (mime, _) = sniff(bytes).ok_or("That file is not a recognized image")?;
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

pub const QR_LEVELS: &[&str] = &["Low (7%)", "Medium (15%)", "Quartile (25%)", "High (30%)"];

/// An SVG QR code (black on white, with the quiet zone) and its size in modules.
pub fn qr_svg(text: &str, level: usize) -> Result<(String, usize), String> {
    use qrcode::render::svg;
    use qrcode::{EcLevel, QrCode};
    let ec = [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H][level.min(3)];
    let code = QrCode::with_error_correction_level(text.as_bytes(), ec)
        .map_err(|_| "Too much text for a QR code at this error-correction level".to_string())?;
    let svg = code
        .render::<svg::Color>()
        .min_dimensions(320, 320)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok((svg, code.width()))
}

/// Every QR code found in an image, as text.
pub fn read_qr(bytes: &[u8]) -> Result<Vec<String>, String> {
    let image = image::load_from_memory(bytes).map_err(|_| "That file is not an image this tool can read".to_string())?;
    let scan = |img: &image::GrayImage| -> Vec<String> {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| img.get_pixel(x as u32, y as u32).0[0]);
        prepared.detect_grids().iter().filter_map(|g| g.decode().ok()).map(|(_, text)| text).collect()
    };
    let grey = image.to_luma8();
    let mut found = scan(&grey);
    // Photos: a code far larger than it needs to be reads better scaled down.
    if found.is_empty() && grey.width().max(grey.height()) > 1600 {
        found = scan(&image.thumbnail(1200, 1200).to_luma8());
    }
    if found.is_empty() {
        return Err("No QR code was found in this image".into());
    }
    found.dedup();
    Ok(found)
}

// ---------------------------------------------------------------- image converter

pub const CONVERT_FORMATS: &[&str] = &["PNG", "JPEG", "WebP", "GIF", "BMP", "TIFF", "ICO"];

#[derive(Debug)]
pub struct ImageMeta {
    pub rows: Vec<(&'static str, String)>,
    /// EXIF tags and their values, as the camera or editor wrote them.
    pub exif: Vec<(String, String)>,
    /// The EXIF data says where the picture was taken.
    pub has_location: bool,
}

fn decode_oriented(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    use image::ImageDecoder as _;
    let bad = |e: image::ImageError| format!("This image could not be read: {e}");
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let mut decoder = reader.into_decoder().map_err(bad)?;
    // Re-encoding drops the EXIF orientation, so turn the pixels instead.
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(bad)?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// What an image file says about itself.
pub fn image_meta(bytes: &[u8]) -> Result<ImageMeta, String> {
    let (mime, ext) = sniff(bytes).ok_or("That file is not a recognized image")?;
    if ext == "svg" {
        return Err("SVG is a vector format: SVG to Code converts those".into());
    }
    let image = image::load_from_memory(bytes).map_err(|e| format!("This image could not be read: {e}"))?;
    let (w, h) = (image.width(), image.height());
    let color = image.color();
    let channels = match color.channel_count() {
        1 => "Greyscale",
        2 => "Greyscale + alpha",
        3 => "RGB",
        _ => "RGBA",
    };
    let mut rows: Vec<(&'static str, String)> = vec![
        ("Format", format!("{} ({mime})", ext.to_uppercase())),
        ("Dimensions", format!("{w} × {h} px")),
        ("Megapixels", format!("{:.2}", w as f64 * h as f64 / 1e6)),
        ("Color", format!("{channels}, {} bits per channel", color.bits_per_pixel() / color.channel_count() as u16)),
        ("Transparency", if color.has_alpha() { "Has an alpha channel".into() } else { "None".into() }),
        ("File size", super::plural(bytes.len(), "byte")),
    ];
    let mut exif_rows = Vec::new();
    let mut has_location = false;
    if let Ok(data) = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(bytes)) {
        for f in data.fields().filter(|f| f.ifd_num == exif::In::PRIMARY) {
            let name = f.tag.to_string();
            has_location |= name.starts_with("GPS");
            // Maker notes and the like are opaque blobs.
            let value = f.display_value().with_unit(&data).to_string();
            if name != "MakerNote" && !value.is_empty() {
                exif_rows.push((name, super::clip(&value, 120)));
            }
        }
        rows.push(("Metadata", super::plural(exif_rows.len(), "EXIF tag")));
    }
    Ok(ImageMeta { rows, exif: exif_rows, has_location })
}

/// Re-encode an image as `CONVERT_FORMATS[format]`. `quality` (1–100) applies
/// to JPEG. Returns the bytes, the file extension, and a note when the result
/// differs from the source in more than its format.
pub fn convert_image(bytes: &[u8], format: usize, quality: u8) -> Result<(Vec<u8>, &'static str, Option<&'static str>), String> {
    use image::ImageFormat as F;
    let image = decode_oriented(bytes)?;
    let bad = |e: image::ImageError| format!("The image could not be converted: {e}");
    let mut out = std::io::Cursor::new(Vec::new());
    let mut note = None;
    let ext = match format {
        1 => {
            // JPEG has no transparency: lay the picture over white.
            let rgba = image.to_rgba8();
            let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
            for (dst, src) in rgb.pixels_mut().zip(rgba.pixels()) {
                let a = src.0[3] as u32;
                for i in 0..3 {
                    dst.0[i] = ((src.0[i] as u32 * a + 255 * (255 - a)) / 255) as u8;
                }
            }
            if image.color().has_alpha() {
                note = Some("JPEG has no transparency; transparent areas are white");
            }
            rgb.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))).map_err(bad)?;
            "jpg"
        }
        6 => {
            // An icon holds at most 256 × 256 pixels.
            let fitted = if image.width() > 256 || image.height() > 256 {
                note = Some("Scaled down to fit the 256 × 256 px limit of ICO");
                image.thumbnail(256, 256)
            } else {
                image
            };
            fitted.to_rgba8().write_to(&mut out, F::Ico).map_err(bad)?;
            "ico"
        }
        other => {
            let (f, ext) = match other {
                0 => (F::Png, "png"),
                2 => (F::WebP, "webp"),
                3 => (F::Gif, "gif"),
                4 => (F::Bmp, "bmp"),
                _ => (F::Tiff, "tiff"),
            };
            if other == 2 {
                note = Some("WebP is written lossless");
            }
            // These encoders want 8-bit channels.
            let eight = if image.color().has_alpha() { image::DynamicImage::ImageRgba8(image.to_rgba8()) } else { image::DynamicImage::ImageRgb8(image.to_rgb8()) };
            eight.write_to(&mut out, f).map_err(bad)?;
            ext
        }
    };
    Ok((out.into_inner(), ext, note))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_between_formats() {
        // 4 × 2, half transparent red on the left, opaque blue on the right.
        let mut src = image::RgbaImage::new(4, 2);
        for (x, _, p) in src.enumerate_pixels_mut() {
            *p = if x < 2 { image::Rgba([255, 0, 0, 0]) } else { image::Rgba([0, 0, 255, 255]) };
        }
        let mut png = std::io::Cursor::new(Vec::new());
        src.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let png = png.into_inner();

        let meta = image_meta(&png).unwrap();
        let get = |k: &str| meta.rows.iter().find(|r| r.0 == k).unwrap().1.clone();
        assert_eq!(get("Format"), "PNG (image/png)");
        assert_eq!(get("Dimensions"), "4 × 2 px");
        assert_eq!(get("Color"), "RGBA, 8 bits per channel");
        assert_eq!(get("Transparency"), "Has an alpha channel");
        assert!(meta.exif.is_empty() && !meta.has_location);

        for (i, ext) in ["png", "jpg", "webp", "gif", "bmp", "tiff", "ico"].iter().enumerate() {
            let (bytes, got, _) = convert_image(&png, i, 90).unwrap();
            assert_eq!(got, *ext);
            assert_eq!(sniff(&bytes).unwrap().1, *ext, "{ext} output should be {ext}");
            let back = image::load_from_memory(&bytes).unwrap();
            assert_eq!((back.width(), back.height()), (4, 2));
        }
        let (jpg, _, note) = convert_image(&png, 1, 95).unwrap();
        assert_eq!(note, Some("JPEG has no transparency; transparent areas are white"));
        let px = image::load_from_memory(&jpg).unwrap().to_rgb8();
        // The transparent half came out white, not black or red.
        assert!(px.get_pixel(0, 0).0.iter().all(|c| *c > 200), "{:?}", px.get_pixel(0, 0));
        assert!(px.get_pixel(3, 1).0[2] > 150 && px.get_pixel(3, 1).0[0] < 100);

        let big = image::DynamicImage::new_rgb8(600, 300);
        let mut buf = std::io::Cursor::new(Vec::new());
        big.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let (ico, _, note) = convert_image(&buf.into_inner(), 6, 90).unwrap();
        assert!(note.unwrap().contains("256"));
        let icon = image::load_from_memory(&ico).unwrap();
        assert_eq!((icon.width(), icon.height()), (256, 128));

        assert!(convert_image(b"not an image", 0, 90).is_err());
        assert!(image_meta(b"<svg xmlns='x'/>").unwrap_err().contains("SVG to Code"));
    }

    #[test]
    fn reads_qr_codes_it_wrote() {
        use qrcode::QrCode;
        let text = "https://example.com/sidekit?q=1";
        let code = QrCode::new(text.as_bytes()).unwrap();
        // Render by hand: 8 px modules with a 4-module quiet zone.
        let (n, scale, quiet) = (code.width(), 8usize, 4usize);
        let side = ((n + quiet * 2) * scale) as u32;
        let colors = code.to_colors();
        let img = image::GrayImage::from_fn(side, side, |x, y| {
            let (mx, my) = ((x as usize / scale).wrapping_sub(quiet), (y as usize / scale).wrapping_sub(quiet));
            let dark = mx < n && my < n && colors[my * n + mx] == qrcode::Color::Dark;
            image::Luma([if dark { 0 } else { 255 }])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert_eq!(read_qr(&png.into_inner()).unwrap(), [text]);

        let blank = image::GrayImage::from_pixel(64, 64, image::Luma([255]));
        let mut png = std::io::Cursor::new(Vec::new());
        blank.write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert_eq!(read_qr(&png.into_inner()).unwrap_err(), "No QR code was found in this image");
        assert!(read_qr(b"nope").is_err());
    }

    // A 3×2 PNG.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAYAAACddGYaAAAAEUlEQVR4nGP4z8DwH4YZkDkAm34L9XKwuTwAAAAASUVORK5CYII=";

    #[test]
    fn images() {
        let i = decode_image(&format!("data:image/png;base64,{PNG}")).unwrap();
        assert_eq!((i.mime, i.size), ("image/png", Some((3, 2))));
        assert_eq!(decode_image(PNG).unwrap().ext, "png");
        assert!(data_uri(&i.bytes).unwrap().starts_with("data:image/png;base64,iVBOR"));
        assert!(decode_image("aGVsbG8=").err().unwrap().contains("not a PNG"));
        assert!(decode_image("%%%").is_err());
        assert_eq!(sniff(b"<svg xmlns='x'/>").unwrap().1, "svg");
        let gif = [b'G', b'I', b'F', b'8', b'9', b'a', 10, 0, 20, 0];
        assert_eq!(dimensions(&gif, "gif"), Some((10, 20)));
    }

    #[test]
    fn qr() {
        let (svg, w) = qr_svg("https://example.com", 1).unwrap();
        assert!(svg.starts_with("<?xml") && svg.contains("<svg"));
        assert_eq!(w, 25);
        assert!(qr_svg(&"x".repeat(4000), 3).is_err());
    }
}
