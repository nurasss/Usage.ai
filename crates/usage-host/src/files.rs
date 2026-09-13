use super::{CancellationToken, HostError};
use std::path::{Path, PathBuf};

/// Stable file identity for replacement/rotation detection.
/// `size shrink` alone is not sufficient: device/inode changes
/// (where available) prove replacement even when size grows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
    pub mtime_ms: i64,
    pub path_hash: String,
}

impl FileIdentity {
    pub fn replaced(&self, other: &FileIdentity) -> bool {
        // Same path hash is guaranteed by the caller (checkpoint key).
        // Replacement is proven by identity change, or by shrink/truncate
        // when the platform exposes no stable device/inode.
        if self.device != 0 || self.inode != 0 || other.device != 0 || other.inode != 0 {
            (self.device, self.inode) != (other.device, other.inode)
        } else {
            other.size < self.size
        }
    }
}

/// A path proven to stay inside its declared product root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedPath {
    pub path: PathBuf,
    pub identity: FileIdentity,
}

/// A product root validated by the host layer. Providers can only
/// obtain one through `scope_root`; they cannot construct it and
/// therefore cannot escape the declared product scope.
#[derive(Debug, Clone)]
pub struct ScopedRoot {
    root: PathBuf,
}

/// A single file proven to live inside its `ScopedRoot`. The raw path
/// stays inside the host boundary: providers see the stable identity
/// (including the path hash) but never the path itself.
#[derive(Debug, Clone)]
pub struct ScopedFile {
    path: PathBuf,
    root: PathBuf,
    identity: FileIdentity,
}

impl ScopedFile {
    pub fn identity(&self) -> &FileIdentity {
        &self.identity
    }

    pub fn path_hash(&self) -> &str {
        &self.identity.path_hash
    }
}

