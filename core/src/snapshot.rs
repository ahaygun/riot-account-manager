//! A snapshot is the content of every session path (files and whole
//! directories) at one moment, packed into a single blob.
//!
//! Riot Client's `lockfile` lives inside one of those directories. It
//! describes the running process, not the account, so it is never stored.

use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    /// (path relative to the `Riot Games` folder, with `/`; file content)
    pub files: Vec<(String, Vec<u8>)>,
}

impl Snapshot {
    /// Reads every session path under `root`. Missing paths are simply absent.
    /// Any read error aborts, so we never store a half snapshot.
    pub fn collect(root: &Path, session_paths: &[String]) -> io::Result<Snapshot> {
        let mut files = Vec::new();
        for rel in session_paths {
            let rel = normalize(rel)?;
            let abs = root.join(&rel);
            match fs::metadata(&abs) {
                Ok(m) if m.is_dir() => walk(&abs, &rel, &mut files)?,
                Ok(_) => files.push((rel, fs::read(&abs)?)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        files.sort();
        Ok(Snapshot { files })
    }

    /// Replaces every session path under `root` with this snapshot's content.
    ///
    /// Files outside the session paths are skipped, not written: snapshots
    /// made by older versions also hold Riot Client's settings folder, which
    /// is no longer treated as part of a session.
    pub fn restore(&self, root: &Path, session_paths: &[String]) -> io::Result<()> {
        let allowed: Vec<String> = session_paths.iter().map(|p| normalize(p)).collect::<Result<_, _>>()?;
        // Validate first, so a bad snapshot changes nothing.
        let mut keep = Vec::new();
        for (rel, data) in &self.files {
            let rel = normalize(rel)?;
            if allowed.iter().any(|a| rel == *a || rel.starts_with(&format!("{a}/"))) {
                keep.push((rel, data));
            }
        }
        clear(root, session_paths)?;
        for (rel, data) in keep {
            let abs = root.join(rel);
            if let Some(parent) = abs.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&abs, data)?;
        }
        Ok(())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend((self.files.len() as u32).to_le_bytes());
        for (path, data) in &self.files {
            out.extend((path.len() as u32).to_le_bytes());
            out.extend(path.as_bytes());
            out.extend((data.len() as u64).to_le_bytes());
            out.extend(data);
        }
        out
    }

    pub fn decode(mut bytes: &[u8]) -> io::Result<Snapshot> {
        let bad = || io::Error::new(io::ErrorKind::InvalidData, "corrupt snapshot");
        let mut take = |n: usize| -> io::Result<Vec<u8>> {
            if bytes.len() < n {
                return Err(bad());
            }
            let mut buf = vec![0u8; n];
            bytes.read_exact(&mut buf)?;
            Ok(buf)
        };
        let count = u32::from_le_bytes(take(4)?.try_into().unwrap());
        let mut files = Vec::new();
        for _ in 0..count {
            let plen = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
            let path = String::from_utf8(take(plen)?).map_err(|_| bad())?;
            let dlen = u64::from_le_bytes(take(8)?.try_into().unwrap());
            let dlen = usize::try_from(dlen).map_err(|_| bad())?;
            files.push((path, take(dlen)?));
        }
        Ok(Snapshot { files })
    }

    /// Stable content hash (FNV-1a), used only to notice changes.
    pub fn hash(&self) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in self.encode() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("{h:016x}")
    }
}

/// Deletes every session path under `root`.
pub fn clear(root: &Path, session_paths: &[String]) -> io::Result<()> {
    for rel in session_paths {
        let abs = root.join(normalize(rel)?);
        let res = match fs::symlink_metadata(&abs) {
            Ok(m) if m.is_dir() => fs::remove_dir_all(&abs),
            Ok(_) => fs::remove_file(&abs),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
        res?;
    }
    Ok(())
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<(String, Vec<u8>)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let child_rel = format!("{rel}/{name}");
        let ty = entry.file_type()?;
        if ty.is_dir() {
            walk(&entry.path(), &child_rel, out)?;
        } else if ty.is_file() && !name.eq_ignore_ascii_case("lockfile") {
            out.push((child_rel, fs::read(entry.path())?));
        }
    }
    Ok(())
}

/// Accepts only plain relative paths (no `..`, no drive, no root) and
/// returns them with `/` separators.
fn normalize(rel: &str) -> io::Result<String> {
    let unified = rel.replace('\\', "/");
    let path = PathBuf::from(&unified);
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => parts.push(p.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("session path must be relative: {rel}"),
                ))
            }
        }
    }
    if parts.is_empty() || unified.contains(':') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("bad session path: {rel}")));
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_roundtrip() {
        let s = Snapshot { files: vec![("a/b.yaml".into(), b"hello".to_vec()), ("c".into(), vec![])] };
        assert_eq!(Snapshot::decode(&s.encode()).unwrap(), s);
        assert!(Snapshot::decode(&s.encode()[..7]).is_err());
    }

    #[test]
    fn restore_skips_files_outside_the_session_paths() {
        let root = std::env::temp_dir().join(format!("qsw-snap-skip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Config")).unwrap();
        fs::write(root.join("Config/settings.yaml"), "muted: true").unwrap();
        let old = Snapshot { files: vec![("Data/s.yaml".into(), b"token".to_vec()), ("Config/settings.yaml".into(), b"muted: false".to_vec())] };
        old.restore(&root, &["Data/s.yaml".to_string()]).unwrap();
        assert_eq!(fs::read_to_string(root.join("Data/s.yaml")).unwrap(), "token");
        assert_eq!(fs::read_to_string(root.join("Config/settings.yaml")).unwrap(), "muted: true", "user settings untouched");
        // A path that escapes is still refused outright.
        let bad = Snapshot { files: vec![("../x".into(), vec![])] };
        assert!(bad.restore(&root, &["Data/s.yaml".to_string()]).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_escaping_paths() {
        assert!(normalize("../x").is_err());
        assert!(normalize("/etc/passwd").is_err());
        assert!(normalize("C:\\x").is_err());
        assert_eq!(normalize("Data\\Cookies").unwrap(), "Data/Cookies");
    }
}
