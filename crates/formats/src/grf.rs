//! `.GRF` graphics files: `EL2G.GRF`, the game's packed image archive, and `PICnnn.GRF`, the
//! screenshots the game writes (docs/FORMATS.md §1–2).
//!
//! `EL2G.GRF` is read once at start-up by `EL2MCGA3:062a` through the main program's archive
//! loader `1000:e2ba` (slot `DS:0094`), which unpacks every entry into EMS and records a far
//! pointer per entry in the table at `DS:5813`. The blitter `EL2MCGA3:02d3` (slots `DS:0220`,
//! `DS:021c`) draws an entry from that table. `PIC000.GRF` is not read by the game: it is the
//! output of the screenshot routine `1000:db9f` (*observed* in code), shipped by accident.

use crate::{Error, Result};

/// Header flag: colour 0 is transparent (`EL2MCGA3:0476` skips it).
pub const FLAG_TRANSPARENT: u8 = 0x01;
/// Header flag: the pixels stay packed in memory and are unpacked at draw time
/// (`EL2MCGA3:03e3`). The archive loader then copies the payload as it is (`1000:e37c`).
pub const FLAG_PACKED: u8 = 0x02;

/// One entry of `EL2G.GRF` as the archive loader leaves it in EMS: an 8-byte header and its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Offset of the entry's length word in the file.
    pub offset: usize,
    /// The 8-byte header, or `None` for a raw entry (flag byte `FFh`, copied without a header).
    pub header: Option<ImageHeader>,
    /// The bytes after the header as stored in memory: pixels, or still packed if
    /// [`FLAG_PACKED`] is set.
    pub data: Vec<u8>,
}

/// The 8-byte image header the blitter `EL2MCGA3:02e9` copies to `CS:010e`–`0115`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageHeader {
    /// Byte 0: [`FLAG_TRANSPARENT`], [`FLAG_PACKED`].
    pub flags: u8,
    /// Byte 1: rows (`CS:010f`, the row count at `EL2MCGA3:031c`).
    pub height: u8,
    /// Bytes 2–3: bytes per row (`CS:0110`).
    pub width: u16,
    /// Bytes 4–5: added to the x position (`EL2MCGA3:0380`).
    pub dx: i16,
    /// Bytes 6–7: added to the y position (`EL2MCGA3:0317`).
    pub dy: i16,
}

impl ImageHeader {
    fn parse(b: &[u8]) -> ImageHeader {
        ImageHeader {
            flags: b[0],
            height: b[1],
            width: u16::from_le_bytes([b[2], b[3]]),
            dx: i16::from_le_bytes([b[4], b[5]]),
            dy: i16::from_le_bytes([b[6], b[7]]),
        }
    }
}

/// A decoded image: palette indices, `width × height`, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    /// Colour 0 is not drawn.
    pub transparent: bool,
}

impl Entry {
    /// The entry's pixels, unpacked if they are stored packed. `None` for a raw entry.
    pub fn image(&self) -> Option<Result<Image>> {
        let h = self.header?;
        let (w, rows) = (h.width as usize, h.height as usize);
        let pixels = if h.flags & FLAG_PACKED != 0 {
            match unpack(&self.data, w * rows) {
                Ok((p, _)) => p,
                Err(e) => return Some(Err(e)),
            }
        } else {
            if self.data.len() < w * rows {
                return Some(Err(Error::Truncated { needed: w * rows, available: self.data.len() }));
            }
            self.data[..w * rows].to_vec()
        };
        Some(Ok(Image { width: w, height: rows, pixels, transparent: h.flags & FLAG_TRANSPARENT != 0 }))
    }
}

/// Port of the back-reference coder shared by `1000:e38d` (archive loader) and `EL2MCGA3:040a`
/// (blitter): unpack `src` until `out_len` bytes are produced. Returns the bytes and how many
/// input bytes were used.
///
/// A control byte `c` below 80h copies the next `c` bytes. Otherwise it copies
/// `−(c SAR 3)` bytes (1–16) from `((c & 7) << 8 | next byte)` bytes back in the output, one
/// byte at a time so a copy may overlap itself (*observed*: `REP MOVSB` at `1000:e3c6`).
pub fn unpack(src: &[u8], out_len: usize) -> Result<(Vec<u8>, usize)> {
    let mut out = Vec::with_capacity(out_len);
    let mut p = 0;
    let byte = |p: usize| src.get(p).copied().ok_or(Error::Truncated { needed: p + 1, available: src.len() });
    while out.len() < out_len {
        let c = byte(p)?;
        p += 1;
        if c < 0x80 {
            let n = c as usize;
            let run = src.get(p..p + n).ok_or(Error::Truncated { needed: p + n, available: src.len() })?;
            out.extend_from_slice(run);
            p += n;
        } else {
            let n = (-((c as i8) >> 3)) as usize;
            let back = ((c as usize & 7) << 8) | byte(p)? as usize;
            p += 1;
            if back == 0 || back > out.len() {
                return Err(Error::Invalid("GRF back-reference before the start of the output"));
            }
            for _ in 0..n {
                out.push(out[out.len() - back]);
            }
        }
    }
    // The original stops when its counter reaches zero; a literal run may overshoot it.
    out.truncate(out_len);
    Ok((out, p))
}

