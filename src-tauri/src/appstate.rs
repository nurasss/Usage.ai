use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use usage_host::{Hosts, ObservedNetwork};
use usage_runtime::{ImportReport, Planner};
use usage_storage::Storage;

use crate::dto::AppSnapshot;

/// Thin composition state. Business orchestration lives in
/// `usage-runtime`; this struct only holds shared handles.
pub struct AppState {
    pub storage: Arc<Mutex<Storage>>,
    pub cached: Mutex<Option<AppSnapshot>>,
    pub coordinator: Arc<usage_runtime::Coordinator>,
    pub hosts: Arc<Hosts>,
    pub network: ObservedNetwork,
    pub planner: Mutex<Planner>,
    pub import_stats: Mutex<HashMap<String, ImportReport>>,
    pub settings_path: PathBuf,
}
