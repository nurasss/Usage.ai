use std::collections::HashSet;
use std::path::PathBuf;
use usage_host::{CancellationToken, Hosts};
use usage_providers::descriptor::ProductDescriptor;

/// A discovered profile root: validated against the product's marker
/// layout, canonicalized, hashed. A candidate is never an account —
/// it becomes one only through explicit user connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredRoot {
    pub path: PathBuf,
    pub root_hash: String,
    pub hint: String,
    pub kind: DiscoveredKind,
    pub valid: bool,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveredKind {
    Default,
    Slug,
    Custom,
}

/// Enumerate profile root candidates for one local product.
/// Allowed sources only (§11.3): default root, known naming
/// convention (`~/.<prefix>-<slug>`), explicit user roots. No home
/// directory scan beyond a single top-level prefix listing, no
/// content reads — validation is metadata-only (marker subdir).
pub fn discover_profile_roots(
    hosts: &Hosts,
    descriptor: &ProductDescriptor,
    custom_roots: &[PathBuf],
    cancel: &CancellationToken,
) -> Vec<DiscoveredRoot> {
    let mut out: Vec<DiscoveredRoot> = vec![];
    let mut seen_hashes = HashSet::new();
    let mut push = |path: PathBuf, kind: DiscoveredKind, out: &mut Vec<DiscoveredRoot>| {
        if cancel.is_cancelled() {
            return;
        }
        let canonical = path.canonicalize().unwrap_or(path);
        let root_hash = usage_host::files::path_hash(&canonical);
        if !seen_hashes.insert(root_hash.clone()) {
            return;
        }
        let hint = canonical
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "default".into());
        let (valid, reason) = validate_product_root(descriptor, &canonical);
        out.push(DiscoveredRoot {
            path: canonical,
            root_hash,
            hint,
            kind,
            valid,
            reason,
        });
    };

    // 1. Default root: env override or $HOME/<dot-dir>.
    if let Some(default) = default_product_root(hosts, descriptor) {
        push(default, DiscoveredKind::Default, &mut out);
    }
    // 2. Known naming convention: single top-level $HOME listing.
    if let (Some(home), Some(prefix)) = (hosts.file_scope.home_dir(), descriptor.profile_dir_prefix)
    {
        if let Ok(names) = hosts.file_scope.child_dir_names(&home) {
            for name in names {
                if cancel.is_cancelled() {
                    break;
                }
                if name.starts_with(prefix) && name.len() > prefix.len() {
                    push(home.join(&name), DiscoveredKind::Slug, &mut out);
                }
            }
        }
    }
    // 3. Explicit user-added roots (already authorized).
    for custom in custom_roots {
        push(custom.clone(), DiscoveredKind::Custom, &mut out);
    }
    out
}

fn default_product_root(hosts: &Hosts, descriptor: &ProductDescriptor) -> Option<PathBuf> {
    if let Some(env_name) = descriptor.local_dir_env {
        if let Some(dir) = hosts.file_scope.env_var(env_name) {
            if !dir.trim().is_empty() {
                return Some(PathBuf::from(dir));
            }
        }
    }
    hosts
        .file_scope
        .home_dir()
        .zip(descriptor.local_dir_name)
        .map(|(home, name)| home.join(name))
}

/// Metadata-only validation: the canonical root must be a directory
/// containing the product's marker (first glob segment, e.g.
/// `sessions` or `projects`). Non-conforming siblings such as
/// `~/.codex-mux` (state dir) or `~/.codex-plusplus` (tool install)
/// are reported invalid with a reason instead of being trusted.
fn validate_product_root(
    descriptor: &ProductDescriptor,
    canonical: &std::path::Path,
) -> (bool, &'static str) {
    if !canonical.is_absolute() || !canonical.is_dir() {
        return (false, "not-a-directory");
    }
    let Some(glob) = descriptor.local_glob else {
        return (false, "no-local-source");
    };
    // Globs may list several roots separated by ';' — any marker counts.
    let markers_valid = glob.split(';').any(|part| {
        let marker = part.split('/').next().unwrap_or("");
        !marker.is_empty() && marker != "**" && canonical.join(marker).is_dir()
    });
    if markers_valid {
        (true, "ok")
    } else {
        (false, "missing-marker")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use usage_host::{
        FileScopeHost, FilesHost, MemoryKeychain, NullLogger, ObservedNetwork, ReqwestHttpHost,
        SystemClock,
    };

    struct SandboxFiles {
        home: PathBuf,
    }

    impl FilesHost for SandboxFiles {}
    impl FileScopeHost for SandboxFiles {
        fn home_dir(&self) -> Option<PathBuf> {
            Some(self.home.clone())
        }
        fn env_var(&self, _name: &str) -> Option<String> {
            None
        }
    }

    fn sandbox_hosts(home: PathBuf) -> Hosts {
        let files: Arc<SandboxFiles> = Arc::new(SandboxFiles { home });
        Hosts {
            clock: Arc::new(SystemClock),
            logger: Arc::new(NullLogger),
            keychain: Arc::new(MemoryKeychain::default()),
            http: Arc::new(ReqwestHttpHost::default()),
            files: files.clone(),
            file_scope: files,
            network: Arc::new(ObservedNetwork::default()),
            process: Arc::new(usage_host::AllowlistedProcess::default()),
            pty: Arc::new(usage_host::AllowlistedPty::default()),
        }
    }

    fn descriptor() -> &'static ProductDescriptor {
        usage_providers::descriptor::find_descriptor("openai", "codex").unwrap()
    }

    #[test]
    fn valid_home_and_invalid_siblings() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".codex").join("sessions")).unwrap();
        std::fs::create_dir_all(home.path().join(".codex-mux")).unwrap();
        std::fs::create_dir_all(home.path().join(".codex-plusplus").join("bin")).unwrap();
        let hosts = sandbox_hosts(home.path().to_path_buf());
        let found = discover_profile_roots(&hosts, descriptor(), &[], &CancellationToken::new());
        let valid: Vec<_> = found.iter().filter(|r| r.valid).collect();
        let invalid: Vec<_> = found.iter().filter(|r| !r.valid).collect();
        assert_eq!(valid.len(), 1);
        assert_eq!(valid[0].hint, ".codex");
        assert_eq!(valid[0].kind, DiscoveredKind::Default);
        // Lookalike siblings are reported, never trusted.
        assert!(invalid.iter().any(|r| r.hint == ".codex-mux"));
        assert!(invalid.iter().any(|r| r.hint == ".codex-plusplus"));
        assert!(invalid.iter().all(|r| r.reason == "missing-marker"));
    }

    #[test]
    fn slug_profile_discovered_as_candidate() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".codex-work").join("sessions")).unwrap();
        let hosts = sandbox_hosts(home.path().to_path_buf());
        let found = discover_profile_roots(&hosts, descriptor(), &[], &CancellationToken::new());
        let slug = found
            .iter()
            .find(|r| r.hint == ".codex-work")
            .expect("slug profile must be discovered");
        assert!(slug.valid);
        assert_eq!(slug.kind, DiscoveredKind::Slug);
    }
}