/// Port of the archive loader `1000:e2ba` (with `1000:e349` and `1000:e378`): every entry of a
/// `EL2G.GRF`-style archive, in file order (the order of the far pointers at `DS:5813`).
///
/// Each entry is a length word `n` (0 ends the archive), then a flag byte. Flag `FFh`: `n` raw
/// bytes follow. Otherwise the flag and 7 more bytes form the 8-byte header, and `n − 8` bytes
/// of data follow: copied as they are if [`FLAG_PACKED`] is set, else unpacked by [`unpack`].
pub fn parse_archive(data: &[u8]) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut p = 0;
    let need = |n: usize| Error::Truncated { needed: n, available: data.len() };
    loop {
        let len = data.get(p..p + 2).ok_or(need(p + 2))?;
        let n = u16::from_le_bytes([len[0], len[1]]) as usize;
        let offset = p;
        p += 2;
        if n == 0 {
            break;
        }
        let flag = *data.get(p).ok_or(need(p + 1))?;
        if flag == 0xff {
            let raw = data.get(p + 1..p + 1 + n).ok_or(need(p + 1 + n))?;
            entries.push(Entry { offset, header: None, data: raw.to_vec() });
            p += 1 + n;
            continue;
        }
        if n < 8 {
            return Err(Error::Invalid("GRF entry shorter than its header"));
        }
        let hdr = data.get(p..p + 8).ok_or(need(p + 8))?;
        let header = ImageHeader::parse(hdr);
        p += 8;
        let body = n - 8;
        let out = if flag & FLAG_PACKED != 0 {
            let raw = data.get(p..p + body).ok_or(need(p + body))?;
            p += body;
            raw.to_vec()
        } else {
            let (out, used) = unpack(&data[p..], body)?;
            p += used;
            out
        };
        entries.push(Entry { offset, header: Some(header), data: out });
    }
    Ok(entries)
}

/// A `PICnnn.GRF` screenshot: the VGA palette and the 320×200 frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    /// 256 DAC entries of 6-bit red, green, blue, as read from port `3C9h` (`1000:ddf5`).
    pub palette: [[u8; 3]; 256],
    /// 64,000 palette indices, row by row, as read from `A000:0000` (`1000:dc0c`).
    pub pixels: Vec<u8>,
}

pub const SCREEN_WIDTH: usize = 320;
pub const SCREEN_HEIGHT: usize = 200;

/// Decode a screenshot written by `1000:db9f`: 768 palette bytes, then the frame run-length
/// coded by `1000:dcb1`/`1000:dc52`. A control byte `c` below 80h is followed by `c + 1`
/// literal bytes; otherwise the next byte repeats `257 − c` times (runs of 2–126, *observed*
/// from the `7Dh` limit at `1000:dd82`). This is the PackBits scheme.
pub fn parse_screenshot(data: &[u8]) -> Result<Screenshot> {
    let pal = data.get(..768).ok_or(Error::Truncated { needed: 768, available: data.len() })?;
    let mut palette = [[0u8; 3]; 256];
    for (i, c) in palette.iter_mut().enumerate() {
        c.copy_from_slice(&pal[i * 3..i * 3 + 3]);
    }
    let total = SCREEN_WIDTH * SCREEN_HEIGHT;
    let mut pixels = Vec::with_capacity(total);
    let mut p = 768;
    while pixels.len() < total {
        let c = *data.get(p).ok_or(Error::Truncated { needed: p + 1, available: data.len() })?;
        p += 1;
        if c < 0x80 {
            let n = c as usize + 1;
            let run = data.get(p..p + n).ok_or(Error::Truncated { needed: p + n, available: data.len() })?;
            pixels.extend_from_slice(run);
            p += n;
        } else {
            let v = *data.get(p).ok_or(Error::Truncated { needed: p + 1, available: data.len() })?;
            p += 1;
            pixels.extend(std::iter::repeat_n(v, 257 - c as usize));
        }
    }
    if pixels.len() != total {
        return Err(Error::Invalid("screenshot runs overshoot 64,000 pixels"));
    }
    Ok(Screenshot { palette, pixels })
}

/// A 6-bit DAC value as an 8-bit display value (`v × 255 / 63`).
pub fn dac_to_8bit(v: u8) -> u8 {
    ((v as u16 & 63) * 255 / 63) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpack_literals_and_overlapping_copy() {
        // 2 literals "ab", then copy 5 from 2 back: c = −5 << 3 = D8h, offset 2.
        let (out, used) = unpack(&[0x02, b'a', b'b', 0xd8, 0x02], 7).unwrap();
        assert_eq!(out, b"abababa");
        assert_eq!(used, 5);
    }

    #[test]
    fn archive_round_trip() {
        // One unpacked image 2×2 (header + LZ), one raw entry, terminator.
        let mut f = vec![0x0c, 0x00, 0x01, 0x02, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00];
        f.extend_from_slice(&[0x04, 1, 2, 3, 4]);
        f.extend_from_slice(&[0x02, 0x00, 0xff, 9, 8]);
        f.extend_from_slice(&[0, 0]);
        let e = parse_archive(&f).unwrap();
        assert_eq!(e.len(), 2);
        let img = e[0].image().unwrap().unwrap();
        assert_eq!((img.width, img.height, img.pixels.clone()), (2, 2, vec![1, 2, 3, 4]));
        assert!(img.transparent);
        assert_eq!(e[1].data, vec![9, 8]);
        assert!(e[1].header.is_none());
    }

    #[test]
    fn screenshot_runs() {
        let mut f = vec![0u8; 768];
        // 64,000 pixels: 500 runs of 128 (c = 257 − 128 = 81h would be 128 > 126, so use 125+3).
        for _ in 0..500 {
            f.extend_from_slice(&[0x84, 7, 0x02, 1, 2, 3]); // 125 × 7, then 3 literals
        }
        let s = parse_screenshot(&f).unwrap();
        assert_eq!(s.pixels.len(), 64000);
        assert_eq!(&s.pixels[124..128], &[7, 1, 2, 3]);
    }
}
