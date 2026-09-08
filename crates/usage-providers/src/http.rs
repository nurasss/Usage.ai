use std::time::Duration;
use usage_core::ProviderError;

/// Allow-listed HTTPS fetch for secret-bearing provider requests.
///
/// Rules (Master Spec §18.6):
/// - only `https` URLs whose host exactly matches an entry of `allowed_hosts`;
/// - redirects are never followed with a secret (any 3xx is an error);
/// - TLS validation is always on (rustls + bundled WebPKI roots);
/// - no header or body is ever logged; callers map errors to safe codes.
pub async fn fetch_json(
    url: &str,
    bearer: &str,
    allowed_hosts: &[&str],
    timeout: Duration,
) -> Result<(u16, serde_json::Value), ProviderError> {
    let parsed: reqwest::Url = url
        .parse()
        .map_err(|_| ProviderError::Unavailable("invalid_provider_url".into()))?;
    if parsed.scheme() != "https" {
        return Err(ProviderError::Unavailable("insecure_provider_url".into()));
    }
    let host = parsed.host_str().unwrap_or_default();
    if !allowed_hosts.contains(&host) {
        return Err(ProviderError::Unavailable("host_not_allowlisted".into()));
    }
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ProviderError::Network("http_client_unavailable".into()))?;
    let response = client
        .get(url)
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                ProviderError::Network("source_timeout".into())
            } else if e.is_connect() {
                ProviderError::Network("connection_failed".into())
            } else {
                ProviderError::Network("request_failed".into())
            }
        })?;
    let status = response.status().as_u16();
    if (300..400).contains(&status) {
        return Err(ProviderError::Unavailable("unexpected_redirect".into()));
    }
    if status == 401 {
        return Err(ProviderError::AuthenticationRequired);
    }
    if status == 403 {
        return Err(ProviderError::InsufficientScope);
    }
    if status == 429 {
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        return Err(ProviderError::RateLimited(retry_after));
    }
    if !(200..300).contains(&status) {
        return Err(ProviderError::Unavailable(format!(
            "provider_status_{status}"
        )));
    }
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|_| ProviderError::Parse("invalid_json_response".into()))?;
    Ok((status, value))
}

/// Stable non-privileged fingerprint for an API credential.
/// Never reversible; safe for diagnostics, identity and dedup keys.
pub fn key_fingerprint(provider: &str, secret: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(provider.as_bytes());
    digest.update([0u8]);
    digest.update(secret.as_bytes());
    format!("{provider}:{}", &format!("{:x}", digest.finalize())[..16])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_non_allowlisted_host_without_network() {
        let result = fetch_json(
            "https://evil.example.com/v1/x",
            "secret",
            &["api.openai.com"],
            Duration::from_secs(5),
        )
        .await;
        assert!(matches!(result, Err(ProviderError::Unavailable(_))));
    }
    #[tokio::test]
    async fn rejects_plain_http_without_network() {
        let result = fetch_json(
            "http://api.openai.com/v1/x",
            "secret",
            &["api.openai.com"],
            Duration::from_secs(5),
        )
        .await;
        assert!(matches!(result, Err(ProviderError::Unavailable(_))));
    }
    #[test]
    fn fingerprint_is_stable_and_truncated() {
        let a = key_fingerprint("openai-api", "sk-test-1");
        let b = key_fingerprint("openai-api", "sk-test-1");
        assert_eq!(a, b);
        assert!(a.starts_with("openai-api:"));
        assert!(!a.contains("sk-test-1"));
    }
}
