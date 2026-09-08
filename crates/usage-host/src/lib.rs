pub mod clock;
pub mod credentials;
pub mod error;
pub mod files;
pub mod http;
pub mod keychain;
pub mod logger;
pub mod network;
pub mod policy;

use std::sync::Arc;

pub use clock::{ClockHost, FixedClock, SystemClock};
pub use credentials::{fingerprint, SecretString};
pub use error::{safe_code, HostError};
pub use files::{FileIdentity, FilesHost, ScopedFiles, ScopedPath};
pub use http::{HttpHost, HttpJsonRequest, HttpJsonResponse, ReqwestHttpHost};
#[cfg(target_os = "macos")]
pub use keychain::KeyringHost;
pub use keychain::{KeychainHost, MemoryKeychain};
pub use logger::{LoggerHost, NullLogger, TracingLogger};
pub use network::{NetworkHost, ObservedNetwork, OnlineState};

/// Scoped OS access bundle handed to provider strategies.
/// Strategies never touch the OS except through these traits.
#[derive(Clone)]
pub struct Hosts {
    pub clock: Arc<dyn ClockHost>,
    pub logger: Arc<dyn LoggerHost>,
    pub keychain: Arc<dyn KeychainHost>,
    pub http: Arc<dyn HttpHost>,
    pub files: Arc<dyn FilesHost>,
    pub network: Arc<dyn NetworkHost>,
}
