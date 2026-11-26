//! Confined file access for automation (AGENTS.md rule 18).
//!
//! An agent entry point (`bayleaf-cli mcp`, the editor's control channel) names the folders it may
//! read and the folders it may write. Every command that touches a file asks the session's
//! [`Confinement`], which refuses anything outside, after resolving `..` and symbolic links.

use std::path::{Component, Path, PathBuf};

/// Whether a path is read or written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

impl Access {
    fn word(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
        }
    }
}

/// The folders automation may read and write. A write root may also be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confinement {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
}

impl Confinement {
    /// Both lists must name at least one existing folder.
    pub fn new(read: &[PathBuf], write: &[PathBuf]) -> Result<Self, String> {
        if read.is_empty() || write.is_empty() {
            return Err("automation needs at least one --automation-read-root and one --automation-write-root".to_owned());
        }
        let canonical = |p: &PathBuf| -> Result<PathBuf, String> {
            let c = p.canonicalize().map_err(|e| format!("automation root {}: {e}", p.display()))?;
            if c.is_dir() { Ok(c) } else { Err(format!("automation root {} is not a folder", p.display())) }
        };
        Ok(Self { read: read.iter().map(canonical).collect::<Result<_, _>>()?, write: write.iter().map(canonical).collect::<Result<_, _>>()? })
    }

    /// Relative paths resolve against the first write root.
    pub fn base(&self) -> &Path {
        self.write.first().map_or(Path::new("."), PathBuf::as_path)
    }

    /// The absolute, resolved path if `access` to it is allowed; otherwise a message that names the
    /// path, the access and the roots.
    pub fn check(&self, path: &Path, access: Access) -> Result<PathBuf, String> {
        let resolved = resolve(path, self.base());
        let roots: Vec<&PathBuf> = match access {
            Access::Read => self.read.iter().chain(&self.write).collect(),
            Access::Write => self.write.iter().collect(),
        };
        if roots.iter().any(|root| resolved.starts_with(root)) {
            Ok(plain(&resolved))
        } else {
            let list: Vec<String> = roots.iter().map(|r| display(r)).collect();
            Err(format!("automation may not {} {}: it is outside the allowed folders ({})", access.word(), display(&resolved), list.join(", ")))
        }
    }

    pub fn read_roots(&self) -> &[PathBuf] {
        &self.read
    }

    pub fn write_roots(&self) -> &[PathBuf] {
        &self.write
    }
}

/// Makes `path` absolute against `base`, removes `.` and `..` without touching the disk, then
/// resolves symbolic links in the longest part that exists.
fn resolve(path: &Path, base: &Path) -> PathBuf {
    let joined = if path.is_absolute() { path.to_path_buf() } else { base.join(path) };
    let mut lexical = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                lexical.pop();
            }
            Component::CurDir => {}
            other => lexical.push(other),
        }
    }
    // Canonicalize the deepest existing ancestor so a link inside a root cannot point outside it.
    let mut existing = lexical.clone();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (existing.file_name().map(ToOwned::to_owned), existing.parent().map(Path::to_path_buf)) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent;
            }
            _ => return lexical,
        }
    }
    let mut out = existing.canonicalize().unwrap_or(existing);
    for name in rest.into_iter().rev() {
        out.push(name);
    }
    out
}

/// `path` without the `\\?\` prefix Windows adds to canonical drive paths, so results and window
/// titles show the path people typed. Other verbatim forms (network shares) are kept.
fn plain(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

/// A path for messages.
fn display(path: &Path) -> String {
    plain(path).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::{Access, Confinement};
    use std::path::{Path, PathBuf};

    fn roots(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("bayleaf-confine-{name}-{}", std::process::id()));
        let read = base.join("read");
        let write = base.join("write");
        std::fs::create_dir_all(&read).unwrap();
        std::fs::create_dir_all(write.join("sub")).unwrap();
        (base, read, write)
    }

    #[test]
    fn allows_inside_and_refuses_outside_with_a_reason() {
        let (base, read, write) = roots("inside");
        let c = Confinement::new(std::slice::from_ref(&read), std::slice::from_ref(&write)).unwrap();
        assert!(c.check(&read.join("a.bscene"), Access::Read).is_ok());
        assert!(c.check(&write.join("sub").join("new.png"), Access::Write).is_ok());
        assert!(c.check(&write.join("x.bscene"), Access::Read).is_ok(), "a write root can be read");
        let refused = c.check(&read.join("a.bscene"), Access::Write).unwrap_err();
        assert!(refused.contains("may not write") && refused.contains("outside"), "{refused}");
        assert!(c.check(&base.join("elsewhere.txt"), Access::Read).is_err());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn dot_dot_cannot_climb_out_and_relative_paths_use_the_write_root() {
        let (base, read, write) = roots("dotdot");
        let c = Confinement::new(&[read], std::slice::from_ref(&write)).unwrap();
        assert!(c.check(Path::new("../../escape.txt"), Access::Write).is_err());
        assert!(c.check(&write.join("sub").join("..").join("..").join("escape.txt"), Access::Write).is_err());
        let resolved = c.check(Path::new("sub/out.png"), Access::Write).unwrap();
        assert!(resolved.ends_with(Path::new("sub").join("out.png")));
        assert!(!resolved.to_string_lossy().starts_with(r"\\?\"), "results keep the plain form: {}", resolved.display());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn both_lists_are_required_and_must_be_folders() {
        let (base, read, write) = roots("required");
        assert!(Confinement::new(&[], std::slice::from_ref(&write)).is_err());
        assert!(Confinement::new(std::slice::from_ref(&read), &[]).is_err());
        assert!(Confinement::new(&[read], &[base.join("missing")]).is_err());
        std::fs::remove_dir_all(&base).ok();
    }
}
