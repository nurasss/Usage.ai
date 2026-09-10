use std::sync::Arc;
#[cfg(target_os = "macos")]
use usage_host::KeyringHost;
#[cfg(not(target_os = "macos"))]
use usage_host::MemoryKeychain;
use usage_host::{Hosts, ObservedNetwork, ReqwestHttpHost, ScopedFiles, SystemClock};

pub const KEYCHAIN_SERVICE: &str = "com.nurasss.usageai";

/// macOS host bundle: native Keychain, scoped files, allow-listed
/// HTTPS, observed network state, system clock, redacting logger.
pub fn macos_hosts(network: &ObservedNetwork) -> Arc<Hosts> {
    let files = Arc::new(ScopedFiles);
    Arc::new(Hosts {
        clock: Arc::new(SystemClock),
        logger: Arc::new(usage_host::TracingLogger),
        keychain: macos_keychain(),
        http: Arc::new(ReqwestHttpHost::default()),
        files: files.clone(),
        file_scope: files,
        network: Arc::new(network.clone()),
        process: Arc::new(usage_host::AllowlistedProcess::with_allowed(
            codex_executables(),
        )),
        pty: Arc::new(usage_host::AllowlistedPty::default()),
    })
}

/// Local Codex CLI candidates. Existence is checked here in the
/// platform layer; the strategy only ever receives paths the host
/// allow-listed. No PATH lookup, no shell.
fn codex_executables() -> Vec<std::path::PathBuf> {
    let mut candidates = vec![
        std::path::PathBuf::from("/usr/local/bin/codex"),
        std::path::PathBuf::from("/opt/homebrew/bin/codex"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        candidates.push(home.join(".npm-global/bin/codex"));
        candidates.push(home.join(".local/bin/codex"));
    }
    candidates
        .into_iter()
        .filter(|p| std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false))
        .collect()
}

fn macos_keychain() -> Arc<dyn usage_host::KeychainHost> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(KeyringHost)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(MemoryKeychain::default())
    }
}
