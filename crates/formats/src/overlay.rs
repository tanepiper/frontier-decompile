//! The `EL2*.OVL` overlays: the common TSR stub, the export block of the drivers and the module
//! header of `EL2M00`–`EL2M19` (docs/OVERLAYS.md §1 and §4). Offsets are in the overlay's load
//! image (the bytes after the MZ header).

use crate::{mz, Error, Result};

/// Load-image offset of the export block (`EL2xxx:005e`).
pub const EXPORT_BLOCK: usize = 0x5e;
/// Load-image offset of the module header H (`ovlseg:006a`), as the main program stores it.
pub const MODULE_HEADER: usize = 0x6a;

/// A far pointer as stored in the file: segment relative to the overlay's load segment
/// (patched by a relocation at load), and offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FarPtr {
    pub seg: u16,
    pub off: u16,
}

impl std::fmt::Display for FarPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04x}:{:04x}", self.seg, self.off)
    }
}

/// What follows the stub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    /// A driver (`EL2MCGA3`, `EL2MATH3`, a sound driver, `EL2SETUP`): far pointers copied by
    /// `load_overlay` (`1000:d34c`) to consecutive main vector slots from `first_slot`.
    Driver { first_slot: u16, exports: Vec<FarPtr> },
    /// A demand-loaded module (`EL2Mnn`): the header read by `load_module` (`1000:d6be`).
    Module(ModuleHeader),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleHeader {
    /// H−04h: far pointer to the `FFh`-terminated byte-pair table (purpose unknown).
    pub byte_pairs: FarPtr,
    /// The pairs themselves, when the pointer stays inside the image.
    pub pairs: Vec<(u8, u8)>,
    /// H+00h: dword copied to `module_word_nn` on first load.
    pub word0: u32,
    /// H+04h…H+30h: the 12 handlers, `+04` first. Unused ones point at a `RETF`.
    pub handlers: [FarPtr; 12],
    /// H+34h: dword copied to `DS:05d6 + slot·4`.
    pub word34: u32,
}

fn le16(d: &[u8], at: usize) -> Result<u16> {
    d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or(Error::Truncated { needed: at + 2, available: d.len() })
}

fn far(d: &[u8], at: usize) -> Result<FarPtr> {
    Ok(FarPtr { off: le16(d, at)?, seg: le16(d, at + 2)? })
}

/// The overlay's load image (the file after its MZ header).
pub fn image(file: &[u8]) -> Result<&[u8]> {
    let h = mz::parse(file)?;
    let start = h.header_size();
    let end = (start + h.image_size()).min(file.len());
    file.get(start..end).ok_or(Error::Truncated { needed: end, available: file.len() })
}

/// Read what follows the stub. `module` picks the module header (the `EL2Mnn` files); the
/// export list of a module is empty (*observed*: first slot 0, terminator at `0062`).
pub fn parse(file: &[u8], module: bool) -> Result<Overlay> {
    let img = image(file)?;
    if module {
        let h = MODULE_HEADER;
        let byte_pairs = far(img, h - 4)?;
        let mut pairs = Vec::new();
        if byte_pairs.seg == 0 {
            let mut p = byte_pairs.off as usize;
            while let Some(&a) = img.get(p) {
                if a == 0xff || pairs.len() >= 256 {
                    break;
                }
                let Some(&b) = img.get(p + 1) else { break };
                pairs.push((a, b));
                p += 2;
            }
        }
        let mut handlers = [FarPtr { seg: 0, off: 0 }; 12];
        for (i, f) in handlers.iter_mut().enumerate() {
            *f = far(img, h + 4 + i * 4)?;
        }
        let word0 = le16(img, h)? as u32 | (le16(img, h + 2)? as u32) << 16;
        let word34 = le16(img, h + 0x34)? as u32 | (le16(img, h + 0x36)? as u32) << 16;
        return Ok(Overlay::Module(ModuleHeader { byte_pairs, pairs, word0, handlers, word34 }));
    }
    let first_slot = le16(img, EXPORT_BLOCK + 2)?;
    let mut exports = Vec::new();
    let mut p = EXPORT_BLOCK + 4;
    loop {
        let f = far(img, p)?;
        if f.seg == 0 && f.off == 0 {
            break;
        }
        exports.push(f);
        p += 4;
        if exports.len() > 128 {
            return Err(Error::Invalid("export block without a terminator"));
        }
    }
    Ok(Overlay::Driver { first_slot, exports })
}

/// Which part of the main vector table a slot belongs to (docs/OVERLAYS.md §2).
pub fn slot_owner(slot: u16) -> &'static str {
    match slot {
        0x0000..=0x01cf => "main program",
        0x01d0..=0x026f => "EL2MCGA3 video",
        0x0270..=0x032b => "EL2MATH3 maths",
        0x032c..=0x0357 => "sound driver",
        0x0358..=0x035b => "EL2SETUP",
        _ => "?",
    }
}
