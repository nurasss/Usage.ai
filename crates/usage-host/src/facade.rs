use super::{
    ClockHost, FilesHost, HttpHost, KeychainHost, LoggerHost, NetworkHost, PTYHost, ProcessHost,
    ScopedKeychain,
};

/// Per-provider host facade. The runtime builds exactly one facade per
/// product from its descriptor: a strategy physically cannot reach a
/// host it was not granted (`None` fields), and the Keychain handle is
/// namespace-scoped to one service.
pub struct HostFacade<'a> {
    pub clock: &'a dyn ClockHost,
    pub logger: &'a dyn LoggerHost,
    pub network: &'a dyn NetworkHost,
    pub files: Option<&'a dyn FilesHost>,
    pub http: Option<&'a dyn HttpHost>,
    pub process: Option<&'a dyn ProcessHost>,
    /// Interactive terminal execution for CLI strategies (e.g. the
    /// installed Claude Code usage command). Granted separately from
    /// `process`: a pty can drive full-screen TUIs.
    pub pty: Option<&'a dyn PTYHost>,
    pub keychain: Option<ScopedKeychain<'a, dyn KeychainHost>>,
    /// The allow-list is descriptor-owned and cannot be selected by a
    /// provider strategy or by an arbitrary URL supplied at runtime.
    pub allowed_hosts: &'static [&'static str],
}

/// Name used at the provider boundary to make the capability cut explicit.
pub type ProviderHostFacade<'a> = HostFacade<'a>;

pub struct FacadePolicy {
    pub files: bool,
    pub http: bool,
    pub process: bool,
    pub pty: bool,
    pub allowed_hosts: &'static [&'static str],
    /// Fixed Keychain service namespace, or `None` for no access.
    pub keychain_service: Option<&'static str>,
}

impl super::Hosts {
    pub fn facade(&self, policy: &FacadePolicy) -> HostFacade<'_> {
        HostFacade {
            clock: self.clock.as_ref(),
            logger: self.logger.as_ref(),
            network: self.network.as_ref(),
            files: policy.files.then_some(self.files.as_ref()),
            http: policy.http.then_some(self.http.as_ref()),
            process: policy.process.then_some(self.process.as_ref()),
            pty: policy.pty.then_some(self.pty.as_ref()),
            keychain: policy
                .keychain_service
                .map(|service| ScopedKeychain::new(self.keychain.as_ref(), service)),
            allowed_hosts: policy.allowed_hosts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use std::sync::Arc;

    fn bundle() -> Hosts {
        let files = Arc::new(ScopedFiles);
        Hosts {
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

    #[test]
    fn local_product_gets_files_only() {
        let hosts = bundle();
        let facade = hosts.facade(&super::FacadePolicy {
            files: true,
            http: false,
            process: false,
            pty: false,
            allowed_hosts: &[],
            keychain_service: None,
        });
        assert!(facade.files.is_some());
        assert!(facade.http.is_none());
        assert!(facade.keychain.is_none());
    }

    #[test]
    fn api_product_gets_http_and_scoped_keychain_only() {
        let hosts = bundle();
        let facade = hosts.facade(&super::FacadePolicy {
            files: false,
            http: true,
            process: false,
            pty: false,
            allowed_hosts: &["api.openai.com"],
            keychain_service: Some("com.nurasss.usageai"),
        });
        assert!(facade.files.is_none());
        assert!(facade.http.is_some());
        assert_eq!(
            facade.keychain.map(|k| k.service().to_string()),
            Some("com.nurasss.usageai".to_string())
        );
    }

    #[test]
    fn discovery_product_gets_no_sensitive_hosts() {
        let hosts = bundle();
        let facade = hosts.facade(&super::FacadePolicy {
            files: false,
            http: false,
            process: false,
            pty: false,
            allowed_hosts: &[],
            keychain_service: None,
        });
        assert!(facade.files.is_none());
        assert!(facade.http.is_none());
        assert!(facade.keychain.is_none());
        assert!(facade.process.is_none());
        assert!(facade.pty.is_none());
    }

    #[test]
    fn cli_product_gets_process_only() {
        let hosts = bundle();
        let facade = hosts.facade(&super::FacadePolicy {
            files: false,
            http: false,
            process: true,
            pty: false,
            allowed_hosts: &[],
            keychain_service: None,
        });
        assert!(facade.process.is_some());
        assert!(facade.pty.is_none());
        assert!(facade.files.is_none());
        assert!(facade.http.is_none());
        assert!(facade.keychain.is_none());
    }

    #[test]
    fn pty_product_gets_pty_without_process() {
        let hosts = bundle();
        let facade = hosts.facade(&super::FacadePolicy {
            files: false,
            http: false,
            process: false,
            pty: true,
            allowed_hosts: &[],
            keychain_service: None,
        });
        assert!(facade.pty.is_some());
        assert!(facade.process.is_none());
    }
}
