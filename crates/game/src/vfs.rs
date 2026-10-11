//! DOS path -> host path mapping.
//!
//! Every DOS path is reduced to a path relative to the game directory (drive
//! letters and a leading `\CYBERMAG\` are stripped).  Lookups go to the
//! writable overlay directory first, then to the read-only original game dir.

use std::path::{Path, PathBuf};

pub struct Vfs {
    /// Original files (extracted CD `CYBERMAG` directory)
    pub game: PathBuf,
    /// Writable directory (config, saves)
    pub overlay: PathBuf,
}

impl Vfs {
    pub fn new(game: PathBuf, overlay: PathBuf) -> Vfs {
        std::fs::create_dir_all(&overlay).ok();
        std::fs::create_dir_all(overlay.join("SAVEGAME")).ok();
        Vfs { game, overlay }
    }

    /// Normalise a DOS path to an upper-case relative path with '/' separators.
    pub fn rel(&self, dos: &str) -> String {
        let mut s = dos.trim().replace('/', "\\").to_ascii_uppercase();
        if s.len() >= 2 && s.as_bytes()[1] == b':' {
            s = s[2..].to_string();
        }
        let mut parts: Vec<&str> = s.split('\\').filter(|p| !p.is_empty() && *p != ".").collect();
        if parts.first() == Some(&"CYBERMAG") {
            parts.remove(0);
        }
        let mut safe = Vec::new();
        for part in parts {
            if part == ".." { safe.pop(); } else { safe.push(part); }
        }
        safe.join("/")
    }

    /// Resolve for reading: overlay first, then original files.
    pub fn resolve_read(&self, dos: &str) -> Option<PathBuf> {
        let rel = self.rel(dos);
        let o = self.overlay.join(&rel);
        if o.exists() {
            return Some(o);
        }
        let g = find_case_insensitive(&self.game, &rel)?;
        Some(g)
    }

    /// Resolve for writing: always in the overlay.
    pub fn resolve_write(&self, dos: &str) -> PathBuf {
        let p = self.overlay.join(self.rel(dos));
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        p
    }

    /// Directory listing (for FindFirst): returns matching file names (upper case).
    pub fn list(&self, dos_pattern: &str) -> Vec<(String, u64, bool)> {
        let rel = self.rel(dos_pattern);
        let (dir, pat) = match rel.rfind('/') {
            Some(i) => (rel[..i].to_string(), rel[i + 1..].to_string()),
            None => (String::new(), rel.clone()),
        };
        let mut out: Vec<(String, u64, bool)> = Vec::new();
        for root in [&self.overlay, &self.game] {
            let d = root.join(&dir);
            if let Ok(rd) = std::fs::read_dir(&d) {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().to_ascii_uppercase();
                    if wildcard(&pat, &name) && !out.iter().any(|(n, _, _)| *n == name) {
                        let md = e.metadata().ok();
                        let isdir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                        out.push((name, md.map(|m| m.len()).unwrap_or(0), isdir));
                    }
                }
            }
        }
        out.sort();
        out
    }
}

fn find_case_insensitive(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        let direct = cur.join(part);
        if direct.exists() {
            cur = direct;
            continue;
        }
        let mut found = None;
        for e in std::fs::read_dir(&cur).ok()?.flatten() {
            if e.file_name().to_string_lossy().eq_ignore_ascii_case(part) {
                found = Some(e.path());
                break;
            }
        }
        cur = found?;
    }
    if cur.exists() {
        Some(cur)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guest_paths_never_escape_the_asset_or_save_root() {
        let root=std::env::temp_dir().join(format!("cybermage-vfs-{}",std::process::id()));
        let vfs=Vfs::new(root.join("game"),root.join("run"));
        assert_eq!(vfs.rel("C:\\CYBERMAG\\SAVEGAME\\..\\SAVE000.DAT"),"SAVE000.DAT");
        for path in ["..\\..\\outside.dat","C:\\CYBERMAG\\..\\..\\outside.dat","/../../outside.dat"] {
            assert_eq!(vfs.rel(path),"OUTSIDE.DAT");
            assert_eq!(vfs.resolve_write(path),root.join("run/OUTSIDE.DAT"));
            assert!(vfs.resolve_read(path).is_none());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// DOS-style wildcard match (`*`, `?`, with `*.*` matching everything).
pub fn wildcard(pat: &str, name: &str) -> bool {
    if pat == "*.*" || pat == "*" {
        return true;
    }
    fn m(p: &[u8], n: &[u8]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some(b'*'), _) => m(&p[1..], n) || (!n.is_empty() && m(p, &n[1..])),
            (Some(b'?'), Some(_)) => m(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a == b => m(&p[1..], &n[1..]),
            _ => false,
        }
    }
    m(pat.as_bytes(), name.as_bytes())
}
