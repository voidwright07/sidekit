//! Compress and decompress with the common stream formats: gzip, zlib, raw
//! DEFLATE, Brotli, Zstandard, bzip2 and XZ.

use std::io::{Read, Write};

use base64::Engine as _;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Codec {
    Gzip,
    Zlib,
    Deflate,
    Brotli,
    Zstd,
    Bzip2,
    Xz,
}

pub const CODECS: &[Codec] = &[Codec::Gzip, Codec::Zlib, Codec::Deflate, Codec::Brotli, Codec::Zstd, Codec::Bzip2, Codec::Xz];

impl Codec {
    pub fn label(self) -> &'static str {
        match self {
            Codec::Gzip => "Gzip",
            Codec::Zlib => "Zlib",
            Codec::Deflate => "Deflate (raw)",
            Codec::Brotli => "Brotli",
            Codec::Zstd => "Zstandard",
            Codec::Bzip2 => "Bzip2",
            Codec::Xz => "XZ",
        }
    }

    /// The file extension a compressed file gets.
    pub fn ext(self) -> &'static str {
        match self {
            Codec::Gzip => "gz",
            Codec::Zlib => "zz",
            Codec::Deflate => "deflate",
            Codec::Brotli => "br",
            Codec::Zstd => "zst",
            Codec::Bzip2 => "bz2",
            Codec::Xz => "xz",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Fastest,
    Balanced,
    Smallest,
}

pub const LEVELS: &[Level] = &[Level::Fastest, Level::Balanced, Level::Smallest];

impl Level {
    /// Each format's own scale, from its fastest setting to its smallest.
    fn pick(self, fastest: u32, balanced: u32, smallest: u32) -> u32 {
        match self {
            Level::Fastest => fastest,
            Level::Balanced => balanced,
            Level::Smallest => smallest,
        }
    }
}

/// Decompressed output larger than this is refused, so a tiny "zip bomb"
/// cannot exhaust memory.
pub const MAX_OUTPUT: u64 = 2 * 1024 * 1024 * 1024;

pub fn compress(data: &[u8], codec: Codec, level: Level) -> Result<Vec<u8>, String> {
    use flate2::Compression;
    let flate = Compression::new(level.pick(1, 6, 9));
    let out = match codec {
        Codec::Gzip => finish(flate2::write::GzEncoder::new(Vec::new(), flate), data, |e| e.finish()),
        Codec::Zlib => finish(flate2::write::ZlibEncoder::new(Vec::new(), flate), data, |e| e.finish()),
        Codec::Deflate => finish(flate2::write::DeflateEncoder::new(Vec::new(), flate), data, |e| e.finish()),
        Codec::Bzip2 => {
            let level = bzip2::Compression::new(level.pick(1, 6, 9));
            finish(bzip2::write::BzEncoder::new(Vec::new(), level), data, |e| e.finish())
        }
        Codec::Xz => finish(liblzma::write::XzEncoder::new(Vec::new(), level.pick(1, 6, 9)), data, |e| e.finish()),
        Codec::Zstd => zstd::encode_all(data, level.pick(1, 3, 19) as i32),
        Codec::Brotli => {
            let mut out = Vec::new();
            {
                let mut w = brotli::CompressorWriter::new(&mut out, 64 * 1024, level.pick(1, 6, 11), 22);
                w.write_all(data).map_err(|e| e.to_string())?;
            }
            Ok(out)
        }
    };
    out.map_err(|e| format!("Could not compress with {}: {e}", codec.label()))
}

fn finish<W: Write>(mut w: W, data: &[u8], end: impl FnOnce(W) -> std::io::Result<Vec<u8>>) -> std::io::Result<Vec<u8>> {
    w.write_all(data)?;
    end(w)
}

pub fn decompress(data: &[u8], codec: Codec) -> Result<Vec<u8>, String> {
    let r = match codec {
        // The multi-stream readers also take files made by joining several
        // compressed files end to end, as `cat a.gz b.gz` does.
        Codec::Gzip => read_all(flate2::read::MultiGzDecoder::new(data)),
        Codec::Zlib => read_all(flate2::read::ZlibDecoder::new(data)),
        Codec::Deflate => read_all(flate2::read::DeflateDecoder::new(data)),
        Codec::Brotli => read_all(brotli::Decompressor::new(data, 64 * 1024)),
        Codec::Zstd => zstd::Decoder::new(data).and_then(read_all),
        Codec::Bzip2 => read_all(bzip2::read::MultiBzDecoder::new(data)),
        Codec::Xz => read_all(liblzma::read::XzDecoder::new_multi_decoder(data)),
    };
    match r {
        Ok(Some(out)) => Ok(out),
        Ok(None) => Err(format!("The decompressed data is larger than {} GB, so it was not loaded", MAX_OUTPUT >> 30)),
        Err(e) => Err(format!("This is not valid {} data: {e}", codec.label())),
    }
}

