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
            cli_executables("codex"),
        )),
        pty: Arc::new(usage_host::AllowlistedPty::with_allowed(cli_executables(
            "claude",
        ))),
    })
}

/// Local CLI candidates for one binary name. Existence is checked
/// here in the platform layer; strategies only ever receive paths
/// the host allow-listed. No PATH lookup, no shell. The PTY
/// allow-list carries the Claude CLI only — nothing else in the
/// product needs interactive terminal execution.
fn cli_executables(name: &str) -> Vec<std::path::PathBuf> {
    let mut candidates = vec![
        std::path::PathBuf::from(format!("/usr/local/bin/{name}")),
        std::path::PathBuf::from(format!("/opt/homebrew/bin/{name}")),
    ];
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        candidates.push(home.join(format!(".npm-global/bin/{name}")));
        candidates.push(home.join(format!(".local/bin/{name}")));
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
