use std::time::Duration;

/// Shared runtime policy bounds.
pub const SOURCE_TIMEOUT: Duration = Duration::from_secs(15);
pub const MAX_HTTP_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_FILE_READ_BYTES: usize = 8 * 1024 * 1024;
pub const QUOTA_TAIL_BYTES: usize = 256 * 1024;
pub const MAX_API_PAGES: usize = 8;
pub const REFRESH_PARALLELISM: usize = 4;
pub const FALLBACK_COOLDOWN: Duration = Duration::from_secs(300);
pub const USER_AGENT: &str = "Usage.ai/1.3.0 (local-first usage monitor)";
