//! Minimal PNG writer — 8-bit RGB, stored (uncompressed) deflate blocks.
//! ~100 lines instead of an image dependency; spectrograms land in ignored
//! output directories, so file size does not matter.

use std::io::Write;
use std::path::Path;

use super::AnalysisError;

/// Write `pixels` (RGB, row-major, `width*height*3` bytes) as a PNG.
pub fn write_rgb(
    path: impl AsRef<Path>,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<(), AnalysisError> {
    if width == 0 || height == 0 {
        return Err(AnalysisError::BadInput("empty image".into()));
    }
    if pixels.len() != width as usize * height as usize * 3 {
        return Err(AnalysisError::BadInput(format!(
            "pixel buffer is {} bytes, expected {}",
            pixels.len(),
            width as usize * height as usize * 3
        )));
    }

    // Raw scanline stream: each row prefixed by filter byte 0 (None).
    let stride = width as usize * 3;
    let mut raw = Vec::with_capacity(pixels.len() + height as usize);
    for row in pixels.chunks(stride) {
        raw.push(0u8);
        raw.extend_from_slice(row);
    }

    // zlib stream: header, stored deflate blocks (<= 65535 bytes), Adler-32.
    let mut idat = vec![0x78u8, 0x01];
    let mut chunks = raw.chunks(65_535).peekable();
    while let Some(block) = chunks.next() {
        let last = chunks.peek().is_none();
        idat.push(u8::from(last));
        let len = block.len() as u16;
        idat.extend_from_slice(&len.to_le_bytes());
        idat.extend_from_slice(&(!len).to_le_bytes());
        idat.extend_from_slice(block);
    }
    idat.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    // bit depth 8, colour type 2 (truecolour), compression/filter/interlace 0.
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);

    let mut out = Vec::with_capacity(idat.len() + 128);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    push_chunk(&mut out, b"IHDR", &ihdr);
    push_chunk(&mut out, b"IDAT", &idat);
    push_chunk(&mut out, b"IEND", &[]);

    let mut file = std::fs::File::create(path).map_err(|e| AnalysisError::Io(e.to_string()))?;
    file.write_all(&out)
        .map_err(|e| AnalysisError::Io(e.to_string()))
}

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

/// CRC-32 (IEEE, reflected) with a locally built table — used per chunk.
pub(crate) struct Crc32 {
    table: [u32; 256],
    value: u32,
}

impl Crc32 {
    pub(crate) fn new() -> Self {
        let mut table = [0u32; 256];
        for (i, entry) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *entry = c;
        }
        Self {
            table,
            value: 0xffff_ffff,
        }
    }

    pub(crate) fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.value = self.table[((self.value ^ b as u32) & 0xff) as usize] ^ (self.value >> 8);
        }
    }

    pub(crate) fn finish(&self) -> u32 {
        self.value ^ 0xffff_ffff
    }
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5_552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_value() {
        // CRC-32 of "123456789" is the classic check value 0xcbf43926.
        let mut crc = Crc32::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0xcbf4_3926);
    }

    #[test]
    fn adler32_known_value() {
        // Adler-32 of "Wikipedia" (RFC 1950 example value).
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn written_png_is_well_formed() {
        let path = std::env::temp_dir().join("telenn_analysis_png_test.png");
        let (w, h) = (70u32, 3u32);
        let pixels: Vec<u8> = (0..w * h * 3).map(|i| (i % 251) as u8).collect();
        write_rgb(&path, w, h, &pixels).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            &bytes[..8],
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
        );

        // Walk the chunks: lengths must line up, CRCs must re-validate.
        let mut pos = 8usize;
        let mut kinds = Vec::new();
        while pos < bytes.len() {
            let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
            let kind = &bytes[pos + 4..pos + 8];
            let data = &bytes[pos + 8..pos + 8 + len];
            let stored =
                u32::from_be_bytes(bytes[pos + 8 + len..pos + 12 + len].try_into().unwrap());
            let mut crc = Crc32::new();
            crc.update(kind);
            crc.update(data);
            assert_eq!(
                crc.finish(),
                stored,
                "chunk {} CRC",
                String::from_utf8_lossy(kind)
            );
            kinds.push(kind.to_vec());
            pos += 12 + len;
        }
        assert_eq!(kinds[0], b"IHDR");
        assert_eq!(*kinds.last().unwrap(), b"IEND");

        // IHDR dimensions round-trip.
        assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), w);
        assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), h);
        std::fs::remove_file(&path).ok();
    }
}
