pub mod account_router;
pub mod aggregation;
pub mod connection_test;
pub mod discovery;
pub mod history_import;
pub mod notifications;
pub mod orchestration;
pub mod refresh;
pub mod registry;
pub mod root_resolver;
pub mod snapshot;

pub use account_router::{attribute_local_import, route, LocalAttribution, Routing};
pub use aggregation::{
    account_usage_and_cost_today, build_aggregates, AccountUsageAndCost, Aggregates, MoneyTotal,
    ProviderCostAggregate, ProviderCostInput, Segment,
};
pub use connection_test::{test_connection, ConnectionTestDiagnostics};
pub use history_import::{import_connection, ImportReport, ImportRequest};
pub use notifications::{PlannedNotice, Planner, ProviderNoticeView, QuotaNoticeView};
pub use orchestration::{
    collect_requests, ensure_local_accounts, import_all_history, stable_local_account, AccountMeta,
};
pub use refresh::{
    AttemptStatus, Coordinator, RefreshOutcome, RefreshRequest, RefreshState, ScopeKey, Trigger,
};
pub use registry::{classification_label, strategies_for, Registry, StrategyKind};
pub use root_resolver::{product_root_candidates, scoped_product_root};
pub use snapshot::{stale_view_from_lkg, StaleView};
