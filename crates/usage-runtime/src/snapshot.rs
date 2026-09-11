use chrono::{DateTime, Utc};
use usage_core::{Coverage, Freshness, Snapshot};
use usage_storage::StoredSnapshot;

/// Stale-While-Revalidate view: a failed refresh never deletes the
/// last-known-good payload. The UI receives old data together with
/// staleness age and the current error metadata.
pub struct StaleView {
    pub snapshot: Snapshot,
    pub age_secs: u64,
}

/// Legacy trust guard (defense in depth for migration 010): any
/// claude-code snapshot still carrying authoritative `Partial` — the
/// RC1 synthetic shape, for which no validated authoritative contract
/// has ever existed — is reclassified to `UnverifiedSemantics` at
/// load. Source-aware (provider/product/coverage triple only);
/// legitimate `Partial` from other products is untouched. Never
/// trusts the persisted enum blindly.
pub fn reclassify_legacy_claude_snapshot(snapshot: &mut Snapshot) {
    if snapshot.provider_id == "anthropic"
        && snapshot.product_id == "claude-code"
        && snapshot.coverage == Coverage::Partial
    {
        snapshot.coverage = Coverage::UnverifiedSemantics;
    }
}

pub fn stale_view_from_lkg(stored: &StoredSnapshot, now: DateTime<Utc>) -> Option<StaleView> {
    let mut snapshot: Snapshot = serde_json::from_str(&stored.payload_json).ok()?;
    let fetched: DateTime<Utc> = stored.fetched_at.parse().ok()?;
    let age = (now - fetched).num_seconds().max(0) as u64;
    snapshot.freshness = Freshness::Stale(age);
    reclassify_legacy_claude_snapshot(&mut snapshot);
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

    /// P0 legacy guard (§8, case A/D): an RC1-persisted synthetic
    /// Claude snapshot (Partial, 20%) reclassifies to
    /// UnverifiedSemantics at load — stale, non-authoritative, and
    /// stable across repeated loads (restart idempotency).
    #[test]
    fn legacy_claude_partial_reclassifies_on_lkg_load() {
        use usage_core::Quota;
        let quota = Quota {
            pool_id: "claude-pty-weekly".into(),
            window_id: None,
            name: "Weekly".into(),
            used: None,
            limit: None,
            used_percent: Some(rust_decimal::Decimal::new(80, 0)),
            remaining_percent: Some(rust_decimal::Decimal::new(20, 0)),
            unit: usage_core::MetricUnit::Percent,
            window_start: None,
            resets_at: None,
            window_kind: usage_core::WindowKind::Unknown,
            source: "claude-pty-usage@test".into(),
        };
        let snapshot = Snapshot {
            account_id: Uuid::nil(),
            provider_id: "anthropic".into(),
            product_id: "claude-code".into(),
            plan_label: None,
            capabilities: [Capability::SubscriptionQuota].into_iter().collect(),
            quotas: vec![quota],
            balances: vec![],
            observed_at: None,
            fetched_at: "2026-09-10T10:00:00Z".parse().unwrap(),
            connection_state: ConnectionState::Connected,
            freshness: Freshness::Fresh,
            coverage: Coverage::Partial,
        };
        assert!(snapshot.coverage.is_authoritative());
        let stored = StoredSnapshot {
            account_id: Uuid::nil().to_string(),
            product_id: "claude-code".into(),
            payload_json: serde_json::to_string(&snapshot).unwrap(),
            observed_at: None,
            fetched_at: "2026-09-10T10:00:00Z".into(),
        };
        for _ in 0..2 {
            let view =
                stale_view_from_lkg(&stored, "2026-09-10T10:05:00Z".parse().unwrap()).unwrap();
            assert_eq!(view.snapshot.coverage, Coverage::UnverifiedSemantics);
            assert!(!view.snapshot.coverage.is_authoritative());
            assert_eq!(view.snapshot.quotas.len(), 1);
        }
    }

    /// Legitimate non-Claude Partial (official paginated costs) keeps
    /// its authoritative-but-incomplete semantics (§6, case C).
    #[test]
    fn legitimate_openai_partial_stays_authoritative() {
        let mut stored = stored_snapshot("2026-09-08T12:00:00Z");
        let mut snapshot: Snapshot = serde_json::from_str(&stored.payload_json).unwrap();
        snapshot.coverage = Coverage::Partial;
        stored.payload_json = serde_json::to_string(&snapshot).unwrap();
        let view = stale_view_from_lkg(&stored, Utc::now()).unwrap();
        assert_eq!(view.snapshot.coverage, Coverage::Partial);
        assert!(view.snapshot.coverage.is_authoritative());
    }
}
