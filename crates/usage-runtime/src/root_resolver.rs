use std::path::{Path, PathBuf};
use usage_host::{CancellationToken, Hosts, ScopedRoot};
use usage_providers::descriptor::ProductDescriptor;

/// Resolve all configured roots for one descriptor. This is the sole
/// resolver used by refresh and history import, so quota discovery and JSONL
/// history cannot silently disagree about `CODEX_HOME` or an alternate
/// product root.
pub fn product_root_candidates(
    hosts: &Hosts,
    descriptor: &ProductDescriptor,
    custom_root: Option<&Path>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(custom) = custom_root.filter(|path| !path.as_os_str().is_empty()) {
        roots.push(normalize_custom_root(descriptor, custom));
    }

    // A user-selected custom root is a separate connection. It must never
    // accidentally add the default root to that managed account.
    if custom_root.is_some() {
        return roots;
    }

    let default_root = descriptor
        .local_dir_env
        .and_then(|name| hosts.file_scope.env_var(name))
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            hosts
                .file_scope
                .home_dir()
                .zip(descriptor.local_dir_name)
                .map(|(home, name)| home.join(name))
        });
    if let Some(default_root) = default_root {
        if !roots.contains(&default_root) {
            roots.push(default_root);
        }
    }
    roots
}

/// Scope exactly one root for a provider refresh. An explicit custom root is
/// authoritative: if it is unavailable, the refresh reports unavailable
/// rather than falling through to the user's default account.
pub fn scoped_product_root(
    hosts: &Hosts,
    descriptor: &ProductDescriptor,
    custom_root: Option<&Path>,
    cancel: &CancellationToken,
) -> Option<ScopedRoot> {
    let candidates = product_root_candidates(hosts, descriptor, custom_root);
    candidates
        .into_iter()
        .find_map(|root| hosts.file_scope.scope_root(&root, cancel).ok())
}

fn normalize_custom_root(descriptor: &ProductDescriptor, custom: &Path) -> PathBuf {
    let file_name = custom.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let child = descriptor
        .local_glob
        .and_then(|glob| glob.split('/').next())
        .filter(|segment| !segment.is_empty() && *segment != "**");
    if file_name == "sessions"
        || file_name == "archived_sessions"
        || child.is_some_and(|segment| file_name == segment)
    {
        custom
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| custom.to_path_buf())
    } else {
        custom.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use usage_host::{
        MemoryKeychain, NullLogger, ObservedNetwork, ReqwestHttpHost, ScopedFiles, SystemClock,
    };

    fn hosts() -> Hosts {
        let files = Arc::new(ScopedFiles);
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

    #[test]
    fn custom_child_directory_normalizes_from_descriptor_glob() {
        let descriptor = usage_providers::descriptor::find_descriptor("openai", "codex").unwrap();
        assert_eq!(
            normalize_custom_root(descriptor, Path::new("/tmp/codex/sessions")),
            PathBuf::from("/tmp/codex")
        );
        assert_eq!(
            normalize_custom_root(descriptor, Path::new("/tmp/codex/archived_sessions")),
            PathBuf::from("/tmp/codex")
        );
    }

    #[test]
    fn explicit_custom_root_is_not_replaced_by_default() {
        let descriptor = usage_providers::descriptor::find_descriptor("openai", "codex").unwrap();
        let roots = product_root_candidates(&hosts(), descriptor, Some(Path::new("/tmp/custom")));
        assert_eq!(roots.first(), Some(&PathBuf::from("/tmp/custom")));
    }
}
