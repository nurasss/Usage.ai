use usage_core::IdentityConfidence;

/// Account/source identity routing decision.
///
/// The active local client today is not guaranteed to be yesterday's
/// account, so every source result with identity evidence is routed:
/// verified match, first-seen weak attribution, explicit mismatch
/// (never auto-merged), or unattributed local data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Routing {
    Attributed {
        confidence: IdentityConfidence,
    },
    Mismatch {
        stored: Option<String>,
        observed: String,
    },
    Unattributed,
}

pub fn route(
    stored_identity: Option<&str>,
    stored_confidence: IdentityConfidence,
    observed: Option<&str>,
) -> Routing {
    match (
        stored_identity.filter(|s| !s.is_empty()),
        observed.filter(|s| !s.is_empty()),
    ) {
        (None, None) => Routing::Unattributed,
        (Some(_), None) => Routing::Attributed {
            confidence: stored_confidence,
        },
        (None, Some(_)) => Routing::Attributed {
            confidence: IdentityConfidence::Weak,
        },
        (Some(stored), Some(seen)) if stored == seen => Routing::Attributed {
            confidence: if stored_confidence == IdentityConfidence::Verified {
                IdentityConfidence::Verified
            } else {
                IdentityConfidence::Weak
            },
        },
        (stored, Some(seen)) => Routing::Mismatch {
            stored: stored.map(|s| s.to_string()),
            observed: seen.to_string(),
        },
    }
}

/// `observed` is the source-observed identity fingerprint, if the
/// source provides any. Local JSONL sources currently do not, so they
/// route purely on the connection's stored confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalAttribution {
    /// Write into this connection (unattributed bucket or match).
    Proceed,
    /// Never write here: not our bucket and no proof it matches.
    Reject,
}

/// Local import attribution rule (B-02):
/// - stored `Unknown` + observed none → the explicit unattributed
///   bucket may receive rows; it is never upgraded implicitly;
/// - stored `Weak`/`Verified` + observed none → reject: unattributable
///   rows must not flow into a previously confirmed account;
/// - observed fingerprint present → first-seen persists `Weak`,
///   mismatch rejects (handled by `route` for API scopes; local
///   connections surface it through the same verdict here).
pub fn attribute_local_import(
    stored_fingerprint: Option<&str>,
    stored_confidence: &str,
    observed: Option<&str>,
) -> LocalAttribution {
    match (
        stored_fingerprint.filter(|s| !s.is_empty()),
        observed.filter(|s| !s.is_empty()),
    ) {
        (None, None) => LocalAttribution::Proceed,
        (Some(_), None) => match stored_confidence {
            "Weak" | "Verified" => LocalAttribution::Reject,
            _ => LocalAttribution::Proceed,
        },
        (None, Some(_)) => LocalAttribution::Proceed,
        (Some(stored), Some(seen)) if stored == seen => LocalAttribution::Proceed,
        (Some(_), Some(_)) => LocalAttribution::Reject,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_local_data_is_never_merged() {
        assert_eq!(
            route(None, IdentityConfidence::Unknown, None),
            Routing::Unattributed
        );
    }
    #[test]
    fn first_seen_identity_is_weak_not_verified() {
        assert_eq!(
            route(None, IdentityConfidence::Unknown, Some("fp1")),
            Routing::Attributed {
                confidence: IdentityConfidence::Weak
            }
        );
    }
    #[test]
    fn mismatch_does_not_merge() {
        assert_eq!(
            route(Some("fp-old"), IdentityConfidence::Verified, Some("fp-new")),
            Routing::Mismatch {
                stored: Some("fp-old".into()),
                observed: "fp-new".into()
            }
        );
    }
    #[test]
    fn verified_match_stays_verified() {
        assert_eq!(
            route(Some("fp"), IdentityConfidence::Verified, Some("fp")),
            Routing::Attributed {
                confidence: IdentityConfidence::Verified
            }
        );
    }
    #[test]
    fn post_fetch_mismatch_discards_and_first_seen_persists_weak() {
        // A contradicted fingerprint must discard, never merge.
        assert!(matches!(
            route(Some("fp-a"), IdentityConfidence::Weak, Some("fp-b")),
            Routing::Mismatch { .. }
        ));
        // First sight attributes Weak; a match keeps its confidence.
        assert_eq!(
            route(None, IdentityConfidence::Unknown, Some("fp-new")),
            Routing::Attributed {
                confidence: IdentityConfidence::Weak
            }
        );
        assert_eq!(
            route(Some("fp"), IdentityConfidence::Weak, Some("fp")),
            Routing::Attributed {
                confidence: IdentityConfidence::Weak
            }
        );
    }
    #[test]
    fn local_unknown_bucket_accepts_unattributed_rows() {
        assert_eq!(
            attribute_local_import(None, "Unknown", None),
            LocalAttribution::Proceed
        );
    }
    #[test]
    fn local_rows_rejected_for_confirmed_connections() {
        assert_eq!(
            attribute_local_import(Some("fp"), "Weak", None),
            LocalAttribution::Reject
        );
        assert_eq!(
            attribute_local_import(Some("fp"), "Verified", None),
            LocalAttribution::Reject
        );
    }
    #[test]
    fn local_mismatch_rejects() {
        assert_eq!(
            attribute_local_import(Some("fp-a"), "Weak", Some("fp-b")),
            LocalAttribution::Reject
        );
        assert_eq!(
            attribute_local_import(None, "Unknown", Some("fp-b")),
            LocalAttribution::Proceed
        );
    }

    #[test]
    fn synthetic_aba_sequence_rejects_and_recovers_without_silent_merge() {
        // Step 1: Initial observation A for unconfigured account -> Weak attribution
        let step1 = route(None, IdentityConfidence::Unknown, Some("identity-A"));
        assert_eq!(
            step1,
            Routing::Attributed {
                confidence: IdentityConfidence::Weak
            }
        );

        // Step 2: Configured account A observes Identity B -> Mismatch (rejected, no silent merge)
        let step2 = route(
            Some("identity-A"),
            IdentityConfidence::Weak,
            Some("identity-B"),
        );
        assert_eq!(
            step2,
            Routing::Mismatch {
                stored: Some("identity-A".into()),
                observed: "identity-B".into(),
            }
        );

        // Step 3: Returns to Identity A -> matched attribution
        let step3 = route(
            Some("identity-A"),
            IdentityConfidence::Weak,
            Some("identity-A"),
        );
        assert_eq!(
            step3,
            Routing::Attributed {
                confidence: IdentityConfidence::Weak
            }
        );

        // Step 4: None identity cannot contaminate confirmed account
        assert_eq!(
            attribute_local_import(Some("identity-A"), "Weak", None),
            LocalAttribution::Reject
        );
    }
}
