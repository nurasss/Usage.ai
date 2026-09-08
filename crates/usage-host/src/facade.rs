use super::{
    ClockHost, FilesHost, HttpHost, KeychainHost, LoggerHost, NetworkHost, ScopedKeychain,
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
    pub keychain: Option<ScopedKeychain<'a, dyn KeychainHost>>,
}

pub struct FacadePolicy {
    pub files: bool,
    pub http: bool,
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
            keychain: policy
                .keychain_service
                .map(|service| ScopedKeychain::new(self.keychain.as_ref(), service)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use std::sync::Arc;

    fn bundle() -> Hosts {
        Hosts {
            clock: Arc::new(SystemClock),
            logger: Arc::new(NullLogger),
            keychain: Arc::new(MemoryKeychain::default()),
            http: Arc::new(ReqwestHttpHost::default()),
            files: Arc::new(ScopedFiles),
            network: Arc::new(ObservedNetwork::default()),
        }
    }

    #[test]
    fn local_product_gets_files_only() {
        let hosts = bundle();
        let facade = hosts.facade(&super::FacadePolicy {
            files: true,
            http: false,
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
            keychain_service: None,
        });
        assert!(facade.files.is_none());
        assert!(facade.http.is_none());
        assert!(facade.keychain.is_none());
    }
}
