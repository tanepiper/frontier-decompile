//! DOS MZ executable header. `FRONTIER.EXE` and every `EL2*.OVL` overlay are MZ images; the
//! overlays are loaded by the main program's own overlay loader (not DOS `EXEC`).

use crate::{Error, Result};

/// The fixed 28-byte MZ header plus the relocation table it points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MzHeader {
    /// Bytes used in the last 512-byte page (0 = whole page).
    pub last_page_bytes: u16,
    /// Number of 512-byte pages in the file image, including the header.
    pub pages: u16,
    /// Header size in 16-byte paragraphs; the load image starts at `header_paragraphs * 16`.
    pub header_paragraphs: u16,
    pub min_alloc: u16,
    pub max_alloc: u16,
    pub initial_ss: u16,
    pub initial_sp: u16,
    pub initial_ip: u16,
    pub initial_cs: u16,
    pub overlay_number: u16,
    /// Segment:offset pairs (relative to the load segment) that DOS fixes up at load time.
    pub relocations: Vec<(u16, u16)>,
}

impl MzHeader {
    /// Size of the load image in bytes (file image minus header), as DOS computes it.
    pub fn image_size(&self) -> usize {
        let pages = self.pages as usize;
        let total = if self.last_page_bytes == 0 {
            pages * 512
        } else {
            pages.saturating_sub(1) * 512 + self.last_page_bytes as usize
        };
        total.saturating_sub(self.header_size())
    }

    /// Offset of the load image in the file.
    pub fn header_size(&self) -> usize {
        self.header_paragraphs as usize * 16
    }
}

fn u16_at(data: &[u8], at: usize) -> Result<u16> {
    data.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or(Error::Truncated { needed: at + 2, available: data.len() })
}

/// Parse an MZ header and its relocation table.
pub fn parse(data: &[u8]) -> Result<MzHeader> {
    if data.get(0..2) != Some(b"MZ") {
        return Err(Error::Invalid("missing MZ signature"));
    }
    let count = u16_at(data, 0x06)? as usize;
    let table = u16_at(data, 0x18)? as usize;
    let mut relocations = Vec::with_capacity(count);
    for i in 0..count {
        let at = table + i * 4;
        relocations.push((u16_at(data, at + 2)?, u16_at(data, at)?));
    }
    Ok(MzHeader {
        last_page_bytes: u16_at(data, 0x02)?,
        pages: u16_at(data, 0x04)?,
        header_paragraphs: u16_at(data, 0x08)?,
        min_alloc: u16_at(data, 0x0a)?,
        max_alloc: u16_at(data, 0x0c)?,
        initial_ss: u16_at(data, 0x0e)?,
        initial_sp: u16_at(data, 0x10)?,
        initial_ip: u16_at(data, 0x14)?,
        initial_cs: u16_at(data, 0x16)?,
        overlay_number: u16_at(data, 0x1a)?,
        relocations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_mz() {
        assert_eq!(parse(b"PK\x03\x04"), Err(Error::Invalid("missing MZ signature")));
    }

    #[test]
    fn image_size_handles_partial_last_page() {
        let mut h = [0u8; 0x1c];
        h[0..2].copy_from_slice(b"MZ");
        h[0x02..0x04].copy_from_slice(&0x10u16.to_le_bytes());
        h[0x04..0x06].copy_from_slice(&2u16.to_le_bytes());
        h[0x08..0x0a].copy_from_slice(&2u16.to_le_bytes());
        h[0x18..0x1a].copy_from_slice(&0x1cu16.to_le_bytes());
        let mz = parse(&h).unwrap();
        assert_eq!(mz.image_size(), 512 + 0x10 - 32);
    }
}
