//! The game directory as the DOS program sees it: a flat, case-insensitive set of files held in
//! memory. Native hosts fill it from `game-data/run`; the browser host fills it from files the
//! player selects. Writes (saved commanders) are tracked so the host can persist them.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub struct Vfs {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
    /// Files written or created since the host last called [`Vfs::take_dirty`].
    dirty: BTreeSet<String>,
}

/// Normalise a DOS path to the key used here: upper case, `\` separators, no drive letter, no
/// leading separator, `.`/`..` resolved.
pub fn normalize(path: &[u8], cwd: &str) -> String {
    let s = String::from_utf8_lossy(path).to_ascii_uppercase().replace('/', "\\");
    let s = if s.len() >= 2 && s.as_bytes()[1] == b':' { s[2..].to_string() } else { s };
    let full = if let Some(rest) = s.strip_prefix('\\') {
        rest.to_string()
    } else if cwd.is_empty() {
        s
    } else {
        format!("{cwd}\\{s}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for p in full.split('\\') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(p),
        }
    }
    parts.join("\\")
}

impl Vfs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: &str, data: Vec<u8>) {
        let key = normalize(name.as_bytes(), "");
        if let Some((dir, _)) = key.rsplit_once('\\') {
            self.dirs.insert(dir.to_string());
        }
        self.files.insert(key, data);
    }

    pub fn insert_dir(&mut self, name: &str) {
        self.dirs.insert(normalize(name.as_bytes(), ""));
    }

    pub fn get(&self, key: &str) -> Option<&Vec<u8>> {
        self.files.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Vec<u8>> {
        self.dirty.insert(key.to_string());
        self.files.get_mut(key)
    }

    pub fn create(&mut self, key: &str) {
        self.files.insert(key.to_string(), Vec::new());
        self.dirty.insert(key.to_string());
    }

    pub fn remove(&mut self, key: &str) -> bool {
        self.dirty.insert(key.to_string());
        self.files.remove(key).is_some()
    }

    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        match self.files.remove(from) {
            Some(d) => {
                self.files.insert(to.to_string(), d);
                self.dirty.insert(from.to_string());
                self.dirty.insert(to.to_string());
                true
            }
            None => false,
        }
    }

    pub fn is_dir(&self, key: &str) -> bool {
        key.is_empty() || self.dirs.contains(key)
    }

    /// Entries in directory `dir` (key form), as `(name, is_dir, size)`.
    pub fn list(&self, dir: &str) -> Vec<(String, bool, u32)> {
        let mut out = Vec::new();
        for d in &self.dirs {
            let (parent, name) = d.rsplit_once('\\').unwrap_or(("", d));
            if parent == dir {
                out.push((name.to_string(), true, 0));
            }
        }
        for (k, v) in &self.files {
            let (parent, name) = k.rsplit_once('\\').unwrap_or(("", k));
            if parent == dir {
                out.push((name.to_string(), false, v.len() as u32));
            }
        }
        out
    }

    /// Files changed since the last call, with their current contents (`None` = deleted).
    pub fn take_dirty(&mut self) -> Vec<(String, Option<Vec<u8>>)> {
        let names = std::mem::take(&mut self.dirty);
        names.into_iter().map(|n| { let d = self.files.get(&n).cloned(); (n, d) }).collect()
    }
}

/// DOS 8.3 wildcard match (`*` and `?`), both sides upper case.
pub fn wildcard(pattern: &str, name: &str) -> bool {
    fn split(s: &str) -> (String, String) {
        match s.rsplit_once('.') {
            Some((a, b)) => (a.to_string(), b.to_string()),
            None => (s.to_string(), String::new()),
        }
    }
    fn part(p: &str, n: &str, len: usize) -> bool {
        let pb: Vec<char> = p.chars().collect();
        let nb: Vec<char> = n.chars().collect();
        let mut i = 0;
        while i < len {
            let pc = pb.get(i).copied().unwrap_or(' ');
            if pc == '*' {
                return true;
            }
            let nc = nb.get(i).copied().unwrap_or(' ');
            if pc != '?' && pc != nc {
                return false;
            }
            i += 1;
        }
        true
    }
    if pattern == "*.*" || pattern == "*" {
        return true;
    }
    let (pn, pe) = split(pattern);
    let (nn, ne) = split(name);
    part(&pn, &nn, 8) && part(&pe, &ne, 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_paths() {
        assert_eq!(normalize(b"c:\\frontier\\el2m00.ovl", ""), "FRONTIER\\EL2M00.OVL");
        assert_eq!(normalize(b"el2m00.ovl", "COMMANDR"), "COMMANDR\\EL2M00.OVL");
        assert_eq!(normalize(b"..\\x", "COMMANDR"), "X");
    }

    #[test]
    fn matches_wildcards() {
        assert!(wildcard("*.*", "LAVE"));
        assert!(wildcard("*.OVL", "EL2M00.OVL"));
        assert!(!wildcard("*.OVL", "EL2G.GRF"));
        assert!(wildcard("EL2M??.OVL", "EL2M12.OVL"));
    }
}