/// Read everything, or `None` when the output passes [`MAX_OUTPUT`].
fn read_all(r: impl Read) -> std::io::Result<Option<Vec<u8>>> {
    let mut out = Vec::new();
    r.take(MAX_OUTPUT + 1).read_to_end(&mut out)?;
    Ok((out.len() as u64 <= MAX_OUTPUT).then_some(out))
}

/// The format named by the data's leading magic bytes. Raw DEFLATE and
/// Brotli have none, so they are never detected here.
pub fn detect(data: &[u8]) -> Option<Codec> {
    match data {
        [0x1f, 0x8b, ..] => Some(Codec::Gzip),
        [0x28, 0xb5, 0x2f, 0xfd, ..] => Some(Codec::Zstd),
        [0xfd, b'7', b'z', b'X', b'Z', 0, ..] => Some(Codec::Xz),
        [b'B', b'Z', b'h', b'1'..=b'9', ..] => Some(Codec::Bzip2),
        // CMF says DEFLATE with a window of at most 32 KB, and CMF·FLG is a multiple of 31.
        [cmf, flg, ..] if cmf & 0x0f == 8 && cmf >> 4 <= 7 && (u16::from(*cmf) << 8 | u16::from(*flg)) % 31 == 0 => Some(Codec::Zlib),
        _ => None,
    }
}

/// Archives hold many files rather than one stream, so they get a clear
/// message instead of a decoding error.
fn archive_kind(data: &[u8]) -> Option<&'static str> {
    match data {
        [b'P', b'K', 3, 4, ..] | [b'P', b'K', 5, 6, ..] => Some("a ZIP archive"),
        [b'7', b'z', 0xbc, 0xaf, 0x27, 0x1c, ..] => Some("a 7-Zip archive"),
        [b'R', b'a', b'r', b'!', 0x1a, 0x07, ..] => Some("a RAR archive"),
        _ if data.get(257..262) == Some(b"ustar") => Some("a tar archive"),
        _ => None,
    }
}

/// Decompress with the format the data announces, falling back to raw
/// DEFLATE and then Brotli, which carry no signature.
pub fn decompress_auto(data: &[u8]) -> Result<(Codec, Vec<u8>), String> {
    if data.is_empty() {
        return Err("There is nothing to decompress".into());
    }
    match detect(data) {
        // Two bytes make a weak signature: raw DEFLATE or Brotli can start
        // with a valid-looking zlib header, so a failure falls through.
        Some(Codec::Zlib) => {
            if let Ok(out) = decompress(data, Codec::Zlib) {
                return Ok((Codec::Zlib, out));
            }
        }
        Some(codec) => return decompress(data, codec).map(|out| (codec, out)),
        None => {}
    }
    if let Some(kind) = archive_kind(data) {
        return Err(format!("This is {kind}, which holds several files. Open it with an archive tool instead."));
    }
    if let Some(out) = deflate_whole(data) {
        return Ok((Codec::Deflate, out));
    }
    if let Ok(out) = decompress(data, Codec::Brotli) {
        return Ok((Codec::Brotli, out));
    }
    Err("This does not look like compressed data in any supported format".into())
}

/// Raw DEFLATE that decodes and uses up every input byte. Other data, Brotli
/// included, often decodes as a short DEFLATE stream with bytes left over.
fn deflate_whole(data: &[u8]) -> Option<Vec<u8>> {
    let mut dec = flate2::bufread::DeflateDecoder::new(data);
    let out = read_all(&mut dec).ok()??;
    (dec.total_in() == data.len() as u64).then_some(out)
}

/// Bytes typed or pasted as text: hexadecimal, or Base64 (standard or URL-safe).
pub fn parse_encoded(s: &str) -> Result<Vec<u8>, String> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let hex = clean.strip_prefix("0x").unwrap_or(&clean);
    if hex.len() % 2 == 0 && !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok((0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect());
    }
    let unpadded = clean.trim_end_matches('=');
    let engines = [base64::engine::general_purpose::STANDARD_NO_PAD, base64::engine::general_purpose::URL_SAFE_NO_PAD];
    engines
        .iter()
        .find_map(|e| e.decode(unpadded).ok())
        .ok_or_else(|| "Paste the compressed data as Base64 or hexadecimal".into())
}

pub fn to_base64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// The name for a file once compressed: `notes.txt` → `notes.txt.gz`.
pub fn compressed_name(name: &str, codec: Codec) -> String {
    format!("{name}.{}", codec.ext())
}

