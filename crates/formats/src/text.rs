//! Text in the binaries: a scanner for the game's strings, for a viewer (docs/FORMATS.md §5).
//!
//! The modules keep their text as `00h`-terminated strings with embedded control codes
//! (*observed* in `EL2M18`: `81h FDh 19h` before a message, `FFh 30h nn` where a value is
//! inserted). How the message tables index them is not traced yet, so this module only finds
//! runs of text; it does not claim which routine reads them.

/// A string found in a binary: its offset in the data scanned and its text, with control
/// bytes shown as `{xx}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub offset: usize,
    pub text: String,
}

fn printable(b: u8) -> bool {
    (0x20..0x7f).contains(&b)
}

/// Every run of at least `min` printable characters (control bytes allowed inside a run once it
/// has started, if printable text follows), ended by `00h`, `0Dh`, `$` or a non-text byte.
pub fn scan(data: &[u8], min: usize) -> Vec<Found> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if !printable(data[i]) {
            i += 1;
            continue;
        }
        let start = i;
        let mut text = String::new();
        let mut letters = 0;
        while i < data.len() {
            let b = data[i];
            if printable(b) {
                text.push(b as char);
                letters += 1;
                i += 1;
            } else if b != 0 && b != 0x0d && letters >= min && i + 1 < data.len() && printable(data[i + 1]) {
                // A control code inside a message (colour, inserted value).
                text.push_str(&format!("{{{b:02x}}}"));
                i += 1;
            } else {
                break;
            }
        }
        if letters >= min {
            out.push(Found { offset: start, text });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_terminated_runs_with_codes() {
        let d = b"\x81\xfd\x19Request clearance\xff0\x16 Control\0ab\0";
        let f = scan(d, 4);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].offset, 3);
        assert_eq!(f[0].text, "Request clearance{ff}0{16} Control");
    }
}
