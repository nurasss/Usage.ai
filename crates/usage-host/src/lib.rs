pub mod cancel;
pub mod clock;
pub mod credentials;
pub mod error;
pub mod facade;
pub mod files;
pub mod http;
pub mod keychain;
pub mod logger;
pub mod network;
pub mod policy;
pub mod process;
pub mod pty;

use std::sync::Arc;

pub use async_trait::async_trait;
pub use cancel::CancellationToken;
pub use clock::{ClockHost, FixedClock, SystemClock};
pub use credentials::{fingerprint, SecretString};
pub use error::{safe_code, HostError};
pub use facade::{HostFacade, ProviderHostFacade};
pub use files::{
    FileIdentity, FileScopeHost, FilesHost, ScopedFile, ScopedFiles, ScopedPath, ScopedRoot,
};
pub use http::{HttpHost, HttpJsonRequest, HttpJsonResponse, ReqwestHttpHost};
#[cfg(target_os = "macos")]
pub use keychain::KeyringHost;
pub use keychain::{KeychainHost, MemoryKeychain, ScopedKeychain};
pub use logger::{LoggerHost, NullLogger, TracingLogger};
pub use network::{NetworkHost, ObservedNetwork, OnlineState};
pub use process::{
    AllowlistedProcess, InteractiveChild, InteractiveRequest, ProcessHost, SpawnOutput,
    SpawnRequest,
};
pub use pty::{AllowlistedPty, PTYHost, PtyChild, PtyInput, PtyRequest, PtySpawnRequest};

/// Scoped OS access bundle handed to provider strategies.
/// Strategies never touch the OS except through these traits.
#[derive(Clone)]
pub struct Hosts {
    pub clock: Arc<dyn ClockHost>,
    pub logger: Arc<dyn LoggerHost>,
    pub keychain: Arc<dyn KeychainHost>,
    pub http: Arc<dyn HttpHost>,
    pub files: Arc<dyn FilesHost>,
    /// Raw root resolution is runtime-only and is never included in the
    /// provider `HostFacade`.
    pub file_scope: Arc<dyn FileScopeHost>,
    pub network: Arc<dyn NetworkHost>,
    pub process: Arc<dyn ProcessHost>,
    pub pty: Arc<dyn PTYHost>,
}

impl Default for Hosts {
    fn default() -> Self {
        let files = Arc::new(ScopedFiles);
        Self {
            clock: Arc::new(SystemClock),
            logger: Arc::new(NullLogger),
            keychain: Arc::new(MemoryKeychain::default()),
            http: Arc::new(ReqwestHttpHost::default()),
            files: files.clone(),
            file_scope: files,
            network: Arc::new(ObservedNetwork::default()),
            process: Arc::new(AllowlistedProcess::default()),
            pty: Arc::new(AllowlistedPty::default()),
        }
    }
}
