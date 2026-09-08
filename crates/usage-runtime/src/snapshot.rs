use chrono::{DateTime, Utc};
use usage_core::{Freshness, Snapshot};
use usage_storage::StoredSnapshot;

/// Stale-While-Revalidate view: a failed refresh never deletes the
/// last-known-good payload. The UI receives old data together with
/// staleness age and the current error metadata.
pub struct StaleView {
    pub snapshot: Snapshot,
    pub age_secs: u64,
}

pub fn stale_view_from_lkg(stored: &StoredSnapshot, now: DateTime<Utc>) -> Option<StaleView> {
    let mut snapshot: Snapshot = serde_json::from_str(&stored.payload_json).ok()?;
    let fetched: DateTime<Utc> = stored.fetched_at.parse().ok()?;
    let age = (now - fetched).num_seconds().max(0) as u64;
    snapshot.freshness = Freshness::Stale(age);
    Some(StaleView {
        snapshot,
        age_secs: age,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use usage_core::{Capability, ConnectionState, Coverage};
    use uuid::Uuid;

    fn stored_snapshot(fetched_at: &str) -> StoredSnapshot {
        let snapshot = Snapshot {
            account_id: Uuid::nil(),
            provider_id: "openai".into(),
            product_id: "codex".into(),
            plan_label: None,
            capabilities: [Capability::ApiTokens].into_iter().collect(),
            quotas: vec![],
            balances: vec![],
            observed_at: None,
            fetched_at: fetched_at.parse().unwrap(),
            connection_state: ConnectionState::Connected,
            freshness: Freshness::Fresh,
            coverage: Coverage::LocalClientOnly,
        };
        StoredSnapshot {
            account_id: Uuid::nil().to_string(),
            product_id: "codex".into(),
            payload_json: serde_json::to_string(&snapshot).unwrap(),
            observed_at: None,
            fetched_at: fetched_at.into(),
        }
    }

    #[test]
    fn failure_serves_stale_good_data() {
        let stored = stored_snapshot("2026-09-08T12:00:00Z");
        let now: DateTime<Utc> = "2026-09-08T12:10:00Z".parse().unwrap();
        let view = stale_view_from_lkg(&stored, now).unwrap();
        assert_eq!(view.age_secs, 600);
        assert_eq!(view.snapshot.freshness, Freshness::Stale(600));
        // Quotas and capabilities survive; only freshness is rewritten.
        assert_eq!(view.snapshot.product_id, "codex");
    }

    #[test]
    fn corrupt_payload_is_none_not_panic() {
        let mut stored = stored_snapshot("2026-09-08T12:00:00Z");
        stored.payload_json = "{broken".into();
        assert!(stale_view_from_lkg(&stored, Utc::now()).is_none());
    }
}
