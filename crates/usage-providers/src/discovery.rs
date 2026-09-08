use async_trait::async_trait;
use std::collections::BTreeSet;
use usage_core::{Account, Capability, ProviderAdapter, ProviderError, Snapshot};

pub struct DiscoveryOnlyAdapter {
    pub provider: &'static str,
    pub product: &'static str,
    pub declared: BTreeSet<Capability>,
    pub hosts: &'static [&'static str],
}
#[async_trait]
impl ProviderAdapter for DiscoveryOnlyAdapter {
    fn provider_id(&self) -> &'static str {
        self.provider
    }
    fn product_id(&self) -> &'static str {
        self.product
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        self.declared.clone()
    }
    fn allowed_hosts(&self) -> &'static [&'static str] {
        self.hosts
    }
    async fn refresh(&self, _: &Account) -> Result<Snapshot, ProviderError> {
        Err(ProviderError::Unavailable(
            "integration_requires_verified_source".into(),
        ))
    }
}
