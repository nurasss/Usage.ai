use crate::{Capability, ConnectionState, Coverage, Freshness};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeDiagnostics {
    pub provider: String,
    pub product: String,
    pub account_alias: String,
    pub connection_state: ConnectionState,
    pub last_refresh_attempt: Option<DateTime<Utc>>,
    pub last_successful_refresh: Option<DateTime<Utc>>,
    pub last_data_observed_at: Option<DateTime<Utc>>,
    pub freshness: Freshness,
    pub coverage: Coverage,
    pub status_class: Option<String>,
    pub connector_version: String,
    pub parser_version: String,
    pub schema_fingerprint: Option<String>,
    pub capabilities_detected: BTreeSet<Capability>,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub last_safe_error_code: Option<String>,
}

pub fn redact_message(input: &str) -> String {
    let sensitive = [
        "authorization",
        "bearer",
        "api_key",
        "apikey",
        "access_token",
        "refresh_token",
        "cookie",
        "password",
    ];
    input
        .split_whitespace()
        .map(|part| {
            if sensitive
                .iter()
                .any(|needle| part.to_ascii_lowercase().contains(needle))
                || looks_like_secret(part)
            {
                "[REDACTED]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn looks_like_secret(value: &str) -> bool {
    value.len() > 24
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redacts_secret_shaped_values() {
        assert_eq!(
            redact_message("token sk-abcdefghijklmnopqrstuvwxyz123456"),
            "token [REDACTED]"
        );
    }
}
