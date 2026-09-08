use crate::{Account, Capability, Snapshot, UsageRecord};
use async_trait::async_trait;
use std::{collections::BTreeSet, path::Path};

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("authentication required")]
    AuthenticationRequired,
    #[error("permission denied")]
    PermissionDenied,
    #[error("insufficient scope")]
    InsufficientScope,
    #[error("rate limited; retry after {0:?}")]
    RateLimited(Option<u64>),
    #[error("network error: {0}")]
    Network(String),
    #[error("source unavailable: {0}")]
    Unavailable(String),
    #[error("parse error: {0}")]
    Parse(String),
}

#[derive(Debug, Clone)]
pub struct ImportBatch {
    pub records: Vec<UsageRecord>,
    pub next_offset: u64,
    pub partial_line: bool,
    pub warnings: Vec<String>,
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn provider_id(&self) -> &'static str;
    fn product_id(&self) -> &'static str;
    fn capabilities(&self) -> BTreeSet<Capability>;
    fn allowed_hosts(&self) -> &'static [&'static str];
    async fn refresh(&self, account: &Account) -> Result<Snapshot, ProviderError>;
    async fn import_local(
        &self,
        _account: &Account,
        _path: &Path,
        offset: u64,
    ) -> Result<ImportBatch, ProviderError> {
        Ok(ImportBatch {
            records: vec![],
            next_offset: offset,
            partial_line: false,
            warnings: vec![],
        })
    }
}
