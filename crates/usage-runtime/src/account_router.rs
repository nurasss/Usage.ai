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
}
