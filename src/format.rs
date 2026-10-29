//! The five image formats, and how a file's real format is recognised.

use std::fmt;

/// An image format imagoverto reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Jpeg,
    Png,
    WebP,
    Jxl,
    Avif,
}

impl Format {
    /// Every format, in the order they are listed to users.
    pub const ALL: [Format; 5] = [
        Format::WebP,
        Format::Avif,
        Format::Jxl,
        Format::Jpeg,
        Format::Png,
    ];

    /// Identify `bytes` by their signature, never by a file name. `None` for
    /// anything that is not one of the five (GIF, SVG, TIFF, …).
    pub fn sniff(bytes: &[u8]) -> Option<Format> {
        const JXL_CONTAINER: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n";
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Format::Jpeg)
        } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Format::Png)
        } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Some(Format::WebP)
        } else if bytes.starts_with(&[0xFF, 0x0A]) || bytes.starts_with(JXL_CONTAINER) {
            Some(Format::Jxl)
        } else if is_avif(bytes) {
            Some(Format::Avif)
        } else {
            None
        }
    }

    /// The value that names this format on the command line
    /// (`--image-format`) and in machine output.
    pub fn name(self) -> &'static str {
        match self {
            Format::Jpeg => "jpeg",
            Format::Png => "png",
            Format::WebP => "webp",
            Format::Jxl => "jxl",
            Format::Avif => "avif",
        }
    }

    /// The format whose [`name`](Format::name) is `name`.
    pub fn from_name(name: &str) -> Option<Format> {
        Format::ALL.into_iter().find(|f| f.name() == name)
    }

    /// The usual file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::Png => "png",
            Format::WebP => "webp",
            Format::Jxl => "jxl",
            Format::Avif => "avif",
        }
    }

    /// The registered media type.
    pub fn media_type(self) -> &'static str {
        match self {
            Format::Jpeg => "image/jpeg",
            Format::Png => "image/png",
            Format::WebP => "image/webp",
            Format::Jxl => "image/jxl",
            Format::Avif => "image/avif",
        }
    }

    /// Whether the format can store transparency.
    pub fn has_alpha(self) -> bool {
        !matches!(self, Format::Jpeg)
    }
}

impl fmt::Display for Format {
    /// The human label: `JPEG`, `PNG`, `WebP`, `JPEG XL`, `AVIF`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Format::Jpeg => "JPEG",
            Format::Png => "PNG",
            Format::WebP => "WebP",
            Format::Jxl => "JPEG XL",
            Format::Avif => "AVIF",
        })
    }
}

/// An ISO-BMFF `ftyp` box whose major or a compatible brand is `avif`/`avis`.
fn is_avif(bytes: &[u8]) -> bool {
    if bytes.len() < 16 || &bytes[4..8] != b"ftyp" {
        return false;
    }
    let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let end = size.clamp(16, bytes.len());
    // Major brand at 8..12, minor version at 12..16, compatible brands after.
    std::iter::once(&bytes[8..12])
        .chain(bytes[16..end].as_chunks::<4>().0.iter().map(|c| &c[..]))
        .any(|b| b == b"avif" || b == b"avis")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_each_signature() {
        assert_eq!(Format::sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(Format::Jpeg));
        assert_eq!(Format::sniff(b"\x89PNG\r\n\x1a\n...."), Some(Format::Png));
        assert_eq!(Format::sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(Format::WebP));
        assert_eq!(Format::sniff(&[0xFF, 0x0A, 0x00]), Some(Format::Jxl));
        assert_eq!(
            Format::sniff(b"\0\0\0\x0cJXL \r\n\x87\n\0\0"),
            Some(Format::Jxl)
        );
        let mut avif = b"\0\0\0\x1cftypmif1\0\0\0\0mif1avifmiaf".to_vec();
        avif.extend_from_slice(&[0; 8]);
        assert_eq!(Format::sniff(&avif), Some(Format::Avif));
    }

    #[test]
    fn refuses_what_it_does_not_read() {
        assert_eq!(Format::sniff(b"GIF89a"), None);
        assert_eq!(Format::sniff(b"II*\0"), None); // TIFF
        assert_eq!(Format::sniff(b"<svg"), None);
        assert_eq!(Format::sniff(b""), None);
        // An ISO-BMFF file that is not AVIF (HEIC).
        assert_eq!(Format::sniff(b"\0\0\0\x18ftypheic\0\0\0\0mif1heic"), None);
        // RIFF that is not WebP.
        assert_eq!(Format::sniff(b"RIFF\0\0\0\0WAVEfmt "), None);
    }

    #[test]
    fn names_round_trip() {
        for f in Format::ALL {
            assert_eq!(Format::from_name(f.name()), Some(f));
        }
        assert_eq!(Format::from_name("jpg"), None);
        assert_eq!(Format::from_name("best"), None);
    }
}
