//! What each shipped file is, from its name and first bytes (docs/FINDINGS.md §1,
//! docs/FORMATS.md).

/// The kinds of file the release ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// `FRONTIER.EXE`: the main program.
    Program,
    /// A driver overlay (`EL2MCGA3`, `EL2MATH3`, `EL2ADLIB`, `EL2ROL`, `EL2SPK`, `EL2SETUP`).
    Driver,
    /// A demand-loaded module `EL2M00`–`EL2M19`.
    Module,
    /// `EL2G.GRF`: the image archive.
    ImageArchive,
    /// `PICnnn.GRF`: a screenshot the game wrote.
    Screenshot,
    /// A Standard MIDI File (`MThd`), the `.ROL` songs.
    Midi,
    /// `LAVE`, `MARS`, `MOON`, `VTOL`, `BATTLE`, `LATIS._0n`: packed start positions or saved
    /// games (*inferred*: all share the first bytes `11 00 C6`; the reader is not traced).
    Position,
    Unknown,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Program => "main program",
            Kind::Driver => "driver overlay",
            Kind::Module => "module overlay",
            Kind::ImageArchive => "image archive",
            Kind::Screenshot => "screenshot",
            Kind::Midi => "MIDI song",
            Kind::Position => "position / save (packed)",
            Kind::Unknown => "unknown",
        }
    }
}

pub fn identify(name: &str, data: &[u8]) -> Kind {
    let upper = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_uppercase();
    if data.starts_with(b"MThd") {
        return Kind::Midi;
    }
    if data.starts_with(b"MZ") {
        if upper.ends_with(".EXE") {
            return Kind::Program;
        }
        let is_module = upper.len() == 10 && upper.starts_with("EL2M") && upper[4..6].bytes().all(|b| b.is_ascii_digit());
        return if is_module { Kind::Module } else { Kind::Driver };
    }
    if upper == "EL2G.GRF" {
        return Kind::ImageArchive;
    }
    if upper.starts_with("PIC") && upper.ends_with(".GRF") && data.len() > 768 {
        return Kind::Screenshot;
    }
    if data.starts_with(&[0x11, 0x00, 0xc6]) {
        return Kind::Position;
    }
    Kind::Unknown
}
