use super::HostError;
use async_trait::async_trait;
use std::time::Duration;

pub struct HttpJsonRequest<'a> {
    pub url: &'a str,
    pub bearer: Option<&'a [u8]>,
    pub allowed_hosts: &'a [&'a str],
    pub timeout: Duration,
    pub max_bytes: usize,
    pub user_agent: &'a str,
}

pub struct HttpJsonResponse {
    pub status: u16,
    pub retry_after_secs: Option<u64>,
    pub body: Option<serde_json::Value>,
}

/// Secret-bearing JSON GET with enforced policy:
/// HTTPS only, exact-host allowlist, redirects denied (any 3xx is an
/// error, credentials are never forwarded), per-request timeout,
/// response size limit, safe user agent, no auth data in errors/logs.
#[async_trait]
pub trait HttpHost: Send + Sync {
    async fn get_json(&self, req: HttpJsonRequest<'_>) -> Result<HttpJsonResponse, HostError>;
}

#[derive(Debug, Default)]
pub struct ReqwestHttpHost {
    max_bytes_default: usize,
    client: tokio::sync::OnceCell<reqwest::Client>,
}

impl ReqwestHttpHost {
    /// One reusable pooled client per host: connection reuse without
    /// per-request rebuilds. Redirects stay denied and the safe
    /// user agent is fixed at construction.
    async fn client(&self) -> Result<&reqwest::Client, HostError> {
        self.client
            .get_or_try_init(|| async {
                reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .user_agent(super::policy::USER_AGENT)
                    .build()
                    .map_err(|_| HostError::Unavailable)
            })
            .await
    }
}

#[async_trait]
impl HttpHost for ReqwestHttpHost {
    async fn get_json(&self, req: HttpJsonRequest<'_>) -> Result<HttpJsonResponse, HostError> {
        let url: reqwest::Url = req.url.parse().map_err(|_| HostError::Policy)?;
        if url.scheme() != "https" {
            return Err(HostError::Policy);
        }
        let host = url.host_str().unwrap_or_default();
        if !req.allowed_hosts.contains(&host) {
            return Err(HostError::Policy);
        }
        let limit = req.max_bytes.min(self.max_bytes_default).max(1024);
        let client = self.client().await?;
        let mut builder = client.get(url);
        if let Some(secret) = req.bearer {
            let token = String::from_utf8(secret.to_vec()).map_err(|_| HostError::Policy)?;
            builder = builder.bearer_auth(token);
        }
        builder = builder.header(reqwest::header::USER_AGENT, req.user_agent);
        // Cancel-safe: dropping the future drops the in-flight request.
        // The per-request timeout races the send, not the client.
        let response = tokio::time::timeout(req.timeout, builder.send())
            .await
            .map_err(|_| HostError::Timeout)?
            .map_err(|e| {
                if e.is_timeout() {
                    HostError::Timeout
                } else {
                    HostError::Unavailable
                }
            })?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(HostError::Policy);
        }
        // Header value is numeric backoff metadata, never a secret.
        let retry_after_secs = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| parse_retry_after(Some(v)));
        if let Some(len) = response.content_length() {
            if len > limit as u64 {
                return Err(HostError::Policy);
            }
        }
        if status == 401 {
            return Err(HostError::Denied);
        }
        if status == 403 {
            return Err(HostError::Forbidden);
        }
        if status == 429 {
            return Err(HostError::RateLimited(retry_after_secs));
        }
        if !(200..300).contains(&status) {
            return Err(HostError::Unavailable);
        }
        let bytes = response.bytes().await.map_err(|e| {
            if e.is_timeout() {
                HostError::Timeout
            } else {
                HostError::Unavailable
            }
        })?;
        if bytes.len() > limit {
            return Err(HostError::Policy);
        }
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| HostError::Unavailable)?;
        Ok(HttpJsonResponse {
            status,
            retry_after_secs,
            body: Some(body),
        })
    }
}

/// Extract `Retry-After` delay in seconds without logging the value.
/// Supports delta-seconds and the IMF-fixdate HTTP-date form; past
/// dates saturate to zero and absurd values are capped.
pub fn parse_retry_after(value: Option<&str>) -> Option<u64> {
    let raw = value?.trim();
    if let Ok(secs) = raw.parse::<u64>() {
        return Some(secs.min(3600));
    }
    let at: std::time::SystemTime = httpdate::parse_http_date(raw).ok()?;
    let delta = at
        .duration_since(std::time::SystemTime::now())
        .unwrap_or_default();
    Some(delta.as_secs().min(3600))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_plain_http_before_any_network() {
        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: "http://api.openai.com/v1/x",
                bearer: None,
                allowed_hosts: &["api.openai.com"],
                timeout: Duration::from_secs(5),
                max_bytes: 1024,
                user_agent: "Usage.ai-test",
            })
            .await;
        assert!(matches!(result, Err(HostError::Policy)));
    }

    #[tokio::test]
    async fn rejects_non_allowlisted_host_before_any_network() {
        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: "https://evil.example.com/v1/x",
                bearer: None,
                allowed_hosts: &["api.openai.com"],
                timeout: Duration::from_secs(5),
                max_bytes: 1024,
                user_agent: "Usage.ai-test",
            })
            .await;
        assert!(matches!(result, Err(HostError::Policy)));
    }

    #[test]
    fn retry_after_parses_digits_and_http_date() {
        assert_eq!(parse_retry_after(Some("77")), Some(77));
        assert_eq!(parse_retry_after(Some("soon")), None);
        assert_eq!(parse_retry_after(None), None);
        // Far-future HTTP-date is capped; past dates saturate to zero.
        assert_eq!(
            parse_retry_after(Some("Wed, 21 Oct 2099 07:28:00 GMT")),
            Some(3600)
        );
        assert_eq!(
            parse_retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT")),
            Some(0)
        );
    }
}
