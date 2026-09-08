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
    Arc::new(Hosts {
        clock: Arc::new(SystemClock),
        logger: Arc::new(usage_host::TracingLogger),
        keychain: macos_keychain(),
        http: Arc::new(ReqwestHttpHost::default()),
        files: Arc::new(ScopedFiles),
        network: Arc::new(network.clone()),
    })
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