/// File extensions each format goes by, the usual one first.
fn exts(codec: Codec) -> &'static [&'static str] {
    match codec {
        Codec::Gzip => &["gz", "gzip"],
        Codec::Zlib => &["zz", "zlib"],
        Codec::Deflate => &["deflate"],
        Codec::Brotli => &["br"],
        Codec::Zstd => &["zst", "zstd"],
        Codec::Bzip2 => &["bz2", "bzip2"],
        Codec::Xz => &["xz"],
    }
}

/// Compressed tarballs' one-word extensions.
const TARBALLS: &[(&str, Codec)] = &[("tgz", Codec::Gzip), ("tbz2", Codec::Bzip2), ("txz", Codec::Xz), ("tzst", Codec::Zstd)];

/// The format a file name's extension names, if any.
pub fn from_name(name: &str) -> Option<Codec> {
    let (_, ext) = name.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    CODECS.iter().copied().find(|&c| exts(c).contains(&ext.as_str())).or_else(|| TARBALLS.iter().find(|(e, _)| *e == ext).map(|(_, c)| *c))
}

/// The name for a file once decompressed: `notes.txt.gz` → `notes.txt`,
/// `site.tgz` → `site.tar`, and anything else gets `.out` added.
pub fn decompressed_name(name: &str, codec: Codec) -> String {
    if let Some((stem, ext)) = name.rsplit_once('.').filter(|(stem, _)| !stem.is_empty()) {
        let ext = ext.to_ascii_lowercase();
        if exts(codec).contains(&ext.as_str()) {
            return stem.to_string();
        }
        if TARBALLS.contains(&(ext.as_str(), codec)) {
            return format!("{stem}.tar");
        }
    }
    format!("{name}.out")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        "The quick brown fox jumps over the lazy dog. ".repeat(200).into_bytes()
    }

    #[test]
    fn round_trips_every_format_and_level() {
        let data = sample();
        for &codec in CODECS {
            for &level in LEVELS {
                let packed = compress(&data, codec, level).unwrap();
                assert!(packed.len() < data.len() / 4, "{codec:?} {level:?} did not shrink repetitive text");
                assert_eq!(decompress(&packed, codec).unwrap(), data, "{codec:?} {level:?}");
                let (found, out) = decompress_auto(&packed).unwrap();
                assert_eq!(out, data);
                assert_eq!(found, codec, "{level:?}");
            }
        }
    }

    #[test]
    fn empty_input_round_trips() {
        for &codec in CODECS {
            let packed = compress(b"", codec, Level::Balanced).unwrap();
            assert_eq!(decompress(&packed, codec).unwrap(), b"", "{codec:?}");
        }
    }

    #[test]
    fn joined_gzip_members_decode_whole() {
        let mut joined = compress(b"hello ", Codec::Gzip, Level::Balanced).unwrap();
        joined.extend(compress(b"world", Codec::Gzip, Level::Balanced).unwrap());
        assert_eq!(decompress(&joined, Codec::Gzip).unwrap(), b"hello world");
    }

    #[test]
    fn rejects_bad_data() {
        assert!(decompress(b"\x1f\x8bnot really gzip", Codec::Gzip).unwrap_err().contains("not valid Gzip"));
        assert!(decompress_auto(b"").is_err());
        let zip = b"PK\x03\x04rest of a zip file";
        assert!(decompress_auto(zip).unwrap_err().contains("ZIP archive"));
    }

    #[test]
    fn parses_hex_and_base64() {
        let gz = compress(b"hi", Codec::Gzip, Level::Balanced).unwrap();
        let hex: String = gz.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(parse_encoded(&hex).unwrap(), gz);
        assert_eq!(parse_encoded(&to_base64(&gz)).unwrap(), gz);
        assert_eq!(parse_encoded("H4sI\nAAAA").unwrap(), parse_encoded("H4sIAAAA").unwrap());
        assert_eq!(parse_encoded("-_8").unwrap(), vec![0xfb, 0xff]);
        assert!(parse_encoded("not *base64*").is_err());
    }

    #[test]
    fn names_output_files() {
        assert_eq!(compressed_name("notes.txt", Codec::Zstd), "notes.txt.zst");
        assert_eq!(decompressed_name("notes.txt.GZ", Codec::Gzip), "notes.txt");
        assert_eq!(decompressed_name("site.tgz", Codec::Gzip), "site.tar");
        assert_eq!(decompressed_name("blob.bin", Codec::Brotli), "blob.bin.out");
        assert_eq!(decompressed_name(".gz", Codec::Gzip), ".gz.out");
        assert_eq!(from_name("logs.TAR.XZ"), Some(Codec::Xz));
        assert_eq!(from_name("site.tgz"), Some(Codec::Gzip));
        assert_eq!(from_name("notes.txt"), None);
        assert_eq!(from_name("README"), None);
    }
}