/// Scoped local-file access for provider strategies.
///
/// - every access declares its product root up front;
/// - roots are canonicalized once; discovered paths must canonicalize
///   strictly inside the root (symlink escape rejected);
/// - when `follow_symlinks` is false, any symlink on the path
///   (file or parent component) is rejected;
/// - raw paths never leave the host boundary except as hashes.
pub trait FilesHost: Send + Sync {
    /// List files matching a relative glob inside a scoped root.
    /// `..` patterns are rejected; escaping entries are skipped.
    fn list_files(
        &self,
        root: &ScopedRoot,
        pattern: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<ScopedFile>, HostError> {
        if pattern.contains("..") {
            return Err(HostError::Policy);
        }
        if cancel.is_cancelled() {
            return Err(HostError::Cancelled);
        }
        let mut out = vec![];
        for sub_pattern in pattern.split(';') {
            let sub_pattern = sub_pattern.trim();
            if sub_pattern.is_empty() {
                continue;
            }
            let full_pattern = format!("{}/{sub_pattern}", root.root.display());
            let entries = glob::glob(&full_pattern).map_err(|_| HostError::Policy)?;
            for entry in entries {
                if cancel.is_cancelled() {
                    return Err(HostError::Cancelled);
                }
                let Ok(path) = entry else { continue };
                let Ok(canonical) = path.canonicalize() else {
                    continue;
                };
                if !canonical.starts_with(&root.root) || !canonical.is_file() {
                    continue;
                }
                let Ok(identity) = file_identity(&canonical) else {
                    continue;
                };
                if !out.iter().any(|f: &ScopedFile| f.path == canonical) {
                    out.push(ScopedFile {
                        path: canonical,
                        root: root.root.clone(),
                        identity,
                    });
                }
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    fn scoped_identity(&self, file: &ScopedFile) -> FileIdentity {
        file.identity.clone()
    }

    fn read_scoped_range(
        &self,
        file: &ScopedFile,
        offset: u64,
        max_bytes: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, HostError> {
        if cancel.is_cancelled() {
            return Err(HostError::Cancelled);
        }
        use std::io::{Read, Seek, SeekFrom};
        let canonical = file.path.canonicalize().map_err(|_| HostError::NotFound)?;
        if canonical != file.path || !canonical.starts_with(&file.root) || !canonical.is_file() {
            return Err(HostError::Policy);
        }
        let mut handle = std::fs::File::open(&canonical).map_err(|_| HostError::NotFound)?;
        let metadata = handle.metadata().map_err(|_| HostError::Unavailable)?;
        let opened_identity = file_identity_from_metadata(&canonical, &metadata);
        if (file.identity.device != 0 || file.identity.inode != 0)
            && (file.identity.device, file.identity.inode)
                != (opened_identity.device, opened_identity.inode)
        {
            return Err(HostError::Policy);
        }
        let size = opened_identity.size;
        let start = offset.min(size);
        handle
            .seek(SeekFrom::Start(start))
            .map_err(|_| HostError::Unavailable)?;
        let mut buf = vec![0u8; max_bytes.min(size.saturating_sub(start) as usize)];
        let mut read = 0usize;
        while read < buf.len() {
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled);
            }
            match handle.read(&mut buf[read..]) {
                Ok(0) => break,
                Ok(n) => read += n,
                Err(_) => return Err(HostError::Unavailable),
            }
        }
        buf.truncate(read);
        Ok(buf)
    }

    fn read_scoped_tail(
        &self,
        file: &ScopedFile,
        max_bytes: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, HostError> {
        let start = file.identity.size.saturating_sub(max_bytes as u64);
        self.read_scoped_range(file, start, max_bytes, cancel)
    }

    /// Read one named file under a scoped root (e.g. `auth.json`).
    /// The relative path must be a single normal segment: no `..`,
    /// no absolute paths, no separators. The resolved file must
    /// canonicalize strictly inside the root.
    fn read_scoped_file(
        &self,
        root: &ScopedRoot,
        name: &str,
        max_bytes: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, HostError> {
        if cancel.is_cancelled() {
            return Err(HostError::Cancelled);
        }
        if name.is_empty()
            || name.contains("..")
            || name.contains('/')
            || name.contains('\\')
            || std::path::Path::new(name).is_absolute()
        {
            return Err(HostError::Policy);
        }
        let candidate = root.root.join(name);
        let canonical = candidate.canonicalize().map_err(|_| HostError::NotFound)?;
        if !canonical.starts_with(&root.root) || !canonical.is_file() {
            return Err(HostError::Policy);
        }
        let identity = file_identity(&canonical)?;
        self.read_scoped_range(
            &ScopedFile {
                path: canonical,
                root: root.root.clone(),
                identity,
            },
            0,
            max_bytes,
            cancel,
        )
    }
}

/// Host-internal root resolution. This raw-path capability is deliberately
/// kept out of `FilesHost`, which is the only filesystem interface exposed to
/// provider strategies through `HostFacade`.
pub trait FileScopeHost: Send + Sync {
    /// Validate one product root: absolute, canonicalizable, directory.
    fn scope_root(&self, root: &Path, cancel: &CancellationToken) -> Result<ScopedRoot, HostError> {
        if cancel.is_cancelled() {
            return Err(HostError::Cancelled);
        }
        if !root.is_absolute() {
            return Err(HostError::Policy);
        }
        let canonical = root.canonicalize().map_err(|_| HostError::NotFound)?;
        if !canonical.is_dir() {
            return Err(HostError::NotFound);
        }
        Ok(ScopedRoot { root: canonical })
    }

    /// Explicit environment/config context for runtime root resolution.
    fn home_dir(&self) -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from)
    }

    fn env_var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    /// Top-level child directory names of `dir` (names only, no paths,
    /// no recursion, no content). Used solely for `<prefix>-<slug>`
    /// profile convention discovery within `$HOME`.
    fn child_dir_names(&self, dir: &Path) -> Result<Vec<String>, HostError> {
        if !dir.is_absolute() {
            return Err(HostError::Policy);
        }
        let canonical = dir.canonicalize().map_err(|_| HostError::NotFound)?;
        if !canonical.is_dir() {
            return Err(HostError::NotFound);
        }
        let mut names = vec![];
        for entry in std::fs::read_dir(&canonical).map_err(|_| HostError::Unavailable)? {
            let Ok(entry) = entry else { continue };
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = entry.file_name().to_str() {
                    names.push(name.to_string());
                }
            }
        }
        names.sort();
        Ok(names)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScopedFiles;

impl ScopedFiles {
    #[cfg(test)]
    fn discover(
        &self,
        root: &Path,
        glob_pattern: &str,
        follow_symlinks: bool,
    ) -> Result<Vec<ScopedPath>, HostError> {
        if !root.is_absolute() || glob_pattern.contains("..") {
            return Err(HostError::Policy);
        }
        let canonical_root = root.canonicalize().map_err(|_| HostError::NotFound)?;
        let full_pattern = format!("{}/{glob_pattern}", canonical_root.display());
        let mut out = vec![];
        let entries = glob::glob(&full_pattern).map_err(|_| HostError::Policy)?;
        for entry in entries {
            let Ok(path) = entry else { continue };
            // Rejected paths (symlink escape, vanished files) are skipped,
            // never fatal to the whole discovery.
            if let Ok(scoped) = self.scope(&canonical_root, &path, follow_symlinks) {
                out.push(scoped);
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    #[cfg(test)]
    fn read_range(
        &self,
        path: &Path,
        offset: u64,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, FileIdentity), HostError> {
        use std::io::{Read, Seek, SeekFrom};
        let identity = file_identity(path)?;
        let mut file = std::fs::File::open(path).map_err(|_| HostError::NotFound)?;
        let start = offset.min(identity.size);
        file.seek(SeekFrom::Start(start))
            .map_err(|_| HostError::Unavailable)?;
        let mut buf = vec![0u8; max_bytes.min(identity.size.saturating_sub(start) as usize)];
        let mut read = 0usize;
        while read < buf.len() {
            match file.read(&mut buf[read..]) {
                Ok(0) => break,
                Ok(n) => read += n,
                Err(_) => return Err(HostError::Unavailable),
            }
        }
        buf.truncate(read);
        Ok((buf, identity))
    }

    #[cfg(test)]
    fn read_tail(
        &self,
        path: &Path,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, FileIdentity), HostError> {
        let identity = file_identity(path)?;
        let start = identity.size.saturating_sub(max_bytes as u64);
        self.read_range(path, start, max_bytes)
    }
}

impl FileScopeHost for ScopedFiles {}

impl FilesHost for ScopedFiles {}

impl ScopedFiles {
    #[cfg(test)]
    fn scope(
        &self,
        canonical_root: &Path,
        path: &Path,
        follow_symlinks: bool,
    ) -> Result<ScopedPath, HostError> {
        if !follow_symlinks && path_contains_symlink(canonical_root, path) {
            return Err(HostError::Policy);
        }
        let canonical = path.canonicalize().map_err(|_| HostError::NotFound)?;
        if !canonical.starts_with(canonical_root) {
            return Err(HostError::Policy);
        }
        if !canonical.is_file() {
            return Err(HostError::NotFound);
        }
        Ok(ScopedPath {
            path: canonical,
            identity: file_identity(path)?,
        })
    }
}

#[cfg(test)]
fn path_contains_symlink(root: &Path, path: &Path) -> bool {
    let mut current = path.to_path_buf();
    loop {
        if let Ok(meta) = std::fs::symlink_metadata(&current) {
            if meta.file_type().is_symlink() {
                return true;
            }
        }
        if current == root {
            break;
        }
        match current.parent() {
            Some(parent) if current != parent => current = parent.to_path_buf(),
            _ => break,
        }
    }
    false
}

pub fn file_identity(path: &Path) -> Result<FileIdentity, HostError> {
    let meta = std::fs::metadata(path).map_err(|_| HostError::NotFound)?;
    Ok(file_identity_from_metadata(path, &meta))
}

fn file_identity_from_metadata(path: &Path, meta: &std::fs::Metadata) -> FileIdentity {
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    #[cfg(unix)]
    let (device, inode) = {
        use std::os::unix::fs::MetadataExt;
        (meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let (device, inode) = (0u64, 0u64);
    FileIdentity {
        device,
        inode,
        size: meta.len(),
        mtime_ms,
        path_hash: path_hash(path),
    }
}

pub fn path_hash(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(path.as_os_str().as_encoded_bytes());
    format!("{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn replacement_detected_by_inode_not_only_size() {
        let a = FileIdentity {
            device: 1,
            inode: 10,
            size: 100,
            mtime_ms: 1,
            path_hash: "x".into(),
        };
        let grown_same = FileIdentity {
            size: 200,
            ..a.clone()
        };
        assert!(!a.replaced(&grown_same));
        let replaced = FileIdentity {
            inode: 11,
            size: 200,
            ..a.clone()
        };
        assert!(a.replaced(&replaced));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let outside = dir.path().join("outside.txt");
        std::fs::write(&outside, "secret").unwrap();
        symlink(&outside, root.join("link.txt")).unwrap();
        let host = ScopedFiles;
        // Escaping links are skipped in both modes (never fatal, never returned).
        assert!(host.discover(&root, "*.txt", false).unwrap().is_empty());
        assert!(host.discover(&root, "link.txt", true).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn internal_symlink_follow_policy() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("real.txt"), "data").unwrap();
        symlink(root.join("real.txt"), root.join("alias.txt")).unwrap();
        let host = ScopedFiles;
        assert!(host.discover(&root, "alias.txt", false).unwrap().is_empty());
        assert_eq!(host.discover(&root, "alias.txt", true).unwrap().len(), 1);
    }

    #[test]
    fn parent_traversal_pattern_rejected() {
        let host = ScopedFiles;
        let result = host.discover(Path::new("/tmp"), "../etc/*.txt", true);
        assert!(matches!(result, Err(HostError::Policy)));
    }

    #[test]
    fn read_range_and_tail() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "hello world").unwrap();
        let host = ScopedFiles;
        let (head, _) = host.read_range(file.path(), 0, 5).unwrap();
        assert_eq!(&head, b"hello");
        let (tail, _) = host.read_tail(file.path(), 1024).unwrap();
        assert!(tail.ends_with(b"world\n"));
    }

    #[test]
    fn cancelled_scan_stops_before_glob_iteration() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("one.txt"), "data").unwrap();
        let token = CancellationToken::new();
        token.cancel();
        let host = ScopedFiles;
        let root = host
            .scope_root(dir.path(), &CancellationToken::new())
            .unwrap();
        assert!(matches!(
            host.list_files(&root, "*.txt", &token),
            Err(HostError::Cancelled)
        ));
    }

    #[test]
    fn exact_directories_list_files_prevents_overscan_into_unauthorized_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        let archived = dir.path().join("archived_sessions");
        let bad = dir.path().join("bad_sessions");
        let other = dir.path().join("random_sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&archived).unwrap();
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::create_dir_all(&other).unwrap();

        std::fs::write(sessions.join("rollout-1.jsonl"), "data1").unwrap();
        std::fs::write(archived.join("rollout-2.jsonl"), "data2").unwrap();
        std::fs::write(bad.join("rollout-3.jsonl"), "data3").unwrap();
        std::fs::write(other.join("rollout-4.jsonl"), "data4").unwrap();

        let host = ScopedFiles;
        let root = host
            .scope_root(dir.path(), &CancellationToken::new())
            .unwrap();

        let pattern = "sessions/**/rollout-*.jsonl;archived_sessions/**/rollout-*.jsonl";
        let f1 = sessions.join("rollout-1.jsonl").canonicalize().unwrap();
        let f2 = archived.join("rollout-2.jsonl").canonicalize().unwrap();
        let f3 = bad.join("rollout-3.jsonl").canonicalize().unwrap();
        let f4 = other.join("rollout-4.jsonl").canonicalize().unwrap();

        let found = host
            .list_files(&root, pattern, &CancellationToken::new())
            .unwrap();

        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|f| f.path == f1));
        assert!(found.iter().any(|f| f.path == f2));
        assert!(!found.iter().any(|f| f.path == f3));
        assert!(!found.iter().any(|f| f.path == f4));
    }
}

#[cfg(test)]
mod scoped_file_tests {
    use super::*;
    #[test]
    fn traversal_and_absolute_names_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let host = ScopedFiles;
        let root = host
            .scope_root(dir.path(), &CancellationToken::new())
            .unwrap();
        // NOTE: scope_root lives on FileScopeHost; ScopedFiles implements it.
        for bad in ["../x", "..", "/etc/passwd", "", "a/b", "a\\b"] {
            assert!(
                host.read_scoped_file(&root, bad, 64, &CancellationToken::new())
                    .is_err(),
                "{bad} must be rejected"
            );
        }
        std::fs::write(dir.path().join("auth.json"), "{}").unwrap();
        let bytes = host
            .read_scoped_file(&root, "auth.json", 64, &CancellationToken::new())
            .unwrap();
        assert_eq!(bytes, b"{}");
    }

    #[cfg(unix)]
    #[test]
    fn scoped_file_rejects_replacement_with_outside_symlink() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let outside = dir.path().join("outside.json");
        std::fs::write(root.join("auth.json"), "inside").unwrap();
        std::fs::write(&outside, "secret").unwrap();
        let host = ScopedFiles;
        let scoped_root = host.scope_root(&root, &CancellationToken::new()).unwrap();
        let file = host
            .list_files(&scoped_root, "auth.json", &CancellationToken::new())
            .unwrap()
            .pop()
            .unwrap();
        std::fs::remove_file(root.join("auth.json")).unwrap();
        symlink(&outside, root.join("auth.json")).unwrap();
        assert!(matches!(
            host.read_scoped_range(&file, 0, 64, &CancellationToken::new()),
            Err(HostError::Policy)
        ));
    }
}
